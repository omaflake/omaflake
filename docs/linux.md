---
title: Linux app
nav_order: 11
permalink: /linux/
---

# omaflake on Linux – the app (plan B119)

A native window written in Rust with [iced](https://iced.rs), in omaflake's look (the same design tokens as the macOS
app, `brand/tokens.toml`), plus a tray icon. Like every omaflake interface it holds no logic of its own: it draws the
pages omaflake's core describes (the presentation model, [api §10](api.md#10-views--what-each-page-shows)) and sends
the actions they name. The host side on Linux (QEMU with KVM, the image builds) is still being brought up (issue #2):
the app runs, but what the core can do on a Linux host is OPEN.

## Install and run

With Nix (aarch64-linux, x86_64-linux):

```sh
nix run github:omaflake/omaflake#omaflake-gui          # try it
nix profile install github:omaflake/omaflake#omaflake-gui
```

The package holds the app `omaflake` and the CLI `omaf`, a desktop entry (`omaflake.desktop`) and the icon in the
hicolor theme, so it shows up in the desktop's app list. It needs a GPU driver with Vulkan (or OpenGL), Wayland or X11;
the wrapper brings the libraries iced loads at run time (Vulkan loader, libGL, Wayland, xkbcommon, the X11 libraries,
fontconfig).

From a checkout (development): `cargo run -p omaflake-gui` – on a Mac too, as a development aid (the macOS app stays the
SwiftUI one; there the tray does not exist). `VM_HOME=<folder>` points it at another data folder.

## How it connects

At start the app runs `omaf service start --json` (the `omaf` next to it, else the one on `PATH`) and connects to the
per-user core service on its socket ([api §1](api.md#1-transport)) – the same service the CLI uses, so a build started
with `omaf build` shows up in the app and the other way round. When no service can be started it runs the core in its
own process instead (a line under the tabs says so; its jobs then end with the app). `OMAFLAKE_IN_PROCESS=1` forces
that. If the service ends (an update, idle exit), the app connects again.

## What it shows

- **Home:** the header (Doctor with its issues, Settings, the data folder, Quit) and the tabs VMs, Images, Recipes,
  Activity with their badges; each list with its filter, its tools (import, refresh, clear) and rows that open to their
  details: switches (clipboard, sound, microphone, camera), info lines and the row's actions.
- **Pages:** New VM, Build Image (progress, running and quiet time, outcome, the live log), Doctor (versions, updates,
  the checks with their actions), Create macOS Base (shown as the core describes it; macOS guests need a Mac – whether the core
  greys its actions out on a Linux host is OPEN), Settings (recipe sources, general switches, guest values, data folder, and the
  app's own tray switch).
- **Questions:** confirmations and typed values (names, passwords, tokens) as a sheet over the window; folders, files
  and packages through the desktop's file chooser (XDG desktop portal), checked by the core before they count.
- **Desktop:** "Show …" and "Open …" use `xdg-open`; "Open Terminal with SSH" and the Doctor's install commands open
  `$TERMINAL`, else the first of foot, kitty, wezterm, alacritty, gnome-terminal, konsole, xterm. Notifications (VM
  ready, image built or failed, …) go to the desktop's notification service while the window is not in front and
  Settings → Notifications is on.
- Everything is English for now (the core's texts carry message keys for translations – OPEN).

## Tray

A StatusNotifierItem, on by default: KDE Plasma, Cinnamon, XFCE and waybar's `tray` module show it; GNOME only
with the AppIndicator extension (not shipped by omaflake). Its tooltip says how many VMs run and builds are under way;
its menu lists the VMs with their status and what can be done now (start, start headless, terminal, shut down), the
running builds, "Open omaflake" and Quit. A click on the icon opens the window.

While the tray is on, closing the window keeps omaflake running in the tray; Quit (in the window or the tray) ends it.
VMs keep running either way, and the service goes on for its other clients and jobs. Switch the tray off in Settings →
This app → Tray icon; the choice is stored per user in `$XDG_CONFIG_HOME/omaflake/linux-app.json` (default
`~/.config/omaflake/`), not in omaflake's own settings.

## waybar

waybar shows the tray icon with its `tray` module:

```jsonc
"modules-right": ["tray"],
"tray": { "spacing": 8 }
```

A `custom` module fed by `omaf status --waybar` (VM and build counts as JSON) is planned (plan B119) – OPEN.

## Development

- `cargo run -p omaflake-gui`; `cargo test -p omaflake-gui` – the tests decode every golden page model of the core
  (`crates/omaflake-core/testdata/api/views/`) and draw it, build the tray menu from them and check the terminal choice.
- `OMAFLAKE_DEMO_VIEWS=crates/omaflake-core/testdata/api/views` draws those golden models without a core (for looking
  at pages without VMs); `OMAFLAKE_DEMO_TAB=images`, `OMAFLAKE_DEMO_PAGE=build` (or `build.failed`, `doctor`,
  `settings`, `settings.linux`, `newVm`, `macBase`, `vm`, `image`), `OMAFLAKE_DEMO_VARIANT=linux,loading` (the home
  tabs' `<page>.<variant>.json` where there is one: `home.linux`, `recipes.loading`) and
  `OMAFLAKE_DEMO_EXPAND=vm:research` pick what is shown. Actions do nothing there.
- Icons: [Lucide](https://lucide.dev) (ISC, see `crates/omaflake-gui/NOTICE`), mapped from the views' icon names in
  `src/icons.rs`. Brand logos are not part of Lucide: macOS guests show an apple, GitHub sources a branch.
