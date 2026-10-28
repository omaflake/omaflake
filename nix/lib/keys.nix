# SPDX-License-Identifier: AGPL-3.0-or-later
# Keys as the catalog writes them (plan B142, schema module/v1 and keymap/v1) and as each program wants them.
#
# A key is a chord – modifiers and a key joined by "+", e.g. "Super+Shift+Return", "Ctrl+g" – or, for editors, a
# sequence of chords separated by spaces ("Space f f"). Modifiers: Super (the Mac's Cmd), Ctrl, Alt, Shift. Keys:
# lowercase letters, digits, punctuation, Plus, Return, Space, Escape, Tab, Backspace, Left, Right, Up, Down, PageUp,
# PageDown, F1–F12, and two placeholders a module expands itself: Digit (1–9) and Arrow (the four arrows).
{ lib }:

let
  chords = key: lib.filter (c: c != "") (lib.splitString " " key);
  parts = chord: lib.splitString "+" chord;
  modsOf = chord: lib.init (parts chord);
  keyOf = chord: lib.last (parts chord);
  single = key: let c = chords key; in
    assert lib.assertMsg (builtins.length c == 1) "\"${key}\" is a sequence – this program takes one chord";
    builtins.head c;
  arrows = [ "Left" "Right" "Up" "Down" ];
  digits = map toString (lib.range 1 9);
in
rec {
  # "Super+Arrow" → [ { key = "Super+Left"; arrow = "Left"; } … ]; "Super+Digit" → [ { key = "Super+1"; digit = "1"; } … ]
  expand = key:
    if lib.hasSuffix "Arrow" key then map (a: { key = lib.removeSuffix "Arrow" key + a; arrow = a; }) arrows
    else if lib.hasSuffix "Digit" key then map (d: { key = lib.removeSuffix "Digit" key + d; digit = d; }) digits
    else [ { inherit key; } ];

  # Hyprland's Lua API (hl.bind): "SUPER + SHIFT + Return", "SUPER + Q", "SUPER + left"
  hyprland = key:
    let
      c = single key;
      k = keyOf c;
      name = {
        Return = "Return"; Space = "space"; Escape = "escape"; Tab = "Tab"; Backspace = "BackSpace";
        Left = "left"; Right = "right"; Up = "up"; Down = "down"; Plus = "plus"; PageUp = "Prior"; PageDown = "Next";
      }.${k} or (if builtins.stringLength k == 1 then lib.toUpper k else k);
    in
    lib.concatStringsSep " + " (map lib.toUpper (modsOf c) ++ [ name ]);

  # Zellij's KDL: "Ctrl g", "Ctrl Shift Enter"; Shift and a letter alone is the capital ("Shift+h" → "H"), as
  # Zellij's presets write it
  zellij = key:
    let
      c = single key;
      k = keyOf c;
      m = modsOf c;
      name = { Return = "Enter"; Escape = "Esc"; Plus = "+"; }.${k} or k;
      letter = builtins.match "[a-z]" k != null;
    in
    if m == [ "Shift" ] && letter then lib.toUpper k else lib.concatStringsSep " " (m ++ [ name ]);

  # WezTerm's key table entry: { key = "Enter"; mods = "SUPER|SHIFT"; } (on macOS SUPER is Cmd)
  wezterm = key:
    let
      c = single key;
      k = keyOf c;
    in
    {
      key = { Return = "Enter"; Escape = "Escape"; Left = "LeftArrow"; Right = "RightArrow"; Up = "UpArrow";
        Down = "DownArrow"; Space = " "; PageUp = "PageUp"; PageDown = "PageDown"; Plus = "+"; }.${k} or k;
      mods = lib.concatStringsSep "|" (map lib.toUpper (modsOf c));
    };

  # Neovim's notation: "Space f f" → "<Space>ff", "Shift+l" → "<S-l>", "Ctrl+Shift+p" → "<C-S-p>"
  neovim = key:
    let
      special = { Return = "CR"; Space = "Space"; Escape = "Esc"; Tab = "Tab"; Backspace = "BS"; Left = "Left";
        Right = "Right"; Up = "Up"; Down = "Down"; PageUp = "PageUp"; PageDown = "PageDown"; Plus = "+"; };
      mod = { Super = "D"; Ctrl = "C"; Alt = "M"; Shift = "S"; };
      one = c:
        let m = modsOf c; k = keyOf c; in
        if m == [ ] then (if special ? ${k} then "<${special.${k}}>" else k)
        else "<${lib.concatMapStrings (x: mod.${x} + "-") m}${special.${k} or k}>";
    in
    lib.concatMapStrings one (chords key);

  # Emacs' kbd notation: "Ctrl+x g" → "C-x g"
  emacs = key:
    let
      mod = { Super = "s"; Ctrl = "C"; Alt = "M"; Shift = "S"; };
      special = { Return = "RET"; Space = "SPC"; Escape = "ESC"; Tab = "TAB"; Backspace = "DEL"; Left = "<left>";
        Right = "<right>"; Up = "<up>"; Down = "<down>"; PageUp = "<prior>"; PageDown = "<next>"; Plus = "+"; };
      one = c: lib.concatMapStrings (x: mod.${x} + "-") (modsOf c) + (special.${keyOf c} or (keyOf c));
    in
    lib.concatMapStringsSep " " one (chords key);

  # For people: "Super+Shift+Return" → "Super + Shift + Return"
  label = key: lib.concatMapStringsSep " " (c: lib.concatStringsSep " + " (parts c)) (chords key);
}
