// SPDX-License-Identifier: AGPL-3.0-or-later
//! The tray icon (plan B119): a StatusNotifierItem (`ksni`; KDE, Cinnamon, XFCE, waybar's `tray`; GNOME only with
//! the AppIndicator extension) with the VMs, their start/stop, running builds, "Open omaflake" and Quit. What it
//! shows is built here from the `vms` page and the jobs, the same on every platform (and tested); the D-Bus side
//! exists only on Linux.

use omaflake_core::api::types::{JobDto, JobKind};
use omaflake_core::api::views::common::{Action, LampState};
use omaflake_core::api::views::vms::VmsPage;

use crate::text::s;

/// What the tray shows
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrayModel {
    pub tooltip: String,
    pub entries: Vec<Entry>,
    /// Something needs attention or works (a running build): the icon's status
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A VM: its name and status, the actions it offers now (start, stop, terminal)
    Vm {
        label: String,
        running: bool,
        actions: Vec<Action>,
    },
    /// A line that only informs (a running build, "No VMs")
    Note(String),
    Separator,
    Open,
    Quit,
}

/// What a click in the tray asks of the app
#[derive(Debug, Clone)]
pub enum Command {
    Open,
    Quit,
    Run(Box<Action>),
}

/// The row actions the tray offers
const OFFERED: [&str; 4] = ["startWindow", "startHeadless", "terminal", "stop"];

/// The tray's menu and tooltip from the (unfiltered) VM list and the jobs
pub fn model(vms: Option<&VmsPage>, jobs: &[JobDto]) -> TrayModel {
    let mut entries = Vec::new();
    let mut running = 0;
    match vms {
        Some(page) if !page.rows.is_empty() => {
            for row in &page.rows {
                let on = row.lamp.state == LampState::On;
                running += on as usize;
                let actions: Vec<Action> = OFFERED
                    .iter()
                    .filter_map(|id| row.actions.iter().find(|a| a.id == *id && a.enabled))
                    .cloned()
                    .collect();
                entries.push(Entry::Vm {
                    label: format!("{} – {}", row.name, s(&row.lamp.help)),
                    running: on,
                    actions,
                });
            }
        }
        Some(_) => entries.push(Entry::Note("No VMs yet".into())),
        None => entries.push(Entry::Note("Connecting …".into())),
    }
    let builds: Vec<&JobDto> = jobs.iter().filter(|j| j.kind == JobKind::Build && j.state.is_active()).collect();
    if !builds.is_empty() {
        entries.push(Entry::Separator);
        for b in &builds {
            let step = b
                .progress
                .as_ref()
                .filter(|p| p.total > 0)
                .map(|p| format!(" ({}/{})", p.step, p.total))
                .unwrap_or_default();
            entries.push(Entry::Note(format!("{}{step}", b.title)));
        }
    }
    entries.push(Entry::Separator);
    entries.push(Entry::Open);
    entries.push(Entry::Quit);
    let mut parts = vec![match running {
        0 => "No VM running".to_string(),
        1 => "1 VM running".to_string(),
        n => format!("{n} VMs running"),
    }];
    match builds.len() {
        0 => {}
        1 => parts.push("1 build".into()),
        n => parts.push(format!("{n} builds")),
    }
    TrayModel { tooltip: parts.join(" · "), entries, busy: !builds.is_empty() }
}

#[cfg(target_os = "linux")]
pub use linux::{stream, TrayHandle};

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::{mpsc, Arc};

    use iced::futures::channel::mpsc::Sender;
    use iced::futures::{SinkExt, Stream};
    use ksni::blocking::TrayMethods;
    use ksni::menu::{MenuItem, StandardItem, SubMenu};

    use super::{Command, Entry, TrayModel};

    /// The brand's app icon (rounded, works on light and dark panels)
    const ICONS: [&[u8]; 2] = [
        include_bytes!("../../../brand/png/icon/omaflake-icon-32.png"),
        include_bytes!("../../../brand/png/icon/omaflake-icon-64.png"),
    ];

    struct Tray {
        model: TrayModel,
        tx: mpsc::Sender<Command>,
        icons: Vec<ksni::Icon>,
    }

    fn pixmap(png: &[u8]) -> Option<ksni::Icon> {
        let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        // ARGB32 in network byte order
        let data = img.pixels().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
        Some(ksni::Icon { width: w as i32, height: h as i32, data })
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "omaflake".into()
        }
        fn title(&self) -> String {
            "omaflake".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icons.clone()
        }
        fn status(&self) -> ksni::Status {
            ksni::Status::Active
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "omaflake".into(), description: self.model.tooltip.clone(), ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.tx.send(Command::Open);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            self.model
                .entries
                .iter()
                .map(|e| match e {
                    Entry::Vm { label, running, actions } => {
                        let submenu = actions
                            .iter()
                            .map(|a| {
                                let action = a.clone();
                                StandardItem {
                                    label: crate::text::s(&a.label),
                                    activate: Box::new(move |t: &mut Tray| {
                                        let _ = t.tx.send(Command::Run(Box::new(action.clone())));
                                    }),
                                    ..Default::default()
                                }
                                .into()
                            })
                            .collect::<Vec<_>>();
                        SubMenu {
                            label: format!("{} {label}", if *running { "●" } else { "○" }),
                            enabled: !submenu.is_empty(),
                            submenu,
                            ..Default::default()
                        }
                        .into()
                    }
                    Entry::Note(text) => {
                        StandardItem { label: text.clone(), enabled: false, ..Default::default() }.into()
                    }
                    Entry::Separator => MenuItem::Separator,
                    Entry::Open => StandardItem {
                        label: "Open omaflake".into(),
                        activate: Box::new(|t: &mut Tray| {
                            let _ = t.tx.send(Command::Open);
                        }),
                        ..Default::default()
                    }
                    .into(),
                    Entry::Quit => StandardItem {
                        label: "Quit".into(),
                        icon_name: "application-exit".into(),
                        activate: Box::new(|t: &mut Tray| {
                            let _ = t.tx.send(Command::Quit);
                        }),
                        ..Default::default()
                    }
                    .into(),
                })
                .collect()
        }
    }

    /// The running tray: the app updates its model, and shuts it down when the user switches it off
    #[derive(Clone)]
    pub struct TrayHandle(Arc<ksni::blocking::Handle<Tray>>);

    impl std::fmt::Debug for TrayHandle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("TrayHandle")
        }
    }

    impl TrayHandle {
        pub fn set(&self, model: TrayModel) {
            let h = self.0.clone();
            // update blocks on the tray's own thread: not on the UI's
            std::thread::spawn(move || {
                h.update(|t| t.model = model);
            });
        }

        pub fn shutdown(&self) {
            let h = self.0.clone();
            std::thread::spawn(move || h.shutdown().wait());
        }
    }

    #[derive(Debug, Clone)]
    pub enum Event {
        Ready(TrayHandle),
        Command(Command),
        Failed(String),
    }

    /// The tray as a stream for `Subscription::run`
    pub fn stream() -> impl Stream<Item = Event> {
        iced::stream::channel(32, async |mut out: Sender<Event>| {
            let (tx, rx) = mpsc::channel();
            let icons = super::linux::ICONS.iter().filter_map(|b| pixmap(b)).collect();
            let tray = Tray { model: TrayModel::default(), tx, icons };
            let spawned = std::thread::spawn(move || tray.spawn()).join();
            match spawned {
                Ok(Ok(handle)) => {
                    let _ = out.send(Event::Ready(TrayHandle(Arc::new(handle)))).await;
                }
                Ok(Err(e)) => {
                    let _ = out.send(Event::Failed(e.to_string())).await;
                    return;
                }
                Err(_) => return,
            }
            std::thread::spawn(move || {
                while let Ok(c) = rx.recv() {
                    if iced::futures::executor::block_on(out.send(Event::Command(c))).is_err() {
                        return;
                    }
                }
            });
            std::future::pending::<()>().await
        })
    }
}

#[cfg(target_os = "linux")]
pub use linux::Event;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::golden;

    #[test]
    fn menu_from_the_vm_list() {
        let page: VmsPage = golden("vms");
        let m = model(Some(&page), &[]);
        let vms: Vec<&Entry> = m.entries.iter().filter(|e| matches!(e, Entry::Vm { .. })).collect();
        assert_eq!(vms.len(), page.rows.len());
        // A running VM offers its terminal and stop, never a start
        let running = m.entries.iter().find_map(|e| match e {
            Entry::Vm { running: true, actions, label } => Some((label, actions)),
            _ => None,
        });
        let (label, actions) = running.expect("the fixture has a running VM");
        assert!(label.contains("Running"), "{label}");
        let ids: Vec<&str> = actions.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"stop") && !ids.contains(&"startWindow"), "{ids:?}");
        assert!(actions.iter().all(|a| a.enabled));
        assert_eq!(m.entries[m.entries.len() - 2..], [Entry::Open, Entry::Quit]);
        assert!(m.tooltip.starts_with("1 VM running"), "{}", m.tooltip);
        assert!(!m.busy);
    }

    #[test]
    fn builds_and_empty_lists() {
        let images: omaflake_core::api::views::images::ImagesPage = golden("images");
        let jobs: Vec<JobDto> = images.jobs.iter().map(|j| j.job.clone()).collect();
        let active = jobs.iter().filter(|j| j.kind == JobKind::Build && j.state.is_active()).count();
        let m = model(None, &jobs);
        assert_eq!(m.entries[0], Entry::Note("Connecting …".into()));
        assert_eq!(m.busy, active > 0);
        let notes = m.entries.iter().filter(|e| matches!(e, Entry::Note(_))).count();
        assert_eq!(notes, 1 + active);
        if active > 0 {
            assert!(m.tooltip.contains("build"), "{}", m.tooltip);
        }
        let empty = VmsPage { rows: vec![], ..golden::<VmsPage>("vms") };
        assert_eq!(model(Some(&empty), &[]).entries[0], Entry::Note("No VMs yet".into()));
        assert_eq!(model(Some(&empty), &[]).tooltip, "No VM running");
    }
}
