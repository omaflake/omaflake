# SPDX-License-Identifier: AGPL-3.0-or-later
# home-manager configuration of the guest user: Hyprland, terminal, shell (Catppuccin Mocha); the editor line is
# neovim.nix or emacs.nix, chosen by the recipe's omaflake.editor (modules/editor.nix)
{ pkgs, lib, settings, osConfig, ... }:

let
  # Catppuccin Mocha
  c = {
    base = "1e1e2e"; mantle = "181825"; crust = "11111b";
    surface0 = "313244"; surface1 = "45475a"; overlay0 = "6c7086";
    text = "cdd6f4"; subtext = "a6adc8";
    blue = "89b4fa"; mauve = "cba6f7"; green = "a6e3a1";
    red = "f38ba8"; peach = "fab387"; yellow = "f9e2af";
  };

in
{
  imports = [ ./neovim.nix ./emacs.nix ];

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
          col = {
            active_border = { colors = [ "rgba(${c.mauve}ff)" "rgba(${c.blue}ff)" ]; angle = 45; };
            inactive_border = "rgba(${c.surface0}ff)";
          };
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
        misc = { disable_hyprland_logo = true; disable_splash_rendering = true; };
      };
    };

    extraConfig = ''
      local mod      = "SUPER" -- Mac: Cmd key
      local terminal = "wezterm"
      local menu     = "fuzzel"

      hl.on("hyprland.start", function()
        hl.exec_cmd("waybar")
        hl.exec_cmd("mako")
        hl.exec_cmd("swaybg -c '#${c.base}'") -- or: swaybg -i ~/wallpaper.png -m fill
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
      hl.bind("Print", hl.dsp.exec_cmd([[grim -g "$(slurp)" - | wl-copy]]))

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
      color_scheme = "Catppuccin Mocha";
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
      colors = {
        background = "${c.base}f2";
        text = "${c.text}ff";
        match = "${c.mauve}ff";
        selection = "${c.surface1}ff";
        selection-text = "${c.text}ff";
        border = "${c.mauve}ff";
      };
      border = { width = 2; radius = 10; };
    };
  };

  # ---------- Notifications ----------
  services.mako = {
    enable = true;
    settings = {
      background-color = "#${c.base}";
      text-color = "#${c.text}";
      border-color = "#${c.mauve}";
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
      modules-right = [ "cpu" "memory" "network" "pulseaudio" ];
      clock.format = "{:%a %d.%m.  %H:%M}";
      cpu.format = " {usage}%";
      memory.format = " {percentage}%";
      network = { format-ethernet = "󰈀 {ipaddr}"; format-disconnected = "󰖪"; };
      pulseaudio.format = " {volume}%";
    };
    style = ''
      * { font-family: "JetBrainsMono Nerd Font"; font-size: 13px; }
      window#waybar { background: alpha(#${c.base}, 0.9); color: #${c.text}; }
      #workspaces button { color: #${c.overlay0}; padding: 0 8px; }
      #workspaces button.active { color: #${c.mauve}; }
      #clock, #cpu, #memory, #network, #pulseaudio { padding: 0 10px; }
      #clock { color: #${c.blue}; }
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
