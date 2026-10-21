---
title: macOS base
nav_order: 9
permalink: /macos-base/
---

# macOS base `mac-base-<os-build>`

**Since 2026-10-16 the base is built automatically** by `omaf macos install` (app: "Install macOS Base …", plan BM29):
IPSW from Apple → `mac-fresh-<build>` → clone → headless first boot with provisioning (user, automatic login, SSH) →
your `~/.ssh/*.pub` into `authorized_keys` → shutdown → read-only. The checklist below documents the manual M8 run
that established these steps; it is kept as background, nothing in it needs to be done by hand any more.

## Background: checklist of the manual run (M8)

The base contains **only** what cannot be done by script or nix-darwin.
Everything else (apps, Dock, defaults, wallpaper) is added declaratively in M9.
Widgets: off in all VMs (nix-darwin, M9) – do not arrange anything here.
For every new macOS version the base is recreated following this list – record every action here.

**Rules**
- Foundation is a **fresh installation from Apple's IPSW** with `omaf-runner install` (M7) – no tart, no Cirrus Labs images (spec §11).
- No Apple ID, no iCloud, no license sign-ins (Microsoft 365 etc.) – those belong in instances.
- No secrets in the repo; passwords only in your head/password manager.
- Anything uncertain: note it here as OPEN instead of guessing.

## Starting point

- IPSW: macOS 27.0.1, build `26A434` (26.6 GB), Apple `latestSupported` (plan BM12), stored in `~/.omaflake/ipsw/`
- Installed with: `omaf-runner install --latest` into `~/.omaflake/bases/mac-fresh-26A434` (never started, read-only);
  the base is an APFS clone of it: `cp -c -R mac-fresh-26A434 mac-base-26A434 && chmod -R u+w mac-base-26A434`
- CPU/RAM/display during setup: 4 cores / 8 GiB / 1920×1200 at 80 ppi, disk 80 GB thin (13 GB used after the first boot)

## Steps (tick off, note deviations)

- [x] First boot with provisioning (plan BM11): `omaf macos install` (asks for the password and passes it to `omaf-runner run … --window --user <user> --full-name "…" --password-stdin --auto-login --ssh`) – user, auto-login and SSH without the Setup Assistant; afterwards check which dialogs still appear – 2026-10-16: boot to SSH in ~20 s; dialogs: none
- [x] Language/region/keyboard: kept the defaults – language `en`, region `en_US`, keyboard `ABC`; per-image changes via nix-darwin
- [x] Apple ID: **skip** – checked: no account (`defaults read MobileMeAccounts` → domain not found)
- [x] Guest user (via provisioning; administrator? check); password only in the password manager – user is an administrator (group `admin`), UID 501, shell `/bin/zsh`; `sudo` asks for the password
- [x] Dialogs off: Location Services, Analytics/Diagnostics, Siri, Screen Time, FileVault (off – OPEN whether needed), "What's New" window – FileVault checked: off (`fdesetup status`); 2026-10-16 in the window: **no dialogs** appeared after the provisioned login
- [x] Note macOS version: `sw_vers` → macOS 27.0.1 (`26A434`)
- [x] Automatic login for the guest user – via provisioning; check that it works – `autoLoginUser` is set
- [ ] Software Update: automatic updates off (images are rebuilt deliberately) – proposal: not in the base; `darwin-guest.nix` sets `AutomaticallyInstallMacOSUpdates = false` in every image (automatic checking is still on in the base)
- [x] Energy saver/screen saver: not in the base – scriptable: `darwin-guest.nix` sets `power.sleep.computer/display = "never"` (in the base: display sleep 10 min)
- [x] Remote Login (SSH) – via provisioning; check – `OpenSSH_10.3` answers
- [x] SSH key: your public SSH key into `~<user>/.ssh/authorized_keys` (e.g. via the share folder) – done once over SSH with the password
- [x] Check from the Mac: `ssh <user>@<IP>` with key, without password – `ssh -o BatchMode=yes` works
- [ ] DECISION: disable password login via SSH (base or nix-darwin) – proposal: nix-darwin (`darwin-guest.nix` sets `PasswordAuthentication no`), so the base keeps password login until the image build has put the key in place
- [x] DECISION: install Nix here or only in the image build script (M9) – decided 2026-10-16: **not in the base**; the build script installs it
- [x] Check: no Apple ID signed in (System Settings, top left) – `MobileMeAccounts` domain missing
- [x] Shut down; make the folder read-only (`chmod -R a-w`) – from now on only clone it (`omaf-runner clone`) – done 2026-10-16

## Notes

- 2026-10-16: development base with the guest user `test` (to be rebuilt with the real user once the pipeline works).
- No Nix in the base (`/nix` missing, no `/etc/synthetic.conf`) – the M9 build script installs it.
