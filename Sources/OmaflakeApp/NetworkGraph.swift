// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

/// The Networks tab's picture (plan B170, spec §11.12): the VMs on the left; this Mac and every network on the right;
/// a line per connection – dashed to the Mac for a VM's own NAT, solid to a network with the VM's address on it, green
/// while the card is connected to the running switch. Below each network, what went between which of its VMs.
///
/// Edited by hand (plan B181; reworked 2026-11-27 – the maintainer: "cool wäre ein Ziehen von Verbindungen"): from a
/// VM's handle (the dot on its right edge) a line is drawn to where the pointer is; let go over a network and the VM
/// joins it. Every connection to a network has × on its address: it takes the VM out. The address's menu does too.
struct NetworkGraphView: View {
    let graph: NetworkGraph
    let model: AppModel
    /// A connection being drawn: from which VM, to where the pointer is
    @State private var drawing: (vm: String, to: CGPoint)?
    /// The connection the pointer is over (its × shows)
    @State private var hovered: String?

    /// `networks.join` / `networks.leave` (api.md); an error is a toast
    private func change(_ method: String, vm: String, network: String) {
        Task { @MainActor in
            do {
                let _: JSONValue? = try await model.client.call(method, JSONValue.object(["network": .string(network), "vm": .string(vm)]))
            } catch {
                model.toast(model.describe(error))
            }
        }
    }

    private let rowHeight: CGFloat = 44
    private let nodeWidth: CGFloat = 170
    private let space = "networkGraph"

    /// The right column: the Mac first, then the networks
    private var targets: [(id: String, title: String, detail: String, symbol: String, on: Bool)] {
        [("host", model.t(graph.host), "", "desktopcomputer", true)]
            + graph.networks.map { ($0.id, $0.label, $0.subnet, "network", $0.running) }
    }

    var body: some View {
        let left = graph.vms
        let right = targets
        let rows = max(left.count, right.count, 1)
        if !graph.vms.isEmpty {
            Text(graph.networks.isEmpty
                 ? "Create a network below, then draw a line from a VM to it."
                 : "Draw a line from a VM's dot to a network to add it; × on a line takes it out.")
                .font(.caption).foregroundStyle(.secondary)
        }
        GeometryReader { geo in
            let width = geo.size.width
            let leftX = nodeWidth / 2
            let rightX = width - nodeWidth / 2
            let y = { (i: Int, count: Int) -> CGFloat in
                // Each column centred in the height of the longer one
                let offset = CGFloat(rows - count) * rowHeight / 2
                return offset + CGFloat(i) * rowHeight + rowHeight / 2
            }
            let handle = { (i: Int) in CGPoint(x: leftX + nodeWidth / 2, y: y(i, left.count)) }
            let inlet = { (j: Int) in CGPoint(x: rightX - nodeWidth / 2, y: y(j, right.count)) }
            // The network under a point (the host takes no VM)
            let network = { (p: CGPoint) -> String? in
                for (j, t) in right.enumerated() where t.id != "host" {
                    let frame = CGRect(x: rightX - nodeWidth / 2, y: y(j, right.count) - rowHeight / 2,
                                       width: nodeWidth, height: rowHeight)
                    if frame.contains(p) { return t.id }
                }
                return nil
            }
            let target = drawing.flatMap { network($0.to) }
            ZStack(alignment: .topLeading) {
                Canvas { context, _ in
                    for link in graph.links {
                        guard let i = left.firstIndex(where: { $0.id == link.vm }),
                              let j = right.firstIndex(where: { $0.id == link.to }) else { continue }
                        let nat = link.kind == "nat"
                        let lit = hovered == "\(link.vm)→\(link.to)"
                        let color: Color = lit ? BrandTokens.Colors.danger
                            : link.active ? (nat ? BrandTokens.Colors.accent : BrandTokens.Colors.success) : .secondary
                        context.stroke(curve(handle(i), inlet(j)),
                                       with: .color(color.opacity(link.active || lit ? 0.85 : 0.35)),
                                       style: StrokeStyle(lineWidth: nat ? 1 : (lit ? 2.4 : 1.6), dash: nat ? [4, 3] : []))
                    }
                    // The line being drawn
                    if let d = drawing, let i = left.firstIndex(where: { $0.id == d.vm }) {
                        let end = target.flatMap { t in right.firstIndex(where: { $0.id == t }) }.map(inlet) ?? d.to
                        context.stroke(curve(handle(i), end), with: .color(BrandTokens.Colors.accent),
                                       style: StrokeStyle(lineWidth: 2, lineCap: .round, dash: target == nil ? [5, 4] : []))
                    }
                }
                // The addresses on the lines, near the network's end, with × to take the VM out
                ForEach(Array(graph.links.enumerated()), id: \.offset) { _, link in
                    if link.kind == "network",
                       let i = left.firstIndex(where: { $0.id == link.vm }),
                       let j = right.firstIndex(where: { $0.id == link.to }) {
                        let a = handle(i), b = inlet(j)
                        let key = "\(link.vm)→\(link.to)"
                        HStack(spacing: 3) {
                            Text(verbatim: model.t(link.label))
                                .font(.caption2.monospacedDigit())
                                .foregroundStyle(link.active ? BrandTokens.Colors.success : .secondary)
                            Button {
                                change("networks.leave", vm: link.vm, network: link.to)
                            } label: {
                                Image(systemName: "xmark.circle.fill").font(.caption2)
                                    .foregroundStyle(hovered == key ? BrandTokens.Colors.danger : .secondary)
                            }
                            .buttonStyle(.borderless)
                            .help(Text(verbatim: String(format: String(localized: "Take %@ Out"), left[i].label)))
                        }
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(Capsule().fill(BrandTokens.Colors.window.opacity(0.9)))
                        .overlay(Capsule().stroke(hovered == key ? BrandTokens.Colors.danger.opacity(0.6) : .clear))
                        .onHover { hovered = $0 ? key : (hovered == key ? nil : hovered) }
                        .position(x: a.x + (b.x - a.x) * 0.68, y: a.y + (b.y - a.y) * 0.68)
                        .help(link.bytes.map { ByteCountFormatter.string(fromByteCount: $0, countStyle: .binary) } ?? "")
                        .contextMenu {
                            Button(String(format: String(localized: "Take %@ Out"), left[i].label), role: .destructive) {
                                change("networks.leave", vm: link.vm, network: link.to)
                            }
                        }
                    }
                }
                ForEach(Array(left.enumerated()), id: \.offset) { i, vm in
                    node(title: vm.label, detail: "", symbol: Icons.symbol(vm.kind)?.main ?? "questionmark.square.dashed",
                         on: vm.running, handle: !graph.networks.isEmpty)
                        .position(x: leftX, y: y(i, left.count))
                        // The whole node starts a line: drawn from its right edge to the pointer
                        .gesture(
                            DragGesture(minimumDistance: 4, coordinateSpace: .named(space))
                                .onChanged { drawing = (vm.id, $0.location) }
                                .onEnded { end in
                                    defer { drawing = nil }
                                    guard let net = network(end.location) else { return }
                                    let member = graph.links.contains { $0.vm == vm.id && $0.to == net }
                                    if !member { change("networks.join", vm: vm.id, network: net) }
                                }
                        )
                }
                ForEach(Array(right.enumerated()), id: \.offset) { j, t in
                    node(title: t.title, detail: t.detail, symbol: t.symbol, on: t.on, targeted: target == t.id)
                        .position(x: rightX, y: y(j, right.count))
                }
            }
            .coordinateSpace(.named(space))
        }
        .frame(height: CGFloat(rows) * rowHeight)
        .padding(.vertical, BrandTokens.Spacing.s)
        if !graph.flows.isEmpty {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                ForEach(Array(graph.flows.enumerated()), id: \.offset) { _, f in
                    Text(verbatim: "\(f.network): \(f.from) → \(f.to)  \(ByteCountFormatter.string(fromByteCount: f.bytes, countStyle: .binary))")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
        }
    }

    /// A connection: a curve that leaves and enters level
    private func curve(_ from: CGPoint, _ to: CGPoint) -> Path {
        var path = Path()
        path.move(to: from)
        let mid = (from.x + to.x) / 2
        path.addCurve(to: to, control1: CGPoint(x: mid, y: from.y), control2: CGPoint(x: mid, y: to.y))
        return path
    }

    private func node(title: String, detail: String, symbol: String, on: Bool, targeted: Bool = false,
                      handle: Bool = false) -> some View {
        HStack(spacing: BrandTokens.Spacing.m) {
            StatusLamp(state: on ? .on : .off)
            Image(systemName: symbol).frame(width: 16).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 0) {
                Text(verbatim: title).font(.callout.weight(.medium)).lineLimit(1)
                if !detail.isEmpty { Text(verbatim: detail).font(.caption2).foregroundStyle(.secondary).lineLimit(1) }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, BrandTokens.Spacing.m)
        .frame(width: nodeWidth, height: rowHeight - 8)
        .background(RoundedRectangle(cornerRadius: 8).fill(targeted ? BrandTokens.Colors.accent.opacity(0.15) : BrandTokens.Colors.card))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(targeted ? BrandTokens.Colors.accent : BrandTokens.Colors.cardBorder,
                                                          lineWidth: targeted ? 2 : 1))
        // A VM's handle: where a line starts
        .overlay(alignment: .trailing) {
            if handle {
                Circle().fill(BrandTokens.Colors.accent).frame(width: 9, height: 9)
                    .overlay(Circle().stroke(BrandTokens.Colors.window, lineWidth: 2))
                    .offset(x: 4.5)
            }
        }
        .contentShape(Rectangle())
    }
}
