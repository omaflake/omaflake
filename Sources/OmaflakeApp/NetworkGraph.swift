// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

/// The Networks tab's picture (plan B170, spec §11.12): the VMs on the left; this Mac and every network on the right;
/// a line per connection – dashed to the Mac for a VM's own NAT, solid to a network with the VM's address on it, green
/// while the card is connected to the running switch. Below each network, what went between which of its VMs.
/// Edited by hand (plan B181): a VM dragged onto a network joins it, an address's menu takes the VM out again.
struct NetworkGraphView: View {
    let graph: NetworkGraph
    let model: AppModel
    /// The network a VM is being dragged over
    @State private var target: String?

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

    private let rowHeight: CGFloat = 40
    private let nodeWidth: CGFloat = 170

    /// The right column: the Mac first, then the networks
    private var targets: [(id: String, title: String, detail: String, symbol: String, on: Bool)] {
        [("host", model.t(graph.host), "", "desktopcomputer", true)]
            + graph.networks.map { ($0.id, $0.label, $0.subnet, "network", $0.running) }
    }

    var body: some View {
        let left = graph.vms
        let right = targets
        let rows = max(left.count, right.count, 1)
        GeometryReader { geo in
            let width = geo.size.width
            let leftX = nodeWidth / 2
            let rightX = width - nodeWidth / 2
            let y = { (i: Int, count: Int) -> CGFloat in
                // Each column centred in the height of the longer one
                let offset = CGFloat(rows - count) * rowHeight / 2
                return offset + CGFloat(i) * rowHeight + rowHeight / 2
            }
            ZStack(alignment: .topLeading) {
                Canvas { context, _ in
                    for link in graph.links {
                        guard let i = left.firstIndex(where: { $0.id == link.vm }),
                              let j = right.firstIndex(where: { $0.id == link.to }) else { continue }
                        let from = CGPoint(x: leftX + nodeWidth / 2, y: y(i, left.count))
                        let to = CGPoint(x: rightX - nodeWidth / 2, y: y(j, right.count))
                        var path = Path()
                        path.move(to: from)
                        path.addCurve(to: to, control1: CGPoint(x: (from.x + to.x) / 2, y: from.y),
                                      control2: CGPoint(x: (from.x + to.x) / 2, y: to.y))
                        let nat = link.kind == "nat"
                        let color: Color = link.active ? (nat ? BrandTokens.Colors.accent : BrandTokens.Colors.success) : .secondary
                        context.stroke(path, with: .color(color.opacity(link.active ? 0.85 : 0.35)),
                                       style: StrokeStyle(lineWidth: nat ? 1 : 1.6, dash: nat ? [4, 3] : []))
                    }
                }
                // The addresses on the lines, near the network's end
                ForEach(Array(graph.links.enumerated()), id: \.offset) { _, link in
                    if link.kind == "network",
                       let i = left.firstIndex(where: { $0.id == link.vm }),
                       let j = right.firstIndex(where: { $0.id == link.to }) {
                        let a = CGPoint(x: leftX + nodeWidth / 2, y: y(i, left.count))
                        let b = CGPoint(x: rightX - nodeWidth / 2, y: y(j, right.count))
                        Text(verbatim: model.t(link.label))
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(link.active ? BrandTokens.Colors.success : .secondary)
                            .padding(.horizontal, 4)
                            .background(Capsule().fill(BrandTokens.Colors.window.opacity(0.85)))
                            .position(x: a.x + (b.x - a.x) * 0.7, y: a.y + (b.y - a.y) * 0.7)
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
                         on: vm.running)
                        .draggable(vm.id)
                        .position(x: leftX, y: y(i, left.count))
                }
                ForEach(Array(right.enumerated()), id: \.offset) { j, t in
                    node(title: t.title, detail: t.detail, symbol: t.symbol, on: t.on, targeted: target == t.id)
                        .dropDestination(for: String.self) { vms, _ in
                            guard t.id != "host", let vm = vms.first, graph.vms.contains(where: { $0.id == vm }) else { return false }
                            change("networks.join", vm: vm, network: t.id)
                            return true
                        } isTargeted: { over in
                            if t.id != "host" { target = over ? t.id : (target == t.id ? nil : target) }
                        }
                        .position(x: rightX, y: y(j, right.count))
                }
            }
        }
        .frame(height: CGFloat(rows) * rowHeight)
        .padding(.vertical, BrandTokens.Spacing.s)
        if !graph.networks.isEmpty && !graph.vms.isEmpty {
            Text("Drag a VM onto a network to add it; an address's menu takes it out.")
                .font(.caption).foregroundStyle(.secondary)
        }
        if !graph.flows.isEmpty {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                ForEach(Array(graph.flows.enumerated()), id: \.offset) { _, f in
                    Text(verbatim: "\(f.network): \(f.from) → \(f.to)  \(ByteCountFormatter.string(fromByteCount: f.bytes, countStyle: .binary))")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
        }
    }

    private func node(title: String, detail: String, symbol: String, on: Bool, targeted: Bool = false) -> some View {
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
        .frame(width: nodeWidth, height: rowHeight - 6)
        .background(RoundedRectangle(cornerRadius: 8).fill(BrandTokens.Colors.card))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(targeted ? BrandTokens.Colors.accent : BrandTokens.Colors.cardBorder,
                                                          lineWidth: targeted ? 2 : 1))
    }
}
