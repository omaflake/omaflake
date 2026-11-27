# omaflake

[omaflake](https://omaflake.dev) – clean, disposable NixOS and macOS VMs on Apple silicon and Linux, declared in Nix ("recipes"), started from a menu bar app or the command line `omaf`.

The command line **`omaf`** and the core service it runs (`omaf serve`).

```sh
cargo install omaflake   # installs the program `omaf`
omaf doctor              # what this computer brings
omaf ui                  # the terminal interface
```

On a Mac the app (omaflake.app) brings `omaf` with everything else it needs (the runner for macOS VMs); this crate gives
the command line alone – on Linux together with [`omaflake-gui`](https://crates.io/crates/omaflake-gui), the app there.
NixOS images need Nix. Documentation: <https://omaflake.dev>.

Licence: AGPL-3.0-or-later. Source and issues: <https://github.com/omaflake/omaflake>. All of omaflake's crates are released together, with omaflake's version.
