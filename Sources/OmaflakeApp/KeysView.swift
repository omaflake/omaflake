// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The keys window (plan B147): a recipe's keys – of an image or a VM through its recipe – as the core's `keys` page
// says, by module and by the mode of its program. A mode shows the way there as key caps (⌃ G › P); a key shows the
// mode it leads to, and a click on it what it does in the program's own words.

struct KeysView: View {
    static let windowID = "keys"
    let model: AppModel
    @State private var search = ""

    var body: some View {
        Group {
            if let ref = model.keysRef, case .keys(let page)? = model.pageModel(ref) {
                content(page)
            } else if let ref = model.keysRef, case .error(let e)? = model.pageModel(ref) {
                Label(e.text.map { model.t($0) } ?? e.message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(BrandTokens.Colors.warning).padding(BrandStyle.margin)
            } else {
                ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 440, minHeight: 360)
    }

    private func content(_ page: KeysPage) -> some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xl) {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
                Text(model.t(page.title)).font(.title3.weight(.semibold))
                Text(model.t(page.subtitle)).font(.caption).foregroundStyle(.secondary)
            }
            TextField(model.t(page.search), text: $search).textFieldStyle(.roundedBorder)
            ScrollView {
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
                    Text(model.t(page.intro)).font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach(Array(page.notes.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                    ForEach(filtered(page.modules)) { ModuleCard(module: $0, model: model) }
                }
                .padding(.bottom, BrandStyle.margin)
            }
        }
        .padding([.horizontal, .top], BrandStyle.margin)
        .navigationTitle(model.t(page.title))
    }

    /// What matches the search: a key's title, its keys, what it does, its mode or module
    private func filtered(_ modules: [KeysPage.Module]) -> [KeysPage.Module] {
        let q = search.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return modules }
        return modules.compactMap { m in
            let moduleHit = m.title.lowercased().contains(q)
            var m = m
            m.groups = m.groups.compactMap { g in
                if moduleHit || model.t(g.title).lowercased().contains(q) { return g }
                var g = g
                g.rows = g.rows.filter { r in
                    model.t(r.title).lowercased().contains(q) || r.keys.contains { $0.lowercased().contains(q) }
                        || (r.does?.lowercased().contains(q) ?? false)
                }
                return g.rows.isEmpty ? nil : g
            }
            return m.groups.isEmpty ? nil : m
        }
    }
}

private struct ModuleCard: View {
    let module: KeysPage.Module
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xl) {
            Text(module.title).font(.headline)
            ForEach(Array(module.groups.enumerated()), id: \.offset) { _, g in
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
                    GroupHeader(group: g, model: model)
                    VStack(spacing: 0) {
                        ForEach(Array(g.rows.enumerated()), id: \.offset) { i, r in
                            if i > 0 { Divider().opacity(0.5) }
                            KeyRowView(row: r, model: model)
                        }
                    }
                }
            }
        }
        .padding(BrandTokens.Spacing.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(BrandTokens.Colors.card, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.card))
        .overlay(RoundedRectangle(cornerRadius: BrandTokens.Radius.card).strokeBorder(BrandTokens.Colors.cardBorder))
    }
}

/// The mode's name, "start" for the one its program starts in, and the way there: ⌃ G › Unlocked › P › Pane
private struct GroupHeader: View {
    let group: KeysPage.Group
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.m) {
            Text(model.t(group.title)).font(.subheadline.weight(.semibold)).foregroundStyle(BrandStyle.cyan)
            if group.start {
                Text("start").font(.system(size: BrandTokens.Icon.badge + 1, weight: .medium))
                    .padding(.horizontal, BrandTokens.Spacing.s).padding(.vertical, 1)
                    .background(BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill), in: Capsule())
                    .help(String(localized: "The program starts in this mode"))
            }
            Spacer(minLength: BrandTokens.Spacing.l)
            if !group.way.isEmpty {
                HStack(spacing: BrandTokens.Spacing.s) {
                    ForEach(Array(group.way.enumerated()), id: \.offset) { i, step in
                        if i > 0 { Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary) }
                        KeyCaps(chords: step.caps)
                    }
                }
                .help(group.way.map { model.t($0.mode) }.joined(separator: " › "))
            }
        }
    }
}

private struct KeyRowView: View {
    let row: KeysPage.Row
    let model: AppModel
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
                HStack(spacing: BrandTokens.Spacing.s) {
                    ForEach(Array(row.caps.enumerated()), id: \.offset) { i, key in
                        if i > 0 { Text("/").font(.caption).foregroundStyle(.tertiary) }
                        KeyCaps(chords: key)
                    }
                }
                .frame(minWidth: 120, alignment: .leading)
                Text(model.t(row.title)).font(.callout).foregroundStyle(row.switches ? .secondary : .primary)
                if row.own {
                    Text("own").font(.system(size: BrandTokens.Icon.badge + 1)).foregroundStyle(.secondary)
                        .help(String(localized: "The program's own key – key bindings do not change it"))
                }
                Spacer(minLength: BrandTokens.Spacing.l)
                if let after = row.after {
                    Label(model.t(after), systemImage: row.backToStart ? "arrow.uturn.backward" : "arrow.right")
                        .font(.caption)
                        .foregroundStyle(row.backToStart ? Color.secondary : BrandStyle.cyan)
                }
            }
            if open {
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                    if let does = row.does {
                        Text(does).font(.caption.monospaced()).textSelection(.enabled)
                    }
                    if let from = row.from {
                        Text(String(format: String(localized: "Key from %@"), from)).font(.caption2).foregroundStyle(.secondary)
                    }
                }
                .padding(.leading, 128)
            }
        }
        .padding(.vertical, BrandTokens.Spacing.m)
        .contentShape(Rectangle())
        .onTapGesture { if row.does != nil || row.from != nil { withAnimation(.snappy) { open.toggle() } } }
        .help(row.does ?? "")
    }
}

/// One key as caps: chords side by side, the caps of a chord joined
struct KeyCaps: View {
    let chords: [[String]]

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.xs) {
            ForEach(Array(chords.enumerated()), id: \.offset) { _, chord in
                HStack(spacing: 1) {
                    ForEach(Array(chord.enumerated()), id: \.offset) { _, cap in
                        Text(cap)
                            .font(.system(size: 11, weight: .medium, design: .rounded))
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .frame(minWidth: 18)
                            .background(BrandTokens.Colors.inset, in: RoundedRectangle(cornerRadius: 4))
                            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(BrandTokens.Colors.windowBorder))
                    }
                }
            }
        }
        .fixedSize()
    }
}
