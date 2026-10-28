// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The key binding studio (plan B147): a key binding set of your own – its name and the set it starts from, then one
// module at a time by the mode of its program (the way there as key caps), each action's keys and the mode it leads
// to afterwards; what overlaps is said at the row. Every choice is a form value of the page.

struct KeymapStudioView: View {
    let page: KeymapStudioPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        Form {
            Section {
                Text(verbatim: model.t(page.intro)).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(page.about) { FormField(field: $0, page: ref.page, model: model) }
            }
            Section {
                FormField(field: page.module, page: ref.page, model: model)
                FormField(field: page.search, page: ref.page, model: model)
                Text(verbatim: model.t(page.changed)).font(.caption).foregroundStyle(.secondary)
                ForEach(Array(page.hints.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
            }
            ForEach(Array(page.groups.enumerated()), id: \.offset) { _, g in
                Section {
                    ForEach(Array(g.rows.enumerated()), id: \.offset) { _, r in KeymapStudioRow(row: r, ref: ref, model: model) }
                } header: {
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text(verbatim: model.t(g.title)).foregroundStyle(BrandStyle.cyan)
                        if g.start { Text("start").font(.caption2).foregroundStyle(.secondary) }
                        Spacer()
                        ForEach(Array(g.way.enumerated()), id: \.offset) { i, step in
                            if i > 0 { Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary) }
                            KeyCaps(chords: step)
                        }
                    }
                }
            }
            HStack {
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
        }
        .formStyle(.grouped)
    }
}

/// One action: its keys (a text field, "h / Left"), the mode afterwards, what it does, an overlap, Undo
private struct KeymapStudioRow: View {
    let row: KeymapStudioPage.Row
    let ref: PageRef
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
            HStack(alignment: .firstTextBaseline) {
                FormField(field: row.key, page: ref.page, model: model)
                if row.changed {
                    Circle().fill(BrandStyle.cyan).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot)
                        .help(String(localized: "Differs from the module's own"))
                }
                if let reset = row.reset { ActionPlainButton(action: reset, model: model) }
            }
            if let after = row.after { FormField(field: after, page: ref.page, model: model) }
            if let conflict = row.conflict {
                Label(model.t(conflict), systemImage: "exclamationmark.triangle.fill").font(.caption)
                    .foregroundStyle(BrandTokens.Colors.warning)
            }
            if let does = row.does {
                Text(verbatim: does).font(.caption.monospaced()).foregroundStyle(.tertiary).textSelection(.enabled)
            }
        }
    }
}
