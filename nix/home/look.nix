# SPDX-License-Identifier: AGPL-3.0-or-later
# The look on the NixOS desktop (plan B135): Hyprland, Waybar, Fuzzel, Mako, the prompt, GTK and the pointer from the
# look's `desktop` colours (looks/), WezTerm and Zellij from the files modules/look.nix puts under /etc/omaflake/look/.
# Vanilla (`desktop = null`) leaves every program its own colours.
{ pkgs, lib, osConfig, ... }:

let
  looks = import ../looks { inherit pkgs lib; };
  look = looks.get osConfig.omaflake.look;
  d = look.desktop;
  rgba = c: "rgba(${c}ff)";
in
lib.mkMerge [
  {
    # WezTerm: the look's settings over home-manager's (a table returned here is merged on top)
    programs.wezterm.extraConfig = ''
      return dofile("/etc/omaflake/look/wezterm.lua").config
    '';
    # Zellij (the neovim line): its theme, from Zellij itself or from the look's folder
    programs.zellij.settings = {
      theme = look.zellij.theme;
      theme_dir = "/etc/omaflake/look/zellij";
    };
  }

  (lib.mkIf (d != null) {
    wayland.windowManager.hyprland.settings.config = {
      general.col = {
        active_border = { colors = map rgba d.border; angle = 45; };
        inactive_border = rgba d.inactiveBorder;
      };
      misc.background_color = "rgb(${d.base})";
    };

    programs.waybar.style = ''
      window#waybar { background: alpha(#${d.base}, 0.9); color: #${d.text}; }
      #workspaces button { color: #${d.muted}; }
      #workspaces button.active { color: #${d.accent}; }
      #clock { color: #${d.clock}; }
    '' + lib.optionalString (d.split != null) ''
      /* the hero's RGB split, finely: the active workspace's number in white with a red and a cyan edge */
      #workspaces button.active {
        color: #${d.text};
        text-shadow: -1px 0 #${builtins.elemAt d.split 0}, 1px 0 #${builtins.elemAt d.split 1};
      }
    '';

    programs.fuzzel.settings.colors = {
      background = "${d.base}f2";
      text = "${d.text}ff";
      match = "${d.accent}ff";
      selection = "${d.selection}ff";
      selection-text = "${d.text}ff";
      border = "${d.accent}ff";
    };

    services.mako.settings = {
      background-color = "#${d.base}";
      text-color = "#${d.text}";
      border-color = "#${d.accent}";
      "urgency=high" = { border-color = "#${d.urgent}"; };
    };
  })

  (lib.mkIf (d != null && d.prompt != null) {
    programs.starship.settings = {
      character = {
        success_symbol = "[❯](bold #${d.prompt.ok})";
        error_symbol = "[❯](bold #${d.prompt.error})";
      };
      directory.style = "bold #${d.prompt.directory}";
    };
    # bat and fzf in the terminal's colours, which the look sets (wezterm.lua)
    home.sessionVariables.BAT_THEME = "ansi";
    programs.fzf.colors = { "bg+" = "-1"; bg = "-1"; };
  })

  (lib.mkIf (d != null && d.gtk != null) {
    gtk = {
      enable = true;
      theme = { inherit (d.gtk) name package; };
    };
    dconf.settings."org/gnome/desktop/interface".color-scheme = if d.dark then "prefer-dark" else "prefer-light";
  })

  (lib.mkIf (d != null && d.cursor != null) {
    home.pointerCursor = {
      inherit (d.cursor) name package;
      size = 24;
      gtk.enable = true;
    };
  })
]
