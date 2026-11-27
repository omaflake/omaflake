# omaflake-core

[omaflake](https://omaflake.dev) – clean, disposable NixOS and macOS VMs on Apple silicon and Linux, declared in Nix ("recipes"), started from a menu bar app or the command line `omaf`.

omaflake's engine: VM control (QEMU, Apple's Virtualization.framework), image builds with Nix, recipes and their
sources, volumes, networks, and the service every interface talks to.

**Internal to omaflake** – no promise of a stable API; it changes with omaflake. To talk to omaflake from a program
of your own, use [`omaflake-sdk`](https://crates.io/crates/omaflake-sdk).

Licence: AGPL-3.0-or-later. Source and issues: <https://github.com/omaflake/omaflake>. All of omaflake's crates are released together, with omaflake's version.
