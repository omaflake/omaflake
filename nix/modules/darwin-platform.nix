# SPDX-License-Identifier: AGPL-3.0-or-later
# The platform's part of a recipe (plan B142), macOS: the host name is the recipe's name, and files modules put into
# the user's home folder once – a starting configuration that then belongs to the user (written as the user, only
# where nothing is yet).
{ config, lib, settings, recipe, ... }:

let
  user = settings.macUser;
  home = "/Users/${user}";
in
{
  options.omaflake = {
    homeFiles = lib.mkOption {
      type = lib.types.attrsOf lib.types.path;
      default = { };
      description = "Files copied into the home folder once (path below it → source), then the user's.";
    };
    homeLinks = lib.mkOption {
      type = lib.types.attrsOf lib.types.path;
      default = { };
      description = "Symlinks created in the home folder once (path below it → target).";
    };
  };

  config = {
    networking.hostName = recipe.name;

    # Everything is created *as the user*: with `install -d` as root the folders on the way became root's and programs
    # could not write their data (plan B77)
    system.activationScripts.postActivation.text = lib.mkIf (config.omaflake.homeFiles != { } || config.omaflake.homeLinks != { }) (lib.mkAfter (''
      asUser() { /usr/bin/sudo -u ${user} -H "$@"; }
    '' + lib.concatStrings (lib.mapAttrsToList (path: source: ''
      if [ ! -e "${home}/${path}" ]; then
        asUser /bin/mkdir -p "$(dirname "${home}/${path}")"
        asUser /bin/cp ${source} "${home}/${path}"
        asUser /bin/chmod 644 "${home}/${path}"
      fi
    '') config.omaflake.homeFiles) + lib.concatStrings (lib.mapAttrsToList (path: target: ''
      if [ ! -e "${home}/${path}" ]; then
        asUser /bin/mkdir -p "$(dirname "${home}/${path}")"
        asUser /bin/ln -s ${target} "${home}/${path}"
      fi
    '') config.omaflake.homeLinks)));
  };
}
