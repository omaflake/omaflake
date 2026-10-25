# SPDX-License-Identifier: AGPL-3.0-or-later
# home-manager configuration of the guest user: Hyprland, terminal, shell; the editor line is neovim.nix or emacs.nix,
# chosen by the recipe's omaflake.editor (modules/editor.nix); the colours are the look's (look.nix, modules/look.nix)
{ pkgs, lib, settings, osConfig, ... }:

let
  # The VM's window changes size → QEMU offers that size as the output's preferred mode (virtio-gpu, a DRM change
  # event), but Hyprland keeps the mode of an output that stays connected ("Skipping connector …", aquamarine with
  # Hyprland 0.56; plan B128). This follows the preferred mode of every connected output and keeps its scale: with
  # scale "auto" Hyprland chose 2 from 1920×1080 on (QEMU's GTK UI gives the EDID no physical size), so a big window
  # turned into a small picture with huge text – the scale chosen at the start stays (Retina on a Mac, 1 elsewhere).
  followDisplay = pkgs.writeShellApplication {
    name = "omaflake-follow-display";
    runtimeInputs = [ pkgs.coreutils pkgs.jq pkgs.systemd osConfig.programs.hyprland.package ];
    text = ''
      apply() {
        for c in /sys/class/drm/card*-*; do
          [ "$(cat "$c/status" 2>/dev/null)" = connected ] || continue
          name=''${c##*/}
          output=''${name#card*-}
          want=$(head -n 1 "$c/modes" 2>/dev/null || true)
          [ -n "$want" ] || continue
          read -r have scale < <(hyprctl -j monitors |
            jq -r --arg o "$output" '.[] | select(.name == $o) | "\(.width)x\(.height) \(.scale)"') || true
          if [ -n "''${have:-}" ] && [ "$have" != "$want" ]; then
            hyprctl eval "hl.monitor({ output = \"$output\", mode = \"$want\", position = \"auto\", scale = $scale })" >/dev/null
          fi
        done
      }
      apply
      udevadm monitor --udev --subsystem-match=drm | while read -r line; do
        case "$line" in
          *change*) sleep 0.3; apply ;;
        esac
      done
    '';
  };

in
{
  imports = [ ./look.nix ./neovim.nix ./emacs.nix ];

  home.username = settings.user;
  home.homeDirectory = "/home/${settings.user}";
  home.stateVersion = "26.05";

  home.packages = with pkgs; [ ripgrep fd bat eza fzf jq btop fastfetch ];

  # ---------- Hyprland ----------
  # Hyprland ≥ 0.56 reads ~/.config/hypr/hyprland.lua; with stateVersion 26.05
  # home-manager generates Lua by default (configType = "lua"). Every key in
  # `settings` becomes a call hl.<name>(...); key bindings and autostart are Lua
  # in extraConfig. API: share/hypr/stubs/hl.meta.lua in the package.
  wayland.windowManager.hyprland = {
    enable = true;
    package = null;        # package comes from the NixOS module
    portalPackage = null;
    configType = "lua";

    settings = {
      # Window size and Retina scale come via EDID (QEMU patch dynamic-display)
      monitor = { output = ""; mode = "preferred"; position = "auto"; scale = "auto"; };

      env = { _args = [ "XCURSOR_SIZE" "24" ]; };

      config = {
        input = {
          # The Mac layout from base.nix (variant "mac", Option as level 3)
          kb_layout = osConfig.services.xserver.xkb.layout;
          kb_variant = osConfig.services.xserver.xkb.variant;
          kb_options = osConfig.services.xserver.xkb.options;
          follow_mouse = 1;
          touchpad.natural_scroll = true;
        };

        # needed in VMs, otherwise the cursor is invisible or flickers
        cursor.no_hardware_cursors = true;

        general = {
          gaps_in = 5;
          gaps_out = 10;
          border_size = 2;
          layout = "dwindle";
        };

        decoration = {
          rounding = 10;
          # blur and shadows cost noticeable performance in a VM – test before enabling
          blur.enabled = false;
          shadow.enabled = false;
        };

        animations.enabled = true;
        dwindle.preserve_split = true;
        # The background colour (look.nix) is drawn by Hyprland itself: it always fills the output, also after the
        # window of the VM changed size (swaybg kept its first size there, plan B128)
        misc = { disable_hyprland_logo = true; disable_splash_rendering = true; };
      };
    };

    extraConfig = ''
      -- Every key omaflake binds starts with SUPER (the Mac's Cmd) or CTRL, optionally with SHIFT (plan B135); plain
      -- keys only inside a mode (the resize mode below)
      local mod      = "SUPER" -- Mac: Cmd key
      local terminal = "wezterm"
      local menu     = "fuzzel"

      hl.on("hyprland.start", function()
        hl.exec_cmd("waybar")
        hl.exec_cmd("mako")
        hl.exec_cmd("${followDisplay}/bin/omaflake-follow-display")
        -- a wallpaper: hl.exec_cmd("swaybg -i ~/wallpaper.png -m fill") (misc.background_color is the plain colour)
      end)

      hl.bind(mod .. " + Return",    hl.dsp.exec_cmd(terminal))
      hl.bind(mod .. " + D",         hl.dsp.exec_cmd(menu))
      hl.bind(mod .. " + Q",         hl.dsp.window.close())
      hl.bind(mod .. " + SHIFT + E", hl.dsp.exit())
      hl.bind(mod .. " + F",         hl.dsp.window.fullscreen())
      hl.bind(mod .. " + V",         hl.dsp.window.float({ action = "toggle" }))
      hl.bind(mod .. " + P",         hl.dsp.window.pseudo())
      hl.bind(mod .. " + J",         hl.dsp.layout("togglesplit"))
      hl.bind(mod .. " + left",      hl.dsp.focus({ direction = "left" }))
      hl.bind(mod .. " + right",     hl.dsp.focus({ direction = "right" }))
      hl.bind(mod .. " + up",        hl.dsp.focus({ direction = "up" }))
      hl.bind(mod .. " + down",      hl.dsp.focus({ direction = "down" }))
      hl.bind(mod .. " + SHIFT + S", hl.dsp.exec_cmd([[grim -g "$(slurp)" - | wl-copy]]))

      -- SUPER + SHIFT + arrows = move the window that way (as Zellij's move mode)
      for _, d in ipairs({ "left", "right", "up", "down" }) do
        hl.bind(mod .. " + SHIFT + " .. d, hl.dsp.window.move({ direction = d }))
      end

      -- SUPER + R = resize mode (as Zellij's): arrows or h j k l, held down they repeat; right/down grow, left/up shrink;
      -- Esc or Enter leaves it
      hl.bind(mod .. " + R", hl.dsp.submap("resize"))
      hl.define_submap("resize", function()
        local steps = {
          { keys = { "right", "l" }, x = 40,  y = 0 },
          { keys = { "left",  "h" }, x = -40, y = 0 },
          { keys = { "down",  "j" }, x = 0,   y = 40 },
          { keys = { "up",    "k" }, x = 0,   y = -40 },
        }
        for _, s in ipairs(steps) do
          for _, key in ipairs(s.keys) do
            hl.bind(key, hl.dsp.window.resize({ x = s.x, y = s.y, relative = true }), { repeating = true })
          end
        end
        hl.bind("escape", hl.dsp.submap("reset"))
        hl.bind("Return", hl.dsp.submap("reset"))
      end)

      -- SUPER + 1..9 = switch workspace, SUPER + SHIFT + 1..9 = move window
      for i = 1, 9 do
        hl.bind(mod .. " + " .. i,             hl.dsp.focus({ workspace = i }))
        hl.bind(mod .. " + SHIFT + " .. i,     hl.dsp.window.move({ workspace = i }))
      end

      hl.bind(mod .. " + mouse:272", hl.dsp.window.drag(),   { mouse = true })
      hl.bind(mod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })
    '';
  };

  # ---------- Terminal ----------
  # WezTerm instead of kitty: kitty draws no text under virgl (GL context/glyphs, plan B23);
  # WezTerm renders cleanly on the GPU (checked 2026-10-16).
  programs.wezterm = {
    enable = true;
    settings = {
      font = lib.generators.mkLuaInline ''wezterm.font("JetBrainsMono Nerd Font")'';
      font_size = 12;
      window_padding = { left = 10; right = 10; top = 10; bottom = 10; };
      window_background_opacity = 0.95;
      hide_tab_bar_if_only_one_tab = true;
      window_close_confirmation = "NeverPrompt";
      enable_wayland = true;
    };
  };

  # ---------- Launcher ----------
  programs.fuzzel = {
    enable = true;
    settings = {
      main = { font = "JetBrainsMono Nerd Font:size=12"; terminal = "wezterm start --"; };
      border = { width = 2; radius = 10; };
    };
  };

  # ---------- Notifications ----------
  services.mako = {
    enable = true;
    settings = {
      border-radius = 10;
      default-timeout = 5000;
      font = "JetBrainsMono Nerd Font 11";
    };
  };

  # ---------- Status bar ----------
  programs.waybar = {
    enable = true;
    settings.mainBar = {
      layer = "top";
      position = "top";
      height = 30;
      modules-left = [ "hyprland/workspaces" ];
      modules-center = [ "clock" ];
      # The active keyboard when the locale has several (plan B141)
      modules-right =
        lib.optional (builtins.length (lib.splitString "," osConfig.services.xserver.xkb.layout) > 1) "hyprland/language"
        ++ [ "cpu" "memory" "network" "pulseaudio" ];
      "hyprland/language".format = "󰌌 {short}";
      clock.format = "{:%a %d.%m.  %H:%M}";
      cpu.format = " {usage}%";
      memory.format = " {percentage}%";
      network = { format-ethernet = "󰈀 {ipaddr}"; format-disconnected = "󰖪"; };
      pulseaudio.format = " {volume}%";
    };
    style = ''
      * { font-family: "JetBrainsMono Nerd Font"; font-size: 13px; }
      #workspaces button { padding: 0 8px; }
      #clock, #cpu, #memory, #network, #pulseaudio { padding: 0 10px; }
    '';
  };

  # ---------- Shell ----------
  programs.zsh = {
    enable = true;
    autosuggestion.enable = true;
    syntaxHighlighting.enable = true;
    shellAliases = {
      ls = "eza --icons";
      ll = "eza -la --icons";
      cat = "bat";
    };
    # start Hyprland after the auto-login on TTY1
    profileExtra = ''
      if [ -z "$WAYLAND_DISPLAY" ] && [ "$XDG_VTNR" = 1 ]; then
        exec start-hyprland  # wrapper with watchdog; starting Hyprland directly warns since 0.5x
      fi
    '';
  };
  programs.starship.enable = true;
  programs.fzf.enable = true;

  programs.git = {
    enable = true;
    settings.user = {
      name = settings.fullName;
    } // lib.optionalAttrs (settings.email != null) { inherit (settings) email; };
  };

  programs.home-manager.enable = true;
}
