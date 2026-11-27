# omaflake-gui

[omaflake](https://omaflake.dev) – clean, disposable NixOS and macOS VMs on Apple silicon and Linux, declared in Nix ("recipes"), started from a menu bar app or the command line `omaf`.

omaflake's Linux app (iced): a window and a tray icon, the same pages as the macOS app – a client of the page models
the core serves.

```sh
cargo install omaflake-gui   # installs the program `omaflake`
```

It needs the libraries of a Linux desktop (Vulkan or OpenGL, Wayland or X11, fontconfig) and `omaf` from
[`omaflake`](https://crates.io/crates/omaflake); the Nix package of omaflake brings both. (OPEN: `cargo install omaflake-gui` on a Linux desktop is not tested yet – plan B192.)

Licence: AGPL-3.0-or-later. Source and issues: <https://github.com/omaflake/omaflake>. All of omaflake's crates are released together, with omaflake's version.
