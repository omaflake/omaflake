# SPDX-License-Identifier: AGPL-3.0-or-later
# Guest module for macOS recipes (nix-darwin), applied on top of a base (docs/macos-base.md, spec §6.5).
# The guest user (`macUser`, set by omaflake from the base) already exists in the base (created by provisioning); nix-darwin does not manage it
# (it is not in users.knownUsers), it only adds settings for it.
{ lib, pkgs, settings, ... }:

let
  user = settings.macUser;

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
    de = { id = -18133; name = "German-DIN-2137"; }; # "German – Standard"
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

  # Images are rebuilt deliberately – no automatic macOS updates in the guest
  system.defaults.SoftwareUpdate.AutomaticallyInstallMacOSUpdates = false;

  system.stateVersion = 7;
}
