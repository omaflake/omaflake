# Guest settings – neutral defaults

The person-specific values of every recipe (guest user, full name, time zone, locale, keyboard, SSH public keys,
macOS guest user). Recipes never contain them; they come from here:

1. **omaflake** replaces this folder when it builds (`--override-input omaflake-guest path:~/.omaflake/guest`, lock
   file untouched): user and keys from the Mac, the macOS user from the base, overrides from the app's settings.
2. **`local/settings.nix`** (git-ignored, for developers) overrides single values on top.
3. Without both (e.g. `nix flake check`), these neutral defaults apply.

Keys: same names as in `local.example/settings.nix`, plus `authorizedKeys` (public key lines).
