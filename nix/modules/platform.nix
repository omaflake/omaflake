# SPDX-License-Identifier: AGPL-3.0-or-later
# The platform's part of a recipe (plan B142), NixOS: the host name is the recipe's name, and the few options modules
# share – the terminal (set by the module providing `terminal`) and the launchers other modules add, which the
# desktop binds. A module learns about the others from its `omaflake` argument (`has`, `keys`), not from here.
{ lib, recipe, ... }:

{
  options.omaflake = {
    terminal = {
      open = lib.mkOption {
        type = lib.types.str;
        default = "xterm";
        description = "The command that opens a terminal window (the terminal module sets it).";
      };
      run = lib.mkOption {
        type = lib.types.str;
        default = "xterm -e";
        description = "The command that runs a program in a new terminal window, followed by the program.";
      };
    };
    launchers = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          key = lib.mkOption { type = lib.types.str; description = "The key, in the catalog's notation (Super+e)."; };
          command = lib.mkOption { type = lib.types.str; description = "What the key starts."; };
        };
      });
      default = [ ];
      description = "Keys that start a program anywhere on the desktop – modules add them, the desktop binds them.";
    };
  };

  config.networking.hostName = recipe.name;
}
