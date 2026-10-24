# SPDX-License-Identifier: AGPL-3.0-or-later
{
  description = "omaflake – builds VM images from recipes (the recipes themselves come from recipe sources)";

  # Use this flake as a `path:` reference (omaflake does that automatically for this folder):
  #   nix build 'path:$PWD#images.aarch64-linux."omaflake.nixos.neovim-minimal"' --override-input omaflake-recipes path:<recipes>
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
    # omaflake itself, for recipes that bring it (`pkgs.omaflake`: the Linux app and `omaf`, e.g. nixos/lab). In a
    # checkout omaflake replaces it with the checkout (`--override-input omaflake <checkout>`), so a lab gets the code
    # as it is on disk; fetched only when a recipe uses `pkgs.omaflake` (plan B123).
    omaflake = {
      url = "github:omaflake/omaflake";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, home-manager, nix-darwin, nix-homebrew, omaflake, omaflake-guest, omaflake-recipes, ... }:
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
        # omaflake leaves "email" out of guest/settings.json when it has none (plan B124: a fresh data folder failed to
        # build with "attribute 'email' missing")
        email = ownSettings.email or null;
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
      # What a recipe of each kind can be built for (plan B114, Platform.swift); recipe.toml lists its own in `systems`
      kindSystems = { nixos = [ "aarch64-linux" "x86_64-linux" ]; macos = [ "aarch64-darwin" ]; };
      # recipe.toml's versions (plan B114, RecipeLibrary.recipeVersion/recipeMinimum): v1 names its "$schema"
      # (docs/schema/recipe/) and its systems; v0 has neither and was made for Apple silicon. omaflake stages only
      # recipes of these versions.
      recipeSchema = "https://omaflake.dev/schema/recipe/v1.json";
      legacySystems = { nixos = [ "aarch64-linux" ]; macos = [ "aarch64-darwin" ]; };
      validName = what: s: lib.assertMsg (builtins.match "[a-z0-9][a-z0-9-]*" s != null)
        "recipe ${what} \"${s}\" – use lowercase letters, digits and -";
      dirsIn = dir: if builtins.pathExists dir
        then builtins.attrNames (lib.filterAttrs (_: t: t == "directory") (builtins.readDir dir)) else [ ];
      # One recipe: its folder …/<author>/<kind>/<name>/ and the ID it is known by
      recipeEntry = id: author: kind: name: folder:
        let
          d = readToml (folder + "/recipe.toml");
          where = "${author}/${kind}/${name}";
          v0 = !(d ? "$schema");
          systems = if v0 then legacySystems.${kind} else d.systems or [ ];
        in
        assert validName "author" author && validName "name" name;
        assert lib.assertMsg ((d.author or author) == author)
          "${where}: author = \"${d.author or ""}\" in recipe.toml, but the folder says ${author}";
        assert lib.assertMsg (v0 || d."$schema" == recipeSchema)
          "${where}: recipe.toml needs \"$schema\" = \"${recipeSchema}\" (it has ${d."$schema" or "none"})";
        assert lib.assertMsg (builtins.isList systems && systems != [ ] && lib.all (s: builtins.elem s kindSystems.${kind}) systems)
          "${where}: systems in recipe.toml must list some of ${lib.concatStringsSep ", " kindSystems.${kind}}";
        {
          inherit id folder systems;
          defaults = { user = if kind == "macos" then settings.macUser else settings.user; }
            // builtins.removeAttrs d [ "author" "kind" "name" "$schema" ] // { inherit author kind name systems; };
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
      # The recipes of a kind that can be built for `system`
      recipesFor = kind: system: builtins.filter (id: builtins.elem system recipeDefaults.${id}.systems) (recipesOfKind kind);
      macRecipes = recipesFor "macos" "aarch64-darwin";

      # What is inside a recipe: its folder's default.nix, and <system>.nix next to it when there is one – the
      # recipe's variant for that architecture (plan B114)
      recipeModules = id: system:
        let variant = recipeFolders.${id} + "/${system}.nix"; in
        [ (recipeFolders.${id} + "/default.nix") ] ++ lib.optional (builtins.pathExists variant) variant;

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

      # A recipe VM for one system = guest hardware (also the root flake's nixosModules.vm-guest) + base + desktop +
      # editor line (the recipe picks it) + the recipe (and its variant for the system) + your local extras
      # pkgs.omaflake for recipes: the root flake's Linux package (app `omaflake`, CLI `omaf`; plan B123) – the input is
      # only fetched when a recipe uses it
      omaflakeOverlay = final: _: {
        omaflake = omaflake.packages.${final.stdenv.hostPlatform.system}.omaflake-gui;
      };

      mkVm = system: recipe: nixpkgs.lib.nixosSystem {
        specialArgs = { inherit settings; };
        modules = [
          ./modules/vm-guest.nix
          ./modules/base.nix
          ./modules/hyprland.nix
          ./modules/editor.nix
          { nixpkgs.hostPlatform = system; nixpkgs.overlays = [ omaflakeOverlay ]; }
        ] ++ recipeModules recipe system ++ optionalLocal "nixos.nix" ++ homeManager;
      };

      # Per system the VMs of the recipes that can be built for it
      vms = lib.genAttrs kindSystems.nixos (system: lib.genAttrs (recipesFor "nixos" system) (mkVm system));

      # UEFI qcow2 per recipe (nixpkgs variant "qemu-efi", labels nixos/ESP).
      # Size "auto" keeps the image small; the CLI applies disk_gb from recipe.toml to the overlay.
      imageOf = vm: vm.config.system.build.images.qemu-efi;
    in
    {
      nixosConfigurations = lib.concatMapAttrs (system: lib.mapAttrs' (f: vm: lib.nameValuePair "vm-${system}-${f}" vm)) vms;

      # What omaflake expects: images.<system>.<recipe> and lib.recipeDefaults
      images = lib.mapAttrs (_: lib.mapAttrs (_: imageOf)) vms;
      lib = { inherit recipeDefaults; };

      # nix build path:$PWD#packages.<system>.<recipe>-image (same as images.<system>.<recipe>)
      packages = lib.mapAttrs (_: lib.mapAttrs' (f: vm: lib.nameValuePair "${f}-image" (imageOf vm))) vms;

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
          modules = [ ./modules/darwin-guest.nix ] ++ recipeModules f "aarch64-darwin" ++ optionalLocal "darwin-guest.nix";
        });
    };
}
