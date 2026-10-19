# SPDX-License-Identifier: AGPL-3.0-or-later
{
  description = "omaflake – builds VM images from recipes (the recipes themselves come from recipe sources)";

  # Use this flake as a `path:` reference (omaflake does that automatically for this folder):
  #   nix build path:$PWD#images.omaflake.nixos.minimal --override-input omaflake-recipes path:<recipes>
  # A `path:` flake sees every file in this folder, including the git-ignored local/ folder with
  # your personal settings. A plain `.#…` (git) reference would silently ignore local/.

  inputs = {
    # The person-specific values (user, keys, locale, …): neutral defaults in guest/; omaflake replaces this input with
    # ~/.omaflake/guest when it builds (guest/README.md). Your local/settings.nix overrides single values on top.
    omaflake-guest = { url = "path:./guest"; flake = false; };
    # The recipes to build, as <author>/<kind>/<name>/ folders (spec §11.3, plan B107): omaflake replaces this input
    # with the recipes of its sources – installed ones and local folders – when it evaluates or builds. The default is
    # empty, so the flake evaluates on its own (nix flake check).
    omaflake-recipes = { url = "path:./no-recipes"; flake = false; };
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/master";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Homebrew itself, declaratively (macOS recipes that want `brew`, e.g. macos/research)
    nix-homebrew.url = "github:zhaofengli/nix-homebrew";
  };

  outputs = { nixpkgs, home-manager, nix-darwin, nix-homebrew, omaflake-guest, omaflake-recipes, ... }:
    let
      inherit (nixpkgs) lib;

      # Developer extras: local/ (git-ignored, yours) – optional; the person-specific values come from omaflake-guest
      localDir = ./local;
      hasLocal = builtins.pathExists (localDir + "/settings.nix");
      optionalLocal = name: lib.optional (builtins.pathExists (localDir + "/${name}")) (localDir + "/${name}");

      # Every *.pub file in local/keys/ may log in as the guest user (public keys are not secrets)
      keysDir = localDir + "/keys";
      authorizedKeyFiles =
        if builtins.pathExists keysDir then
          map (f: keysDir + "/${f}")
            (builtins.filter (lib.hasSuffix ".pub") (builtins.attrNames (builtins.readDir keysDir)))
        else [ ];

      # omaflake's values (or the neutral defaults), then local/settings.nix on top
      guest = builtins.fromJSON (builtins.readFile "${omaflake-guest}/settings.json");
      ownSettings = guest // lib.optionalAttrs hasLocal (import (localDir + "/settings.nix"));
      settings = ownSettings // {
        # The macOS guest user is fixed by the base (omaflake passes it); default: same as `user`
        macUser = if (ownSettings.macUser or null) != null then ownSettings.macUser else ownSettings.user;
        authorizedKeys = guest.authorizedKeys or [ ];
        inherit authorizedKeyFiles;
      };

      # Recipes, one folder each: <author>/<kind>/<name>/ with recipe.toml (how a VM from it runs – one source for Nix,
      # the omaf CLI and the app), default.nix (what is inside it) and its own files (omaflake/recipes, CONTRIBUTING.md).
      # Its ID names outputs, images and instances (plan B110: "<author>.<kind>.<name>@<content hash>" when staged).
      readToml = f: builtins.fromTOML (builtins.readFile f);
      kinds = [ "nixos" "macos" ];
      validName = what: s: lib.assertMsg (builtins.match "[a-z0-9][a-z0-9-]*" s != null)
        "recipe ${what} \"${s}\" – use lowercase letters, digits and -";
      dirsIn = dir: if builtins.pathExists dir
        then builtins.attrNames (lib.filterAttrs (_: t: t == "directory") (builtins.readDir dir)) else [ ];
      # One recipe: its folder …/<author>/<kind>/<name>/ and the ID it is known by
      recipeEntry = id: author: kind: name: folder:
        let d = readToml (folder + "/recipe.toml"); in
        assert validName "author" author && validName "name" name;
        assert lib.assertMsg ((d.author or author) == author)
          "${author}/${kind}/${name}: author = \"${d.author or ""}\" in recipe.toml, but the folder says ${author}";
        {
          inherit id folder;
          defaults = { user = if kind == "macos" then settings.macUser else settings.user; }
            // builtins.removeAttrs d [ "author" "kind" "name" ] // { inherit author kind name; };
        };
      # Staged by omaflake (plan B110): recipes.json names each recipe – ID "<author>.<kind>.<name>@<content hash>" →
      # "<tree>/<author>/<kind>/<name>", every recipe in a tree of its own with the recipes it requires next to it.
      # Without it (a recipes checkout, e.g. for `nix flake check`): <author>/<kind>/<name>/ folders, ID without a hash.
      stagedMap = omaflake-recipes + "/recipes.json";
      recipeEntries =
        if builtins.pathExists stagedMap then
          lib.mapAttrsToList (id: rel:
            let p = lib.splitString "/" rel; in
            assert lib.assertMsg (builtins.length p == 4 && builtins.elem (builtins.elemAt p 2) kinds) "recipes.json: ${id} → ${rel}";
            recipeEntry id (builtins.elemAt p 1) (builtins.elemAt p 2) (builtins.elemAt p 3) (omaflake-recipes + "/${rel}"))
            (lib.importJSON stagedMap)
        else
          lib.concatMap (author: lib.concatMap (kind: lib.concatMap (name:
            let folder = omaflake-recipes + "/${author}/${kind}/${name}"; in
            lib.optional (builtins.pathExists (folder + "/recipe.toml"))
              (recipeEntry "${author}.${kind}.${name}" author kind name folder))
            (dirsIn (omaflake-recipes + "/${author}/${kind}"))) kinds) (dirsIn omaflake-recipes);
      recipeDefaults = lib.listToAttrs (map (e: lib.nameValuePair e.id e.defaults) recipeEntries);
      recipeFolders = lib.listToAttrs (map (e: lib.nameValuePair e.id e.folder) recipeEntries);
      recipesOfKind = kind: builtins.attrNames (lib.filterAttrs (_: d: d.kind == kind) recipeDefaults);
      recipes = recipesOfKind "nixos";
      macRecipes = recipesOfKind "macos";

      # What is inside a recipe: its folder's default.nix
      recipeModule = id: recipeFolders.${id} + "/default.nix";

      homeManager = [
        home-manager.nixosModules.home-manager
        {
          home-manager.useGlobalPkgs = true;
          home-manager.useUserPackages = true;
          home-manager.backupFileExtension = "bak";
          home-manager.extraSpecialArgs = { inherit settings; };
          home-manager.users.${settings.user}.imports = [ ./home ] ++ optionalLocal "home.nix";
        }
      ];

      # A recipe VM = guest hardware (also the root flake's nixosModules.vm-guest) + base + desktop + the recipe + your local extras
      mkVm = recipe: nixpkgs.lib.nixosSystem {
        specialArgs = { inherit settings; };
        modules = [
          ./modules/vm-guest.nix
          ./modules/base.nix
          ./modules/hyprland.nix
          (recipeModule recipe)
        ] ++ optionalLocal "nixos.nix" ++ homeManager;
      };

      vms = lib.genAttrs recipes mkVm;

      # UEFI qcow2 per recipe (nixpkgs variant "qemu-efi", labels nixos/ESP).
      # Size "auto" keeps the image small; the CLI applies disk_gb from recipe.toml to the overlay.
      imageOf = f: vms.${f}.config.system.build.images.qemu-efi;
    in
    {
      nixosConfigurations = lib.mapAttrs' (f: vm: lib.nameValuePair "vm-${f}" vm) vms;

      # What omaflake expects: images.<recipe> and lib.recipeDefaults
      images = lib.genAttrs recipes imageOf;
      lib = { inherit recipeDefaults; };

      # nix build path:$PWD#packages.aarch64-linux.<recipe>-image (same as images.<recipe>)
      packages.aarch64-linux = lib.mapAttrs' (f: _: lib.nameValuePair "${f}-image" (imageOf f)) vms;

      darwinConfigurations =
        # nix-darwin host with a Linux builder for the images (docs/linux-builder.md)
        {
          linux-builder = nix-darwin.lib.darwinSystem {
            modules = [ ./hosts/linux-builder.nix ] ++ optionalLocal "darwin.nix";
          };
          # First stage on a fresh Mac: nix-darwin with the default builder from the binary cache – a changed builder
          # is itself a Linux build and needs a running builder (docs/linux-builder.md, "Two-stage setup")
          linux-builder-bootstrap = nix-darwin.lib.darwinSystem {
            modules = [ ./hosts/linux-builder.nix ({ lib, ... }: { nix.linux-builder.config = lib.mkForce { }; }) ]
              ++ optionalLocal "darwin.nix";
          };
        }
        # macOS guests: darwin-guest + the recipe, activated on a clone of the base (spec §6.5, M9)
        // lib.genAttrs macRecipes (f: nix-darwin.lib.darwinSystem {
          specialArgs = { inherit settings nix-homebrew; };
          modules = [ ./modules/darwin-guest.nix (recipeModule f) ] ++ optionalLocal "darwin-guest.nix";
        });
    };
}
