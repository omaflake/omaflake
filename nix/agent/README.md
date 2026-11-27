# omaflake-agent

[omaflake](https://omaflake.dev) – clean, disposable NixOS and macOS VMs on Apple silicon and Linux, declared in Nix ("recipes"), started from a menu bar app or the command line `omaf`.

omaflake's guest agent: it runs inside omaflake's VMs (NixOS and macOS) and talks to omaflake on the host over the
virtio port `org.omaflake.agent` – status, clipboard, camera, MIDI and the diagnosis.

omaflake builds it into its images with Nix; it is on crates.io so the code that runs in every VM can be read for each
version. You do not install it yourself.

Licence: AGPL-3.0-or-later. Source and issues: <https://github.com/omaflake/omaflake>. All of omaflake's crates are released together, with omaflake's version.
