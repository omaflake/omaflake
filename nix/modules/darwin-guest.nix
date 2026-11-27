# SPDX-License-Identifier: AGPL-3.0-or-later
# Guest module for macOS recipes (nix-darwin), applied on top of a base (docs/macos-base.md, spec §6.5).
# The guest user (`macUser`, set by omaflake from the base) already exists in the base (created by provisioning); nix-darwin does not manage it
# (it is not in users.knownUsers), it only adds settings for it.
{ config, lib, pkgs, settings, ... }:

let
  user = settings.macUser;
  # omaflake's FSKit module in omaflake Tools (unchained images, plan B190) and where its volumes' folders are mounted
  fskitModule = "/Applications/omaflake Tools.app/Contents/Extensions/omaflake-volumes.appex";
  fskitVolumes = "/private/var/omaflake/volumes";
  # The omaflake agent (spec §11.9): in macOS guests it carries MIDI (plan B161)
  omaflakeAgent = pkgs.callPackage ../agent/package.nix { };

  # Language and region from `locale` in local/settings.nix: "de_DE.UTF-8" → de_DE / de-DE
  localeId = builtins.head (lib.splitString "." settings.locale);
  language = builtins.replaceStrings [ "_" ] [ "-" ] localeId;
  languages = lib.unique [ language "en-US" ];

  # Keyboard layout from `keyboard`; values read from a guest after choosing the layout by hand (plan BM24).
  # Other layouts: set `macKeyboardLayout = { id = …; name = "…"; }` in local/settings.nix.
  # One macOS layout per XKB name the app offers (GuestChoices.keyboards): the country's default layout on macOS.
  # Names and IDs from macOS 27's own table (AppleKeyboardLayouts-L.dat, plan B46); de/us also checked in a guest (BM24).
  knownLayouts = {
    us = { id = 252; name = "ABC"; };               # macOS default
    gb = { id = 2; name = "British"; };
    # The Mac's classic "German" (@ = Option+L, as base.nix says) – not "German – Standard" (German-DIN-2137, id
    # -18133), where @ and other keys sit elsewhere (maintainer, 2026-11-22: @ came out as "˛")
    de = { id = 3; name = "German"; };
    ch = { id = 19; name = "Swiss German"; };
    at = { id = 92; name = "Austrian"; };
    fr = { id = 1; name = "French"; };
    es = { id = 87; name = "Spanish - ISO"; };
    it = { id = 4; name = "Italian"; };
    nl = { id = 26; name = "Dutch"; };
    se = { id = 224; name = "Swedish"; };
    no = { id = 12; name = "Norwegian"; };
    dk = { id = 9; name = "Danish"; };
    fi = { id = 17; name = "Finnish"; };
    pl = { id = 30788; name = "Polish Pro"; };       # XKB "pl" is the programmers' layout
    pt = { id = 10; name = "Portuguese"; };
    cz = { id = -14193; name = "Czech"; };           # QWERTZ like XKB "cz"
    # plan B141, read from macOS 27's AppleKeyboardLayouts-L.dat like the others (B46): the PC layouts, as XKB's
    ru = { id = 19458; name = "RussianWin"; };       # "Russian – PC" (ЙЦУКЕН as on Windows, like XKB "ru")
    ua = { id = -2354; name = "Ukrainian-PC"; };
    gr = { id = -18944; name = "Greek"; };
  };
  # The locale's keyboards (plan B141): several input sources, the first selected; macOS switches with its own keys
  keyboards = if (settings.keyboards or [ ]) != [ ] then settings.keyboards else [ settings.keyboard ];
  layoutOf = k: knownLayouts.${k}
    or (throw "darwin-guest: no macOS layout known for keyboard \"${k}\" – set macKeyboardLayout in local/settings.nix");
  layouts = if settings ? macKeyboardLayout then [ settings.macKeyboardLayout ] else map layoutOf keyboards;
  layout = builtins.head layouts;
  sourceOf = l: {
    InputSourceKind = "Keyboard Layout";
    "KeyboardLayout ID" = l.id;
    "KeyboardLayout Name" = l.name;
  };
  layoutSource = sourceOf layout;
in
{
  nixpkgs.hostPlatform = "aarch64-darwin";

  # Settings that act on a user (Dock, defaults, homebrew) apply to the primary user
  system.primaryUser = user;
  users.users.${user} = {
    home = "/Users/${user}";
    # Public keys from omaflake (your ~/.ssh) and local/keys/ – for `omaf ssh` and the image build
    openssh.authorizedKeys.keyFiles = settings.authorizedKeyFiles;
    openssh.authorizedKeys.keys = settings.authorizedKeys;
  };

  # Clipboard with the Mac (omaf-runner --clipboard → virtio console port org.omaflake.clipboard), like NixOS guests.
  # Not a LaunchAgent: macOS would announce it as a background item in every clone (plan BM25). omaflake starts it
  # over SSH once the guest answers (VZBackend.waitForSSH, with the host name); an SSH session reaches the
  # logged-in user's pasteboard. Exits at once when the VM runs without the port.
  environment.systemPackages = [
    (pkgs.writeShellScriptBin "omaflake-clipboard-agent" ''
      pgrep -f clipboard-agent-macos.py >/dev/null && exit 0
      nohup ${pkgs.python3}/bin/python3 ${./clipboard-agent-macos.py} >/tmp/omaflake-clipboard.log 2>&1 </dev/null &
    '')
    # The omaflake agent, for MIDI (plan B161): it makes the Mac's MIDI devices virtual endpoints here, which only the
    # logged-in user's apps see when it runs in that user's session – so omaflake starts it over SSH like the clipboard
    # helper (BM25), as the guest user. Whether CoreMIDI reaches the user's MIDI server from an SSH session: OPEN
    # (pbpaste does reach the pasteboard). Exits at once when the VM runs without the agent port.
    (pkgs.writeShellScriptBin "omaflake-agent-start" ''
      pgrep -xq omaflake-agent && exit 0
      # The log outlives restarts (/tmp is emptied at each): the last five starts are kept, so what the agent saw
      # before a restart that "fixed it" can still be read (plan B183)
      logs="$HOME/Library/Logs/omaflake"
      mkdir -p "$logs"
      for n in 4 3 2 1; do
        [ -f "$logs/agent.$n.log" ] && mv -f "$logs/agent.$n.log" "$logs/agent.$((n + 1)).log"
      done
      [ -f "$logs/agent.log" ] && mv -f "$logs/agent.log" "$logs/agent.1.log"
      nohup ${omaflakeAgent}/bin/omaflake-agent >"$logs/agent.log" 2>&1 </dev/null &
    '')
    # Volumes in the Finder's sidebar (plan B189): macOS lists under Locations only what DiskArbitration mounts – of
    # virtio-fs only the device with its automount tag ("My Shared Files"; the guest's virtiofs.fs takes media with
    # the content hint AppleVirtIOFS, and only that tag has one). A volume mounted with mount_virtiofs is neither local
    # nor a server's (`NSURLVolumeIsLocalKey` false), so the Finder shows it nowhere. Until omaflake Tools brings its
    # own file system (FSKit, next step), each volume is a favorite: this adds the mounted ones and takes out those it
    # added whose volume is gone (its list in Application Support) – the user's own favorites stay. omaflake starts it
    # over SSH with the helpers above, after each start (volumes change only then): from a launch agent the favorites
    # took no path under /Volumes (checked in a macOS 27 guest – /tmp worked, so most likely the privacy rules for
    # network volumes; OPEN), over SSH they do – and a launch agent would bring a background item notice at every
    # start (BM25). It waits up to 30 s for the volumes omaflake-volumes mounts.
    # Plan B190: where the volumes' folders are hidden (an unchained image with omaflake's FSKit module), it first
    # mounts each as a volume of the module at /Volumes/<tag> – under Locations like an external disk; it registers
    # the module and switches it on (FSKit runs only modules in the user's enabledModules.plist – what System
    # Settings writes – and fskit_agent reads that at its start). Those are no favorites.
    (pkgs.writeShellScriptBin "omaflake-volumes-session" ''
      mkdir -p "$HOME/Library/Logs/omaflake"
      exec >> "$HOME/Library/Logs/omaflake/volumes-session.log" 2>&1
      tags=$(/usr/sbin/ioreg -r -c AppleVirtIOFS -d 1 2>/dev/null | /usr/bin/grep -c '"AppleVirtIOFSTag"' || true)
      n=0
      while [ "$(/sbin/mount | /usr/bin/grep -c '^virtio-fs on ')" -lt "''${tags:-0}" ] && [ "$n" -lt 15 ]; do
        n=$((n + 1)); /bin/sleep 2
      done
      if [ -d ${fskitVolumes} ] && [ -d "${fskitModule}" ]; then
        if ! /usr/bin/pluginkit -m -i app.omaflake.guest.volumes | /usr/bin/grep -q volumes; then
          /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
            -f -R "/Applications/omaflake Tools.app"
          /usr/bin/pluginkit -a "${fskitModule}"
          /bin/sleep 2
        fi
        # FSKit makes enabledModules.plist at its first use in a fresh VM (checked) – so: mount, and when the module is
        # off, switch it on and try again
        enable() {
          enabled="$HOME/Library/Group Containers/group.com.apple.fskit.settings/enabledModules.plist"
          [ -f "$enabled" ] || return 0
          /usr/bin/plutil -p "$enabled" | /usr/bin/grep -q '"app.omaflake.guest.volumes"' && return 0
          /usr/bin/plutil -insert 0 -string app.omaflake.guest.volumes "$enabled"
          /bin/launchctl kickstart -k "user/$(/usr/bin/id -u)/com.apple.fskit.fskit_agent"
          echo "$(date): switched the FSKit module on"
          /bin/sleep 3
        }
        for folder in ${fskitVolumes}/*; do
          [ -d "$folder" ] || continue
          tag=$(/usr/bin/basename "$folder")
          /sbin/mount | /usr/bin/grep -qF " on /Volumes/$tag (" && continue
          for try in 1 2 3 4 5; do
            /sbin/mount -F -t omaflakefs "$folder" "/Volumes/$tag" && { echo "$(date): mounted $tag"; break; }
            enable
            /bin/sleep 2
          done
        done
      fi
      set --
      while IFS= read -r p; do
        [ -n "$p" ] && set -- "$@" "$p"
      done <<EOF
      $(/sbin/mount | /usr/bin/sed -n 's|^virtio-fs on \(/Volumes/.*\) (AppleVirtIOFS.*|\1|p')
      EOF
      echo "$(date): favorites: $*"
      exec /usr/bin/osascript -l JavaScript ${pkgs.writeText "omaflake-volume-favorites.js" ''
      // argv: <state file> <mounted volume path>... – keeps the Finder sidebar's favorites in step with the volumes
      ObjC.import('CoreServices');
      function run(argv) {
        const state = argv[0];
        const real = p => $(p).stringByResolvingSymlinksInPath.js;
        const mounted = argv.slice(1).map(real);
        const s = $.NSString.stringWithContentsOfFileEncodingError(state, $.NSUTF8StringEncoding, null);
        const ours = s.isNil() ? [] : s.js.split('\n').filter(l => l).map(real);
        const list = $.LSSharedFileListCreate($(), $.kLSSharedFileListFavoriteItems, $());
        const items = ObjC.castRefToObject($.LSSharedFileListCopySnapshot(list, null));
        const present = [];
        const log = [];
        for (let i = 0; i < items.count; i++) {
          const item = items.objectAtIndex(i);
          let path = null;
          try { path = real(ObjC.castRefToObject($.LSSharedFileListItemCopyResolvedURL(item, 3, null)).path.js); } catch (e) {}
          const name = ObjC.castRefToObject($.LSSharedFileListItemCopyDisplayName(item)).js;
          // One of ours whose volume is gone: its path is no volume now, or it no longer resolves (then by its name)
          const gone = ours.find(p => mounted.indexOf(p) < 0 && (path === p || (path === null && name === p.split('/').pop())));
          if (gone) { $.LSSharedFileListItemRemove(list, item); log.push('removed ' + gone); continue; }
          if (path) present.push(path);
        }
        // At login an insert can get lost (the list's service not ready yet): check it, and try again for 20 s
        const listed = () => {
          const now = ObjC.castRefToObject($.LSSharedFileListCopySnapshot(list, null));
          const paths = [];
          for (let i = 0; i < now.count; i++) {
            try {
              paths.push(real(ObjC.castRefToObject(
                $.LSSharedFileListItemCopyResolvedURL(now.objectAtIndex(i), 3, null)).path.js));
            } catch (e) {}
          }
          return paths;
        };
        for (let attempt = 0; attempt < 10; attempt++) {
          if (attempt > 0) delay(2);
          const missing = mounted.filter(p => (attempt === 0 ? present : listed()).indexOf(p) < 0);
          if (missing.length === 0) break;
          for (const p of missing) {
            $.LSSharedFileListInsertItemURL(list, $.kLSSharedFileListItemLast, $(), $(), $.NSURL.fileURLWithPath(p), $(), $());
            log.push('added ' + p + (attempt > 0 ? ' (again)' : ""));
          }
        }
        $.NSFileManager.defaultManager.createDirectoryAtPathWithIntermediateDirectoriesAttributesError(
          $(state).stringByDeletingLastPathComponent, true, $(), null);
        $(mounted.join('\n') + '\n').writeToFileAtomicallyEncodingError(state, true, $.NSUTF8StringEncoding, null);
        return log.join('\n');
      }
      ''} "$HOME/Library/Application Support/omaflake/volume-favorites" "$@"
    '')
  ];

  # Remote Login is switched on by provisioning in the base; keys only, no passwords (plan BM11)
  services.openssh.enable = true;
  services.openssh.extraConfig = ''
    PasswordAuthentication no
    KbdInteractiveAuthentication no
  '';

  # omaflake stops a VM via SSH: `sudo shutdown -h now` without a password,
  # because requestStop() hangs on the confirmation dialog when headless (plan BM17).
  # It also names each instance after itself (scutil, plan BM28); networking.hostName is the image's name.
  security.sudo.extraConfig = ''
    ${user} ALL=(root) NOPASSWD: /sbin/shutdown, /usr/sbin/scutil --set HostName *, /usr/sbin/scutil --set LocalHostName *, /usr/sbin/scutil --set ComputerName *
  '';

  nix.enable = true;
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  # The guest user may add the Mac's unsigned paths (`nix copy --no-check-sigs`, BM19) – an upgrade in place (B178).
  # The image build adds this line to the installer's nix.conf, but nix-darwin writes its own over it (plan B189).
  # (nix-darwin's default `root` stays: the list is merged)
  nix.settings.trusted-users = [ user ];

  time.timeZone = lib.mkDefault settings.timeZone;

  # Language, region and keyboard for the guest user and system-wide (takes effect at the next login)
  system.defaults.CustomUserPreferences.NSGlobalDomain = {
    AppleLanguages = languages;
    AppleLocale = localeId;
  };
  system.defaults.CustomSystemPreferences."/Library/Preferences/.GlobalPreferences" = {
    AppleLanguages = languages;
    AppleLocale = localeId;
  };
  system.defaults.CustomUserPreferences."com.apple.HIToolbox" = {
    AppleEnabledInputSources = map sourceOf layouts ++ [
      { "Bundle ID" = "com.apple.CharacterPaletteIM"; InputSourceKind = "Non Keyboard Input Method"; }
    ];
    AppleSelectedInputSources = [ layoutSource ];
    AppleInputSourceHistory = [ layoutSource ];
    AppleCurrentKeyboardLayoutInputSourceID = "com.apple.keylayout.${layout.name}";
  };

  # Widgets off in all VMs (plan BM1); a VM never sleeps on its own
  system.defaults.WindowManager.StandardHideWidgets = true;
  system.defaults.WindowManager.StageManagerHideWidgets = true;
  power.sleep.computer = "never";
  power.sleep.display = "never";
  system.defaults.screensaver.askForPassword = false;

  # Background items under a name (plan BM25): macOS announces every launch daemon as a background item, named after
  # the program it starts – nix-darwin's own and omaflake's run through /bin/sh (they wait for the Nix store) and were
  # announced as "sh". They start through /usr/local/libexec/omaflake instead – outside /nix, so it is there at boot –
  # and are announced as "omaflake" (checked in a macOS 27 guest: a job started through a script of that kind is
  # recorded under the script's name). AssociatedBundleIdentifiers (omaflake Tools) was ignored: most likely macOS
  # takes it only when the program and the app share a team (OPEN, with a Developer ID). nix-darwin has no option for
  # a job's plist beyond its serviceConfig, so the plists of the jobs that start /bin/sh are written again.
  system.activationScripts.extraActivation.text = lib.mkAfter ''
    mkdir -p /usr/local/libexec
    install -m 0755 ${pkgs.writeText "omaflake-launchd-start" ''
      #!/bin/sh
      # omaflake's start for launchd jobs (plan BM25): macOS names a background item after this file
      exec "$@"
    ''} /usr/local/libexec/omaflake
  '';
  environment.launchDaemons = lib.mapAttrs' (_: daemon: {
    name = "${daemon.serviceConfig.Label}.plist";
    value.text = lib.mkForce (lib.generators.toPlist { escape = true; } (daemon.serviceConfig
      // lib.optionalAttrs (lib.take 1 (daemon.serviceConfig.ProgramArguments or [ ]) == [ "/bin/sh" ]) {
        ProgramArguments = [ "/usr/local/libexec/omaflake" ] ++ daemon.serviceConfig.ProgramArguments;
      }));
  }) config.launchd.daemons;

  # Cards on omaflake's networks between VMs (spec §11.12) come and go with the VM's networks: at every boot macOS
  # gets a network service (DHCP) for a card it has not seen. The switch hands out no router, so the default route
  # stays on the VM's own NAT. OPEN: checked in a guest with a second card.
  launchd.daemons.omaflake-network-cards.serviceConfig = {
    ProgramArguments = [ "/usr/sbin/networksetup" "-detectnewhardware" ];
    RunAtLoad = true;
  };

  # Volumes (plan B187): folders of the host, one virtio-fs device each; its tag is the name the user gave the volume in
  # this VM, mounted at /Volumes/<tag> – the same path as in NixOS guests. The tags are in the I/O Registry
  # (AppleVirtIOFS, AppleVirtIOFSTag – checked in a macOS 27 guest); macOS's own automount tag is left to macOS.
  # Plan B189: at boot launchd starts daemons before the Nix store's volume is mounted – the script in /nix/store was
  # not found (exit 127, the 0.8.1 images), so it waits for /nix/store like nix-darwin's own daemons
  # (`/bin/wait4path`); the devices may not be in the I/O Registry yet either – it waits up to a minute for them.
  # Each is mounted by root with the guest user as the owner of its files (`-u`/`-g`; checked in a macOS 27 guest: the
  # host's files writable by the user). Plan B190: where omaflake Tools carries the FSKit module and AMFI lets it start
  # (an unchained image: the boot argument), the virtio-fs mount is hidden at ${fskitVolumes}/<tag> and /Volumes/<tag> is
  # left to the user's session, which mounts the module's volume over it (omaflake-volumes-session).
  launchd.daemons.omaflake-volumes.serviceConfig = {
    ProgramArguments = [ "/bin/sh" "-c" "/bin/wait4path /nix/store && exec /bin/sh ${pkgs.writeText "omaflake-volumes.sh" ''
      set -u
      exec >> /var/log/omaflake-volumes.log 2>&1
      tags() {
        list=$(/usr/sbin/ioreg -r -c AppleVirtIOFS -d 1 -a 2>/dev/null) || return 0
        [ -n "$list" ] || return 0
        i=0
        while tag=$(printf '%s' "$list" | /usr/bin/plutil -extract "$i.AppleVirtIOFSTag" raw -o - - 2>/dev/null); do
          i=$((i + 1))
          case "$tag" in ""|.*|*/*|com.apple.virtio-fs.automount) continue ;; esac
          printf '%s\n' "$tag"
        done
      }
      fskit=no
      if [ -d "${fskitModule}" ] && /usr/sbin/nvram boot-args 2>/dev/null | /usr/bin/grep -q amfi_get_out_of_my_way=1; then
        fskit=yes
        /bin/mkdir -p ${fskitVolumes}
        /bin/chmod 755 /private/var/omaflake ${fskitVolumes}
      fi
      n=0
      while [ -z "$(tags)" ] && [ "$n" -lt 30 ]; do n=$((n + 1)); /bin/sleep 2; done
      tags | while IFS= read -r tag; do
        at="/Volumes/$tag"
        if [ "$fskit" = yes ]; then
          at="${fskitVolumes}/$tag"
          /bin/mkdir -p "/Volumes/$tag"
          /usr/sbin/chown ${user}:staff "/Volumes/$tag"
        fi
        /sbin/mount | /usr/bin/grep -qF " on $at (" && continue
        /bin/mkdir -p "$at"
        /usr/sbin/chown ${user}:staff "$at"
        if /sbin/mount_virtiofs -u ${user} -g staff "$tag" "$at"; then
          echo "$(date): mounted $tag at $at"
        else
          echo "$(date): could not mount $tag"
          /bin/rmdir "$at" 2>/dev/null
        fi
      done
    ''}" ];
    RunAtLoad = true;
  };

  # A larger disk (plan B164): omaflake grows the VM's disk.img while it is stopped; at the next boot the system
  # container takes the new space. A macOS VM's disk is ISC container, system container, then the fallback recovery
  # container (Apple_APFS_Recovery, 5.4 GB, checked in a macOS 27 guest) – it sits behind the system container, so it
  # is removed first; the paired recovery volume inside the system container stays. Only when there is new space
  # (more than 1 GiB not in any partition); it waits for /nix/store as omaflake-volumes does (plan B189). OPEN: checked on
  # a Mac with a grown VM.
  launchd.daemons.omaflake-grow-disk.serviceConfig = {
    ProgramArguments = [ "/bin/sh" "-c" "/bin/wait4path /nix/store && exec /bin/sh ${pkgs.writeText "omaflake-grow-disk.sh" ''
      set -u
      exec >> /var/log/omaflake-grow-disk.log 2>&1
      x() { plutil -extract "$1" raw -o - - 2>/dev/null; }
      container=$(diskutil info -plist / | x APFSContainerReference) || exit 0
      store=$(diskutil apfs list -plist "$container" | x Containers.0.PhysicalStores.0.DeviceIdentifier) || exit 0
      disk=$(diskutil info -plist "$store" | x ParentWholeDisk) || exit 0
      total=$(diskutil info -plist "$disk" | x TotalSize) || exit 0
      used=0; recovery=""; i=0
      while size=$(diskutil list -plist "$disk" | x "AllDisksAndPartitions.0.Partitions.$i.Size"); do
        used=$((used + size))
        if [ "$(diskutil list -plist "$disk" | x "AllDisksAndPartitions.0.Partitions.$i.Content")" = Apple_APFS_Recovery ]; then
          recovery=$(diskutil list -plist "$disk" | x "AllDisksAndPartitions.0.Partitions.$i.DeviceIdentifier")
        fi
        i=$((i + 1))
      done
      [ $((total - used)) -gt 1073741824 ] || exit 0
      echo "$(date): $disk has $(( (total - used) >> 20 )) MiB new space; growing $store"
      yes | diskutil repairDisk "$disk"
      if [ -n "$recovery" ]; then
        diskutil eraseVolume free free "$recovery" || exit 1
      fi
      diskutil apfs resizeContainer "$store" 0
    ''}" ];
    RunAtLoad = true;
  };

  # Images are rebuilt deliberately – no automatic macOS updates in the guest
  system.defaults.SoftwareUpdate.AutomaticallyInstallMacOSUpdates = false;

  system.stateVersion = 7;
}
