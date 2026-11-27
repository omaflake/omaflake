// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Networks tab's picture in the Linux app – the same as the macOS app's (Sources/OmaflakeApp/NetworkGraph.swift,
//! plan B170, B181, B185): the VMs on the left, this computer and every network on the right, a line per connection –
//! dashed to the computer for a VM's own NAT, solid to a network with the VM's address, green while connected.
//! A drag from a VM draws a line to the pointer; let go over a network and the VM joins it (`networks.join`); × on a
//! connection's address takes the VM out (`networks.leave`).
//!
//! What is where is plain geometry (`Layout`), tested without a window; the canvas draws it and turns the pointer into
//! messages.

use iced::mouse;
use iced::widget::canvas::{self, Cache, Canvas, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use omaflake_sdk::text::Text as ViewText;
use omaflake_sdk::views::common::Action;
use omaflake_sdk::views::networks::NetworkGraph;
use serde_json::json;

use crate::app::Message;
use crate::style::{self, text_size};
use crate::text::s;

pub const ROW_HEIGHT: f32 = 44.0;
pub const NODE_WIDTH: f32 = 170.0;
const NODE_HEIGHT: f32 = ROW_HEIGHT - 8.0;
/// The × of an address: its radius around its middle
const CLOSE_RADIUS: f32 = 7.0;

/// Where the nodes and the connections' addresses are, for a width
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The VMs: id and the middle of its node
    pub vms: Vec<(String, Point)>,
    /// The computer ("host") and the networks: id and the middle of its node
    pub targets: Vec<(String, Point)>,
}

impl Layout {
    pub fn new(g: &NetworkGraph, width: f32) -> Layout {
        let rows = g.vms.len().max(g.networks.len() + 1).max(1);
        let y = |i: usize, count: usize| {
            (rows - count) as f32 * ROW_HEIGHT / 2.0 + i as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0
        };
        let left = NODE_WIDTH / 2.0;
        let right = (width - NODE_WIDTH / 2.0).max(left + NODE_WIDTH);
        let vms = g.vms.iter().enumerate().map(|(i, v)| (v.id.clone(), Point::new(left, y(i, g.vms.len())))).collect();
        let count = g.networks.len() + 1;
        let targets = std::iter::once("host".to_string())
            .chain(g.networks.iter().map(|n| n.id.clone()))
            .enumerate()
            .map(|(j, id)| (id, Point::new(right, y(j, count))))
            .collect();
        Layout { vms, targets }
    }

    pub fn height(g: &NetworkGraph) -> f32 {
        g.vms.len().max(g.networks.len() + 1).max(1) as f32 * ROW_HEIGHT
    }

    fn node(at: Point) -> Rectangle {
        Rectangle::new(
            Point::new(at.x - NODE_WIDTH / 2.0, at.y - NODE_HEIGHT / 2.0),
            Size::new(NODE_WIDTH, NODE_HEIGHT),
        )
    }

    /// The point a VM's lines leave from (its right edge) and a target's lines arrive at (its left edge)
    pub fn handle(&self, vm: &str) -> Option<Point> {
        self.vms.iter().find(|(id, _)| id == vm).map(|(_, p)| Point::new(p.x + NODE_WIDTH / 2.0, p.y))
    }

    pub fn inlet(&self, target: &str) -> Option<Point> {
        self.targets.iter().find(|(id, _)| id == target).map(|(_, p)| Point::new(p.x - NODE_WIDTH / 2.0, p.y))
    }

    /// The VM whose node is under `p`
    pub fn vm_at(&self, p: Point) -> Option<&str> {
        self.vms.iter().find(|(_, at)| Self::node(*at).contains(p)).map(|(id, _)| id.as_str())
    }

    /// The network whose node is under `p` – the computer takes no VM
    pub fn network_at(&self, p: Point) -> Option<&str> {
        self.targets
            .iter()
            .filter(|(id, _)| id != "host")
            .find(|(_, at)| Self::node(*at).contains(p))
            .map(|(id, _)| id.as_str())
    }

    /// Where a connection's address sits: 68 % of the way to the network, as in the macOS app
    pub fn address_at(&self, vm: &str, network: &str) -> Option<Point> {
        let (a, b) = (self.handle(vm)?, self.inlet(network)?);
        Some(Point::new(a.x + (b.x - a.x) * 0.68, a.y + (b.y - a.y) * 0.68))
    }

    /// The middle of a connection's × (after its address of `label_width`)
    pub fn close_at(&self, vm: &str, network: &str, label_width: f32) -> Option<Point> {
        self.address_at(vm, network).map(|p| Point::new(p.x + label_width / 2.0 + CLOSE_RADIUS + 2.0, p.y))
    }
}

/// An address's width in the caption size, about (monospaced digits and dots)
fn label_width(label: &str) -> f32 {
    label.chars().count() as f32 * text_size::CAPTION2 * 0.6
}

/// The picture as a canvas, as wide as there is room and as high as its rows
pub fn view(g: &NetworkGraph) -> Element<'static, Message> {
    let height = Layout::height(g);
    Canvas::new(Graph { graph: g.clone(), cache: Cache::new() })
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .into()
}

struct Graph {
    graph: NetworkGraph,
    cache: Cache,
}

/// What the pointer does
#[derive(Debug, Default)]
pub struct State {
    /// A line being drawn: from which VM, and where the pointer is
    drawing: Option<(String, Point)>,
    /// The connection whose × the pointer is on
    over_close: Option<(String, String)>,
}

fn change(method: &str, vm: &str, network: &str) -> Message {
    Message::Run(
        Box::new(Action::call(
            "graph",
            ViewText::verbatim(vm.to_string()),
            None,
            method,
            json!({"network": network, "vm": vm}),
        )),
        None,
    )
}

impl Graph {
    fn connections(&self) -> impl Iterator<Item = (&str, &str)> {
        self.graph.links.iter().filter(|l| l.kind == "network").map(|l| (l.vm.as_str(), l.to.as_str()))
    }

    /// The connection whose × is under `p`
    fn close_under(&self, layout: &Layout, p: Point) -> Option<(String, String)> {
        self.graph.links.iter().filter(|l| l.kind == "network").find_map(|l| {
            let c = layout.close_at(&l.vm, &l.to, label_width(&s(&l.label)))?;
            (c.distance(p) <= CLOSE_RADIUS + 2.0).then(|| (l.vm.clone(), l.to.clone()))
        })
    }
}

/// A connection: a curve that leaves and enters level
fn curve(from: Point, to: Point) -> Path {
    Path::new(|b| {
        let mid = (from.x + to.x) / 2.0;
        b.move_to(from);
        b.bezier_curve_to(Point::new(mid, from.y), Point::new(mid, to.y), to);
    })
}

fn dashed(color: Color, width: f32) -> Stroke<'static> {
    Stroke { line_dash: canvas::LineDash { segments: &[4.0, 3.0], offset: 0 }, ..Stroke::default() }
        .with_color(color)
        .with_width(width)
}

impl canvas::Program<Message> for Graph {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let layout = Layout::new(&self.graph, bounds.width);
        let at = cursor.position_in(bounds);
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = at?;
                if let Some((vm, network)) = self.close_under(&layout, p) {
                    return Some(canvas::Action::publish(change("networks.leave", &vm, &network)).and_capture());
                }
                if !self.graph.networks.is_empty()
                    && let Some(vm) = layout.vm_at(p)
                {
                    state.drawing = Some((vm.to_string(), p));
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let close = at.and_then(|p| self.close_under(&layout, p));
                let mut changed = close != state.over_close;
                state.over_close = close;
                if let (Some((_, to)), Some(p)) = (state.drawing.as_mut(), at) {
                    *to = p;
                    changed = true;
                }
                changed.then(canvas::Action::request_redraw)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let (vm, _) = state.drawing.take()?;
                let joined = at
                    .and_then(|p| layout.network_at(p))
                    .filter(|n| !self.connections().any(|(v, to)| v == vm && to == *n))
                    .map(|n| change("networks.join", &vm, n));
                Some(match joined {
                    Some(m) => canvas::Action::publish(m).and_capture(),
                    None => canvas::Action::request_redraw(),
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let layout = Layout::new(&self.graph, bounds.width);
        let g = &self.graph;
        // What does not move with the pointer is drawn once (the nodes); the rest each frame
        let nodes = self.cache.draw(renderer, bounds.size(), |frame| {
            let has_networks = !g.networks.is_empty();
            for v in &g.vms {
                if let Some((_, at)) = layout.vms.iter().find(|(id, _)| id == &v.id) {
                    node(frame, *at, &v.label, "", v.running, false, has_networks);
                }
            }
            let host = s(&g.host);
            if let Some((_, at)) = layout.targets.first() {
                node(frame, *at, &host, "", true, false, false);
            }
            for n in &g.networks {
                if let Some((_, at)) = layout.targets.iter().find(|(id, _)| id == &n.id) {
                    node(frame, *at, &n.label, &n.subnet, n.running, false, false);
                }
            }
        });
        let mut frame = Frame::new(renderer, bounds.size());
        for l in &g.links {
            let (Some(a), Some(b)) = (layout.handle(&l.vm), layout.inlet(&l.to)) else { continue };
            let nat = l.kind == "nat";
            let lit = state.over_close.as_ref().is_some_and(|(v, n)| v == &l.vm && n == &l.to);
            let color = if lit {
                style::DANGER
            } else if l.active {
                if nat { style::ACCENT } else { style::SUCCESS }
            } else {
                style::SECONDARY
            };
            let color = style::alpha(color, if l.active || lit { 0.85 } else { 0.35 });
            let path = curve(a, b);
            if nat {
                frame.stroke(&path, dashed(color, 1.0));
            } else {
                frame.stroke(&path, Stroke::default().with_color(color).with_width(if lit { 2.4 } else { 1.6 }));
            }
        }
        // The line being drawn: dashed until it is over a network, which then lights up
        let target = state.drawing.as_ref().and_then(|(_, p)| layout.network_at(*p).map(str::to_string));
        if let Some((vm, p)) = &state.drawing
            && let Some(a) = layout.handle(vm)
        {
            let end = target.as_deref().and_then(|t| layout.inlet(t)).unwrap_or(*p);
            let path = curve(a, end);
            if target.is_some() {
                frame.stroke(&path, Stroke::default().with_color(style::ACCENT).with_width(2.0));
            } else {
                frame.stroke(&path, dashed(style::ACCENT, 2.0));
            }
        }
        if let Some(t) = &target
            && let Some((_, at)) = layout.targets.iter().find(|(id, _)| id == t)
        {
            let r = Layout::node(*at);
            frame.stroke(
                &Path::rounded_rectangle(r.position(), r.size(), 8.0.into()),
                Stroke::default().with_color(style::ACCENT).with_width(2.0),
            );
        }
        // The addresses with their ×
        for l in g.links.iter().filter(|l| l.kind == "network") {
            let label = s(&l.label);
            let width = label_width(&label);
            let (Some(at), Some(close)) = (layout.address_at(&l.vm, &l.to), layout.close_at(&l.vm, &l.to, width))
            else {
                continue;
            };
            let lit = state.over_close.as_ref().is_some_and(|(v, n)| v == &l.vm && n == &l.to);
            let pill = Rectangle::new(
                Point::new(at.x - width / 2.0 - 5.0, at.y - 9.0),
                Size::new(width + 2.0 * CLOSE_RADIUS + 14.0, 18.0),
            );
            frame.fill(
                &Path::rounded_rectangle(pill.position(), pill.size(), 9.0.into()),
                style::alpha(style::WINDOW, 0.9),
            );
            if lit {
                frame.stroke(
                    &Path::rounded_rectangle(pill.position(), pill.size(), 9.0.into()),
                    Stroke::default().with_color(style::alpha(style::DANGER, 0.6)),
                );
            }
            frame.fill_text(Text {
                content: label,
                position: at,
                color: if l.active { style::SUCCESS } else { style::SECONDARY },
                size: text_size::CAPTION2.into(),
                align_x: iced::widget::text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                ..Text::default()
            });
            frame.fill(&Path::circle(close, CLOSE_RADIUS), if lit { style::DANGER } else { style::tertiary() });
            let arm = CLOSE_RADIUS * 0.45;
            for (dx, dy) in [(arm, arm), (arm, -arm)] {
                frame.stroke(
                    &Path::line(close - Vector::new(dx, dy), close + Vector::new(dx, dy)),
                    Stroke::default().with_color(style::WINDOW).with_width(1.5),
                );
            }
        }
        let _ = cursor;
        vec![nodes, frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        let layout = Layout::new(&self.graph, bounds.width);
        match cursor.position_in(bounds) {
            _ if state.drawing.is_some() => mouse::Interaction::Grabbing,
            Some(p) if self.close_under(&layout, p).is_some() => mouse::Interaction::Pointer,
            Some(p) if !self.graph.networks.is_empty() && layout.vm_at(p).is_some() => mouse::Interaction::Grab,
            _ => mouse::Interaction::default(),
        }
    }
}

/// A node: lamp, title (and its detail below), in a card; a VM has its handle on the right
fn node(frame: &mut Frame, at: Point, title: &str, detail: &str, on: bool, targeted: bool, handle: bool) {
    let r = Layout::node(at);
    let shape = Path::rounded_rectangle(r.position(), r.size(), 8.0.into());
    frame.fill(&shape, if targeted { style::alpha(style::ACCENT, 0.15) } else { style::CARD });
    frame.stroke(&shape, Stroke::default().with_color(if targeted { style::ACCENT } else { style::CARD_BORDER }));
    frame.fill(&Path::circle(Point::new(r.x + 12.0, at.y), 4.0), if on { style::SUCCESS } else { style::tertiary() });
    let (title_y, detail_y) = if detail.is_empty() { (at.y, at.y) } else { (at.y - 7.0, at.y + 8.0) };
    frame.fill_text(Text {
        content: title.to_string(),
        position: Point::new(r.x + 24.0, title_y),
        color: style::TEXT,
        size: text_size::CALLOUT.into(),
        max_width: NODE_WIDTH - 32.0,
        align_y: iced::alignment::Vertical::Center,
        ..Text::default()
    });
    if !detail.is_empty() {
        frame.fill_text(Text {
            content: detail.to_string(),
            position: Point::new(r.x + 24.0, detail_y),
            color: style::SECONDARY,
            size: text_size::CAPTION2.into(),
            max_width: NODE_WIDTH - 32.0,
            align_y: iced::alignment::Vertical::Center,
            ..Text::default()
        });
    }
    if handle {
        let p = Point::new(r.x + r.width, at.y);
        frame.fill(&Path::circle(p, 6.5), style::WINDOW);
        frame.fill(&Path::circle(p, 4.5), style::ACCENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omaflake_sdk::views::networks::{GraphLink, GraphNetwork, GraphVm};

    fn graph() -> NetworkGraph {
        let vm = |id: &str| GraphVm { id: id.into(), label: id.into(), kind: "nixos".into(), running: true };
        NetworkGraph {
            host: ViewText::verbatim("This computer"),
            networks: vec![GraphNetwork {
                id: "dev".into(),
                label: "dev".into(),
                subnet: "10.231.1.0/24".into(),
                running: true,
            }],
            vms: vec![vm("a"), vm("b"), vm("c")],
            links: vec![GraphLink {
                vm: "a".into(),
                to: "dev".into(),
                kind: "network".into(),
                label: ViewText::verbatim("10.231.1.10"),
                active: true,
                bytes: None,
            }],
            flows: vec![],
        }
    }

    #[test]
    fn nodes_ends_and_hits() {
        let g = graph();
        let l = Layout::new(&g, 600.0);
        assert_eq!(Layout::height(&g), 3.0 * ROW_HEIGHT);
        // Three VMs on the left; the computer and dev on the right, centred in the height of three rows
        assert_eq!(l.vms[1].1, Point::new(NODE_WIDTH / 2.0, 1.5 * ROW_HEIGHT));
        assert_eq!(l.targets[0].1, Point::new(600.0 - NODE_WIDTH / 2.0, ROW_HEIGHT));
        assert_eq!(l.handle("a"), Some(Point::new(NODE_WIDTH, ROW_HEIGHT / 2.0)));
        assert_eq!(l.inlet("dev"), Some(Point::new(600.0 - NODE_WIDTH, 2.0 * ROW_HEIGHT)));
        assert_eq!(l.vm_at(Point::new(10.0, 1.5 * ROW_HEIGHT)), Some("b"));
        assert_eq!(l.vm_at(Point::new(300.0, 1.5 * ROW_HEIGHT)), None);
        assert_eq!(l.network_at(Point::new(590.0, 2.0 * ROW_HEIGHT)), Some("dev"));
        assert_eq!(l.network_at(Point::new(590.0, ROW_HEIGHT)), None, "the computer takes no VM");
        // The × of a's connection: after its address, and the graph finds it there
        let close = l.close_at("a", "dev", label_width("10.231.1.10")).unwrap();
        let graph = Graph { graph: g.clone(), cache: Cache::new() };
        assert_eq!(graph.close_under(&l, close), Some(("a".into(), "dev".into())));
        assert_eq!(graph.close_under(&l, Point::new(close.x + 30.0, close.y)), None);
    }
}
