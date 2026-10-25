// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The main page's tabs (plan B91) as the core's pages `vms`, `images`, `recipes` and `activity` say: their rows,
// filters and tools. Every row works the same (plan B94): a click on it opens its details below.

struct HomeList: View {
    let tab: AppModel.HomeTab
    let model: AppModel

    var body: some View {
        switch model.model(model.listRef(tab)) {
        case .vms(let p)?:
            section(p.head) {
                ForEach(p.rows) { VmRowView(row: $0, model: model) }
                ForEach(Array(p.unreadable.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
            }
        case .images(let p)?:
            section(p.head) {
                ForEach(p.jobs) { BuildJobRowView(row: $0, model: model) }
                ForEach(p.rows) { ImageRowView(row: $0, model: model) }
                ForEach(Array(p.unreadable.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
            }
            if let bases = p.bases { BasesSectionView(section: bases, model: model) }
        case .recipes(let p)?:
            section(p.head) {
                if let updates = p.updates { RecipeUpdatesView(updates: updates, model: model) }
                if let suggestion = p.suggestion { HintRow(hint: suggestion, model: model) }
                if p.loading, let text = p.loadingText {
                    HStack(spacing: BrandTokens.Spacing.l) {
                        ProgressView().controlSize(.small)
                        Text(verbatim: model.t(text)).font(.callout).foregroundStyle(.secondary)
                        Spacer()
                    }
                    .padding(BrandTokens.Spacing.cardPadding)
                }
                ForEach(p.rows) { RecipeRowView(row: $0, model: model) }
                if let title = p.offeredTitle, !p.offered.isEmpty {
                    Text(verbatim: model.t(title)).font(.caption.weight(.medium)).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, BrandTokens.Spacing.xl).padding(.top, BrandTokens.Spacing.l)
                }
                ForEach(p.offered) { OfferedRowView(row: $0, model: model) }
                ForEach(Array((p.unreadable + p.errors).enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                if p.labHidden, let hint = p.labHint { HintRow(hint: hint, model: model) }
            }
            .onAppear {
                // The remote catalogs are looked into when the tab opens, never in the background
                if !p.catalogsLoaded, !p.loading, let refresh = p.head.actions.first(where: { $0.id == "refresh" }) {
                    model.run(refresh)
                }
            }
        case .looks(let p)?:
            section(p.head) {
                if p.loading, let text = p.loadingText {
                    HStack(spacing: BrandTokens.Spacing.l) {
                        ProgressView().controlSize(.small)
                        Text(verbatim: model.t(text)).font(.callout).foregroundStyle(.secondary)
                        Spacer()
                    }
                    .padding(BrandTokens.Spacing.cardPadding)
                }
                ForEach(p.rows) { LookRowView(row: $0, model: model) }
                if let title = p.offeredTitle, !p.offered.isEmpty {
                    Text(verbatim: model.t(title)).font(.caption.weight(.medium)).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, BrandTokens.Spacing.xl).padding(.top, BrandTokens.Spacing.l)
                }
                ForEach(p.offered) { LookRowView(row: $0, model: model) }
                ForEach(Array(p.unreadable.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                Text(verbatim: model.t(p.footer)).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, BrandTokens.Spacing.xl).padding(.top, BrandTokens.Spacing.l)
            }
            .onAppear {
                // The catalogs are looked into when the tab opens, never in the background
                if !p.catalogsLoaded, !p.loading, let refresh = p.head.actions.first(where: { $0.id == "refresh" }) {
                    model.run(refresh)
                }
            }
        case .activity(let p)?:
            section(p.head) {
                ForEach(p.entries) { ActivityRow(entry: $0, model: model) }
            }
        case .error(let e)?:
            Text(verbatim: e.text.map { model.t($0) } ?? e.message).font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
        default:
            ProgressView().controlSize(.small).frame(maxWidth: .infinity).padding(.top, 40)
        }
    }

    private func section(_ head: ListHead, @ViewBuilder rows: () -> some View) -> some View {
        ListSection(title: model.t(head.title), count: head.count, of: head.of,
                    filter: head.filter.map { AnyView(FilterMenu(filter: $0, tab: tab, model: model)) }) {
            HStack(spacing: BrandTokens.Spacing.xxs) {
                ForEach(head.actions) { ActionIcon(action: $0, model: model) }
            }
        } content: {
            if let empty = head.empty { HintRow(hint: empty, model: model) }
            rows()
        }
    }
}

// MARK: - VMs

struct VmRowView: View {
    let row: VmRow
    let model: AppModel

    var body: some View {
        let busy = model.isBusy(row.name)
        ExpandableRow {
            HStack(spacing: BrandTokens.Spacing.xl) {
                StatusLamp(state: busy ? .working : .init(row.lamp.state)).help(Text(verbatim: model.t(row.lamp.help)))
                KindIcon(badge: row.kind, model: model).frame(width: 16)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text(verbatim: row.name).fontWeight(.medium)
                        // What it has switched on, at a glance; the external drive and a newer image as before
                        let features = row.badges.filter { !["external", "upgradable"].contains($0.id) }
                        ForEach(row.badges.filter { $0.id == "external" }) { BadgeView(badge: $0, model: model) }
                        HStack(spacing: BrandTokens.Spacing.s) {
                            ForEach(features) { BadgeView(badge: $0, model: model, small: true) }
                        }
                        ForEach(row.badges.filter { $0.id == "upgradable" }) { BadgeView(badge: $0, model: model) }
                    }
                    Text(verbatim: model.t(row.subtitle)).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                // While an action works on it: the share folder stays, a spinner instead of start/stop
                let starting = busy || row.vm.busy
                ForEach(row.actions.filter { !starting || $0.id == "showShare" }) { ActionIcon(action: $0, model: model) }
                if starting { ProgressView().controlSize(.small) }
            }
        } detail: {
            DetailBox {
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
                    HStack(spacing: BrandTokens.Spacing.m) {
                        ForEach(Array(row.detail.switches.enumerated()), id: \.element.id) { i, a in
                            SwitchChip(action: a, model: model)
                            if i == 0 { Divider().frame(height: 14) }
                        }
                    }
                    if let note = row.detail.switchesNote {
                        Text(verbatim: model.t(note)).font(.caption2).foregroundStyle(.secondary)
                    }
                }
                InfoLines(lines: row.detail.info, model: model)
                DetailActions(actions: row.detail.actions, model: model)
            }
        }
    }
}

/// A row's info lines, selectable
struct InfoLines: View {
    let lines: [InfoLine]
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
            ForEach(Array(lines.enumerated()), id: \.offset) { InfoLineView(line: $1, model: model) }
        }
        .textSelection(.enabled)
    }
}

/// A row's actions below its details: export and update on the left, what changes or ends it on the right; the
/// reason a greyed-out one gives is the tooltip of the whole line (a disabled button shows none)
struct DetailActions: View {
    let actions: [ViewAction]
    let model: AppModel

    var body: some View {
        let leading = actions.filter { ["export", "update"].contains($0.id) }
        let trailing = actions.filter { !["export", "update"].contains($0.id) }
        let reason = actions.first { !$0.enabled && $0.reason != nil }?.reason
        HStack(spacing: BrandTokens.Spacing.l) {
            ForEach(leading) { ActionChip(action: $0, model: model) }
            Spacer()
            ForEach(trailing) { ActionChip(action: $0, model: model) }
        }
        .contentShape(Rectangle())
        .help(Text(verbatim: model.t(reason)))
    }
}

// MARK: - Images

/// A build that waits, runs or failed: its state, progress; a click runs its primary action (`open`: the build page)
struct BuildJobRowView: View {
    let row: BuildJobRow
    let model: AppModel

    private var primary: ViewAction? { row.open.primary ? row.open : row.actions.primary }

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Group {
                switch row.state {
                case .queued: Image(systemName: "clock").foregroundStyle(.secondary)
                case .checking, .running: ProgressView().controlSize(.mini)
                case .failed: Image(systemName: "xmark.octagon.fill").foregroundStyle(BrandStyle.red)
                case .finished: Image(systemName: "checkmark.circle.fill").foregroundStyle(BrandStyle.cyan)
                default: Image(systemName: "minus.circle").foregroundStyle(.secondary)
                }
            }
            .frame(width: 16)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                HStack(spacing: BrandTokens.Spacing.m) {
                    Text(verbatim: row.name).font(.callout.weight(.medium))
                    KindIcon(badge: row.kind, model: model).font(.caption2)
                    Text(verbatim: model.t(row.title)).font(.caption.weight(.medium))
                    if let count = row.count {
                        Text(verbatim: model.t(count)).font(.caption2.monospaced()).foregroundStyle(.secondary)
                    }
                }
                if let fraction = row.fraction {
                    ProgressView(value: fraction).progressViewStyle(.linear).tint(BrandStyle.cyan).controlSize(.mini)
                }
            }
            Spacer()
            ForEach(row.actions) { a in
                ActionIcon(action: a, model: model, symbol: a.id == "dismiss" ? "xmark" : "xmark.circle")
            }
            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.secondary).frame(width: 14)
        }
        .padding(.leading, 10).padding(.trailing, BrandTokens.Spacing.xl).padding(.vertical, 7)
        .contentShape(Rectangle())
        .onTapGesture { if let primary { model.run(primary) } }
        .help(Text(verbatim: model.t(primary?.label)))
    }
}

/// One built image (plan B93): what it is built from, when, the VMs on it; "New VM" from exactly this image (B100)
struct ImageRowView: View {
    let row: ImageRow
    let model: AppModel

    var body: some View {
        ExpandableRow {
            HStack(spacing: BrandTokens.Spacing.xl) {
                StatusLamp(state: .init(row.lamp.state)).help(Text(verbatim: model.t(row.lamp.help)))
                KindIcon(badge: row.kind, model: model).frame(width: 16)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text(verbatim: row.name).font(.callout.weight(.medium)).lineLimit(1).truncationMode(.middle)
                            .help(row.name)
                        ForEach(row.badges) { BadgeView(badge: $0, model: model) }
                    }
                    Text(verbatim: model.t(row.subtitle))
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        .help(row.usedBy.joined(separator: ", "))
                }
                Spacer(minLength: 4)
                ForEach(row.actions) { ActionIcon(action: $0, model: model) }
            }
        } detail: {
            DetailBox {
                InfoLines(lines: row.detail.info, model: model)
                DetailActions(actions: row.detail.actions, model: model)
            }
        }
    }
}

// MARK: - Looks

/// One look (plan B141): lamp (in use), a strip of its colours, name and what it is, the quick action; opened: its
/// description, source, usage and actions
struct LookRowView: View {
    let row: LooksPage.Row
    let model: AppModel

    var body: some View {
        ExpandableRow {
            HStack(spacing: BrandTokens.Spacing.xl) {
                StatusLamp(state: .init(row.lamp.state)).help(Text(verbatim: model.t(row.lamp.help)))
                // A look's colours; a module's row has none
                if let swatches = row.swatches { LookSwatches(colors: swatches) }
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text(verbatim: row.name).font(.callout.weight(.medium)).lineLimit(1)
                        ForEach(row.badges) { BadgeView(badge: $0, model: model) }
                    }
                    Text(verbatim: model.t(row.subtitle)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 4)
                ForEach(row.actions) { ActionIcon(action: $0, model: model) }
            }
        } detail: {
            DetailBox {
                InfoLines(lines: row.detail.info, model: model)
                DetailActions(actions: row.detail.actions, model: model)
            }
        }
    }
}

/// A look's colours as a small strip of rounded squares; a plain look shows a dashed outline
struct LookSwatches: View {
    let colors: [String]

    var body: some View {
        if colors.isEmpty {
            RoundedRectangle(cornerRadius: 3).strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [2]))
                .foregroundStyle(.secondary).frame(width: 44, height: 14)
        } else {
            HStack(spacing: 1) {
                ForEach(Array(colors.prefix(11).enumerated()), id: \.offset) { _, hex in
                    Rectangle().fill(Color(hex: hex)).frame(width: 4, height: 14)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 3))
            .overlay(RoundedRectangle(cornerRadius: 3).strokeBorder(.quaternary, lineWidth: 0.5))
        }
    }
}

extension Color {
    /// "#RRGGBB" (a look's colours); anything else is grey
    init(hex: String) {
        let s = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        let v = UInt32(s, radix: 16) ?? 0x808080
        self.init(red: Double((v >> 16) & 0xff) / 255, green: Double((v >> 8) & 0xff) / 255, blue: Double(v & 0xff) / 255)
    }
}

// MARK: - macOS bases

/// The macOS bases under the images (plan B137): one line each with Show in Finder and Remove
struct BasesSectionView: View {
    let section: BasesSection
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            Text(verbatim: model.t(section.title)).font(.caption.weight(.medium)).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            // The base being created: its progress, also after its page was left; a click opens it again
            if let job = section.job { BuildJobRowView(row: job, model: model) }
            ForEach(section.rows) { r in
                HStack(spacing: BrandTokens.Spacing.xl) {
                    StatusLamp(state: .init(r.lamp.state)).help(Text(verbatim: model.t(r.lamp.help)))
                    Image(systemName: "apple.logo").frame(width: 16).foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                        HStack(spacing: BrandTokens.Spacing.m) {
                            Text(verbatim: r.name).font(.callout.weight(.medium)).lineLimit(1)
                            ForEach(r.badges) { BadgeView(badge: $0, model: model) }
                        }
                        Text(verbatim: model.t(r.subtitle)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    ForEach(r.actions) { ActionIcon(action: $0, model: model) }
                }
                .padding(BrandTokens.Spacing.cardPadding)
            }
            if let empty = section.empty { HintRow(hint: empty, model: model) }
            HStack {
                Text(verbatim: model.t(section.footer)).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
                // Without a base the hint above has it – not a second button
                if section.empty == nil { ActionPlainButton(action: section.create, model: model) }
            }
        }
        .padding(.top, BrandTokens.Spacing.xl)
    }
}

// MARK: - Recipes

/// One recipe line (plan B111): whether its image is up to date, building it, its versions
struct RecipeRowView: View {
    let row: RecipeRow
    let model: AppModel

    var body: some View {
        ExpandableRow {
            HStack(spacing: BrandTokens.Spacing.xl) {
                StatusLamp(state: .init(row.lamp.state)).help(Text(verbatim: model.t(row.lamp.help)))
                KindIcon(badge: row.kind, model: model).frame(width: 16)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.s) {
                        Text(verbatim: row.name).fontWeight(.medium)
                        ForEach(row.secondary, id: \.self) {
                            Text(verbatim: "· \($0)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        ForEach(row.badges) { BadgeView(badge: $0, model: model) }
                    }
                    if !row.tags.isEmpty { TagList(tags: row.tags) }
                }
                Spacer()
                ForEach(row.actions) { ActionIcon(action: $0, model: model) }
            }
        } detail: {
            DetailBox {
                if let description = row.detail.description {
                    Text(description).font(.caption).fixedSize(horizontal: false, vertical: true)
                }
                InfoLines(lines: row.detail.info, model: model)
                // What is missing or wrong, its keys (plan B142)
                ForEach(Array((row.detail.notes ?? []).enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                if let keys = row.detail.keys, !keys.isEmpty {
                    Text("Keys").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                    InfoLines(lines: keys, model: model)
                }
                // The line's versions, always as sub-rows – words, export and remove always in the same place
                VStack(spacing: 0) {
                    ForEach(Array(row.detail.versions.enumerated()), id: \.element.id) { i, v in
                        if i > 0 { Divider() }
                        HStack(spacing: BrandTokens.Spacing.l) {
                            Text(verbatim: v.words).font(.caption.monospaced()).help(Text(verbatim: v.id)).textSelection(.enabled)
                            Text(verbatim: model.t(v.state)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            Spacer()
                            ForEach(v.actions) { ActionIcon(action: $0, model: model) }
                        }
                        .padding(.horizontal, BrandTokens.Spacing.l)
                        .padding(.vertical, BrandTokens.Spacing.xs)
                    }
                }
                .background(BrandTokens.Colors.inset, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.small, style: .continuous))
                DetailActions(actions: row.detail.actions, model: model)
            }
        }
    }
}

/// A recipe a remote source offers that is not installed (plan B107)
struct OfferedRowView: View {
    let row: OfferedRow
    let model: AppModel

    var body: some View {
        ExpandableRow {
            HStack(spacing: BrandTokens.Spacing.xl) {
                StatusLamp(state: .init(row.lamp.state)).help(Text(verbatim: model.t(row.lamp.help)))
                KindIcon(badge: row.kind, model: model).frame(width: 16)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.s) {
                        Text(verbatim: row.name).fontWeight(.medium).foregroundStyle(.secondary)
                        Text(verbatim: "· \(row.author)").font(.caption).foregroundStyle(.secondary)
                        ForEach(row.badges) { BadgeView(badge: $0, model: model) }
                    }
                    if !row.tags.isEmpty { TagList(tags: row.tags) }
                }
                Spacer()
                ForEach(row.actions) { ActionIcon(action: $0, model: model) }
            }
        } detail: {
            DetailBox {
                Text(row.detail.description).font(.caption).fixedSize(horizontal: false, vertical: true)
                InfoLines(lines: row.detail.info, model: model)
                if let warning = row.detail.warning {
                    Text(verbatim: model.t(warning))
                        .font(.caption2).foregroundStyle(BrandTokens.Colors.warning).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

/// Newer versions of downloaded recipes in the sources (plan B111, B114)
struct RecipeUpdatesView: View {
    let updates: RecipesPage.Updates
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(spacing: BrandTokens.Spacing.l) {
                Image(systemName: "arrow.down.circle.fill").foregroundStyle(BrandStyle.cyan)
                Text(verbatim: model.t(updates.title)).font(.callout)
                Spacer()
                if let all = updates.all { ActionChip(action: all, model: model) }
            }
            ForEach(updates.rows) { u in
                HStack(spacing: BrandTokens.Spacing.l) {
                    KindIcon(badge: u.kind, model: model).frame(width: 16)
                    Text(verbatim: u.name).font(.caption)
                    Text(verbatim: "· \(u.source)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    Spacer()
                    ActionChip(action: u.action, model: model)
                }
            }
            Text(verbatim: model.t(updates.note))
                .font(.caption2).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .padding(BrandTokens.Spacing.cardPadding)
    }
}

// MARK: - Activity

/// One line of the activity: time and what happened
struct ActivityRow: View {
    let entry: ActivityPage.Entry
    let model: AppModel

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
            Text(entry.time, format: .dateTime.hour().minute()).font(.caption.monospacedDigit()).foregroundStyle(.tertiary)
            Text(verbatim: model.t(entry.text)).font(.caption).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, BrandTokens.Spacing.xl).padding(.vertical, BrandTokens.Spacing.m)
    }
}
