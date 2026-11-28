# SPDX-License-Identifier: AGPL-3.0-or-later
# The last line of defence for what catalogs call data: looks (look.toml, schema look/v1) and key binding sets
# (keymap.toml, schema keymap/v2) come from sources – also other people's – and end up inside the guest's configuration
# files (Lua, Emacs Lisp, KDL, …). Whatever omaflake and tools/catalog.py checked before, the flake checks again when it
# evaluates, and a build fails with a message that names the look or the set and the field. Nothing here escapes: a
# value is either of a shape that needs no escaping in any of the programs' notations, or it is refused.
#
# Every check returns true or throws (`builtins.tryEval` catches it – lib/validate-tests.nix).
{ lib }:

let
  inherit (builtins) isString isList isAttrs isBool match elem length all attrNames typeOf toJSON filter head;

  # ---------- Messages ----------
  # A value for a message: JSON (so control characters show as escapes), shortened
  show = v:
    if isString v then toJSON (if builtins.stringLength v > 60 then builtins.substring 0 60 v + "…" else v)
    else "a ${typeOf v}";
  fail = where: what: throw "${lib.removeSuffix ": " where} – ${what}";
  # "look \"X\": " + "colors" → "look \"X\": colors"; + "accent" → "look \"X\": colors.accent"
  sub = where: key: if lib.hasSuffix ": " where then where + key else "${where}.${key}";

  # ---------- Text ----------
  # Characters no title, description or name has: C0 and C1 control characters (newline, tab, escape, …), DEL, the
  # Unicode line and paragraph separators and the bidirectional controls. (A Nix string cannot hold U+0000.)
  controlCodes = lib.range 1 31 ++ lib.range 127 159 ++ [ 8232 8233 ] ++ lib.range 8234 8238 ++ lib.range 8294 8297;
  controls = map (n: builtins.fromJSON "\"\\u${lib.fixedWidthString 4 "0" (lib.toHexString n)}\"") controlCodes;
  noControls = s: builtins.replaceStrings controls (map (_: "") controls) s == s;

  matches = pattern: s: isString s && match pattern s != null;

  # ---------- Checks: where → value → true, or throw ----------
  text = where: v:
    if isString v && noControls v then true
    else fail where "one line of text without control characters, not ${show v}";
  pattern = p: what: where: v: if matches p v then true else fail where "${what}, not ${show v}";
  bool = where: v: if isBool v then true else fail where "true or false, not ${show v}";
  choice = choices: where: v:
    if isString v && elem v choices then true else fail where "one of ${lib.concatStringsSep ", " choices}, not ${show v}";
  const = value: where: v: if v == value then true else fail where "${toJSON value}, not ${show v}";
  listOf = counts: what: item: where: v:
    if isList v && counts (length v) then all (i: item "${where}[${toString i}]" (builtins.elemAt v i)) (lib.range 0 (length v - 1))
    else fail where "a list of ${what}, not ${if isList v then "${toString (length v)} entries" else show v}";
  # A table with these keys only (`known`: key → check; `other`: key → check or null for keys by pattern)
  tableWith = { known, required ? [ ], other ? (_: null) }: where: v:
    if !isAttrs v then fail where "a table, not ${show v}"
    else
      let
        checkOf = k: known.${k} or (other k);
        unknown = filter (k: checkOf k == null) (attrNames v);
        missing = filter (k: !(v ? ${k})) required;
      in
      if unknown != [ ] then fail where "has no ${toJSON (head unknown)} (${lib.concatStringsSep ", " (attrNames known)})"
      else if missing != [ ] then fail where "needs ${head missing}"
      else all (k: (checkOf k) (sub where k) v.${k}) (attrNames v);
  table = known: required: tableWith { inherit known required; };

  # ---------- Looks (schema look/v1) ----------
  colorPattern = "#[0-9a-fA-F]{6}";
  namePattern = "[a-z0-9][a-z0-9-]*";
  versionPattern = "(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)";
  color = pattern colorPattern "a colour \"#RRGGBB\"";
  colors = counts: what: listOf counts what color;

  # What a look may choose from (looks/adapter.nix uses the same sets)
  lookChoices = {
    appearance = [ "dark" "light" ];
    gtk = [ "adw-gtk3" "adw-gtk3-dark" ];
    cursorTheme = [
      "Bibata-Modern-Classic" "Bibata-Modern-Ice" "Bibata-Modern-Amber"
      "Bibata-Original-Classic" "Bibata-Original-Ice" "Bibata-Original-Amber"
    ];
    macos = {
      accent = [ "multicolor" "graphite" ];
      icons = [ "default" "clear" ];
      folders = [ "default" "graphite" ];
      wallpaper = [ "default" "black" ];
    };
    # Per editor and theme: its options
    neovim = {
      catppuccin.flavour = [ "latte" "frappe" "macchiato" "mocha" ];
      rose-pine.variant = [ "main" "moon" "dawn" ];
      modus.style = [ "modus_vivendi" "modus_operandi" ];
    };
    emacs = {
      catppuccin.flavour = [ "latte" "frappe" "macchiato" "mocha" ];
      modus.style = [ "modus-vivendi" "modus-operandi" ];
    };
  };

  # [neovim] / [emacs]: a theme omaflake knows with that theme's options
  editor = themes: where: v:
    if !isAttrs v then fail where "a table with theme, not ${show v}"
    else if !(v ? theme) then fail where "needs theme"
    else if !(isString v.theme && themes ? ${v.theme}) then
      fail (sub where "theme") "one of ${lib.concatStringsSep ", " (attrNames themes)}, not ${show v.theme}"
    else table ({ theme = const v.theme; } // lib.mapAttrs (_: choice) themes.${v.theme}) [ "theme" ] where v;

  lookTable = plain: tableWith {
    known = {
      "$schema" = const "https://omaflake.dev/schema/look/v1.json";
      author = pattern namePattern "lowercase letters, digits and -";
      license = text;
      title = text;
      description = text;
      tags = listOf (n: n >= 1) "at least one tag" text;
      homepage = text;
      version = pattern versionPattern "a version like \"1.0.0\"";
      appearance = choice lookChoices.appearance;
      plain = bool;
      colors = table
        (lib.genAttrs [ "background" "surface" "selection" "text" "subtext" "muted" "accent" "urgent" "cursor" ] (_: color))
        [ "background" "surface" "selection" "text" "subtext" "muted" "accent" "urgent" ];
      terminal = table { normal = colors (n: n == 8) "8 colours"; bright = colors (n: n == 8) "8 colours"; } [ "normal" "bright" ];
      desktop = table {
        border = colors (n: n == 1 || n == 2) "1 or 2 colours";
        inactive_border = color;
        clock = color;
        split = colors (n: n == 2) "2 colours";
        prompt = table { ok = color; error = color; directory = color; } [ ];
        gtk = choice lookChoices.gtk;
        cursor_theme = choice lookChoices.cursorTheme;
      } [ ];
      neovim = editor lookChoices.neovim;
      emacs = editor lookChoices.emacs;
      macos = table (lib.mapAttrs (_: choice) lookChoices.macos) [ ];
    };
    # A look that sets colours needs them (look/v1's allOf); the other required fields are for people, not for a build
    required = lib.optionals (!plain) [ "appearance" "colors" "terminal" ];
    other = k: if matches "description_[a-z]{2}(-[A-Za-z]+)?" k then text else null;
  };

  # Its name for a message, without trusting it
  lookWhere = data: "look ${show (if isAttrs data then data.title or "without title" else "")}: ";

  # ---------- Keys ----------
  # A chord: modifiers and one key joined by "+"; a key: chords separated by one space ("Space f f"). The key is a
  # name, a lowercase letter, a digit or one of - = [ ] ; , . / – nothing that means something inside a quoted string
  # of Lua, KDL, Emacs Lisp or Vim's notation (no quotes, backslash, backtick, <, >, blanks or control characters).
  # The same as tools/catalog.py's KEY in omaflake/catalog.
  modifiers = [ "Super" "Ctrl" "Alt" "Shift" ];
  namedKeys = [ "Return" "Space" "Escape" "Tab" "Backspace" "Left" "Right" "Up" "Down" "PageUp" "PageDown" "Plus" "Digit" "Arrow" ]
    ++ map (i: "F${toString i}") (lib.range 1 12);
  # A key that is its own name: a lowercase letter, a digit or one of = ; , . / - [ ]
  singlePattern = "[a-z0-9=;,./-]";
  isChord = chord:
    let p = lib.splitString "+" chord; mods = lib.init p; name = lib.last p; in
    all (m: elem m modifiers) mods && lib.allUnique mods
    && (elem name namedKeys || name == "[" || name == "]" || matches singlePattern name);
  modePattern = namePattern;
  refPattern = "${namePattern}/${namePattern}";

  isKey = k: isString k && k != "" && all isChord (lib.splitString " " k);
  key = where: v:
    if isKey v then true
    else fail where "a key like \"Super+Shift+Return\", \"Ctrl+g\" or \"Space f f\" (modifiers ${lib.concatStringsSep ", " modifiers}; a key's name, a lowercase letter, a digit or one of - = [ ] ; , . /), not ${show v}";
  # One key or a list of at least one
  keys = where: v:
    if isList v then (if v == [ ] then fail where "at least one key" else all (key where) v) else key where v;

  # What a key binding set gives an action (keymap/v2): a key, several keys, or a table with `key` and/or `after` –
  # never anything else, so a set cannot change what an action does. → { key?; after?; }
  keymapEntry = where: v:
    if isAttrs v then
      let extra = attrNames (builtins.removeAttrs v [ "key" "after" ]); in
      if extra != [ ] then
        fail where "a key binding set gives an action a key and/or `after`, not ${lib.concatMapStringsSep ", " toJSON extra}"
      else if v == { } then fail where "needs key and/or after"
      else
        lib.optionalAttrs (v ? key) (builtins.seq (keys (sub where "key") v.key) { inherit (v) key; })
        // lib.optionalAttrs (v ? after)
          (builtins.seq (pattern modePattern "a mode's name" (sub where "after") v.after) { inherit (v) after; })
    else builtins.seq (keys where v) { key = v; };
in
rec {
  inherit lookChoices colorPattern modifiers namedKeys singlePattern modePattern refPattern namePattern isKey keymapEntry;

  # One line of text without control characters: true or false
  isText = s: isString s && noControls s;

  # A key (or a list of keys) → itself, or throw
  checkedKey = where: v: builtins.seq (keys where v) v;

  # "#RRGGBB" → "RRGGBB", or throw – for whatever writes a colour into a configuration
  hexOf = where: v: builtins.seq (color where v) (lib.removePrefix "#" v);

  # One of `choices` → itself, or throw
  chosen = where: choices: v: builtins.seq (choice choices where v) v;

  # A look's data (look.toml as read) → itself, or throw
  look = data:
    let plain = isAttrs data && (data.plain or false) == true; in
    builtins.seq (lookTable plain (lookWhere data) data) data;

  # A look's title for a comment or a file: one line of text
  lookTitle = data: if data ? title then builtins.seq (text (lookWhere data + "title") data.title) data.title else "look";

  # A key binding set's `keys` (keymap.toml as read; `name`: the set's reference) → module → action → { key?; after?; },
  # or throw
  keymap = name: data:
    let where = "key bindings ${name}: "; in
    if !isAttrs data then fail where "[keys.\"<author>/<name>\"] tables, not ${show data}"
    else lib.mapAttrs (ref: actions:
      if !matches refPattern ref then fail where "[keys.\"<author>/<name>\"] names a module, not ${toJSON ref}"
      else if !isAttrs actions then fail (where + ref) "a table of action = key, not ${show actions}"
      else lib.mapAttrs (action: v:
        if !matches namePattern action then fail (where + ref) "an action's name uses lowercase letters, digits and -, not ${toJSON action}"
        else keymapEntry "${where}${ref} ${action}" v) actions) data;

  # A module's bindings in a recipe: its actions (module.toml – the module's own, like its code), each with the key
  # and `after` the recipe's key binding sets give it, in order (a later set wins). A set gives nothing else, nothing
  # to an action the module does not have and nothing to a program's own keys (`own = true`); its `after` is one of
  # the module's modes. Every key that comes out is a key.
  #   where: the recipe, for messages; ref: the module; sets: [ { name; keys = <keymap … above>; } ]
  bindings = { where, ref, actions, modes ? { }, sets }:
    let
      settable = lib.filterAttrs (_: a: !(a.own or false)) actions;
      merged = lib.foldl (acc: set:
        acc // lib.mapAttrs (a: e:
          if e ? after && !(modes ? ${e.after}) then
            fail "key bindings ${set.name}: ${ref} ${a}.after" "a mode of the module, not ${show e.after}"
          else acc.${a} // e)
          (lib.filterAttrs (a: _: settable ? ${a}) (set.keys.${ref} or { }))) actions sets;
    in
    lib.mapAttrs (a: b:
      if !(b ? key) then fail "${where}: ${ref} ${a}" "the action has no key"
      else builtins.seq (keys "${where}: ${ref} ${a}: key" b.key) b) merged;
}
