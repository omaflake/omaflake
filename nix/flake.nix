# SPDX-License-Identifier: AGPL-3.0-or-later
{
  description = "omaflake – builds VM images from recipes (recipes, modules and key bindings come from catalogs)";

  # Use this flake as a `path:` reference (omaflake does that automatically for this folder):
  #   nix build 'path:$PWD#images.aarch64-linux."omaflake.nixos.neovim-minimal"' --override-input omaflake-catalog path:<catalog>
  # A `path:` flake sees every file in this folder, including the git-ignored local/ folder with
  # your personal settings. A plain `.#…` (git) reference would silently ignore local/.

  inputs = {
    # The person-specific values (user, keys, locale, …): neutral defaults in guest/; omaflake replaces this input with
    # ~/.omaflake/guest when it builds (guest/README.md). Your local/settings.nix overrides single values on top.
    omaflake-guest = { url = "path:./guest"; flake = false; };
    # The recipes to build with their modules and key bindings, as a catalog lays them out (recipes/, modules/,
    # keymaps/; spec §11.3, plan B142): omaflake replaces this input with what its sources have – installed and local –
    # when it evaluates or builds. The default is empty, so the flake evaluates on its own (nix flake check).
    omaflake-catalog = { url = "path:./no-catalog"; flake = false; };
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

  outputs = { nixpkgs, home-manager, nix-darwin, nix-homebrew, omaflake, omaflake-guest, omaflake-catalog, ... }:
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
        # The look chosen for the build (modules/look.nix); null: none, every program its own colours
        look = ownSettings.look or null;
        # The macOS guest user is fixed by the base (omaflake passes it); default: same as `user`
        macUser = if (ownSettings.macUser or null) != null then ownSettings.macUser else ownSettings.user;
        authorizedKeys = guest.authorizedKeys or [ ];
        inherit authorizedKeyFiles;
      };

      # Recipes are data (recipe.toml, schema recipe/v2, plan B142): a platform, the modules in it, the key binding
      # sets, what a VM from it needs and how it runs. Modules are Nix (modules/<author>/<platform>/<name>/: module.toml
      # and default.nix), key binding sets data (keymaps/<author>/<name>/keymap.toml). Which version of a module or set
      # is used, omaflake decides when it stages a build (one tree per recipe, recipes.json); here a reference
      # "<author>/<name>" is the one in the recipe's tree.
      readToml = f: builtins.fromTOML (builtins.readFile f);
      kinds = [ "nixos" "macos" ];
      # What a recipe of each platform can be built for (plan B114, Platform.swift); recipe.toml lists its own
      kindSystems = { nixos = [ "aarch64-linux" "x86_64-linux" ]; macos = [ "aarch64-darwin" ]; };
      recipeSchema = "https://omaflake.dev/schema/recipe/v2.json";
      moduleSchema = "https://omaflake.dev/schema/module/v2.json";
      keymapSchema = "https://omaflake.dev/schema/keymap/v2.json";
      validName = what: s: lib.assertMsg (builtins.match "[a-z0-9][a-z0-9-]*" s != null)
        "${what} \"${s}\" – use lowercase letters, digits and -";
      dirsIn = dir: if builtins.pathExists dir
        then builtins.attrNames (lib.filterAttrs (_: t: t == "directory") (builtins.readDir dir)) else [ ];
      # A reference "author/name" or { id = "author/name"; version | min | max } → "author/name"
      refOf = r: if builtins.isString r then r else r.id;
      splitRef = r: let p = lib.splitString "/" r; in
        assert lib.assertMsg (builtins.length p == 2) "\"${r}\" is no reference – write <author>/<name>";
        { author = builtins.elemAt p 0; name = builtins.elemAt p 1; };
      keyLib = import ./lib/keys.nix { inherit lib; };

      # One recipe: its tree (catalog-shaped), …/recipes/<author>/<kind>/<name>/, and the ID it is known by
      recipeEntry = id: tree: author: kind: name:
        let
          folder = tree + "/recipes/${author}/${kind}/${name}";
          d = readToml (folder + "/recipe.toml");
          where = "${author}/${kind}/${name}";
          systems = d.systems or [ ];
          refs = map refOf (d.modules or [ ]);
          # A module of the recipe: its folder in the tree and its module.toml
          moduleOf = ref:
            let r = splitRef ref; dir = tree + "/modules/${r.author}/${kind}/${r.name}"; in
            assert lib.assertMsg (builtins.pathExists (dir + "/module.toml"))
              "${where}: the module ${ref} (${kind}) is not here – install it, or add the source that has it";
            let m = readToml (dir + "/module.toml"); in
            assert lib.assertMsg ((m."$schema" or "") == moduleSchema)
              "${ref}: module.toml needs \"$schema\" = \"${moduleSchema}\"";
            { inherit ref dir; actions = m.actions or { }; modes = m.modes or { }; };
          modules = map moduleOf refs;
          # Keys: each module's defaults, then the recipe's key binding sets in order (a later set wins)
          sets = map (k:
            let r = splitRef (refOf k); f = tree + "/keymaps/${r.author}/${r.name}/keymap.toml"; in
            assert lib.assertMsg (builtins.pathExists f) "${where}: the key bindings ${refOf k} are not here";
            let m = readToml f; in
            assert lib.assertMsg ((m."$schema" or "") == keymapSchema) "${refOf k}: keymap.toml needs \"$schema\" = \"${keymapSchema}\"";
            m.keys or { }) (d.keymaps or [ ]);
          # A set's entry is a key ("Ctrl+g"), several keys, or a table with `key` and/or `after` (keymap/v2)
          entry = v: if builtins.isAttrs v then v else { key = v; };
          bindings = lib.listToAttrs (map (m: lib.nameValuePair m.ref
            (lib.foldl (acc: set: acc // lib.mapAttrs (a: v: acc.${a} // entry v)
              (lib.filterAttrs (a: _: m.actions ? ${a}) (set.${m.ref} or { }))) m.actions sets)) modules);
          keys = lib.mapAttrs (_: lib.mapAttrs (_: b: b.key)) bindings;
        in
        assert validName "recipe author" author && validName "recipe name" name;
        assert lib.assertMsg ((d.author or author) == author)
          "${where}: author = \"${d.author or ""}\" in recipe.toml, but the folder says ${author}";
        assert lib.assertMsg ((d."$schema" or "") == recipeSchema)
          "${where}: recipe.toml needs \"$schema\" = \"${recipeSchema}\" (it has ${d."$schema" or "none"})";
        assert lib.assertMsg ((d.platform or "") == kind) "${where}: platform = \"${d.platform or ""}\", but the folder says ${kind}";
        assert lib.assertMsg (builtins.isList systems && systems != [ ] && lib.all (s: builtins.elem s kindSystems.${kind}) systems)
          "${where}: systems in recipe.toml must list some of ${lib.concatStringsSep ", " kindSystems.${kind}}";
        {
          inherit id systems modules keys bindings;
          recipe = { inherit name author kind; modules = refs; };
          # What omaflake reads (`lib.recipeDefaults`): the VM's values flat, as before recipe v2
          defaults = { user = if kind == "macos" then settings.macUser else settings.user; }
            // builtins.removeAttrs d [ "author" "$schema" "requirements" "vm" "modules" "keymaps" "look" "locales" ]
            // (d.requirements or { }) // (d.vm or { })
            // { inherit author kind name systems; modules = refs; keymaps = map refOf (d.keymaps or [ ]); }
            // lib.optionalAttrs (d ? look) { look = refOf d.look; }
            // lib.optionalAttrs (d ? locales) { inherit (d) locales; };
        };
      # Staged by omaflake: recipes.json names each recipe – ID "<author>.<kind>.<name>@<content hash>" → its tree
      # ({ tree; recipe = "<author>/<kind>/<name>"; }), every recipe in a tree of its own with the modules and key
      # bindings it uses. Without it (a catalog checkout, e.g. for `nix flake check`): the input is the one tree, IDs
      # without a hash.
      stagedMap = omaflake-catalog + "/recipes.json";
      recipeEntries =
        if builtins.pathExists stagedMap then
          lib.mapAttrsToList (id: e:
            let p = lib.splitString "/" e.recipe; in
            assert lib.assertMsg (builtins.length p == 3 && builtins.elem (builtins.elemAt p 1) kinds) "recipes.json: ${id} → ${e.recipe}";
            recipeEntry id (omaflake-catalog + "/${e.tree}") (builtins.elemAt p 0) (builtins.elemAt p 1) (builtins.elemAt p 2))
            (lib.importJSON stagedMap)
        else
          let root = omaflake-catalog + "/recipes"; in
          lib.concatMap (author: lib.concatMap (kind: lib.concatMap (name:
            lib.optional (builtins.pathExists (root + "/${author}/${kind}/${name}/recipe.toml"))
              (recipeEntry "${author}.${kind}.${name}" omaflake-catalog author kind name))
            (dirsIn (root + "/${author}/${kind}"))) kinds) (dirsIn root);
      recipes = lib.listToAttrs (map (e: lib.nameValuePair e.id e) recipeEntries);
      recipeDefaults = lib.mapAttrs (_: e: e.defaults) recipes;
      recipesOfKind = kind: builtins.attrNames (lib.filterAttrs (_: d: d.kind == kind) recipeDefaults);
      # The recipes of a kind that can be built for `system`
      recipesFor = kind: system: builtins.filter (id: builtins.elem system recipeDefaults.${id}.systems) (recipesOfKind kind);
      macRecipes = recipesFor "macos" "aarch64-darwin";

      # A recipe's modules, each with its `omaflake` argument: its keys (the recipe's), its bindings (each action with
      # its key, modes, `after` and `does` after the recipe's key binding sets – to write a program's configuration),
      # its modes, every module's keys, whether the recipe has another module (`has "author/name"` – for what it
      # enhances), the key notations (lib/keys.nix)
      recipeModules = id:
        let e = recipes.${id}; in
        map (m:
          let
            f = import (m.dir + "/default.nix");
            arg = {
              id = m.ref;
              keys = e.keys.${m.ref};
              bindings = e.bindings.${m.ref};
              inherit (m) modes;
              allKeys = e.keys;
              has = ref: builtins.elem ref e.recipe.modules;
              inherit keyLib;
              inherit (e) recipe;
            };
          in
          {
            _file = m.dir + "/default.nix";
            imports = [ (lib.setFunctionArgs (args: f (args // { omaflake = arg; })) (builtins.removeAttrs (lib.functionArgs f) [ "omaflake" ])) ];
          }) e.modules;

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

      # pkgs.omaflake for modules: the root flake's Linux package (app `omaflake`, CLI `omaf`; plan B123) – the input is
      # only fetched when a module uses it
      omaflakeOverlay = final: _: {
        omaflake = omaflake.packages.${final.stdenv.hostPlatform.system}.omaflake-gui;
      };

      # A recipe VM for one system = the platform (guest hardware – also the root flake's nixosModules.vm-guest –,
      # user, locale, the look, home-manager) + the recipe's modules + your local extras
      mkVm = system: id: nixpkgs.lib.nixosSystem {
        specialArgs = { inherit settings; inherit (recipes.${id}) recipe; };
        modules = [
          ./modules/vm-guest.nix
          ./modules/base.nix
          ./modules/look.nix
          ./modules/platform.nix
          { nixpkgs.hostPlatform = system; nixpkgs.overlays = [ omaflakeOverlay ]; }
        ] ++ recipeModules id ++ optionalLocal "nixos.nix" ++ homeManager;
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
        # macOS guests: the platform (darwin-guest, the look) + the recipe's modules, activated on a clone of the base
        # (spec §6.5, M9)
        // lib.genAttrs macRecipes (id: nix-darwin.lib.darwinSystem {
          specialArgs = { inherit settings nix-homebrew; inherit (recipes.${id}) recipe; };
          modules = [ ./modules/darwin-guest.nix ./modules/look.nix ./modules/darwin-platform.nix ] ++ recipeModules id
            ++ optionalLocal "darwin-guest.nix";
        });
    };
}
