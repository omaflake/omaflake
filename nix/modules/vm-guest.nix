# SPDX-License-Identifier: AGPL-3.0-or-later
# Hardware abstraction for QEMU guests (UEFI, virtio; the system set by the image flake: aarch64 with HVF on a Mac, the
# host's architecture with KVM or TCG on a Linux host – plan B124)
# Replaces a machine-specific hardware-configuration.nix for images.
{ config, lib, pkgs, ... }:

let
  omaflakeAgent = pkgs.callPackage ../agent/package.nix { };
in
{
  options.vm.shareFolder.enable = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = "Mount the shared Mac folder (virtio-9p, tag \"share\") at /mnt/share.";
  };

  options.vm.bootSplash.enable = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = "omaflake's loading screen while the guest boots (Plymouth) instead of a black window until the desktop.";
  };

  config = {
    nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";

    boot.loader.systemd-boot.enable = true;
    # No boot menu wait (plan B155): it cost five seconds of a black window at every start; a key held at boot still
    # shows it (systemd-boot), for an older generation
    boot.loader.timeout = lib.mkDefault 0;
    boot.loader.efi.canTouchEfiVariables = lib.mkDefault false;
    boot.initrd.availableKernelModules = [ "virtio_pci" "virtio_blk" "virtio_scsi" "xhci_pci" "usbhid" ];
    boot.growPartition = true;
    # Kernel and boot messages on the serial port too (omaflake writes it to serial.log; systemd starts a login there
    # for the last console): aarch64's firmware names its PL011 to the kernel (ACPI SPCR), x86_64's COM1 must be named
    boot.kernelParams = lib.mkMerge [
      (lib.mkIf pkgs.stdenv.hostPlatform.isx86_64 [ "console=tty0" "console=ttyS0,115200" ])
      # The loading screen (below): quiet, so no kernel or systemd lines are drawn over it. Plymouth draws only text
      # ("details") as soon as a serial console is active – ours is – unless told to ignore them (plymouth's
      # ply-device-manager.c, create_devices_from_terminals)
      (lib.mkIf config.vm.bootSplash.enable [
        "quiet"
        "splash"
        "plymouth.ignore-serial-consoles"
        "rd.udev.log_level=3"
        "udev.log_priority=3"
        "rd.systemd.show_status=auto"
      ])
    ];

    # File systems by label – labels must match the image build (see spec, M1)
    fileSystems."/" = lib.mkDefault {
      device = "/dev/disk/by-label/nixos";
      fsType = "ext4";
      autoResize = true;
    };
    fileSystems."/boot" = lib.mkDefault {
      device = "/dev/disk/by-label/ESP";
      fsType = "vfat";
      options = [ "umask=077" ];
    };

    # Guest integration
    services.qemuGuest.enable = true;
    services.spice-vdagentd.enable = true;

    # The omaflake agent (spec §11.9): hello and status, the clipboard and the Mac's camera over the one virtio-serial
    # port org.omaflake.agent – omaflake knows the VM is up without SSH. udev starts it when the port appears (no port –
    # another hypervisor – no agent). Runs as root for what it will do later (host name, shutdown), without
    # capabilities; group "users" so the sessions' clipboard part may use its socket in /run/omaflake-agent.
    systemd.services.omaflake-agent = {
      description = "omaflake agent – talks to omaflake on the Mac";
      unitConfig.ConditionPathExists = "/dev/virtio-ports/org.omaflake.agent";
      serviceConfig = {
        ExecStart = lib.getExe omaflakeAgent;
        Group = "users";
        RuntimeDirectory = "omaflake-agent";
        RuntimeDirectoryMode = "0750";
        Restart = "always";
        RestartSec = 3;
        CapabilityBoundingSet = "";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_UNIX" "AF_NETLINK" "AF_INET" "AF_INET6" ];
        RestrictNamespaces = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
      };
    };

    # omaflake agent: started by udev when its port appears
    services.udev.extraRules = ''
      KERNEL=="vport*", ATTR{name}=="org.omaflake.agent", TAG+="systemd", ENV{SYSTEMD_WANTS}+="omaflake-agent.service"
    '';

    # Clipboard with the Mac (text, PNG): the agent's session part in the user's Wayland session, talking to the
    # system agent (plan B106; replaces clipboard-agent.py). Inactive without the port or a Wayland compositor.
    systemd.user.services.omaflake-agent-session = {
      description = "Clipboard with the Mac (omaflake agent, session part)";
      wantedBy = [ "default.target" ];
      unitConfig.ConditionPathExists = "/dev/virtio-ports/org.omaflake.agent";
      path = [ pkgs.wl-clipboard pkgs.coreutils ];
      # Wait for the compositor's socket, then run; restart if the session or the agent goes away
      script = ''
        until wl=$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -m1 '^wayland-[0-9]*$'); do sleep 2; done
        export WAYLAND_DISPLAY="$wl"
        exec ${lib.getExe omaflakeAgent} session
      '';
      serviceConfig = { Restart = "always"; RestartSec = 3; };
    };

    # The Mac's camera as /dev/video42 "Mac Camera" (v4l2loopback), fed by the agent – only while an application reads
    # it (the Mac camera is on only then); the agent replaces camera-agent.py (plan B106)
    boot.extraModulePackages = [ config.boot.kernelPackages.v4l2loopback ];
    boot.kernelModules = [ "v4l2loopback" ];
    boot.extraModprobeConfig = ''
      options v4l2loopback devices=1 video_nr=42 card_label="Mac Camera" exclusive_caps=1 max_openers=10
    '';
    # mpv's default GPU API tries Vulkan first; under virgl that ends in an empty window (camera, videos) –
    # OpenGL works. libmpv reads /etc/mpv (plan B39).
    environment.etc."mpv/mpv.conf".text = ''
      gpu-api=opengl
    '';
    # The loading screen (plan B155): Plymouth's spinner with omaflake's mark (brand/generate.py writes it here) from the
    # kernel's start until the desktop – the graphics driver in the initrd, so it shows that early; quiet, so no kernel
    # or systemd lines are drawn over it (they stay in the journal; the serial log has fewer kernel lines then)
    boot.plymouth = lib.mkIf config.vm.bootSplash.enable {
      enable = true;
      theme = "spinner";
      logo = ./omaflake-splash.png;
    };
    boot.initrd.kernelModules = lib.mkIf config.vm.bootSplash.enable [ "virtio_gpu" ];
    boot.consoleLogLevel = lib.mkIf config.vm.bootSplash.enable 3;
    boot.initrd.verbose = lib.mkIf config.vm.bootSplash.enable false;

    # Shared folder from the Mac; the VM still boots if it is missing
    fileSystems."/mnt/share" = lib.mkIf config.vm.shareFolder.enable {
      device = "share";
      fsType = "9p";
      options = [ "trans=virtio" "version=9p2000.L" "nofail" "x-systemd.automount" ];
    };
  };
}
