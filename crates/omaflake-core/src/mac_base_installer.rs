// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of MacBaseInstaller.swift.
//!
//! A macOS base from Apple's IPSW (spec §6.5 step 1, docs/macos-base.md), shared by `omaf macos install` and the app:
//!   `omaf-runner install` → `bases/mac-fresh-<build>` (never started, read-only) → APFS clone `bases/mac-base-<build>`
//!   with a new identity → first boot with provisioning (user, automatic login, SSH; plan BM11) → your SSH public
//!   keys into the guest user's authorized_keys → shut down → read-only. No manual steps (plan BM29).
//! omaflake never distributes macOS itself: the IPSW comes from Apple, the base is built on the user's Mac.
//! Existing folders are reused, never overwritten. The password goes to the runner and ssh/sudo on stdin or via
//! an askpass helper that reads it from its environment – never into a file or an argument.
//!
//! Not 1:1: the first-boot runner is a `std::process::Child` (Swift `Process`); its stdin is a pipe that gets the
//! password and is closed, its output is appended to `logs/first-boot-<base>.log`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{IoContext, Result, codes};
use crate::file_format::{FileFormat, Step};
use crate::ipsw_downloader::{IpswDownloader, KeepAwake, MacOsRelease, free_space, gigabytes};
use crate::mac_image_builder::BuildVm;
use crate::mac_vm::{DhcpLeases, MacVmBundle, RunnerClient};
use crate::paths::Paths;
use crate::platform::home_dir;
use crate::prerequisites::Prerequisites;
use crate::qemu_runtime::tail;
use crate::shell::{RunOptions, Shell, ShellResult, StreamOptions};
use crate::sockets::{self, Ports};
use crate::trash::Disposal;
use crate::vm_err;
use crate::vz_backend::VzBackend;

/// v1 → v2 (plan B166): `ownKey` and `version` are new; a base from before does not know omaflake's own key.
/// v2 → v3 (plan B184): `unchained` is new; a base from before is not
static BASE_STEPS: [Step; 3] = [|_| Ok(()), |_| Ok(()), |_| Ok(())];

/// `omaflake-base.json` in a base folder (plan B139): the credential its guest user was created from – macOS image
/// builds take its password from there, so nobody types one. Bases from before have none until one is assigned.
/// Since plan B166 it is written right after the clone (an unfinished base continues with its credential) and
/// carries whether omaflake's own SSH key went into the guest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseRecord {
    /// The credential's ID ("c-…")
    pub credential: String,
    /// Its name when it was set, for when it is gone
    #[serde(rename = "credentialName")]
    pub credential_name: String,
    /// The guest user
    pub user: String,
    /// omaflake's own SSH key is in the guest's authorized_keys (plan B160) – false for bases from before
    #[serde(rename = "ownKey", default)]
    pub own_key: bool,
    /// The macOS version ("27.0.1") when it was known (Apple's IPSW)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// An unchained base (mac_base_unchain.rs, plan B184): System Integrity Protection is off in it and in every
    /// image and VM made from it
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unchained: bool,
}

impl BaseRecord {
    pub const FILE: &'static str = "omaflake-base.json";
    pub const FILE_FORMAT: FileFormat = FileFormat::new("base", "omaflake-base.json", 1, &BASE_STEPS);

    /// A base's record, None for a base without one (or one that cannot be read)
    pub fn of(dir: &Path) -> Option<BaseRecord> {
        let data = fs::read(dir.join(Self::FILE)).ok()?;
        Self::FILE_FORMAT.decode::<BaseRecord>(&data).ok().map(|(r, _)| r)
    }

    /// Writes the record into a base folder – also a finished, read-only one (made writable for it, then read-only
    /// again)
    pub fn write(&self, dir: &Path) -> Result<()> {
        let data = Self::FILE_FORMAT.encode(self)?;
        let mode = fs::metadata(dir).with_path(dir)?.permissions().mode();
        let readonly = mode & 0o200 == 0;
        if readonly {
            fs::set_permissions(dir, fs::Permissions::from_mode(mode | 0o200)).with_path(dir)?;
        }
        let file = dir.join(Self::FILE);
        let _ = fs::set_permissions(&file, fs::Permissions::from_mode(0o644));
        let written = crate::file_format::write_atomic(&file, &data);
        if readonly {
            let _ = fs::set_permissions(&file, fs::Permissions::from_mode(0o444));
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(mode));
        }
        written
    }
}

#[derive(Debug, Clone)]
pub struct MacBaseInstaller {
    pub paths: Paths,
    pub vz: VzBackend,
}

/// What to install (Swift `MacBaseInstaller.Request`)
#[derive(Debug, Clone)]
pub struct BaseRequest {
    /// Local IPSW; None = the latest supported by this Mac, downloaded from Apple into `~/.omaflake/ipsw/`
    pub ipsw: Option<PathBuf>,
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    pub user: String,
    pub full_name: String,
    /// Password of the new guest user; None = stop after the clone, without the first boot
    pub password: Option<String>,
    /// Public keys for the guest user's authorized_keys (default: ~/.ssh/*.pub)
    pub authorized_keys: Vec<String>,
    /// Show the first boot in a window (headless otherwise)
    pub window: bool,
    /// The credential the guest user comes from (plan B139) – it names the base (plan B166) and is recorded in it
    pub credential: Option<(String, String)>,
    /// A base this one supersedes (plan B166): never reused as the new base's folder – the caller removes it once the
    /// new one is ready (`supersede`)
    pub replace: Option<PathBuf>,
}

impl BaseRequest {
    /// Swift defaults: 4 CPUs, 8 GB memory, 80 GB disk, no IPSW, the default public keys, headless
    pub fn new(user: &str, full_name: &str, password: Option<String>) -> Self {
        BaseRequest {
            ipsw: None,
            cpus: 4,
            memory_gb: 8,
            disk_gb: 80,
            user: user.into(),
            full_name: full_name.into(),
            password,
            authorized_keys: MacBaseInstaller::default_public_keys(),
            window: false,
            credential: None,
            replace: None,
        }
    }
}

/// Swift `MacBaseInstaller.Outcome`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseOutcome {
    /// The base for this build existed already; nothing was changed
    Existing(PathBuf),
    /// New base; `ready` = provisioned, keys installed, shut down and read-only
    Created { dir: PathBuf, ready: bool },
}

impl MacBaseInstaller {
    /// Marker of a finished base (keys installed, shut down, read-only); not one of the VM files, so clones skip it.
    /// It holds the guest user's name.
    pub const READY_MARKER: &'static str = ".omaflake-base-ready";
    /// Space left over after a download on its own (plan B146)
    pub const DOWNLOAD_RESERVE: i64 = 2_000_000_000;

    pub fn new(paths: Paths, vz: VzBackend) -> Self {
        MacBaseInstaller { paths, vz }
    }

    /// The Mac user's SSH public keys (`~/.ssh/*.pub`, sorted, trimmed, only "ssh-…" and "ecdsa-…") – the keys
    /// `omaf ssh` and the image build log in with.
    pub fn default_public_keys() -> Vec<String> {
        // omaflake's own key first (plan B160), then the user's
        let own = crate::ssh_identity::SshIdentity::standard().public_key();
        own.into_iter().chain(Self::user_public_keys()).collect()
    }

    /// The user's keys in ~/.ssh (as `default_public_keys` before plan B160)
    pub fn user_public_keys() -> Vec<String> {
        let dir = home_dir().join(".ssh");
        let mut names: Vec<String> = fs::read_dir(&dir)
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.retain(|n| n.ends_with(".pub"));
        names.sort();
        names
            .iter()
            .filter_map(|n| fs::read_to_string(dir.join(n)).ok())
            .map(|text| text.trim().to_string())
            .filter(|k| k.starts_with("ssh-") || k.starts_with("ecdsa-"))
            .collect()
    }

    /// The latest macOS this Mac can run as a VM (asks Apple, no download), None if unknown.
    pub fn latest_release(&self) -> Option<MacOsRelease> {
        MacOsRelease::latest(&self.vz.runner)
    }

    /// Build of the latest IPSW supported by this Mac, None if unknown.
    pub fn latest_build(&self) -> Option<String> {
        self.latest_release().map(|r| r.build)
    }

    /// Downloads the IPSW of `release` into `ipsw/` (continues an interrupted download); `started` gets the
    /// downloader so the caller can pause it (`IpswDownloader::cancel`). Checks free space first: what is still to
    /// download (IPSW size or 27 GB, less a partial file) plus `reserve` – `Self::DOWNLOAD_RESERVE` for a download on
    /// its own, `Prerequisites::MAC_SPACE_NEEDED` when a base is created from it (plan B146). Progress goes to `log` in
    /// whole percent ("Download: <n> %").
    pub fn download_ipsw(
        &self,
        release: &MacOsRelease,
        reserve: i64,
        started: &dyn Fn(&IpswDownloader),
        progress: &(dyn Fn(i64, i64) + Send + Sync + '_),
        log: &(dyn Fn(&str) + Send + Sync + '_),
    ) -> Result<PathBuf> {
        let file = IpswDownloader::file(release, &self.paths.ipsw());
        if file.exists() {
            return Ok(file);
        }
        let received =
            fs::metadata(IpswDownloader::resume_file_for(release, &self.paths.ipsw())).map(|m| m.len() as i64);
        let needed = (release.bytes.unwrap_or(27_000_000_000) - received.unwrap_or(0)).max(0) + reserve;
        if let Some(free) = free_space(&self.paths.root)
            && free < needed
        {
            return Err(if reserve >= Prerequisites::MAC_SPACE_NEEDED {
                vm_err!(
                    codes::PREREQUISITE,
                    "Not enough space: {} free, about {} needed (IPSW, installation, image)",
                    gigabytes(free),
                    gigabytes(needed)
                )
            } else {
                vm_err!(
                    codes::PREREQUISITE,
                    "Not enough space for the download: {} free, about {} needed",
                    gigabytes(free),
                    gigabytes(needed)
                )
            });
        }
        let downloader = IpswDownloader::new(release.clone(), &self.paths.ipsw());
        started(&downloader);
        log(&format!("Downloading macOS {} ({}) from Apple → {}", release.version, release.build, file.display()));
        let last = LastPercent::default();
        KeepAwake::during("Downloading macOS", || {
            downloader.run(&|done, total| {
                progress(done, total);
                if total > 0
                    && let Some(pct) = last.next(done * 100 / total)
                {
                    log(&format!("Download: {pct} %"));
                }
            })
        })
    }

    /// "macOS 27.0.1 (26A434)" in the first line → "26A434"
    pub fn build_from_latest(output: &str) -> Option<String> {
        let line = output.split('\n').find(|l| !l.is_empty())?;
        let (open, close) = (line.rfind('(')?, line.rfind(')')?);
        (open < close).then(|| line[open + 1..close].to_string())
    }

    /// `bases/mac-fresh-<build>`
    pub fn fresh(&self, build: &str) -> PathBuf {
        self.paths.bases().join(format!("mac-fresh-{build}"))
    }

    /// `bases/mac-base-<build>`
    pub fn base(&self, build: &str) -> PathBuf {
        self.paths.bases().join(format!("mac-base-{build}"))
    }

    /// "Work Laptop" → "work-laptop": a credential's name as a part of a folder name
    pub fn credential_slug(name: &str) -> String {
        let mut out = String::new();
        for c in name.trim().chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() {
                out.push(c);
            } else if !out.ends_with('-') {
                out.push('-');
            }
        }
        out.trim_matches('-').chars().take(32).collect::<String>().trim_end_matches('-').to_string()
    }

    /// The folder a base of `build` for `credential` (ID, name) gets (plan B166): the credential's base of this build
    /// when there is one; else `mac-base-<build>` while that is free (the first base of a build), else
    /// `mac-base-<build>-<credential>` (with `-2`, `-3`, … while taken). `avoid` (a base being replaced) is never
    /// chosen. Without a credential: `mac-base-<build>`.
    pub fn base_for(&self, build: &str, credential: Option<(&str, &str)>, avoid: Option<&Path>) -> PathBuf {
        let first = self.base(build);
        let Some((id, name)) = credential else { return first };
        let usable = |dir: &Path| avoid != Some(dir);
        let prefix = format!("mac-base-{build}-");
        let mut names: Vec<String> = fs::read_dir(self.paths.bases())
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.retain(|n| *n == file_name(&first) || n.starts_with(&prefix));
        names.sort();
        if let Some(own) = names
            .iter()
            .map(|n| self.paths.bases().join(n))
            .find(|dir| usable(dir) && BaseRecord::of(dir).is_some_and(|r| r.credential == id))
        {
            return own;
        }
        if !first.exists() && usable(&first) {
            return first;
        }
        let slug = Some(Self::credential_slug(name)).filter(|s| !s.is_empty()).unwrap_or_else(|| id.to_string());
        let mut n = 1;
        loop {
            let dir = if n == 1 {
                self.paths.bases().join(format!("{prefix}{slug}"))
            } else {
                self.paths.bases().join(format!("{prefix}{slug}-{n}"))
            };
            if !dir.exists() && usable(&dir) {
                return dir;
            }
            n += 1;
        }
    }

    /// The base that `install` would end at, if it exists already (only known up front without `ipsw`).
    pub fn existing_base(&self, r: &BaseRequest) -> Option<PathBuf> {
        if r.ipsw.is_some() {
            return None;
        }
        let credential = r.credential.as_ref().map(|(id, name)| (id.as_str(), name.as_str()));
        let dir = self.base_for(&self.latest_build()?, credential, r.replace.as_deref());
        Self::is_ready(&dir).then_some(dir)
    }

    /// After a replacement (plan B166): deletes the superseded base `old` for real (a product action, `Disposal`) and
    /// gives the new one `old`'s name when it took the place of a base of the same build and credential (so "Create
    /// Again" keeps the name). The caller checks that no image build uses `old`. Returns where the new base is.
    pub fn supersede(&self, old: &Path, new: &Path, log: &(dyn Fn(&str) + Send + Sync + '_)) -> Result<PathBuf> {
        if old == new || !old.exists() {
            return Ok(new.to_path_buf());
        }
        let build_of = |dir: &Path| MacVmBundle::new(dir).load_config().ok().and_then(|c| c.os_build);
        let same_place = build_of(new).is_some_and(|b| build_of(old).as_deref() == Some(b.as_str()))
            && match (BaseRecord::of(old), BaseRecord::of(new)) {
                (Some(o), Some(n)) => o.credential == n.credential,
                (None, _) => true,
                (Some(_), None) => false,
            };
        Disposal::delete(&[old])?;
        log(&format!("Removed {} – {} replaces it", file_name(old), file_name(new)));
        if !same_place {
            return Ok(new.to_path_buf());
        }
        match fs::rename(new, old) {
            Ok(()) => {
                let logs = self.paths.logs();
                let _ = fs::rename(
                    logs.join(format!("first-boot-{}.log", file_name(new))),
                    logs.join(format!("first-boot-{}.log", file_name(old))),
                );
                log(&format!("{} is now {}", file_name(new), file_name(old)));
                Ok(old.to_path_buf())
            }
            Err(e) => {
                log(&format!("{} keeps its name ({})", file_name(new), crate::error::os_text(&e)));
                Ok(new.to_path_buf())
            }
        }
    }

    /// Runs all steps (keeping the Mac awake); progress (including the runner's) goes to `log` line by line. Blocks
    /// until the first-boot VM is shut down.
    pub fn install(
        &self,
        r: &BaseRequest,
        started: &dyn Fn(&IpswDownloader),
        progress: &(dyn Fn(i64, i64) + Send + Sync + '_),
        log: &(dyn Fn(&str) + Send + Sync + '_),
    ) -> Result<BaseOutcome> {
        KeepAwake::during("Creating a macOS base", || self.install_awake(r, started, progress, log))
    }

    fn install_awake(
        &self,
        r: &BaseRequest,
        started: &dyn Fn(&IpswDownloader),
        progress: &(dyn Fn(i64, i64) + Send + Sync + '_),
        log: &(dyn Fn(&str) + Send + Sync + '_),
    ) -> Result<BaseOutcome> {
        self.vz.check_entitlement()?;
        fs::create_dir_all(self.paths.bases()).with_path(&self.paths.bases())?;

        // 1. Fresh installation, named after the installed build; the IPSW from Apple unless one was given
        let mut version = None;
        let fresh_vm = match &r.ipsw {
            Some(ipsw) => self.install_fresh(r, ipsw, log)?,
            None => {
                let Some(release) = self.latest_release() else {
                    return Err(vm_err!(
                        codes::NETWORK,
                        "Apple's macOS catalog is not reachable – online? (or choose an IPSW file)"
                    ));
                };
                let fresh = self.fresh(&release.build);
                version = Some(release.version.clone());
                if fresh.exists() {
                    log(&format!("{} exists – reusing it", file_name(&fresh)));
                    fresh
                } else {
                    let ipsw = self.download_ipsw(&release, Prerequisites::MAC_SPACE_NEEDED, started, progress, log)?;
                    self.install_fresh(r, &ipsw, log)?
                }
            }
        };
        let build = MacVmBundle::new(&fresh_vm).load_config()?.os_build.unwrap_or_else(|| "unknown".into());

        // 2. Base = APFS clone with a new machine identifier and MAC – one per build and credential (plan B166)
        let credential = r.credential.as_ref().map(|(id, name)| (id.as_str(), name.as_str()));
        let base_dir = self.base_for(&build, credential, r.replace.as_deref());
        if Self::is_ready(&base_dir) {
            log(&format!("{} exists already – nothing to do", file_name(&base_dir)));
            return Ok(BaseOutcome::Existing(base_dir));
        }
        if base_dir.exists() {
            // An unfinished base (e.g. no SSH on its first boot): provisioning is done, continue after it
            let Some(password) = &r.password else { return Ok(BaseOutcome::Created { dir: base_dir, ready: false }) };
            log(&format!("{} is unfinished – continuing (same password as before)", file_name(&base_dir)));
            self.finish(&base_dir, r, password, false, log)?;
            return Ok(BaseOutcome::Created { dir: base_dir, ready: true });
        }
        Shell::check(
            &self.vz.runner,
            &["clone".to_string(), path_string(&fresh_vm), path_string(&base_dir)],
            &RunOptions::default(),
        )?;
        log(&format!("Base {} cloned from {}", base_dir.display(), file_name(&fresh_vm)));
        if let Some((id, name)) = &r.credential {
            // Right away: an unfinished base continues with its credential, and the folder name finds it again
            BaseRecord {
                credential: id.clone(),
                credential_name: name.clone(),
                user: r.user.clone(),
                own_key: crate::ssh_identity::SshIdentity::standard()
                    .public_key()
                    .is_some_and(|own| r.authorized_keys.iter().any(|k| k.trim() == own.trim())),
                version,
                unchained: false,
            }
            .write(&base_dir)?;
        }
        let Some(password) = &r.password else { return Ok(BaseOutcome::Created { dir: base_dir, ready: false }) };
        if r.authorized_keys.is_empty() {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "No SSH public key in ~/.ssh – create one first: ssh-keygen -t ed25519"
            ));
        }
        self.finish(&base_dir, r, password, true, log)?;
        Ok(BaseOutcome::Created { dir: base_dir, ready: true })
    }

    /// The macOS bases in `bases/` (`mac-base-*`, and the unchained `mac-unchained-*` of plan B184), newest name
    /// first (plan B137): ready or not, with what is known
    pub fn list(paths: &Paths) -> Vec<BaseInfo> {
        let mut names: Vec<String> = fs::read_dir(paths.bases())
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.retain(|n| n.starts_with("mac-base-") || n.starts_with(Self::UNCHAINED_PREFIX));
        names.sort();
        names.reverse();
        names
            .into_iter()
            .map(|name| {
                let dir = paths.bases().join(&name);
                let os_build = crate::mac_vm::MacVmBundle::new(dir.clone()).load_config().ok().and_then(|c| c.os_build);
                let created =
                    fs::metadata(&dir).and_then(|m| m.modified()).ok().and_then(crate::json::from_system_time);
                let record = BaseRecord::of(&dir);
                // The newest of its credential (plan B166) – among the unchained ones for an unchained base
                let credential = record.as_ref().map(|r| r.credential.as_str());
                let newest = if Self::is_unchained(&dir) {
                    credential.and_then(|c| Self::newest_unchained_for(paths, Some(c)))
                } else {
                    crate::mac_image_builder::MacImageBuilder::newest_base_for(paths, credential)
                };
                BaseInfo {
                    ready: Self::is_ready(&dir),
                    user: Self::user_of(&dir, &paths.logs()),
                    record,
                    newest: newest.as_deref() == Some(dir.as_path()),
                    size_bytes: allocated(&dir),
                    os_build,
                    created,
                    name,
                    path: dir,
                }
            })
            .collect()
    }

    pub fn is_ready(dir: &Path) -> bool {
        dir.join(Self::READY_MARKER).exists()
    }

    /// Guest user of a base: written into its ready marker; for older bases from the log of its first boot
    /// (`logs/first-boot-<base folder>.log`).
    pub fn user_of(dir: &Path, logs: &Path) -> Option<String> {
        if let Ok(text) = fs::read_to_string(dir.join(Self::READY_MARKER)) {
            let user = text.trim();
            if !user.is_empty() {
                return Some(user.to_string());
            }
        }
        let log = fs::read_to_string(logs.join(format!("first-boot-{}.log", file_name(dir)))).unwrap_or_default();
        Self::user_from_first_boot_log(&log)
    }

    /// "[00:12:48] Provisioning: user alice, auto-login on, SSH on" → "alice" (the last such line)
    pub(crate) fn user_from_first_boot_log(log: &str) -> Option<String> {
        const MARK: &str = "Provisioning: user ";
        let line = log.split('\n').rfind(|l| l.contains(MARK))?;
        let rest = &line[line.find(MARK)? + MARK.len()..];
        let user: String = rest.chars().take_while(|c| *c != ',' && *c != ' ').collect();
        (!user.is_empty()).then_some(user)
    }

    /// Step 3: boot (the first one with provisioning), keys, shutdown, read-only. The first boot after an
    /// installation sometimes brings SSH up only much later (BM30); then the VM is restarted once – provisioning is
    /// done by then. The runner runs as a child (`Command`, stdin piped for `--password-stdin`, output appended to the first-boot log).
    fn finish(
        &self,
        dir: &Path,
        r: &BaseRequest,
        password: &str,
        provision: bool,
        log: &(dyn Fn(&str) + Send + Sync + '_),
    ) -> Result<()> {
        fs::create_dir_all(self.paths.logs()).with_path(&self.paths.logs())?;
        let name = file_name(dir);
        let log_file = self.paths.logs().join(format!("first-boot-{name}.log"));
        let socket = dir.join("runner.sock");
        let pid_file = dir.join("runner.pid");
        let out = OpenOptions::new().create(true).append(true).open(&log_file).with_path(&log_file)?;
        let shown = if r.window { " in a window" } else { " (headless)" };

        let boot = |provisioning: bool| -> Result<Child> {
            log(&if provisioning {
                format!(
                    "First boot{shown} with provisioning: user {}, automatic login, SSH (log: {})",
                    r.user,
                    log_file.display()
                )
            } else {
                format!("Booting {name}{shown}")
            });
            let mut args: Vec<String> = vec![
                "run".into(),
                path_string(dir),
                "--socket".into(),
                path_string(&socket),
                "--pid-file".into(),
                path_string(&pid_file),
            ];
            if provisioning {
                args.extend(["--user", &r.user, "--full-name", &r.full_name].map(String::from));
                args.extend(["--password-stdin", "--auto-login", "--ssh"].map(String::from));
            }
            if r.window {
                args.push("--window".into());
            }
            let mut child = Command::new(&self.vz.runner)
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::from(out.try_clone().with_path(&log_file)?))
                .stderr(Stdio::from(out.try_clone().with_path(&log_file)?))
                .spawn()
                .map_err(|e| vm_err!("Cannot run {}: {}", self.vz.runner, crate::error::os_text(&e)))?;
            // Dropping stdin (after the password) closes it
            if let Some(mut stdin) = child.stdin.take()
                && provisioning
            {
                let _ = stdin.write_all(format!("{password}\n").as_bytes());
            }
            Ok(child)
        };
        let force_off = |p: &mut Child| {
            if !matches!(p.try_wait(), Ok(None)) {
                return;
            }
            if RunnerClient::send("kill", &socket, RunnerClient::DEFAULT_TIMEOUT).is_err() {
                crate::spawn::send_signal(p.id() as crate::spawn::Pid, libc::SIGTERM);
            }
            let _ = p.wait();
        };
        let mac = MacVmBundle::new(dir).load_config()?.mac_address;
        // Address from the DHCP leases once its SSH server answers, None after `seconds`
        let wait_for_ssh = |p: &mut Child, seconds: u64| -> Result<Option<String>> {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < deadline {
                if !matches!(p.try_wait(), Ok(None)) {
                    return Err(vm_err!("omaf-runner exited:\n{}", tail(&log_file, 3)));
                }
                if let Some(a) = DhcpLeases::ip(&mac)
                    && Ports::ssh_banner(&a, 22, sockets::DEFAULT_TIMEOUT).is_some()
                {
                    return Ok(Some(a));
                }
                std::thread::sleep(Duration::from_secs(3));
            }
            Ok(None)
        };

        let mut runner = boot(provision)?;
        let steps = (|| -> Result<()> {
            let mut ip = wait_for_ssh(&mut runner, if provision { 480 } else { 300 })?;
            if ip.is_none() && provision {
                log("No SSH yet – restarting the VM once (provisioning is done)");
                force_off(&mut runner);
                runner = boot(false)?;
                ip = wait_for_ssh(&mut runner, 300)?;
            }
            let Some(ip) = ip else {
                return Err(vm_err!(
                    "No SSH from {name} (log: {}) – try Create Base again, it continues",
                    log_file.display()
                ));
            };
            log(&format!("Guest at {ip}, installing {} SSH key(s)", r.authorized_keys.len()));

            // Keys with password login (askpass), then check the key login
            let keys = format!("{}\n", r.authorized_keys.join("\n"));
            let mut installed = false;
            for _ in 0..20 {
                installed = self
                    .password_ssh(
                        &ip,
                        &r.user,
                        password,
                        "umask 077; mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys",
                        keys.as_bytes(),
                    )
                    .is_ok_and(|res| res.status == 0);
                if installed {
                    break;
                }
                std::thread::sleep(Duration::from_secs(3));
            }
            if !installed {
                return Err(vm_err!(
                    codes::AUTH,
                    "Could not log in as {} with the password to install the SSH keys",
                    r.user
                ));
            }
            let target = format!("{}@{ip}", r.user);
            let key_login = Shell::run(
                &crate::platform::host_tool("/usr/bin/ssh"),
                &ssh_args(&[&target, "true"]),
                &RunOptions::default(),
            )?;
            if key_login.status != 0 {
                return Err(vm_err!(codes::AUTH, "SSH key login as {} failed after installing the keys", r.user));
            }

            // Shut down (sudo with the password on stdin), then read-only
            log("Shutting down");
            let input = format!("{password}\n");
            let _ = Shell::run(
                &crate::platform::host_tool("/usr/bin/ssh"),
                &ssh_args(&[&target, "sudo -S -p '' /sbin/shutdown -h now"]),
                &RunOptions { input: Some(input.as_bytes()), ..Default::default() },
            );
            let off = Instant::now() + Duration::from_secs(180);
            while matches!(runner.try_wait(), Ok(None)) {
                if Instant::now() >= off {
                    return Err(vm_err!("The base did not shut down"));
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            Ok(())
        })();
        if let Err(e) = steps {
            force_off(&mut runner);
            log(&format!("The base is left as it is for inspection: {}", dir.display()));
            return Err(e);
        }
        let _ = Disposal::delete(&[&socket, &pid_file]);
        // The marker carries the guest user: image builds log in as this user (and must match the recipes' macUser)
        let _ = fs::write(dir.join(Self::READY_MARKER), format!("{}\n", r.user));
        Shell::check(
            &crate::platform::host_tool("/bin/chmod"),
            &["-R", "a-w", &path_string(dir)],
            &RunOptions::default(),
        )?;
        log(&format!("Base ready: {} (read-only)", dir.display()));
        Ok(())
    }

    /// ssh with password authentication: an askpass helper (`cache/askpass.sh`, no secret in it) prints the password
    /// from its environment (`OMAFLAKE_ASKPASS`), which only this ssh process and its helper get.
    fn password_ssh(&self, host: &str, user: &str, password: &str, command: &str, input: &[u8]) -> Result<ShellResult> {
        let askpass = self.paths.cache().join("askpass.sh");
        if !askpass.exists() {
            fs::create_dir_all(self.paths.cache()).with_path(&self.paths.cache())?;
            crate::file_format::write_atomic(&askpass, b"#!/bin/sh\nprintf '%s\\n' \"$OMAFLAKE_ASKPASS\"\n")?;
            fs::set_permissions(&askpass, fs::Permissions::from_mode(0o700)).with_path(&askpass)?;
        }
        let mut args: Vec<String> = [
            "-o",
            "PubkeyAuthentication=no",
            "-o",
            "PreferredAuthentications=password,keyboard-interactive",
            "-o",
            "NumberOfPasswordPrompts=1",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "LogLevel=ERROR",
        ]
        .map(String::from)
        .to_vec();
        args.extend([format!("{user}@{host}"), command.to_string()]);
        let askpass = path_string(&askpass);
        let environment = [
            ("SSH_ASKPASS", askpass.as_str()),
            ("SSH_ASKPASS_REQUIRE", "force"),
            ("DISPLAY", ":0"),
            ("OMAFLAKE_ASKPASS", password),
        ];
        Shell::run(
            &crate::platform::host_tool("/usr/bin/ssh"),
            &args,
            &RunOptions { input: Some(input), environment: &environment, ..Default::default() },
        )
    }

    /// `omaf-runner install` into `bases/.install-<unix time>`, renamed to mac-fresh-<build> and made read-only.
    fn install_fresh(&self, r: &BaseRequest, ipsw: &Path, log: &(dyn Fn(&str) + Send + Sync + '_)) -> Result<PathBuf> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let tmp = self.paths.bases().join(format!(".install-{now}"));
        let args: Vec<String> = vec![
            "install".into(),
            path_string(&tmp),
            "--cpus".into(),
            r.cpus.to_string(),
            "--memory-gb".into(),
            r.memory_gb.to_string(),
            "--disk-gb".into(),
            r.disk_gb.to_string(),
            "--ipsw".into(),
            path_string(ipsw),
        ];
        let result =
            Shell::stream(&self.vz.runner, &args, &StreamOptions { merge_stdout: true, ..Default::default() }, log)?;
        if result.status != 0 {
            if tmp.exists() {
                let _ = Disposal::delete(&[&tmp]);
                return Err(vm_err!("Installation failed ({}); the incomplete folder was deleted", result.status));
            }
            return Err(vm_err!("Installation failed ({})", result.status));
        }
        let build = MacVmBundle::new(&tmp).load_config()?.os_build.unwrap_or_else(|| "unknown".into());
        let dir = self.fresh(&build);
        if dir.exists() {
            Disposal::delete(&[&tmp])?;
            log(&format!("{} existed already – using it; the new installation was deleted", file_name(&dir)));
        } else {
            fs::rename(&tmp, &dir).with_path(&dir)?;
            Shell::check(
                &crate::platform::host_tool("/bin/chmod"),
                &["-R", "a-w", &path_string(&dir)],
                &RunOptions::default(),
            )?;
            log(&format!("Installed: {} (read-only, never started)", dir.display()));
        }
        Ok(dir)
    }
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `BuildVm::ssh_options` (with omaflake's own key) followed by `rest`
fn ssh_args(rest: &[&str]) -> Vec<String> {
    let mut a = BuildVm::ssh_options();
    a.extend(rest.iter().map(|s| s.to_string()));
    a
}

/// Reports a percentage only when it changed (log lines in whole percent). Swift `LastPercent`.
#[derive(Debug, Default)]
pub(crate) struct LastPercent {
    last: std::sync::Mutex<Option<i64>>,
}

impl LastPercent {
    pub(crate) fn next(&self, pct: i64) -> Option<i64> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if *last == Some(pct) {
            return None;
        }
        *last = Some(pct);
        Some(pct)
    }
}

/// A macOS base as `MacBaseInstaller::list` finds it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseInfo {
    /// "mac-base-26A434"
    pub name: String,
    pub path: PathBuf,
    /// The macOS build it was installed from
    pub os_build: Option<String>,
    /// Its guest user (the ready marker)
    pub user: Option<String>,
    /// The credential it was created from or assigned (plan B139)
    pub record: Option<BaseRecord>,
    /// Installed completely (new macOS images can be built from it)
    pub ready: bool,
    /// The newest ready base of its credential: macOS images with that credential are built from it (plan B166)
    pub newest: bool,
    pub created: Option<crate::json::Date>,
    /// Allocated blocks of its files (the disk is sparse)
    pub size_bytes: Option<i64>,
}

/// Allocated bytes of the files in a folder and its subfolders (not through links)
fn allocated(dir: &Path) -> Option<i64> {
    use std::os::unix::fs::MetadataExt;
    fn walk(dir: &Path) -> i64 {
        let Ok(entries) = fs::read_dir(dir) else { return 0 };
        entries
            .flatten()
            .map(|e| match e.metadata() {
                Ok(m) if m.is_dir() => walk(&e.path()),
                Ok(m) if m.is_file() => m.blocks() as i64 * 512,
                _ => 0,
            })
            .sum()
    }
    dir.is_dir().then(|| walk(dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance_store::InstanceStore;
    use crate::paths::tests::temp_paths;
    use std::sync::Mutex;

    fn installer(p: &Paths, runner: &str) -> MacBaseInstaller {
        MacBaseInstaller::new(
            p.clone(),
            VzBackend::new(p.clone(), InstanceStore::new(p.clone(), "/usr/bin/false"), runner),
        )
    }

    // BaseUserTests
    #[test]
    fn user_from_marker_or_first_boot_log() {
        let (_t, p) = temp_paths();
        let dir = p.bases().join("mac-base-1");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(p.logs()).unwrap();
        assert_eq!(MacBaseInstaller::user_of(&dir, &p.logs()), None);
        // older base: empty marker, user from the log of its first boot (last provisioning line wins)
        fs::write(dir.join(MacBaseInstaller::READY_MARKER), "").unwrap();
        fs::write(
            p.logs().join("first-boot-mac-base-1.log"),
            "[23:37:39] Provisioning: user alice, auto-login on, SSH on\n[00:12:48] Provisioning: user bob, auto-login on, SSH on\n",
        )
        .unwrap();
        assert_eq!(MacBaseInstaller::user_of(&dir, &p.logs()).as_deref(), Some("bob"));
        // current base: the marker carries the user
        fs::write(dir.join(MacBaseInstaller::READY_MARKER), "carol\n").unwrap();
        assert_eq!(MacBaseInstaller::user_of(&dir, &p.logs()).as_deref(), Some("carol"));
        assert_eq!(MacBaseInstaller::user_from_first_boot_log("Provisioning: user , x"), None);
    }

    // VZBackendTests.latestBuildFromRunnerOutput
    #[test]
    fn latest_build_from_runner_output() {
        let out = "macOS 27.0.1 (26A434)\nURL:   https://example.invalid/x.ipsw\nSize:  26.6 GB\n";
        assert_eq!(MacBaseInstaller::build_from_latest(out).as_deref(), Some("26A434"));
        assert_eq!(MacBaseInstaller::build_from_latest(""), None);
        assert_eq!(MacBaseInstaller::build_from_latest("a ) ("), None);
    }

    #[test]
    fn percentages_are_reported_once() {
        let last = LastPercent::default();
        assert_eq!(last.next(0), Some(0));
        assert_eq!(last.next(0), None);
        assert_eq!(last.next(1), Some(1));
    }

    #[test]
    fn public_keys_are_ssh_keys() {
        assert!(
            MacBaseInstaller::default_public_keys().iter().all(|k| k.starts_with("ssh-") || k.starts_with("ecdsa-"))
        );
    }

    #[test]
    fn bases_and_fresh_installations_are_named_after_the_build() {
        let (_t, p) = temp_paths();
        let i = installer(&p, "/usr/bin/false");
        assert_eq!(i.fresh("26A434"), p.bases().join("mac-fresh-26A434"));
        assert_eq!(i.base("26A434"), p.bases().join("mac-base-26A434"));
        // a given IPSW: the base is not known up front
        let mut r = BaseRequest::new("u", "U", None);
        r.ipsw = Some("/x.ipsw".into());
        assert_eq!(i.existing_base(&r), None);
        // a runner that cannot ask Apple: unknown
        assert_eq!(i.latest_build(), None);
    }

    /// Plan B166: one base per build and credential, the first keeps the plain name
    #[test]
    fn bases_are_named_per_build_and_credential() {
        let (_t, p) = temp_paths();
        let i = installer(&p, "/usr/bin/false");
        assert_eq!(MacBaseInstaller::credential_slug("  Work Laptop! "), "work-laptop");
        assert_eq!(MacBaseInstaller::credential_slug("ÄÖ"), "");
        let work = ("c-1", "Work");
        let home = ("c-2", "Home");
        // The first base of a build
        assert_eq!(i.base_for("26A434", Some(work), None), p.bases().join("mac-base-26A434"));
        let record = |dir: &Path, (id, name): (&str, &str), own_key: bool| {
            fs::create_dir_all(dir).unwrap();
            BaseRecord {
                credential: id.into(),
                credential_name: name.into(),
                user: "u".into(),
                own_key,
                version: None,
                unchained: false,
            }
            .write(dir)
            .unwrap();
        };
        let first = p.bases().join("mac-base-26A434");
        record(&first, work, false);
        // Its credential finds it again; another credential gets its own name
        assert_eq!(i.base_for("26A434", Some(work), None), first);
        let second = p.bases().join("mac-base-26A434-home");
        assert_eq!(i.base_for("26A434", Some(home), None), second);
        record(&second, home, true);
        assert_eq!(i.base_for("26A434", Some(home), None), second);
        // A slug taken by another credential: numbered; without a slug the ID
        assert_eq!(i.base_for("26A434", Some(("c-3", "home")), None), p.bases().join("mac-base-26A434-home-2"));
        assert_eq!(i.base_for("26A434", Some(("c-4", "…")), None), p.bases().join("mac-base-26A434-c-4"));
        // Replacing: never the base being replaced
        assert_eq!(i.base_for("26A434", Some(work), Some(&first)), p.bases().join("mac-base-26A434-work"));
        // Another build: its plain name again
        assert_eq!(i.base_for("27A1", Some(home), None), p.bases().join("mac-base-27A1"));
        // The newest base per credential (ready ones only)
        assert_eq!(crate::mac_image_builder::MacImageBuilder::newest_base_for(&p, Some("c-2")), None);
        fs::write(second.join(MacBaseInstaller::READY_MARKER), "u\n").unwrap();
        fs::write(first.join(MacBaseInstaller::READY_MARKER), "u\n").unwrap();
        assert_eq!(crate::mac_image_builder::MacImageBuilder::newest_base_for(&p, Some("c-2")), Some(second.clone()));
        assert_eq!(crate::mac_image_builder::MacImageBuilder::newest_base_for(&p, Some("c-1")), Some(first.clone()));
        assert_eq!(crate::mac_image_builder::MacImageBuilder::base_for_credential(&p, Some("c-9")), None);
        let listed = MacBaseInstaller::list(&p);
        assert!(listed.iter().all(|b| b.newest));
        assert_eq!(
            listed.iter().find(|b| b.path == first).and_then(|b| b.record.clone()).map(|r| r.own_key),
            Some(false)
        );
    }

    /// Plan B166: Create Again – the new base takes the old one's name once the old one is gone
    #[test]
    fn a_replaced_base_gives_its_name_to_the_new_one() {
        let (_t, p) = temp_paths();
        let i = installer(&p, "/usr/bin/false");
        let make = |name: &str, build: &str| {
            let dir = p.bases().join(name);
            fs::create_dir_all(&dir).unwrap();
            MacVmBundle::new(&dir)
                .save(&crate::mac_vm::MacVmConfig::new(4, 8192, 80, "02:00:00:00:00:01", Some(build.into())))
                .unwrap();
            BaseRecord {
                credential: "c-1".into(),
                credential_name: "Work".into(),
                user: "u".into(),
                own_key: true,
                version: None,
                unchained: false,
            }
            .write(&dir)
            .unwrap();
            dir
        };
        let old = make("mac-base-26A434", "26A434");
        let new = make("mac-base-26A434-work", "26A434");
        let lines = Mutex::new(Vec::new());
        let now = i.supersede(&old, &new, &|l: &str| lines.lock().unwrap().push(l.to_string())).unwrap();
        assert_eq!(now, old);
        assert!(!new.exists() && BaseRecord::of(&old).is_some());
        assert_eq!(lines.lock().unwrap().len(), 2);
        // A newer build keeps its own name
        let newer = make("mac-base-27A1", "27A1");
        assert_eq!(i.supersede(&old, &newer, &|_: &str| {}).unwrap(), newer);
        assert!(!old.exists());
    }

    /// A stand-in runner whose `install <dir> …` writes a config.json with the build, or fails
    fn install_runner(dir: &Path, build: &str, fail: bool) -> String {
        fs::create_dir_all(dir).unwrap();
        let file = dir.join("omaf-runner");
        let body = if fail {
            "mkdir -p \"$2\"; echo 'Error: restore failed' >&2; exit 3".to_string()
        } else {
            format!(
                "mkdir -p \"$2\" && echo 'Installing: 50 %' && printf '{{\"cpus\":4,\"memoryMB\":8192,\"diskGB\":80,\"display\":{{\"width\":1920,\"height\":1200,\"ppi\":80}},\"macAddress\":\"02:00:00:00:00:01\",\"osBuild\":\"{build}\"}}' > \"$2/config.json\""
            )
        };
        fs::write(&file, format!("#!/bin/sh\n[ \"$1\" = install ] || exit 2\n{body}\n")).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        file.to_string_lossy().into_owned()
    }

    #[test]
    fn a_fresh_installation_is_named_after_its_build_and_read_only() {
        let (_t, p) = temp_paths();
        fs::create_dir_all(p.bases()).unwrap();
        let i = installer(&p, &install_runner(&p.root.join("bin"), "27A1", false));
        let lines = Mutex::new(Vec::new());
        let log = |l: &str| lines.lock().unwrap().push(l.to_string());
        let r = BaseRequest::new("u", "U", None);
        let dir = i.install_fresh(&r, Path::new("/x.ipsw"), &log).unwrap();
        assert_eq!(dir, p.bases().join("mac-fresh-27A1"));
        assert!(!crate::platform::is_writable(&dir));
        assert_eq!(MacVmBundle::new(&dir).load_config().unwrap().os_build.as_deref(), Some("27A1"));
        let lines = lines.into_inner().unwrap();
        assert_eq!(lines[0], "Installing: 50 %");
        assert!(lines[1].starts_with("Installed: ") && lines[1].ends_with("mac-fresh-27A1 (read-only, never started)"));
        // the same build again: the existing one is used, the new installation deleted
        std::thread::sleep(Duration::from_millis(1100)); // another .install-<seconds> folder
        let again = i.install_fresh(&r, Path::new("/x.ipsw"), &|_: &str| {}).unwrap();
        assert_eq!(again, dir);
        let left: Vec<_> = fs::read_dir(p.bases()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("mac-fresh-27A1")]);
        Disposal::delete(&[&dir]).unwrap();
    }

    #[test]
    fn a_failed_installation_deletes_its_folder() {
        let (_t, p) = temp_paths();
        fs::create_dir_all(p.bases()).unwrap();
        let i = installer(&p, &install_runner(&p.root.join("bin"), "", true));
        let lines = Mutex::new(Vec::new());
        let log = |l: &str| lines.lock().unwrap().push(l.to_string());
        let e = i.install_fresh(&BaseRequest::new("u", "U", None), Path::new("/x.ipsw"), &log).unwrap_err();
        assert_eq!(e.message(), "Installation failed (3); the incomplete folder was deleted");
        assert_eq!(fs::read_dir(p.bases()).unwrap().count(), 0);
        assert_eq!(*lines.lock().unwrap(), vec!["Error: restore failed"]);
    }

    #[test]
    fn a_downloaded_ipsw_is_not_fetched_again() {
        let (_t, p) = temp_paths();
        let i = installer(&p, "/usr/bin/false");
        let release = MacOsRelease {
            version: "27.0.1".into(),
            build: "26A434".into(),
            url: "https://example.invalid/x.ipsw".into(),
            bytes: Some(1),
        };
        fs::create_dir_all(p.ipsw()).unwrap();
        fs::write(p.ipsw().join("26A434.ipsw"), "x").unwrap();
        let file = i.download_ipsw(&release, 0, &|_| panic!("no download"), &|_, _| {}, &|_| {}).unwrap();
        assert_eq!(file, p.ipsw().join("26A434.ipsw"));
    }
}
