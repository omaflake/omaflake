// SPDX-License-Identifier: AGPL-3.0-or-later
//! Upgrading a running VM in place (plan B178): the newer image's system goes into the guest and is switched to –
//! the VM keeps its disk, its files and its settings. A plain upgrade (`VmBackend::upgrade`) gives the VM a fresh disk
//! from the new image instead, and everything in it is gone.
//!
//! 1. The system of the target image: NixOS `nixosConfigurations."vm-<system>-<recipe>".config.system.build.toplevel`,
//!    macOS the nix-darwin system the image was activated with (its `storePath`, else `darwinConfigurations.<recipe>`),
//!    built with the image's credential, look and locale – what a build of the image needs, mostly at hand already.
//! 2. `nix copy --no-check-sigs` into the guest over SSH (the guest's user is one of its Nix's trusted users: NixOS
//!    base.nix `@wheel`, macOS images since BM19).
//! 3. Switched to: NixOS `nix-env -p /nix/var/nix/profiles/system --set` and `switch-to-configuration switch` through
//!    the sudo rule images from 0.8.0 on carry (vm-guest.nix); macOS `<system>/activate` with the base credential's
//!    password on sudo's stdin, as the image build does.
//!
//! The VM's image stays its disk's base; `InstanceConfig::system_image` names the image whose system runs now.

use crate::error::{Result, codes};
use crate::image_store::{Image, ImageStore};
use crate::models::{GuestKind, InstanceConfig};
use crate::paths::Paths;
use crate::platform::Platform;
use crate::shell::{RunOptions, Shell, StreamOptions};
use crate::vm_err;

pub struct InPlace {
    pub paths: Paths,
}

/// ssh's options for `nix copy` (NIX_SSHOPTS, split at spaces – so no paths): a VM's host key is new per instance,
/// and the copy goes only to the VM's own address
const NIX_SSHOPTS: &str = "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR";

impl InPlace {
    pub fn new(paths: Paths) -> Self {
        InPlace { paths }
    }

    /// The flake output of the system a recipe's VM runs
    pub fn system_attribute(kind: GuestKind, recipe: &str) -> Result<String> {
        Ok(match kind {
            GuestKind::Nixos => {
                let Some(system) = Platform::host_system(GuestKind::Nixos) else {
                    return Err(vm_err!(codes::UNSUPPORTED_SYSTEM, "NixOS guests do not run on this computer"));
                };
                format!("nixosConfigurations.\"vm-{system}-{recipe}\".config.system.build.toplevel")
            }
            GuestKind::Macos => format!("darwinConfigurations.\"{recipe}\".system"),
        })
    }

    /// The store path of `image`'s system on this computer – built when it is not here (any longer)
    pub fn system(&self, image: &Image, flake: &str, log: &(dyn Fn(&str) + Send + Sync)) -> Result<String> {
        if image.info.kind == GuestKind::Macos
            && let Some(p) = image.info.store_path.as_deref().filter(|p| std::path::Path::new(p).exists())
        {
            return Ok(p.to_string());
        }
        let store = ImageStore::new(self.paths.clone());
        let attribute = Self::system_attribute(image.info.kind, image.recipe())?;
        let choice = image.info.choice(&self.paths);
        let mut args = vec![
            "build".to_string(),
            format!("{}#{attribute}", ImageStore::flake_ref(flake)),
            "--no-link".into(),
            "--print-out-paths".into(),
            "-L".into(),
        ];
        args.extend(store.guest_arguments_for(flake, &choice));
        args.extend(crate::linux_builder::LinuxBuilder::new(self.paths.clone()).arguments());
        log(&format!("Building the system of {}", image.name));
        let r = Shell::stream("nix", &args, &StreamOptions::default(), log)?;
        let path = r.stdout.lines().rfind(|l| l.starts_with("/nix/store/")).map(str::to_string);
        match path {
            Some(p) if r.status == 0 => Ok(p),
            _ => Err(vm_err!("Building the system of {} failed ({}) – see the lines above", image.name, r.status)),
        }
    }

    /// The Nix store of the running VM `c` at `host` (and `port`), for `nix copy`
    pub fn store_url(&self, c: &InstanceConfig, host: &str, port: Option<u16>) -> String {
        let key = crate::ssh_identity::SshIdentity::new(&self.paths).key();
        let key = key.to_string_lossy().replace('%', "%25").replace(' ', "%20");
        let at = match port {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        let mut url = format!("ssh-ng://{}@{at}?ssh-key={key}", c.guest_user());
        // Non-interactive SSH into macOS has no Nix in PATH (BM19)
        if c.guest_kind() == GuestKind::Macos {
            url.push_str("&remote-program=/nix/var/nix/profiles/default/bin/nix-daemon");
        }
        url
    }

    /// macOS (plan B189): the command in the guest (sh, the password on stdin) that makes `user` one of its Nix's
    /// trusted users for the copy. The image build adds the line to the installer's nix.conf, but nix-darwin's
    /// activation replaces the file with its own (`trusted-users = root`; images from this fix on name the user) – so
    /// `nix copy --no-check-sigs` was refused. The symlink becomes a file with the line, and the daemon is restarted;
    /// the switch puts nix-darwin's symlink back before it activates.
    pub fn trust_command(user: &str) -> String {
        let u = crate::builder_maintenance::shell_quote(user);
        let script = format!(
            "f=/etc/nix/nix.conf; grep -Eq \"^trusted-users = (.* )?\"{u}\"( |$)\" \"$f\" && exit 0; \
             t=$(mktemp) && grep -v '^trusted-users' \"$f\" > \"$t\" && echo \"trusted-users = root \"{u} >> \"$t\" && \
             rm -f \"$f\" && mv \"$t\" \"$f\" && chmod 644 \"$f\" && \
             launchctl kickstart -k system/org.nixos.nix-daemon && sleep 3"
        );
        format!("sudo -S -p '' sh -c {}", crate::builder_maintenance::shell_quote(&script))
    }

    /// The command in the guest that switches to `system` (sh); macOS reads the password from stdin
    pub fn switch_command(kind: GuestKind, system: &str) -> String {
        let q = crate::builder_maintenance::shell_quote(system);
        match kind {
            // Exactly the two commands vm-guest.nix allows without a password
            GuestKind::Nixos => format!(
                "sudo -n /run/current-system/sw/bin/nix-env -p /nix/var/nix/profiles/system --set {q} && \
                 sudo -n /nix/var/nix/profiles/system/bin/switch-to-configuration switch"
            ),
            GuestKind::Macos => format!(
                "sudo -S -p '' sh -c {}",
                crate::builder_maintenance::shell_quote(&format!(
                    "{{ [ -L /etc/nix/nix.conf ] || ln -sfn /etc/static/nix/nix.conf /etc/nix/nix.conf; }} && \
                     /nix/var/nix/profiles/default/bin/nix-env -p /nix/var/nix/profiles/system --set {q} && {q}/activate"
                ))
            ),
        }
    }

    /// Copies `system` into the running VM and switches to it. `ssh` is the VM's ssh command line without the remote
    /// command (`VmBackend::ssh_arguments(c, &[])`); `password` the guest user's (macOS).
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &self,
        c: &InstanceConfig,
        system: &str,
        store: &str,
        ssh: &[String],
        password: Option<&str>,
        log: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<()> {
        let kind = c.guest_kind();
        let (program, rest) = ssh.split_first().ok_or_else(|| vm_err!("No ssh command for {}", c.name))?;
        let input = password.map(|p| format!("{p}\n").into_bytes());
        if kind == GuestKind::Macos {
            log("Letting the guest's Nix take the new system");
            let mut args = rest.to_vec();
            args.push(Self::trust_command(&c.guest_user()));
            let r = Shell::run(program, &args, &RunOptions { input: input.as_deref(), ..Default::default() })?;
            for line in format!("{}{}", r.stdout, r.stderr).lines().filter(|l| !l.trim().is_empty()) {
                log(line);
            }
            if r.status != 0 {
                return Err(vm_err!("Making {}'s user trusted by its Nix failed ({})", c.name, r.status));
            }
        }
        log("Copying the new system into the VM");
        let env = [("NIX_SSHOPTS", NIX_SSHOPTS)];
        let copy = ["copy", "--no-check-sigs", "--to", store, system];
        let r = Shell::stream("nix", &copy, &StreamOptions { environment: &env, ..Default::default() }, log)?;
        if r.status != 0 {
            return Err(vm_err!("Copying the new system into {} failed ({}) – see the lines above", c.name, r.status));
        }
        log("Switching to the new system");
        let mut args = rest.to_vec();
        args.push(Self::switch_command(kind, system));
        let r = Shell::run(program, &args, &RunOptions { input: input.as_deref(), ..Default::default() })?;
        for line in format!("{}{}", r.stdout, r.stderr).lines().filter(|l| !l.trim().is_empty()) {
            log(line);
        }
        if r.status != 0 {
            let output = format!("{}{}", r.stdout, r.stderr);
            if kind == GuestKind::Nixos && output.contains("a password is required") {
                return Err(vm_err!(
                    codes::PREREQUISITE,
                    "{} cannot switch its system without a password – its image is older than omaflake 0.8.0. Upgrade it once with a fresh disk (Upgrade to New Image …).",
                    c.name
                ));
            }
            return Err(vm_err!("Switching {} to the new system failed ({})", c.name, r.status));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_attributes() {
        assert_eq!(
            InPlace::system_attribute(GuestKind::Macos, "omaflake.macos.dev").unwrap(),
            "darwinConfigurations.\"omaflake.macos.dev\".system"
        );
        if let Some(system) = Platform::host_system(GuestKind::Nixos) {
            assert_eq!(
                InPlace::system_attribute(GuestKind::Nixos, "omaflake.nixos.dev").unwrap(),
                format!("nixosConfigurations.\"vm-{system}-omaflake.nixos.dev\".config.system.build.toplevel")
            );
        }
    }

    #[test]
    fn switch_commands() {
        let n = InPlace::switch_command(GuestKind::Nixos, "/nix/store/abc-nixos-system");
        assert_eq!(
            n,
            "sudo -n /run/current-system/sw/bin/nix-env -p /nix/var/nix/profiles/system --set '/nix/store/abc-nixos-system' && \
             sudo -n /nix/var/nix/profiles/system/bin/switch-to-configuration switch"
        );
        let m = InPlace::switch_command(GuestKind::Macos, "/nix/store/abc-darwin-system");
        assert!(m.starts_with("sudo -S -p '' sh -c '"));
        assert!(m.contains(
            "--set '\\''/nix/store/abc-darwin-system'\\'' && '\\''/nix/store/abc-darwin-system'\\''/activate"
        ));
        assert!(m.contains("ln -sfn /etc/static/nix/nix.conf /etc/nix/nix.conf"), "{m}");
        let t = InPlace::trust_command("me");
        assert!(
            t.starts_with("sudo -S -p '' sh -c '") && t.contains("trusted-users = root") && t.contains("nix-daemon"),
            "{t}"
        );
    }

    #[test]
    fn store_urls() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::new(t.path().join("data with space"), None);
        let up = InPlace::new(p.clone());
        let mut c = crate::models::tests::config("dev");
        c.user = Some("me".into());
        let url = up.store_url(&c, "127.0.0.1", Some(2222));
        assert!(url.starts_with("ssh-ng://me@127.0.0.1:2222?ssh-key="), "{url}");
        assert!(url.contains("data%20with%20space") && !url.contains(' '), "{url}");
        assert!(!url.contains("remote-program"));
    }
}
