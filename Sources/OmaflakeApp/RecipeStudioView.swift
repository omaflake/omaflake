// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The recipe studio (plan B147, B148): a recipe of your own put together like a kit. On the left the modules are
// found – a search with suggestions while typing, tags and capabilities as chips, five modules a page, each opening
// its details; on the right the recipe – its name, its modules with what is missing or does not fit, its look and keys,
// the VM (folded). Narrow, the two are one column, the recipe first. Every choice is a form value of the page; the
// core says what follows from it.

struct RecipeStudioView: View {
    let page: RecipeStudioPage
    let ref: PageRef
    let model: AppModel

    /// Wide enough for the catalog and the recipe side by side
    private static let twoColumns: CGFloat = 720

    var body: some View {
        VStack(spacing: 0) {
            GeometryReader { geo in
                if geo.size.width >= Self.twoColumns {
                    HStack(alignment: .top, spacing: 0) {
                        Form { intro; finder }.omaflakeForm()
                        Divider()
                        Form { recipe }.omaflakeForm()
                    }
                } else {
                    Form { intro; recipe; finder }.omaflakeForm()
                }
            }
            Divider()
            HStack {
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
            .padding(.horizontal, BrandStyle.margin).padding(.vertical, BrandTokens.Spacing.xl)
        }
    }

    @ViewBuilder private var intro: some View {
        if model.helpOn { Section { HelpNote(page.intro, model: model) }.listRowBackground(Color.clear) }
    }

    // MARK: Finding modules

    @ViewBuilder private var finder: some View {
        let c = page.catalog
        Section {
            ModuleSearch(catalog: c, ref: ref, model: model)
            if !c.tags.isEmpty {
                ChipFlow(spacing: BrandTokens.Spacing.s) {
                    ForEach(c.tags) { TagChip(tag: $0, model: model) }
                }
                .padding(.vertical, BrandTokens.Spacing.xs)
            }
            if let empty = c.empty {
                Text(verbatim: model.t(empty)).font(.callout).foregroundStyle(.secondary)
            }
            ForEach(c.rows) { CatalogModuleRow(module: $0, model: model).id($0.reference) }
            if c.pages.count > 1 {
                HStack {
                    Text(verbatim: model.t(c.pages.text)).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    PagerButton(icon: "chevron.left", action: c.pages.previous, model: model)
                    Text(verbatim: "\(c.pages.index + 1) / \(c.pages.count)").font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                    PagerButton(icon: "chevron.right", action: c.pages.next, model: model)
                }
            }
        } header: {
            HStack {
                Text(verbatim: model.t(c.title))
                Spacer()
                if let reset = c.reset { ActionPlainButton(action: reset, model: model) }
                Text(verbatim: model.t(c.count)).foregroundStyle(.secondary)
            }
        }
    }

    // MARK: The recipe

    @ViewBuilder private var recipe: some View {
        Section {
            ForEach(page.about) { FormField(field: $0, page: ref.page, model: model) }
        } header: { Text(verbatim: model.t(page.titles.about)) }

        Section {
            if let empty = page.recipe.empty {
                Text(verbatim: model.t(empty)).font(.callout).foregroundStyle(.secondary)
            }
            ForEach(page.recipe.rows) { RecipeModuleRow(module: $0, model: model) }
            ForEach(Array(page.recipe.hints.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
        } header: {
            HStack {
                Text(verbatim: model.t(page.recipe.title))
                Spacer()
                if let keys = page.recipe.keys { ActionPlainButton(action: keys, model: model) }
            }
        }

        Section {
            ForEach(page.dress) { FormField(field: $0, page: ref.page, model: model) }
        } header: { Text(verbatim: model.t(page.titles.dress)) }

        // The key binding sets, as many as wanted, in order (plan B153)
        Section {
            if model.helpOn { HelpNote(page.keymaps.help, model: model) }
            if let empty = page.keymaps.empty {
                Text(verbatim: model.t(empty)).font(.callout).foregroundStyle(.secondary)
            }
            ForEach(page.keymaps.rows) { KeymapRow(row: $0, model: model) }
            if !page.keymaps.add.isEmpty {
                Menu {
                    ForEach(page.keymaps.add) { a in Button(model.t(a.label)) { model.run(a) } }
                } label: {
                    Label("Add Key Bindings", systemImage: "plus")
                }
                .menuStyle(.borderlessButton).fixedSize()
            }
        } header: { Text(verbatim: model.t(page.keymaps.title)) }

        Section {
            DisclosureGroup {
                ForEach(page.vm) { FormField(field: $0, page: ref.page, model: model) }
            } label: {
                Text(verbatim: vmSummary).foregroundStyle(.secondary)
            }
        } header: { Text(verbatim: model.t(page.titles.vm)) }
    }

    /// The VM folded: what it needs at least, in one line ("At least 4 CPU cores · …")
    private var vmSummary: String {
        page.vm.prefix(3).map { model.t($0.label) }.joined(separator: " · ")
    }
}

// MARK: - The search with suggestions

/// The search field; while something is typed, the core's suggestions below it – ↑/↓ choose, Enter adds, Esc closes
private struct ModuleSearch: View {
    let catalog: RecipeStudioPage.Catalog
    let ref: PageRef
    let model: AppModel
    @FocusState private var focused: Bool
    @State private var selected = 0
    @State private var closed = false

    private var field: Field { catalog.search }
    private var text: String { (model.formValue(ref.page, field) ?? field.value).string ?? "" }
    private var shown: [RecipeStudioPage.Suggestion] { focused && !closed && !text.isEmpty ? catalog.suggest : [] }

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
            HStack(spacing: BrandTokens.Spacing.m) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField(model.t(field.label), text: Binding(get: { text }, set: {
                    closed = false
                    selected = 0
                    model.setFormValue(ref.page, field, $0.isEmpty ? nil : .string($0), typing: true)
                }), prompt: field.placeholder.map { Text(verbatim: model.t($0)) })
                .labelsHidden()
                .textFieldStyle(.plain)
                .focused($focused)
                .onSubmit { take(selected) }
                .onKeyPress(.downArrow) { move(1) }
                .onKeyPress(.upArrow) { move(-1) }
                .onKeyPress(.escape) {
                    guard !shown.isEmpty else { return .ignored }
                    closed = true
                    return .handled
                }
                if !text.isEmpty {
                    Button { model.setFormValue(ref.page, field, nil) } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                    .help("Clear")
                }
            }
            if !shown.isEmpty {
                VStack(spacing: 0) {
                    ForEach(Array(shown.enumerated()), id: \.element.id) { i, s in
                        Button { take(i) } label: {
                            HStack(spacing: BrandTokens.Spacing.m) {
                                Text(verbatim: s.title).font(.callout.weight(.medium))
                                Text(verbatim: model.t(s.subtitle)).font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                Image(systemName: s.inside ? "checkmark" : "plus")
                                    .foregroundStyle(s.inside ? BrandTokens.Colors.success : BrandStyle.cyan)
                            }
                            .padding(.horizontal, BrandTokens.Spacing.l).padding(.vertical, BrandTokens.Spacing.s)
                            .background(i == selected ? BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill) : .clear,
                                        in: RoundedRectangle(cornerRadius: 5))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(s.action == nil)
                        .help(s.inside ? String(localized: "In the recipe") : model.t(s.action?.label))
                    }
                }
                .padding(BrandTokens.Spacing.xs)
                .background(BrandTokens.Colors.inset, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.card))
                .overlay(RoundedRectangle(cornerRadius: BrandTokens.Radius.card).strokeBorder(BrandTokens.Colors.cardBorder))
            }
        }
    }

    private func move(_ by: Int) -> KeyPress.Result {
        guard !shown.isEmpty else { return .ignored }
        selected = (selected + by + shown.count) % shown.count
        return .handled
    }

    private func take(_ i: Int) {
        guard shown.indices.contains(i), let action = shown[i].action else { return }
        model.run(action)
        selected = 0
    }
}

// MARK: - Chips, rows, the pager

/// A tag or capability: a click turns it on or off
private struct TagChip: View {
    let tag: RecipeStudioPage.Tag
    let model: AppModel

    var body: some View {
        Button { model.run(tag.action) } label: {
            HStack(spacing: BrandTokens.Spacing.xs) {
                Text(verbatim: model.t(tag.label))
                Text(verbatim: "\(tag.count)").foregroundStyle(tag.on ? BrandStyle.cyan.opacity(0.7) : Color.secondary.opacity(0.7))
            }
            .font(.caption.weight(tag.on ? .semibold : .regular))
            .foregroundStyle(tag.on ? BrandStyle.cyan : .secondary)
            .padding(.horizontal, BrandTokens.Spacing.l).padding(.vertical, 3)
            .background(tag.on ? BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill) : .clear, in: Capsule())
            .overlay(Capsule().strokeBorder(tag.on ? BrandStyle.cyan.opacity(0.6) : BrandStyle.cardBorder))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
    }
}

/// A module of the catalog: name, badges, what it is; a click opens its details; Add, or a check when it is in the
/// recipe (a click on it takes it out)
private struct CatalogModuleRow: View {
    let module: RecipeStudioPage.Module
    let model: AppModel
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(alignment: .center, spacing: BrandTokens.Spacing.l) {
                Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    .rotationEffect(.degrees(open ? 90 : 0)).frame(width: 12)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text(verbatim: module.title).font(.callout.weight(.medium))
                        ForEach(module.badges) { BadgeView(badge: $0, model: model) }
                    }
                    Text(verbatim: module.description).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: BrandTokens.Spacing.l)
                Button { model.run(module.action) } label: {
                    Image(systemName: module.inside ? "checkmark.circle.fill" : "plus.circle.fill")
                        .font(.system(size: 18))
                        .foregroundStyle(module.inside ? BrandTokens.Colors.success : BrandStyle.cyan)
                }
                .buttonStyle(.plain)
                .help(Text(verbatim: module.inside ? String(localized: "In the recipe – a click takes it out") : model.t(module.action.label)))
            }
            .contentShape(Rectangle())
            .onTapGesture { withAnimation(.easeInOut(duration: 0.15)) { open.toggle() } }
            if open {
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
                    ForEach(Array(module.details.enumerated()), id: \.offset) { _, line in
                        if line.label.key == "About" {
                            Text(verbatim: model.t(line.value)).font(.caption).fixedSize(horizontal: false, vertical: true)
                        } else {
                            InfoLineView(line: line, model: model)
                        }
                    }
                    if !module.detailActions.isEmpty {
                        HStack { ForEach(module.detailActions) { ActionPlainButton(action: $0, model: model) } }
                    }
                }
                .padding(.leading, 24)
                .transition(.opacity)
            }
        }
        .padding(.vertical, BrandTokens.Spacing.xs)
        .help(module.reference)
    }
}

/// A module of the recipe: its name and what it is, a button to take it out
private struct RecipeModuleRow: View {
    let module: RecipeStudioPage.Module
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                HStack(spacing: BrandTokens.Spacing.m) {
                    Text(verbatim: module.title).font(.callout.weight(.medium))
                    ForEach(module.badges) { BadgeView(badge: $0, model: model) }
                }
                Text(verbatim: model.t(module.subtitle)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            Button { model.run(module.action) } label: {
                Image(systemName: "xmark.circle").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .help(Text(verbatim: model.t(module.action.label)))
        }
        .help(module.reference)
    }
}

/// A key binding set of the recipe: its name, where it stands, Earlier, Later and Remove
private struct KeymapRow: View {
    let row: RecipeStudioPage.Keymaps.Row
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Image(systemName: "keyboard").foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                Text(verbatim: row.title).font(.callout.weight(.medium))
                Text(verbatim: model.t(row.subtitle)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            ForEach([("arrow.up", row.up), ("arrow.down", row.down)], id: \.0) { icon, action in
                Button { if let action { model.run(action) } } label: { Image(systemName: icon) }
                    .buttonStyle(.borderless)
                    .disabled(action == nil)
                    .help(Text(verbatim: model.t(action?.label)))
            }
            Button { model.run(row.remove) } label: {
                Image(systemName: "xmark.circle").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .help(Text(verbatim: model.t(row.remove.label)))
        }
        .help(row.reference)
    }
}

private struct PagerButton: View {
    let icon: String
    let action: ViewAction?
    let model: AppModel

    var body: some View {
        Button { if let action { model.run(action) } } label: {
            Image(systemName: icon).frame(width: 22, height: 18).contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .disabled(action == nil)
        .help(Text(verbatim: model.t(action?.label)))
    }
}

/// Chips side by side, wrapping onto the next line
struct ChipFlow: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews)
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0,
                      height: rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func arrange(width: CGFloat, _ subviews: Subviews) -> [Row] {
        var rows = [Row()]
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(.unspecified)
            if !rows[rows.count - 1].items.isEmpty && rows[rows.count - 1].width + spacing + size.width > width {
                rows.append(Row())
            }
            var r = rows[rows.count - 1]
            r.width += (r.items.isEmpty ? 0 : spacing) + size.width
            r.height = max(r.height, size.height)
            r.items.append(i)
            rows[rows.count - 1] = r
        }
        return rows
    }
}
