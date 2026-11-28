# SPDX-License-Identifier: AGPL-3.0-or-later
# Tests of lib/validate.nix, lib/keys.nix and looks/ – evaluated only (flake output `checks`, `nix flake check`):
# what a look or a key binding set must not get into a guest's configuration is refused, what the catalog writes is
# taken. → the names of the tests that failed (none: all passed).
{ lib, pkgs }:

let
  validate = import ./validate.nix { inherit lib; };
  keyLib = import ./keys.nix { inherit lib validate; };
  looks = import ../looks { inherit pkgs lib; };

  # Everything of a look that becomes text in the guest
  render = look:
    let l = looks.get look; in
    [ (looks.nameOf look) l.wezterm l.zellij.theme l.zellij.kdl l.nvim.lua l.emacs.elisp ]
    ++ lib.optionals (l.desktop != null) [ l.desktop.base l.desktop.border l.desktop.gtk.name l.desktop.prompt l.desktop.split ]
    ++ lib.optional (l.desktop != null && l.desktop.cursor != null) l.desktop.cursor.name
    ++ lib.optional (l.macos != null) (builtins.attrValues l.macos);
  accepted = v: (builtins.tryEval (builtins.deepSeq v true)).success;
  refused = v: !(accepted v);

  eight = c: lib.genList (_: c) 8;
  look = {
    "$schema" = "https://omaflake.dev/schema/look/v1.json";
    version = "1.0.0";
    author = "someone";
    license = "MIT";
    title = "Rosé – a look";
    description = "Text with \"quotes\", a \\ and more.";
    description_de = "Text.";
    tags = [ "dark" ];
    appearance = "dark";
    colors = {
      background = "#000000"; surface = "#121212"; selection = "#2A2A2A"; text = "#ffffff"; subtext = "#B3B3B3";
      muted = "#6E6E6E"; accent = "#00E5FF"; urgent = "#FF3B3B";
    };
    terminal = { normal = eight "#101010"; bright = eight "#F0F0F0"; };
  };
  with' = lib.recursiveUpdate look;
  inject = "#fff\"; os.execute(\"x\") --";

  actions = {
    quit = { title = "Quit"; key = "Ctrl+q"; does = "Quit"; mode = "normal"; };
    lock = { title = "Lock"; key = "Ctrl+g"; after = "locked"; };
    leader = { title = "Leader"; key = "Space"; own = true; };
  };
  modes = { normal = { title = "Normal"; start = true; }; locked = { title = "Locked"; }; };
  bind = keys: validate.bindings {
    where = "someone/nixos/test";
    ref = "someone/zellij";
    inherit actions modes;
    sets = [ { name = "other/keys"; keys = validate.keymap "other/keys" { "someone/zellij" = keys; }; } ];
  };

  tests = {
    # ---------- Looks ----------
    look-taken = accepted (render look);
    look-plain-taken = accepted (render { title = "Vanilla"; plain = true; }) && accepted (render null);
    look-all-fields-taken = accepted (render (with' {
      homepage = "https://example.org";
      colors.cursor = "#ABCDEF";
      desktop = {
        border = [ "#000000" "#ffffff" ]; inactive_border = "#111111"; clock = "#222222"; split = [ "#000000" "#ffffff" ];
        prompt = { ok = "#000000"; error = "#ffffff"; }; gtk = "adw-gtk3"; cursor_theme = "Bibata-Modern-Ice";
      };
      neovim = { theme = "rose-pine"; variant = "dawn"; };
      emacs = { theme = "modus"; style = "modus-operandi"; };
      macos = { accent = "graphite"; icons = "clear"; folders = "default"; wallpaper = "black"; };
    }));
    look-title-with-newline-refused = refused (render (with' { title = "x\nos.execute(\"x\")"; }));
    look-title-with-escape-refused = refused (render (with' { title = "x\\u001b[2Jy"; } // { title = builtins.fromJSON "\"x\\u001b[2Jy\""; }));
    look-title-with-tab-or-return-refused = refused (render (with' { title = "a\tb"; })) && refused (render (with' { title = "a\rb"; }));
    look-title-with-line-separator-refused = refused (render (look // { title = builtins.fromJSON "\"a\\u2028b\""; }));
    look-title-not-text-refused = refused (render (look // { title = [ "x" ]; }));
    look-description-with-newline-refused = refused (render (with' { description = "a\nb"; }))
      && refused (render (with' { description_de = "a\nb"; }));
    look-colour-injection-refused = refused (render (with' { colors.accent = inject; }))
      && refused (render (with' { terminal.normal = eight inject; }))
      && refused (render (with' { desktop.border = [ inject ]; }))
      && refused (render (with' { desktop.prompt.ok = inject; }));
    look-colour-shapes-refused = lib.all (c: refused (render (with' { colors.text = c; })))
      [ "#fff" "#ffffff80" "ffffff" "#gggggg" "#ffffff\n" " #ffffff" 16777215 null ];
    look-terminal-of-seven-refused = refused (render (look // { terminal = { normal = lib.genList (_: "#000000") 7; bright = eight "#000000"; }; }));
    look-missing-colours-refused = refused (render (builtins.removeAttrs look [ "colors" ]))
      && refused (render (look // { colors = builtins.removeAttrs look.colors [ "urgent" ]; }));
    look-unknown-key-refused = refused (render (look // { lua = "os.execute('x')"; }))
      && refused (render (with' { colors.extra = "#000000"; }))
      && refused (render (with' { desktop.font = "Mono"; }))
      && refused (render (look // { description_x = "x"; }));
    look-choice-refused = refused (render (with' { appearance = "dark\" .. os.execute(\"x\") .. \""; }))
      && refused (render (with' { neovim = { theme = "catppuccin"; flavour = "mocha\", x = os.execute(\"x\"), y = \""; }; }))
      && refused (render (with' { neovim = { theme = "modus"; variant = "main"; }; }))
      && refused (render (with' { neovim = { theme = "gruvbox"; }; }))
      && refused (render (with' { emacs = { theme = "catppuccin"; flavour = "mocha) (shell-command \"x\""; }; }))
      && refused (render (with' { emacs = { theme = "modus"; style = "modus-vivendi-palette-overrides nil) (shell-command \"x\") (setq x"; }; }))
      && refused (render (with' { emacs = { flavour = "mocha"; }; }))
      && refused (render (with' { desktop.gtk = "Adwaita\"; exec x"; }))
      && refused (render (with' { desktop.cursor_theme = "x\ny"; }))
      && refused (render (with' { macos.accent = "blue"; }));
    look-types-refused = refused (render (look // { plain = "true"; }))
      && refused (render (look // { colors = "none"; }))
      && refused (render (look // { tags = "dark"; }))
      && refused (render (look // { version = "1.0"; }))
      && refused (render (look // { "$schema" = "https://omaflake.dev/schema/look/v2.json"; }))
      && refused (render "a look");
    # The adapter alone refuses as well
    adapter-refuses-by-itself =
      let adapt = import ../looks/adapter.nix { inherit pkgs lib; }; in
      refused ((adapt (with' { colors.accent = inject; })).wezterm)
      && refused ((adapt (with' { neovim = { theme = "catppuccin"; flavour = "x\"y"; }; })).nvim.lua)
      && refused ((adapt (with' { emacs = { theme = "modus"; style = "x) (y"; }; })).emacs.elisp);

    # ---------- Key binding sets ----------
    keymap-taken =
      (bind { quit = "Ctrl+x"; lock = { key = [ "Ctrl+g" "Super+Shift+Return" ]; after = "normal"; }; }) == {
        quit = actions.quit // { key = "Ctrl+x"; };
        lock = actions.lock // { key = [ "Ctrl+g" "Super+Shift+Return" ]; after = "normal"; };
        inherit (actions) leader;
      };
    keymap-after-alone-keeps-the-key = (bind { lock = { after = "normal"; }; }).lock == actions.lock // { after = "normal"; };
    keymap-does-refused = refused (bind { quit = { key = "Ctrl+x"; does = "Run \"sh\" \"-c\" \"x\""; }; });
    keymap-other-fields-refused = lib.all (f: refused (bind { quit = { key = "Ctrl+x"; ${f} = true; }; }))
      [ "does" "mode" "own" "desktop" "context" "title" "anything" ];
    keymap-empty-entry-refused = refused (bind { quit = { }; }) && refused (bind { quit = [ ]; }) && refused (bind { quit = 1; });
    keymap-after-refused = refused (bind { lock = { after = "nowhere"; }; })
      && refused (bind { lock = { after = "normal\"; Run \"sh"; }; });
    keymap-own-key-stays = (bind { leader = "Ctrl+l"; }).leader == actions.leader;
    keymap-unknown-action-ignored = (bind { other = "Ctrl+o"; }) == actions;
    keymap-names-refused = refused (validate.keymap "other/keys" { "no module" = { }; })
      && refused (validate.keymap "other/keys" { "someone/zellij" = "Ctrl+q"; })
      && refused (validate.keymap "other/keys" { "someone/zellij"."an action" = "Ctrl+q"; })
      && refused (validate.keymap "other/keys" [ ]);
    module-key-checked = refused (validate.bindings {
      where = "someone/nixos/test"; ref = "someone/x"; sets = [ ];
      actions.run = { title = "Run"; key = "Super+\""; };
    });

    # ---------- Keys ----------
    keys-taken = lib.all validate.isKey [
      "Super+Shift+Return" "Ctrl+g" "Space f f" "Shift+l" "Super+Arrow" "Super+Shift+Digit" "Digit" "Escape" "Tab"
      "Ctrl+Alt+Shift+Super+F12" "F1" "PageUp" "Plus" "=" "-" "[" "]" ";" "," "." "/" "a" "0" "Ctrl+c" "Space t p"
    ];
    keys-refused = lib.all (k: !(validate.isKey k) && refused (bind { quit = k; })) [
      "Super+\"" "\"" "'" "\\" "Super+\\" "`" "Super+Q" "A" "Hyper+x" "Super+Super+x" "Super+" "+" "Ctrl++" "" " " "a  b"
      " a" "a " "a\tb" "a\nb" "Ctrl+g\n" "<" ">" "<C-x>" "Super+<" "Return\", hl.dsp.exec_cmd(\"x\")); hl.bind(\"SUPER + x"
      "ab" "F13" "F0" "return" "é" "Super + x" "{" "}" "(" ")" "$" "#" "|" "~" "*" "?" "!" "@" "%" "^" "&" "_" ":"
    ];
    key-with-quote-refused = refused (bind { quit = "Ctrl+\""; }) && refused (bind { quit = [ "Ctrl+x" "'" ]; })
      && refused (bind { quit = { key = "\""; }; });
    keylib-refuses-by-itself = lib.all (f: refused (f "Super+\"") && refused (f "Super+\\") && refused (f "a\nb"))
      (with keyLib; [ hyprland zellij wezterm neovim emacs label expand ]);
    keylib-notations = keyLib.hyprland "Super+Shift+Return" == "SUPER + SHIFT + Return"
      && keyLib.hyprland "Super+q" == "SUPER + Q"
      && keyLib.zellij "Ctrl+g" == "Ctrl g" && keyLib.zellij "Shift+h" == "H"
      && keyLib.wezterm "Super+Shift+Return" == { key = "Enter"; mods = "SUPER|SHIFT"; }
      && keyLib.neovim "Space f f" == "<Space>ff" && keyLib.neovim "Ctrl+Shift+p" == "<C-S-p>"
      && keyLib.emacs "Ctrl+x g" == "C-x g"
      && map (e: e.key) (keyLib.expand "Super+Arrow") == [ "Super+Left" "Super+Right" "Super+Up" "Super+Down" ]
      && builtins.length (keyLib.expand "Super+Digit") == 9;

    # ---------- Text ----------
    text = validate.isText "Rosé Pine Dawn – \"mild\" \\ 'light'" && !(validate.isText "a\nb") && !(validate.isText "a\tb")
      && !(validate.isText (builtins.fromJSON "\"a\\u007fb\"")) && !(validate.isText (builtins.fromJSON "\"a\\u0085b\""))
      && !(validate.isText (builtins.fromJSON "\"a\\u202eb\"")) && !(validate.isText 1);
  };
in
builtins.attrNames (lib.filterAttrs (_: passed: !(builtins.tryEval passed).success || !(builtins.tryEval passed).value) tests)
