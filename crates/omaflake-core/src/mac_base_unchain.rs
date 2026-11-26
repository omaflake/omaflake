// SPDX-License-Identifier: AGPL-3.0-or-later
//! An "unchained" macOS base (plan B184): a copy of a base with System Integrity Protection off, for what a normal
//! guest refuses – first of all omaflake's own camera extension before omaflake is signed with a Developer ID.
//! Nothing here is a crack: `csrutil disable` in macOS Recovery is Apple's own switch, and only a person can throw
//! it – Recovery asks in its own windows, there is no SSH there, and Virtualization.framework has no setting for it.
//!
//!   base `mac-base-<…>` (ready, read-only) → clone with a new identity `bases/mac-unchained-<…>` → started in a
//!   window; when the protection is still on: shut down → macOS Recovery in a window, where the user types
//!   `csrutil disable` and shuts down → started again and checked (`csrutil status` over SSH) → omaflake Tools with
//!   the camera extension copied to /Applications and started, the user approves the extension in System Settings
//!   → shut down → read-only.
//!
//! The state is in the VM's files, so clones keep it (checked 2026-11-26): images built from an unchained base, and
//! their VMs, are unchained. An unfinished one continues where it stopped. Never the default: an unchained base has
//! its own name prefix, which none of the searches for "the base of a credential" look at.
//!
//! The same for an image (`unchain_image`): the unchained copy of exactly that image, without building anything –
//! what the image holds (its system, language and keyboard, the agent) stays. The steps are the user's again for
//! each image; from an unchained base they are done once for every image built from it.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{IoContext, Result, codes};
use crate::ipsw_downloader::KeepAwake;
use crate::mac_base_installer::{BaseOutcome, BaseRecord, MacBaseInstaller};
use crate::mac_image_builder::BuildVm;
use crate::mac_vm::{DhcpLeases, MacVmBundle, RunnerClient};
use crate::qemu_runtime::tail;
use crate::shell::{RunOptions, Shell, ShellResult};
use crate::sockets::{self, Ports};
use crate::trash::Disposal;
use crate::vm_err;

/// What `systemextensionsctl list` calls omaflake's camera extension (Sources/omaflake-camera)
pub const CAMERA_EXTENSION: &str = "app.omaflake.guest.camera";
/// How the job of `macos.unchainBase` starts its title (the base's name follows) – its row is told by it
pub const JOB_TITLE: &str = "Make the unchained copy of";
/// The app's folder name, on the Mac and in the guest's /Applications
pub const TOOLS_APP: &str = "omaflake Tools.app";

/// What to unchain
#[derive(Debug, Clone)]
pub struct UnchainRequest {
    /// A ready base (`bases/mac-base-…`)
    pub source: PathBuf,
    /// Its guest user's password (from its credential) – for the shutdowns
    pub password: String,
    /// "omaflake Tools.app" with the camera extension (`MacBaseInstaller::unchained_tools`); None: the base gets
    /// no Tools – the camera extension can be installed later
    pub tools: Option<PathBuf>,
    /// The keyboard of this computer (an XKB name, "de"): a base has the US keyboard, and the camera extension's
    /// approval asks for the password – typed on the keyboard at hand. None: left as it is
    pub keyboard: Option<String>,
}

/// macOS keyboard layouts by XKB name – the table of nix/modules/darwin-guest.nix (`knownLayouts`), which images
/// get theirs from; an unknown name leaves the base's keyboard as it is
pub(crate) fn mac_layout(xkb: &str) -> Option<(i64, &'static str)> {
    Some(match xkb {
        "us" => (252, "ABC"),
        "gb" => (2, "British"),
        "de" => (3, "German"),
        "ch" => (19, "Swiss German"),
        "at" => (92, "Austrian"),
        "fr" => (1, "French"),
        "es" => (87, "Spanish - ISO"),
        "it" => (4, "Italian"),
        "nl" => (26, "Dutch"),
        "se" => (224, "Swedish"),
        "no" => (12, "Norwegian"),
        "dk" => (9, "Danish"),
        "fi" => (17, "Finnish"),
        "pl" => (30788, "Polish Pro"),
        "pt" => (10, "Portuguese"),
        "cz" => (-14193, "Czech"),
        "ru" => (19458, "RussianWin"),
        "ua" => (-2354, "Ukrainian-PC"),
        "gr" => (-18944, "Greek"),
        _ => return None,
    })
}

/// The shell commands that make `layout` the guest user's only keyboard, from its next login
pub(crate) fn keyboard_commands((id, name): (i64, &str)) -> String {
    let source = format!(
        "<dict><key>InputSourceKind</key><string>Keyboard Layout</string><key>KeyboardLayout ID</key>\
         <integer>{id}</integer><key>KeyboardLayout Name</key><string>{name}</string></dict>"
    );
    ["AppleEnabledInputSources", "AppleSelectedInputSources", "AppleInputSourceHistory"]
        .iter()
        .map(|key| format!("defaults write com.apple.HIToolbox {key} -array '{source}'"))
        .chain([format!(
            "defaults write com.apple.HIToolbox AppleCurrentKeyboardLayoutInputSourceID -string 'com.apple.keylayout.{name}'"
        )])
        .collect::<Vec<_>>()
        .join(" && ")
}

type Log<'a> = &'a (dyn Fn(&str) + Send + Sync + 'a);
/// Where the copy is: step `n` of `STEPS` and what happens there – for a job's progress (plan B184)
pub type Steps<'a> = &'a (dyn Fn(u32, &str) + Send + Sync + 'a);

/// How many steps an unchained copy has
pub const STEPS: u32 = 6;
pub const STEP_COPY: &str = "Copying";
pub const STEP_START: &str = "Starting the copy";
pub const STEP_RECOVERY: &str = "macOS Recovery: switching System Integrity Protection off";
pub const STEP_RECOVERY_BY_HAND: &str = "Your turn: macOS Recovery in the VM's window";
pub const STEP_CHECK: &str = "Checking";
pub const STEP_APPROVE: &str = "Your turn: approve the camera extension in the VM's window";
pub const STEP_SHUT_DOWN: &str = "Shutting down";
/// Every step's title – each has German (a test checks)
pub const STEP_TITLES: [&str; 7] =
    [STEP_COPY, STEP_START, STEP_RECOVERY, STEP_RECOVERY_BY_HAND, STEP_CHECK, STEP_APPROVE, STEP_SHUT_DOWN];

/// How Recovery ended
enum RecoveryEnd {
    /// The VM is off – after `halt`, or because the user shut it down; the next start says whether it worked
    Off,
    /// The automatic steps found no way on: the user does it in the window
    GaveUp(String),
}

impl MacBaseInstaller {
    /// Folder names of unchained bases start with this – not with "mac-base-", so no search for a credential's base
    /// finds one by accident
    pub const UNCHAINED_PREFIX: &'static str = "mac-unchained-";
    /// How long the user has in Recovery, and to approve the camera extension
    const PATIENCE: Duration = Duration::from_secs(30 * 60);

    pub fn is_unchained(dir: &Path) -> bool {
        dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with(Self::UNCHAINED_PREFIX))
    }

    /// "mac-base-26A434-work" → `bases/mac-unchained-26A434-work`
    pub fn unchained_for(&self, source: &Path) -> PathBuf {
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let rest = name.strip_prefix("mac-base-").unwrap_or(&name);
        self.paths.bases().join(format!("{}{rest}", Self::UNCHAINED_PREFIX))
    }

    /// The newest ready unchained base of `credential` (its ID) – unchained macOS images with that credential are
    /// built from it. Sorted like the bases: by macOS build, then by name
    pub fn newest_unchained_for(paths: &crate::paths::Paths, credential: Option<&str>) -> Option<PathBuf> {
        let mut names: Vec<String> = fs::read_dir(paths.bases())
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.retain(|n| n.starts_with(Self::UNCHAINED_PREFIX));
        let order = |n: &str| {
            let rest = n.strip_prefix(Self::UNCHAINED_PREFIX).unwrap_or(n).to_string();
            (rest.split('-').next().unwrap_or_default().to_string(), rest)
        };
        names.sort_by_key(|n| order(n));
        names.iter().rev().map(|n| paths.bases().join(n)).find(|dir| {
            Self::is_ready(dir)
                && (credential.is_none() || BaseRecord::of(dir).map(|r| r.credential).as_deref() == credential)
        })
    }

    /// omaflake Tools with the camera extension, as this omaflake ships it: `OMAFLAKE_TOOLS` (development), else
    /// `Contents/Resources/Guest/unchained/` of the app this `omaf` is in, else – a build signed with a Developer ID
    /// has one Tools for every guest – `Contents/Resources/Guest/`; in a checkout `build/unchained/`
    pub fn unchained_tools() -> Option<PathBuf> {
        if let Some(custom) = std::env::var_os("OMAFLAKE_TOOLS").map(PathBuf::from) {
            return custom.exists().then_some(custom);
        }
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok())?;
        Self::unchained_tools_from(&exe)
    }

    /// omaflake Tools for every macOS image (2026-11-27, the maintainer: "omaflake Tools immer installieren"): the
    /// build for its kind of base – with the camera extension for an unchained one (`unchained_tools`), else the
    /// plain one (`Contents/Resources/Guest/`, in a checkout `build/`; `OMAFLAKE_TOOLS_PLAIN` in development)
    pub fn guest_tools(unchained: bool) -> Option<PathBuf> {
        if unchained {
            return Self::unchained_tools();
        }
        if let Some(custom) = std::env::var_os("OMAFLAKE_TOOLS_PLAIN").map(PathBuf::from) {
            return custom.exists().then_some(custom);
        }
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok())?;
        Self::plain_tools_from(&exe)
    }

    pub(crate) fn plain_tools_from(exe: &Path) -> Option<PathBuf> {
        let dir = exe.parent()?;
        [dir.join("../Resources/Guest").join(TOOLS_APP), dir.join("../../build").join(TOOLS_APP)]
            .into_iter()
            .find(|app| app.join("Contents/Info.plist").is_file())
    }

    pub(crate) fn unchained_tools_from(exe: &Path) -> Option<PathBuf> {
        let dir = exe.parent()?;
        let has_extension = |app: &PathBuf| app.join("Contents/Library/SystemExtensions").is_dir();
        [
            dir.join("../Resources/Guest/unchained").join(TOOLS_APP),
            dir.join("../Resources/Guest").join(TOOLS_APP),
            // target/<profile>/omaf in a checkout, CARGO_TARGET_DIR aside: the bundle scripts' output
            dir.join("../../build/unchained").join(TOOLS_APP),
        ]
        .into_iter()
        .find(has_extension)
    }

    /// Makes the unchained copy of a base (keeping the Mac awake). `log` gets what happens and what the user is to do
    pub fn unchain(&self, r: &UnchainRequest, log: Log<'_>, step: Steps<'_>) -> Result<BaseOutcome> {
        KeepAwake::during("Making an unchained macOS base", || self.unchain_awake(r, log, step))
    }

    fn unchain_awake(&self, r: &UnchainRequest, log: Log<'_>, step: Steps<'_>) -> Result<BaseOutcome> {
        let source_name = name_of(&r.source);
        if Self::is_unchained(&r.source) {
            return Err(vm_err!("{source_name} is unchained already"));
        }
        if !Self::is_ready(&r.source) {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "{source_name} is unfinished – finish it first: omaf macos install"
            ));
        }
        let Some(record) = BaseRecord::of(&r.source) else {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "{source_name} has no credential – assign one first: omaf macos assign"
            ));
        };
        self.vz.check_entitlement()?;
        let dir = self.unchained_for(&r.source);
        let name = name_of(&dir);
        if Self::is_ready(&dir) {
            log(&format!("{name} exists already – nothing to do"));
            return Ok(BaseOutcome::Existing(dir));
        }
        step(1, STEP_COPY);
        if dir.exists() {
            log(&format!("{name} is unfinished – continuing"));
        } else {
            Shell::check(
                &self.vz.runner,
                &["clone".to_string(), path_string(&r.source), path_string(&dir)],
                &RunOptions::default(),
            )?;
            log(&format!("{name} cloned from {source_name}"));
            BaseRecord { unchained: true, ..record.clone() }.write(&dir)?;
        }

        let session = Session::new(self, &dir, &record.user, &r.password)?;
        let keyboard = r.keyboard.as_deref().and_then(mac_layout);
        let outcome = self.unchain_steps(&session, r.tools.as_deref(), keyboard, log, step);
        if let Err(e) = outcome {
            session.force_off();
            log(&format!("{name} is left as it is – the same command continues: {}", dir.display()));
            return Err(e);
        }
        let _ = Disposal::delete(&[&session.socket, &session.pid_file]);
        let _ = fs::write(dir.join(Self::READY_MARKER), format!("{}\n", record.user));
        Shell::check(
            &crate::platform::host_tool("/bin/chmod"),
            &["-R", "a-w", &path_string(&dir)],
            &RunOptions::default(),
        )?;
        log(&format!("Unchained base ready: {} (read-only)", dir.display()));
        Ok(BaseOutcome::Created { dir, ready: true })
    }

    /// The unchained copy of an image (keeping the Mac awake): `images/<recipe>@<words of its own>`, with
    /// `unchained` in its image.json – never its recipe's current image. `user` and `password`: the image's guest
    /// user (its credential's). An existing copy is returned as it is; an unfinished one continues
    pub fn unchain_image(
        &self,
        image: &crate::image_store::Image,
        user: &str,
        password: &str,
        tools: Option<&Path>,
        log: Log<'_>,
        step: Steps<'_>,
    ) -> Result<crate::image_store::Image> {
        KeepAwake::during("Making an unchained macOS image", || {
            self.unchain_image_awake(image, user, password, tools, log, step)
        })
    }

    /// The name of an image's unchained copy – the one a build of the same system from an unchained base gets
    pub fn unchained_image_name(image: &crate::image_store::Image) -> String {
        let system = image.info.store_path.clone().unwrap_or_else(|| image.name.clone());
        crate::mac_image_builder::MacImageBuilder::image_name(&image.info.recipe, &system, true)
    }

    fn unchain_image_awake(
        &self,
        image: &crate::image_store::Image,
        user: &str,
        password: &str,
        tools: Option<&Path>,
        log: Log<'_>,
        step: Steps<'_>,
    ) -> Result<crate::image_store::Image> {
        use crate::image_store::ImageStore;
        if image.kind() != crate::models::GuestKind::Macos {
            return Err(vm_err!("{} is not a macOS image", image.name));
        }
        if image.info.unchained {
            return Err(vm_err!("{} is unchained already", image.name));
        }
        self.vz.check_entitlement()?;
        let images = ImageStore::new(self.paths.clone());
        let name = Self::unchained_image_name(image);
        let target = self.paths.image_dir(&name);
        if target.exists() {
            log(&format!("{name} exists already – nothing to do"));
            return images.image(&name);
        }
        // Built like an image: in a folder the lists do not show, renamed once it is whole
        let work = self.paths.images().join(format!(".build-{name}"));
        step(1, STEP_COPY);
        if work.exists() {
            log(&format!("The unchained copy of {} is unfinished – continuing", image.name));
        } else {
            Shell::check(
                &self.vz.runner,
                &["clone".to_string(), path_string(&image.path), path_string(&work)],
                &RunOptions::default(),
            )?;
            log(&format!("{} cloned", image.name));
        }
        let session = Session::new(self, &work, user, password)?;
        // The image's keyboard is its locale's: nothing to set
        if let Err(e) = self.unchain_steps(&session, tools, None, log, step) {
            session.force_off();
            log(&format!("The copy is left as it is – the same action continues: {}", work.display()));
            return Err(e);
        }
        let _ = Disposal::delete(&[&session.socket, &session.pid_file]);
        let mut info = image.info.clone();
        info.unchained = true;
        info.created = crate::json::now();
        ImageStore::write_info(&info, &work)?;
        fs::rename(&work, &target).with_path(&target)?;
        Shell::check(
            &crate::platform::host_tool("/bin/chmod"),
            &["-R", "a-w", &path_string(&target)],
            &RunOptions::default(),
        )?;
        log(&format!("Unchained image ready: {name}"));
        images.image(&name)
    }

    fn unchain_steps(
        &self,
        s: &Session<'_>,
        tools: Option<&Path>,
        keyboard: Option<(i64, &'static str)>,
        log: Log<'_>,
        step: Steps<'_>,
    ) -> Result<()> {
        let name = name_of(&s.dir);
        step(2, STEP_START);
        log(&format!("Starting {name} (log: {})", s.log_file.display()));
        let mut ip = s.boot(false)?;
        // This computer's keyboard for a base (it has the US one): from the next login, so before the restarts
        let mut restart_for_keyboard = false;
        if let Some(layout) = keyboard {
            let current = s
                .ssh(&ip, "defaults read com.apple.HIToolbox AppleCurrentKeyboardLayoutInputSourceID 2>/dev/null")
                .map(|r| r.stdout)
                .unwrap_or_default();
            if current.trim() != format!("com.apple.keylayout.{}", layout.1) {
                let set = s.ssh(&ip, &keyboard_commands(layout))?;
                if set.status == 0 {
                    log(&format!("Keyboard: {} (this computer's), from its next start", layout.1));
                    restart_for_keyboard = true;
                } else {
                    log(&format!("The keyboard could not be set ({}) – it stays as it is", set.stderr.trim()));
                }
            }
        }
        if !Self::protection_is_off(&s.ssh(&ip, "csrutil status")?.stdout) {
            restart_for_keyboard = false; // Recovery restarts it anyway
            // Recovery in English with a US keyboard, whatever the copy's language: no language to choose, keys
            // where the steps expect them (`prev-lang:kbd`, put back after)
            let shown = s.ssh(&ip, "/usr/sbin/nvram prev-lang:kbd 2>/dev/null").map(|r| r.stdout).unwrap_or_default();
            let original = crate::recovery_screen::nvram_value(&shown);
            let english = s.sudo(&ip, "/usr/sbin/nvram prev-lang:kbd=en-US:0").is_ok_and(|r| r.status == 0);
            log(&if english {
                format!(
                    "Recovery's language: English with a US keyboard (was {})",
                    original.as_deref().unwrap_or("not set")
                )
            } else {
                "Recovery's language could not be set – it comes up as the copy has it".to_string()
            });
            log("System Integrity Protection is on – shutting down for macOS Recovery");
            s.shut_down(&ip)?;
            step(3, STEP_RECOVERY);
            match s.recovery_automatic(log)? {
                RecoveryEnd::Off => log("Recovery is off"),
                RecoveryEnd::GaveUp(why) => {
                    step(3, STEP_RECOVERY_BY_HAND);
                    log(&format!("The automatic steps stopped ({why}) – your turn, in the VM's window:"));
                    log("  1. Menu Utilities → Terminal");
                    log("  2. Type: csrutil disable   – answer y, then the user's name and password");
                    log("  3. Type: halt   (or Apple menu → Shut Down)");
                    s.show();
                    s.wait_until_off(Self::PATIENCE)?;
                }
            }
            step(4, STEP_CHECK);
            log(&format!("Starting {name} again"));
            ip = s.boot(false)?;
            if english {
                let put_back = match &original {
                    Some(value) => format!("/usr/sbin/nvram prev-lang:kbd={value}"),
                    None => "/usr/sbin/nvram -d prev-lang:kbd".to_string(),
                };
                if !s.sudo(&ip, &put_back).is_ok_and(|r| r.status == 0) {
                    log("Recovery's language could not be put back – it stays English");
                }
            }
            let status = s.ssh(&ip, "csrutil status")?.stdout;
            if !Self::protection_is_off(&status) {
                s.shut_down(&ip)?;
                return Err(vm_err!(
                    "System Integrity Protection is still on ({}) – run the command again and type csrutil disable \
                     in Recovery's Terminal",
                    status.trim()
                ));
            }
        } else {
            step(4, STEP_CHECK);
        }
        log("System Integrity Protection is off");
        if restart_for_keyboard {
            log("Restarting once for the keyboard");
            s.shut_down(&ip)?;
            ip = s.boot(false)?;
        }
        match tools {
            Some(tools) => {
                step(5, STEP_APPROVE);
                self.install_tools(s, &ip, tools, log)?
            }
            None => log("No omaflake Tools with the camera extension in this omaflake – the copy gets none"),
        }
        step(6, STEP_SHUT_DOWN);
        log("Shutting down");
        s.shut_down(&ip)
    }

    /// "System Integrity Protection status: disabled." – also with the custom configuration Apple prints when
    /// single protections were switched on again
    pub(crate) fn protection_is_off(status: &str) -> bool {
        status.lines().any(|l| l.contains("System Integrity Protection status:") && l.contains("disabled"))
    }

    /// The extension's state in `systemextensionsctl list` ("activated enabled"), None when it is not listed
    pub(crate) fn camera_extension_state(list: &str) -> Option<String> {
        let line = list.lines().rfind(|l| l.contains(CAMERA_EXTENSION))?;
        Some(line.rsplit_once('[')?.1.trim_end().trim_end_matches(']').to_string())
    }

    /// Copies omaflake Tools to the guest's /Applications, starts it (it asks macOS to activate the camera
    /// extension) and waits for the user's approval in the VM's window
    fn install_tools(&self, s: &Session<'_>, ip: &str, tools: &Path, log: Log<'_>) -> Result<()> {
        log(&format!("Copying {TOOLS_APP} into the guest"));
        // An unfinished run's copy goes aside in the guest, never deleted
        s.ssh(
            ip,
            &format!(
                "[ ! -e '/Applications/{TOOLS_APP}' ] || {{ mkdir -p ~/_to_delete && mv '/Applications/{TOOLS_APP}' \
                 ~/_to_delete/omaflake-Tools-$(date +%s).app; }}"
            ),
        )?;
        let mut args = vec!["-r".to_string()];
        args.extend(BuildVm::ssh_options());
        args.extend([path_string(tools), format!("{}@{ip}:/Applications/", s.user)]);
        let copied = Shell::run(&crate::platform::host_tool("/usr/bin/scp"), &args, &RunOptions::default())?;
        if copied.status != 0 {
            return Err(vm_err!("Could not copy {TOOLS_APP} into the guest: {}", copied.stderr.trim()));
        }
        let app = format!("/Applications/{TOOLS_APP}");
        s.ssh(
            ip,
            &format!("mkdir -p ~/Library/Logs/omaflake && : > ~/Library/Logs/omaflake/tools.log && open -n '{app}'"),
        )?;
        s.show();
        log("Approve the camera extension in the VM's window when macOS asks:");
        log("  System Settings → General → Login Items & Extensions → Camera Extensions → switch it on");
        let deadline = Instant::now() + Self::PATIENCE;
        loop {
            let answer = s.ssh(ip, "cat ~/Library/Logs/omaflake/tools.log").map(|r| r.stdout).unwrap_or_default();
            if answer.contains("FINISHED") {
                break;
            }
            if let Some(failed) = answer.lines().find(|l| l.contains("FAILED") || l.contains("TIMEOUT")) {
                return Err(vm_err!("The camera extension was not activated: {}", failed.trim()));
            }
            if Instant::now() >= deadline {
                return Err(vm_err!("The camera extension was not approved in time – run the command again"));
            }
            s.still_running()?;
            std::thread::sleep(Duration::from_secs(3));
        }
        let list = s.ssh(ip, "systemextensionsctl list")?.stdout;
        match Self::camera_extension_state(&list).as_deref() {
            Some("activated enabled") => {
                log("The camera extension is active: apps in VMs of this base get \"omaflake Camera\"");
                Ok(())
            }
            state => {
                Err(vm_err!("The camera extension is {} – expected activated enabled", state.unwrap_or("not listed")))
            }
        }
    }
}

fn name_of(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The base's VM across its starts: one runner at a time, its output appended to `logs/unchain-<base>.log`
struct Session<'a> {
    installer: &'a MacBaseInstaller,
    dir: PathBuf,
    user: String,
    password: String,
    socket: PathBuf,
    pid_file: PathBuf,
    log_file: PathBuf,
    mac: String,
    runner: std::sync::Mutex<Option<Child>>,
    /// How the runner sends Shift and the others (`Robot.Modifiers`): the one the Terminal probe found to work
    modifiers: std::sync::Mutex<&'static str>,
}

impl<'a> Session<'a> {
    fn new(installer: &'a MacBaseInstaller, dir: &Path, user: &str, password: &str) -> Result<Self> {
        let logs = installer.paths.logs();
        fs::create_dir_all(&logs).with_path(&logs)?;
        Ok(Session {
            installer,
            dir: dir.to_path_buf(),
            user: user.into(),
            password: password.into(),
            socket: dir.join("runner.sock"),
            pid_file: dir.join("runner.pid"),
            log_file: logs.join(format!("unchain-{}.log", name_of(dir))),
            mac: MacVmBundle::new(dir).load_config()?.mac_address,
            runner: std::sync::Mutex::new(None),
            modifiers: std::sync::Mutex::new("device"),
        })
    }

    fn spawn(&self, recovery: bool) -> Result<()> {
        let out = OpenOptions::new().create(true).append(true).open(&self.log_file).with_path(&self.log_file)?;
        let mut args: Vec<String> = vec![
            "run".into(),
            path_string(&self.dir),
            "--socket".into(),
            path_string(&self.socket),
            "--pid-file".into(),
            path_string(&self.pid_file),
            // A window nobody sees: the automatic steps type into it and read its screen; it is shown when the
            // user is needed (Recovery by hand, the extension's approval)
            "--quiet-window".into(),
        ];
        if recovery {
            args.push("--recovery".into());
        }
        let child = Command::new(&self.installer.vz.runner)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(out.try_clone().with_path(&self.log_file)?))
            .stderr(Stdio::from(out))
            .spawn()
            .map_err(|e| vm_err!("Cannot run {}: {}", self.installer.vz.runner, crate::error::os_text(&e)))?;
        *self.runner.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        Ok(())
    }

    /// The runner has not exited
    fn running(&self) -> bool {
        self.runner.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    fn still_running(&self) -> Result<()> {
        if self.running() {
            return Ok(());
        }
        Err(vm_err!("The VM went off (omaf-runner exited):\n{}", tail(&self.log_file, 3)))
    }

    /// Starts the VM and waits for its SSH server; its address
    fn boot(&self, recovery: bool) -> Result<String> {
        self.spawn(recovery)?;
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            self.still_running()?;
            if let Some(a) = DhcpLeases::ip(&self.mac)
                && Ports::ssh_banner(&a, 22, sockets::DEFAULT_TIMEOUT).is_some()
            {
                return Ok(a);
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        Err(vm_err!("No SSH from {} (log: {})", name_of(&self.dir), self.log_file.display()))
    }

    /// Waits until the VM is off (the user shut it down)
    fn wait_until_off(&self, patience: Duration) -> Result<()> {
        let deadline = Instant::now() + patience;
        while self.running() {
            if Instant::now() >= deadline {
                self.force_off();
                return Err(vm_err!("macOS Recovery was not shut down in time – run the command again"));
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        Ok(())
    }

    /// The window becomes visible (the user is needed)
    fn show(&self) {
        let _ = RunnerClient::send("show", &self.socket, RunnerClient::DEFAULT_TIMEOUT);
    }

    fn robot(&self, command: &str, args: serde_json::Value, timeout: u64) -> Result<serde_json::Value> {
        RunnerClient::request(
            &self.socket,
            &serde_json::json!({ "command": command, "args": args }),
            Duration::from_secs(timeout),
        )
    }

    /// The text on the screen; Err while there is no picture
    fn screen(&self) -> Result<Vec<crate::recovery_screen::Line>> {
        let answer = self.robot("screen", serde_json::json!({}), 90)?;
        Ok(serde_json::from_value(answer["lines"].clone()).unwrap_or_default())
    }

    fn click(&self, (x, y): crate::recovery_screen::Point) {
        let _ = self.robot("click", serde_json::json!({ "x": x, "y": y }), 5);
        std::thread::sleep(Duration::from_millis(1500));
    }

    fn modifiers(&self) -> &'static str {
        *self.modifiers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn key(&self, key: &str) {
        let _ = self.robot("key", serde_json::json!({ "key": key, "modifiers": self.modifiers() }), 5);
        std::thread::sleep(Duration::from_secs(1));
    }

    fn type_text(&self, text: &str, layout: crate::recovery_screen::Layout) -> Result<()> {
        let args = serde_json::json!({ "text": text, "layout": layout.as_str(), "modifiers": self.modifiers() });
        self.robot("type", args, 90)?;
        std::thread::sleep(Duration::from_millis(2500));
        Ok(())
    }

    /// Starts macOS Recovery and switches System Integrity Protection off without hands (plan B184): reads the
    /// screen, clicks and types – startup options → Options → (a language, a user) → the utilities → Terminal →
    /// `csrutil disable` and what it asks → `halt`. Gives up – the window shown, the user goes on – when it finds no
    /// way on within a few minutes; the user may also take over any time
    fn recovery_automatic(&self, log: Log<'_>) -> Result<RecoveryEnd> {
        use crate::recovery_screen::{Layout, Screen, classify};
        self.spawn(true)?;
        let deadline = Instant::now() + Duration::from_secs(8 * 60);
        let mut layout = Layout::Us;
        let (mut options_clicked, mut user_clicked, mut password_typed) = (false, false, false);
        let (mut shortcut_tries, mut menu_open, mut shown) = (0, false, false);
        let mut utilities_seen: Option<Vec<String>> = None;
        let mut no_picture_since: Option<Instant> = None;
        std::thread::sleep(Duration::from_secs(10));
        loop {
            if !self.running() {
                return Ok(RecoveryEnd::Off);
            }
            if Instant::now() >= deadline {
                return Ok(RecoveryEnd::GaveUp("no way on within 8 minutes".into()));
            }
            let lines = match self.screen() {
                Ok(lines) if !lines.is_empty() => {
                    no_picture_since = None;
                    lines
                }
                other => {
                    // No picture (or nothing on it): the quiet window may not be drawn – shown once, then on
                    let since = *no_picture_since.get_or_insert_with(Instant::now);
                    if !shown && since.elapsed() > Duration::from_secs(45) {
                        let why = other.err().map(|e| format!(" ({})", e.message())).unwrap_or_default();
                        log(&format!("No picture of Recovery yet{why} – showing the window"));
                        self.show();
                        shown = true;
                    } else if since.elapsed() > Duration::from_secs(180) {
                        return Ok(RecoveryEnd::GaveUp("no picture of the screen".into()));
                    }
                    std::thread::sleep(Duration::from_secs(4));
                    continue;
                }
            };
            match classify(&lines, &self.user, menu_open) {
                Screen::StartupOptions { options, continue_at, layout: seen } => {
                    if let Some(l) = seen {
                        layout = l;
                    }
                    match continue_at {
                        Some(at) if options_clicked => {
                            log(&format!("Recovery: startup options → Options (keyboard {})", layout.as_str()));
                            self.click(at);
                            std::thread::sleep(Duration::from_secs(10));
                        }
                        _ => {
                            self.click(options);
                            options_clicked = true;
                        }
                    }
                }
                Screen::Language { pick } => {
                    log("Recovery: choosing English");
                    if let Some(at) = pick {
                        self.click(at);
                    }
                    layout = Layout::Us;
                    self.key("return");
                    std::thread::sleep(Duration::from_secs(5));
                }
                Screen::User { user } => {
                    if let (Some(at), false) = (user, user_clicked) {
                        log("Recovery: choosing the user");
                        self.click(at);
                        user_clicked = true;
                    } else if !password_typed {
                        log("Recovery: typing the user's password");
                        if self.type_text(&format!("{}\n", self.password), layout).is_err() {
                            return Ok(RecoveryEnd::GaveUp("the password cannot be typed on this keyboard".into()));
                        }
                        password_typed = true;
                        std::thread::sleep(Duration::from_secs(5));
                    } else {
                        std::thread::sleep(Duration::from_secs(4));
                    }
                }
                Screen::Utilities { menu } => {
                    // The shortcut once the window looked the same twice (too early, it does nothing), then the menu
                    let seen: Vec<String> = lines.iter().map(|l| l.text.clone()).collect();
                    if utilities_seen.as_ref() != Some(&seen) {
                        utilities_seen = Some(seen);
                        std::thread::sleep(Duration::from_secs(3));
                    } else if shortcut_tries < 2 {
                        log("Recovery: Terminal (⇧⌘T)");
                        self.key("cmd+shift+t");
                        shortcut_tries += 1;
                        std::thread::sleep(Duration::from_secs(5));
                    } else if let Some(at) = menu {
                        log("Recovery: Terminal from the Utilities menu");
                        self.click(at);
                        menu_open = true;
                    } else {
                        std::thread::sleep(Duration::from_secs(4));
                    }
                }
                Screen::Menu { terminal } => {
                    self.click(terminal);
                    menu_open = false;
                    std::thread::sleep(Duration::from_secs(4));
                }
                Screen::Terminal { .. } => return self.in_terminal(layout, log),
                Screen::Unknown => std::thread::sleep(Duration::from_secs(4)),
            }
        }
    }

    /// In Recovery's Terminal (a root shell): `csrutil disable` with its questions answered, then `halt`
    fn in_terminal(&self, layout: crate::recovery_screen::Layout, log: Log<'_>) -> Result<RecoveryEnd> {
        use crate::recovery_screen::{Prompt, Screen, classify, csrutil_prompt, protection_off};
        let terminal = || -> Vec<String> {
            match self.screen().map(|lines| classify(&lines, &self.user, false)) {
                Ok(Screen::Terminal { text }) => text,
                _ => vec![],
            }
        };
        log("Recovery: Terminal is open");
        // Does Shift reach the guest? (A password with capitals or symbols needs it; in the first tries on a Mac it
        // did not – "Authentication failed".) The first way of sending it that types SHIFTOK is kept
        let mut works = false;
        for mode in ["device", "plain", "keys"] {
            *self.modifiers.lock().unwrap_or_else(|e| e.into_inner()) = mode;
            self.type_text("echo SHIFTOK\n", layout)?;
            let text = terminal();
            if crate::recovery_screen::shift_works(&text) {
                log(&format!("Recovery: Shift reaches the VM (modifiers {mode})"));
                works = true;
                break;
            }
            log(&format!("Recovery: Shift did not arrive with modifiers {mode}"));
        }
        if !works && self.password.chars().any(|c| crate::recovery_screen::needs_shift(c, layout)) {
            return Ok(RecoveryEnd::GaveUp(
                "Shift does not reach the VM, and the password needs it (capitals or symbols)".into(),
            ));
        }
        self.type_text("csrutil status\n", layout)?;
        if protection_off(&terminal()) != Some(true) {
            log("Recovery: csrutil disable");
            self.type_text("csrutil disable\n", layout)?;
            let (mut yes, mut user, mut password) = (false, false, false);
            let mut done = false;
            for _ in 0..15 {
                std::thread::sleep(Duration::from_secs(2));
                let text = terminal();
                match csrutil_prompt(&text) {
                    Prompt::Done => {
                        done = true;
                        break;
                    }
                    Prompt::Failed => {
                        let said = text.last().cloned().unwrap_or_default();
                        return Ok(RecoveryEnd::GaveUp(format!("csrutil said: {said}")));
                    }
                    Prompt::YesNo if !yes => {
                        yes = true;
                        self.type_text("y\n", layout)?;
                    }
                    Prompt::User if !user => {
                        user = true;
                        self.type_text(&format!("{}\n", self.user), layout)?;
                    }
                    Prompt::Password if !password => {
                        password = true;
                        if self.type_text(&format!("{}\n", self.password), layout).is_err() {
                            return Ok(RecoveryEnd::GaveUp("the password cannot be typed on this keyboard".into()));
                        }
                    }
                    _ => {}
                }
            }
            if !done {
                // It may have said nothing to read: the status says
                self.type_text("csrutil status\n", layout)?;
                if protection_off(&terminal()) != Some(true) {
                    return Ok(RecoveryEnd::GaveUp("csrutil disable did not finish".into()));
                }
            }
        }
        log("Recovery: System Integrity Protection is off – halt");
        self.type_text("halt\n", layout)?;
        let off = Instant::now() + Duration::from_secs(120);
        while self.running() && Instant::now() < off {
            std::thread::sleep(Duration::from_secs(2));
        }
        if self.running() {
            return Ok(RecoveryEnd::GaveUp("the VM did not go off after halt".into()));
        }
        Ok(RecoveryEnd::Off)
    }

    /// A command as root in the guest: sudo gets the password on stdin
    fn sudo(&self, ip: &str, command: &str) -> Result<ShellResult> {
        let mut args = BuildVm::ssh_options();
        args.extend([format!("{}@{ip}", self.user), format!("sudo -S -p '' {command}")]);
        let input = format!("{}\n", self.password);
        Shell::run(
            &crate::platform::host_tool("/usr/bin/ssh"),
            &args,
            &RunOptions { input: Some(input.as_bytes()), ..Default::default() },
        )
    }

    /// A command in the guest as its user, with omaflake's key
    fn ssh(&self, ip: &str, command: &str) -> Result<ShellResult> {
        let mut args = BuildVm::ssh_options();
        args.extend([format!("{}@{ip}", self.user), command.to_string()]);
        Shell::run(&crate::platform::host_tool("/usr/bin/ssh"), &args, &RunOptions::default())
    }

    /// `sudo shutdown` with the password on stdin, then waits for the runner to end
    fn shut_down(&self, ip: &str) -> Result<()> {
        let mut args = BuildVm::ssh_options();
        args.extend([format!("{}@{ip}", self.user), "sudo -S -p '' /sbin/shutdown -h now".to_string()]);
        let input = format!("{}\n", self.password);
        let _ = Shell::run(
            &crate::platform::host_tool("/usr/bin/ssh"),
            &args,
            &RunOptions { input: Some(input.as_bytes()), ..Default::default() },
        );
        let off = Instant::now() + Duration::from_secs(180);
        while self.running() {
            if Instant::now() >= off {
                return Err(vm_err!(
                    codes::AUTH,
                    "{} did not shut down – is the credential's password the guest user's?",
                    name_of(&self.dir)
                ));
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        Ok(())
    }

    /// Ends a runner that still runs: its `kill`, else SIGTERM
    fn force_off(&self) {
        let mut guard = self.runner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(child) = guard.as_mut() else { return };
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        if RunnerClient::send("kill", &self.socket, RunnerClient::DEFAULT_TIMEOUT).is_err() {
            crate::spawn::send_signal(child.id() as crate::spawn::Pid, libc::SIGTERM);
        }
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance_store::InstanceStore;
    use crate::paths::Paths;
    use crate::paths::tests::temp_paths;
    use crate::vz_backend::VzBackend;

    fn installer(p: &Paths) -> MacBaseInstaller {
        MacBaseInstaller::new(
            p.clone(),
            VzBackend::new(p.clone(), InstanceStore::new(p.clone(), "/usr/bin/false"), "/usr/bin/false"),
        )
    }

    fn base(p: &Paths, name: &str, credential: &str, ready: bool) -> PathBuf {
        let dir = p.bases().join(name);
        fs::create_dir_all(&dir).unwrap();
        BaseRecord {
            credential: credential.into(),
            credential_name: "Work".into(),
            user: "alice".into(),
            own_key: true,
            version: None,
            unchained: MacBaseInstaller::is_unchained(&dir),
        }
        .write(&dir)
        .unwrap();
        if ready {
            fs::write(dir.join(MacBaseInstaller::READY_MARKER), "alice\n").unwrap();
        }
        dir
    }

    #[test]
    fn an_unchained_base_is_named_after_its_base_and_never_taken_for_one() {
        let (_tmp, p) = temp_paths();
        let i = installer(&p);
        let plain = base(&p, "mac-base-26A434-work", "c-1", true);
        let dir = i.unchained_for(&plain);
        assert_eq!(name_of(&dir), "mac-unchained-26A434-work");
        assert_eq!(name_of(&i.unchained_for(&p.bases().join("mac-base-26A434"))), "mac-unchained-26A434");
        assert!(MacBaseInstaller::is_unchained(&dir) && !MacBaseInstaller::is_unchained(&plain));

        assert_eq!(MacBaseInstaller::newest_unchained_for(&p, Some("c-1")), None);
        base(&p, "mac-unchained-26A434-work", "c-1", false);
        assert_eq!(MacBaseInstaller::newest_unchained_for(&p, Some("c-1")), None, "unfinished");
        let ready = base(&p, "mac-unchained-26A434-work", "c-1", true);
        let newer = base(&p, "mac-unchained-27A100", "c-1", true);
        let other = base(&p, "mac-unchained-28A1", "c-2", true);
        assert_eq!(MacBaseInstaller::newest_unchained_for(&p, Some("c-1")), Some(newer));
        assert_eq!(MacBaseInstaller::newest_unchained_for(&p, Some("c-2")), Some(other.clone()));
        assert_eq!(MacBaseInstaller::newest_unchained_for(&p, None), Some(other), "any credential");
        assert_eq!(MacBaseInstaller::newest_unchained_for(&p, Some("c-9")), None);
        // The searches for a credential's base never see an unchained one
        use crate::mac_image_builder::MacImageBuilder;
        assert_eq!(MacImageBuilder::newest_base_for(&p, Some("c-1")), Some(plain.clone()));
        assert_eq!(MacImageBuilder::newest_base_in(&p), Some(plain.clone()));
        assert_eq!(i.base_for("26A434", Some(("c-1", "Work")), None), plain);
        assert!(BaseRecord::of(&ready).unwrap().unchained && !BaseRecord::of(&plain).unwrap().unchained);
    }

    #[test]
    fn unchaining_refuses_what_cannot_be_unchained() {
        let (_tmp, p) = temp_paths();
        let i = installer(&p);
        let request = |source: PathBuf| UnchainRequest { source, password: "x".into(), tools: None, keyboard: None };
        let message = |r: Result<BaseOutcome>| r.unwrap_err().message().to_string();
        let unfinished = base(&p, "mac-base-26A434", "c-1", false);
        assert!(message(i.unchain(&request(unfinished), &|_| {}, &|_, _| {})).contains("unfinished"));
        let unchained = base(&p, "mac-unchained-26A434", "c-1", true);
        assert!(message(i.unchain(&request(unchained.clone()), &|_| {}, &|_, _| {})).contains("unchained already"));
        let bare = p.bases().join("mac-base-27A1");
        fs::create_dir_all(&bare).unwrap();
        fs::write(bare.join(MacBaseInstaller::READY_MARKER), "alice\n").unwrap();
        assert!(message(i.unchain(&request(bare), &|_| {}, &|_, _| {})).contains("no credential"));
    }

    #[test]
    fn the_keyboard_is_the_table_of_the_images() {
        assert_eq!(mac_layout("de"), Some((3, "German")));
        assert_eq!(mac_layout("us"), Some((252, "ABC")));
        assert_eq!(mac_layout("xx"), None);
        let commands = keyboard_commands((3, "German"));
        assert_eq!(commands.matches("defaults write com.apple.HIToolbox").count(), 4);
        assert!(commands.contains("<key>KeyboardLayout ID</key><integer>3</integer>"));
        assert!(commands.contains("<key>KeyboardLayout Name</key><string>German</string>"));
        assert!(commands.ends_with("AppleCurrentKeyboardLayoutInputSourceID -string 'com.apple.keylayout.German'"));
        // Every layout of darwin-guest.nix's table is here, with its id
        let nix =
            fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/modules/darwin-guest.nix")).unwrap();
        let mut seen = 0;
        let rows = nix.lines().filter(|l| !l.trim_start().starts_with('#'));
        for line in rows.filter(|l| l.contains("= { id = ") && l.contains("name = \"")) {
            let key = line.trim().split(' ').next().unwrap();
            let id: i64 = line.split("id = ").nth(1).unwrap().split(';').next().unwrap().trim().parse().unwrap();
            let name = line.split("name = \"").nth(1).unwrap().split('"').next().unwrap();
            assert_eq!(mac_layout(key), Some((id, name)), "{key}");
            seen += 1;
        }
        assert!(seen >= 19, "{seen} layouts read from darwin-guest.nix");
    }

    #[test]
    fn an_images_unchained_copy_has_the_name_of_a_build_from_an_unchained_base() {
        use crate::image_store::{Image, ImageInfo};
        use crate::mac_image_builder::MacImageBuilder;
        use crate::models::GuestKind;
        let system = "/nix/store/00000000000000000000000000000000-darwin-system";
        let info = ImageInfo::new(
            "omaflake.macos.minimal",
            None,
            GuestKind::Macos,
            "aarch64-darwin",
            "/flake",
            "r",
            Some(system.into()),
            crate::json::now(),
        );
        let image = Image { name: "omaflake.macos.minimal@a".into(), info, path: PathBuf::from("/x") };
        let name = MacBaseInstaller::unchained_image_name(&image);
        assert_eq!(name, MacImageBuilder::image_name("omaflake.macos.minimal", system, true));
        assert_ne!(name, MacImageBuilder::image_name("omaflake.macos.minimal", system, false));
        // What cannot be unchained says so before anything is started
        let (_tmp, p) = temp_paths();
        let i = installer(&p);
        let mut unchained = image.clone();
        unchained.info.unchained = true;
        let message =
            i.unchain_image(&unchained, "u", "p", None, &|_| {}, &|_, _| {}).unwrap_err().message().to_string();
        assert!(message.contains("unchained already"), "{message}");
        let mut nixos = image.clone();
        nixos.info.kind = GuestKind::Nixos;
        let message = i.unchain_image(&nixos, "u", "p", None, &|_| {}, &|_, _| {}).unwrap_err().message().to_string();
        assert!(message.contains("not a macOS image"), "{message}");
    }

    #[test]
    fn csrutil_and_systemextensionsctl_are_read() {
        assert!(MacBaseInstaller::protection_is_off("System Integrity Protection status: disabled.\n"));
        assert!(!MacBaseInstaller::protection_is_off("System Integrity Protection status: enabled.\n"));
        assert!(!MacBaseInstaller::protection_is_off(""));
        let list = "2 extension(s)\n--- com.apple.system_extension.cmio\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n\
                    *\t*\t-\tapp.omaflake.guest.camera (0.8.0/20261126.205545)\tomaflake Camera\t[activated enabled]\n\
                    *\t*\t-\torg.example.other (1/1)\tOther\t[activated waiting for user]\n";
        assert_eq!(MacBaseInstaller::camera_extension_state(list).as_deref(), Some("activated enabled"));
        assert_eq!(MacBaseInstaller::camera_extension_state("0 extension(s)\n"), None);
    }

    #[test]
    fn the_tools_with_the_extension_are_found_beside_omaf() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("omaflake.app/Contents");
        let exe = app.join("MacOS/omaf");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(MacBaseInstaller::unchained_tools_from(&exe), None);
        // The plain Tools (no extension inside) are not it
        fs::create_dir_all(app.join("Resources/Guest").join(TOOLS_APP).join("Contents/MacOS")).unwrap();
        assert_eq!(MacBaseInstaller::unchained_tools_from(&exe), None);
        let unchained = app.join("Resources/Guest/unchained").join(TOOLS_APP);
        fs::create_dir_all(unchained.join("Contents/Library/SystemExtensions")).unwrap();
        let found = MacBaseInstaller::unchained_tools_from(&exe).unwrap();
        assert_eq!(found.canonicalize().unwrap(), unchained.canonicalize().unwrap());
        // The plain build for every image: the one in Guest/ (an app – its Info.plist)
        assert_eq!(MacBaseInstaller::plain_tools_from(&exe), None);
        let plain = app.join("Resources/Guest").join(TOOLS_APP);
        fs::write(plain.join("Contents/Info.plist"), "<plist/>").unwrap();
        let found = MacBaseInstaller::plain_tools_from(&exe).unwrap();
        assert_eq!(found.canonicalize().unwrap(), plain.canonicalize().unwrap());
    }
}
