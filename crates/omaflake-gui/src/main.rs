// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's Linux app (plan B119): a native window with iced, drawing the pages the core describes (the
//! presentation model, docs/api.md §10) in omaflake's look, plus a tray icon. It holds no logic of its own: it
//! connects to the per-user core service (B120), subscribes to the pages it shows and sends the actions they name.
//! It also runs on macOS – a development aid only; the macOS app is SwiftUI.

mod app;
mod backend;
mod icons;
mod pages;
mod prefs;
mod style;
mod system;
mod text;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod tray;
mod view;
mod widgets;

use app::App;

fn main() -> iced::Result {
    iced::daemon(App::boot, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|_: &App, _| style::theme())
        .font(icons::FONT_BYTES)
        .default_font(style::UI)
        .antialiasing(true)
        .run()
}
