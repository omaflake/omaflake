// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The building blocks of the core's pages (docs/api.md §10.3) drawn with the app's components: actions as icon
// buttons, action buttons, chips and plain buttons; lamps, badges, info lines, hints, filters and fields.

/// The core's icon names as SF Symbols, with the small symbol some buttons carry at the bottom right
enum Icons {
    static func symbol(_ name: String?) -> (main: String, sub: String?)? {
        switch name {
        case "vm": ("desktopcomputer", nil)
        case "image": ("shippingbox", nil)
        case "recipe": ("book.closed", nil)
        case "activity": ("clock", nil)
        case "look": ("paintpalette", nil)
        case "module": ("square.stack.3d.up", nil)
        case "gauge": ("gauge.with.dots.needle.33percent", nil)
        case "copy": ("doc.on.doc", nil)
        case "edit": ("pencil", nil)
        case "license": ("scroll", nil)
        case "nixos": ("snowflake", nil)
        case "macos": ("apple.logo", nil)
        case "start-window": ("desktopcomputer", "play.fill")
        case "start-headless": ("terminal", "play.fill")
        case "stop": ("stop.fill", nil)
        case "terminal": ("terminal", nil)
        case "folder": ("folder", nil)
        case "export": ("square.and.arrow.up", nil)
        case "import": ("square.and.arrow.down", nil)
        case "external": ("externaldrive.badge.plus", nil)
        case "hide": ("eye.slash", nil)
        case "upgrade": ("arrow.up.circle", nil)
        case "reset": ("arrow.counterclockwise", nil)
        case "trash": ("trash", nil)
        case "build": ("shippingbox", "hammer.fill")
        case "new-vm": ("desktopcomputer", "plus")
        case "create": ("plus.circle", nil)
        case "cancel": ("xmark.circle", nil)
        case "install", "download": ("arrow.down.circle", nil)
        case "refresh": ("arrow.clockwise", nil)
        case "pause": ("pause.circle", nil)
        case "apply": ("checkmark.circle", nil)
        case "clean": ("sparkles", nil)
        case "guide": ("book", nil)
        case "log": ("doc.text", nil)
        case "doctor": ("stethoscope", nil)
        case "settings": ("gearshape", nil)
        case "key": ("key", nil)
        case "globe": ("globe", nil)
        case "quit": ("power", nil)
        case "up": ("arrow.up", nil)
        case "add": ("plus", nil)
        case "clipboard": ("doc.on.clipboard", nil)
        case "sound": ("speaker.wave.2.fill", nil)
        case "microphone": ("mic.fill", nil)
        case "camera": ("camera.fill", nil)
        case "ephemeral": ("sparkles", nil)
        case "network-isolated": ("lock.shield", nil)
        case "network-none": ("wifi.slash", nil)
        case "upgradable": ("arrow.up.circle.fill", nil)
        case "lab": ("flask", nil)
        case "official": ("checkmark.seal.fill", nil)
        case "lock": ("lock.fill", nil)
        case "lock-warning": ("lock.trianglebadge.exclamationmark", nil)
        case "github": ("chevron.left.forwardslash.chevron.right", nil)
        case "url": ("globe", nil)
        case "warning": ("exclamationmark.triangle.fill", nil)
        case "ok", "finished": ("checkmark.circle.fill", nil)
        case "filter": ("line.3.horizontal.decrease.circle", nil)
        case "failed": ("xmark.octagon.fill", nil)
        case "cancelled": ("minus.circle", nil)
        default: nil
        }
    }

    /// A badge's symbol (a row's state at a glance – a few differ from the buttons of the same name)
    static func badge(_ name: String) -> String? {
        switch name {
        case "external": "externaldrive"
        case "upgrade": "arrow.up.circle.fill"
        default: symbol(name)?.main
        }
    }
}

extension Tone {
    var color: Color {
        switch self {
        case .accent: BrandStyle.cyan
        case .success: BrandTokens.Colors.success
        case .warning: BrandTokens.Colors.warning
        case .danger: BrandStyle.red
        case .normal, .unknown: .secondary
        }
    }
}

extension StatusLamp.State {
    init(_ s: LampState) {
        switch s {
        case .on: self = .on
        case .attention: self = .attention
        case .working: self = .working
        case .off, .unknown: self = .off
        }
    }
}

extension ViewAction {
    /// The tooltip: why it is greyed out, else its help, else its name
    func tooltip(_ model: AppModel) -> String {
        model.t(enabled ? (help ?? label) : (reason ?? help ?? label))
    }

    /// The Doctor's warning sign in place of an action it blocks
    var isDoctorWarning: Bool { method == "ui.show" && icon == "warning" }
}

/// An action as a small icon button (a row's or a list's tools); the Doctor's warning as the orange sign; a spinner
/// while it runs
struct ActionIcon: View {
    let action: ViewAction
    let model: AppModel
    /// Overrides the symbol (a few rows draw an action with their own)
    var symbol: String? = nil
    var form: PageRef? = nil

    var body: some View {
        if action.busy || model.isRunning(action) {
            ProgressView().controlSize(.small)
                .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight)
        } else if action.isDoctorWarning {
            Button { model.run(action) } label: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: BrandTokens.Icon.toolbar)).foregroundStyle(BrandTokens.Colors.warning)
                    .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(Text(verbatim: action.tooltip(model)))
        } else {
            let s = Icons.symbol(action.icon)
            IconButton(icon: symbol ?? s?.main ?? "questionmark", help: action.tooltip(model), badge: action.badge ?? 0,
                       subIcon: symbol == nil ? s?.sub : nil, subIconColor: BrandStyle.cyan,
                       iconColor: action.destructive ? BrandStyle.red : nil) { model.run(action, form: form) }
                .disabled(!action.enabled)
        }
    }
}

/// An action in a row's details: symbol and name in a thin frame, red for removing
struct ActionChip: View {
    let action: ViewAction
    let model: AppModel

    var body: some View {
        ActionButton(icon: Icons.symbol(action.icon)?.main ?? "circle", title: model.t(action.label),
                     color: action.destructive ? BrandStyle.red : .primary) { model.run(action) }
            .disabled(!action.enabled)
            .help(Text(verbatim: action.enabled ? model.t(action.help) : model.t(action.reason)))
    }
}

/// An action as a plain button (forms, the Doctor); a spinner next to it while it runs
struct ActionPlainButton: View {
    let action: ViewAction
    let model: AppModel
    var form: PageRef? = nil
    var isDefault = false

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.m) {
            if action.busy || model.isRunning(action) { ProgressView().controlSize(.small) }
            Button(model.t(action.label), role: action.destructive ? .destructive : nil) { model.run(action, form: form) }
                .disabled(!action.enabled || model.isRunning(action))
                .help(Text(verbatim: action.enabled ? model.t(action.help) : model.t(action.reason)))
                .modifier(DefaultAction(on: isDefault))
        }
    }

    struct DefaultAction: ViewModifier {
        let on: Bool
        func body(content: Content) -> some View {
            if on { content.keyboardShortcut(.defaultAction) } else { content }
        }
    }
}

/// A switch action as a chip (a VM's clipboard, sound, …): cyan while on
struct SwitchChip: View {
    let action: ViewAction
    let model: AppModel

    var body: some View {
        FeatureChip(icon: Icons.symbol(action.icon)?.main ?? "circle", title: model.t(action.label), isOn: action.on ?? false,
                    help: action.enabled ? model.t(action.help) : model.t(action.reason)) { model.run(action) }
            .disabled(!action.enabled)
    }
}

/// A badge next to a name: its symbol in its tone, or its short label in a capsule
struct BadgeView: View {
    let badge: Badge
    let model: AppModel
    /// The small size of a row's feature symbols (else the caption's)
    var small = false

    var body: some View {
        Group {
            if let label = badge.label {
                if badge.id == "outdated" {
                    Text(verbatim: model.t(label)).font(.caption2.weight(.semibold)).foregroundStyle(badge.tone.color)
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .overlay(Capsule().strokeBorder(badge.tone.color.opacity(BrandTokens.Opacity.tintedBorder)))
                } else {
                    Text(verbatim: model.t(label)).font(.caption2).foregroundStyle(badge.tone.color).lineLimit(1).fixedSize()
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(badge.tone.color.opacity(BrandTokens.Opacity.tintFill), in: Capsule())
                }
            } else if let symbol = Icons.badge(badge.icon) {
                let color: Color = badge.icon == "lab" ? BrandStyle.yellow : badge.tone == .normal ? .secondary : badge.tone.color
                Image(systemName: symbol)
                    .font(small ? .system(size: BrandTokens.Icon.badge, weight: .semibold) : .caption)
                    .foregroundStyle(color)
            }
        }
        .help(Text(verbatim: model.t(badge.help)))
    }
}

/// Guest kind at a glance: snowflake for NixOS, Apple logo for macOS
struct KindIcon: View {
    let badge: Badge
    let model: AppModel

    var body: some View {
        Image(systemName: Icons.symbol(badge.icon)?.main ?? "questionmark")
            .foregroundStyle(.secondary)
            .help(Text(verbatim: model.t(badge.help)))
    }
}

/// "Label  value" with a fixed label column; long values are shortened in the middle, the whole one is the tooltip.
/// A note follows the value in its tone; a tone without a note colours the value.
struct InfoLineView: View {
    let line: InfoLine
    let model: AppModel

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.xl) {
            Text(verbatim: model.t(line.label)).foregroundStyle(.secondary).lineLimit(1).frame(width: 80, alignment: .trailing)
            value.lineLimit(1).truncationMode(.middle)
                .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                .help(Text(verbatim: line.full ?? ""))
        }
        .font(.caption)
    }

    private var value: Text {
        let v = Text(verbatim: shown(line.value))
        if let note = line.note {
            let n = Text(verbatim: model.t(note)).foregroundStyle(line.tone?.color ?? .secondary)
            return Text("\(v) · \(n)")
        }
        if let tone = line.tone { return v.foregroundStyle(tone.color) }
        return v
    }

    /// Data that is a path: shortened with ~ like the Finder
    private func shown(_ t: ViewText) -> String {
        let s = model.t(t)
        return t.key == nil && s.hasPrefix("/") ? (s as NSString).abbreviatingWithTildeInPath : s
    }
}

/// A line with a symbol: an empty list, a filter that leaves nothing, a file that cannot be read, a source that failed
struct HintRow: View {
    let hint: Hint
    let model: AppModel

    var body: some View {
        HStack(alignment: hint.tone == .warning ? .firstTextBaseline : .center, spacing: BrandTokens.Spacing.l) {
            Image(systemName: Icons.symbol(hint.icon)?.main ?? "info.circle")
                .foregroundStyle(hint.tone == .normal ? AnyShapeStyle(.tertiary) : AnyShapeStyle(hint.tone.color))
            Text(verbatim: model.t(hint.text))
                .font(hint.tone == .warning ? .caption : .callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            Spacer()
            if let action = hint.action {
                if action.id == "resetFilter" {
                    Button(model.t(action.label)) { model.run(action) }.controlSize(.small)
                } else {
                    ActionChip(action: action, model: model)
                }
            }
        }
        .padding(BrandTokens.Spacing.cardPadding)
    }
}

/// What the Doctor reports against an action, with a button to it
struct BlockersBanner: View {
    let blockers: Blockers
    let model: AppModel

    var body: some View {
        HStack(alignment: .top, spacing: BrandTokens.Spacing.xl) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                Text(verbatim: model.t(blockers.title)).fontWeight(.medium)
                ForEach(Array(blockers.items.enumerated()), id: \.offset) { _, line in
                    Text(verbatim: "• \(model.t(line.title)): \(model.t(line.hint))").font(.caption)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer()
            Button(model.t(blockers.action.label)) { model.run(blockers.action) }
        }
        .padding(BrandTokens.Spacing.cardPadding)
        .background(BrandTokens.Colors.warning.opacity(BrandTokens.Opacity.tintFill), in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
    }
}

/// The filter of a list (plan B101): the core's groups – choices inline, switches as toggles; the symbol is filled
/// while a filter is set
struct FilterMenu: View {
    let filter: ViewFilter
    let tab: AppModel.HomeTab
    let model: AppModel

    private func binding(_ g: ViewFilter.Group) -> Binding<JSONValue> {
        Binding(get: { model.filters[tab]?[g.id] ?? g.value }, set: { value in
            var f = model.filters[tab] ?? [:]
            f[g.id] = value
            model.filters[tab] = f
            model.subscribe()
        })
    }

    var body: some View {
        Menu {
            ForEach(filter.groups.filter { $0.kind == .choice }) { g in
                Picker(model.t(g.label), selection: binding(g)) {
                    ForEach(Array(g.options.enumerated()), id: \.offset) { _, o in
                        if let symbol = Icons.symbol(o.icon)?.main {
                            Label(model.t(o.label), systemImage: symbol).tag(o.value)
                        } else {
                            Text(verbatim: model.t(o.label)).tag(o.value)
                        }
                    }
                }
                .pickerStyle(.inline)
            }
            let toggles = filter.groups.filter { $0.kind == .toggle }
            if !toggles.isEmpty {
                Divider()
                ForEach(toggles) { g in
                    let b = binding(g)
                    Toggle(model.t(g.label), isOn: Binding(get: { b.wrappedValue == .bool(true) }, set: { b.wrappedValue = .bool($0) }))
                }
            }
            Divider()
            Button("Reset Filter") { model.filters[tab] = nil; model.subscribe() }.disabled(!filter.active)
        } label: {
            Image(systemName: filter.active ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                .font(BrandTokens.Fonts.toolbarIcon)
                .foregroundStyle(filter.active ? BrandStyle.cyan : .primary)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight)
        .help("Filter")
    }
}

/// "12.3 GB / 26.6 GB" with a small bar while the IPSW downloads
struct DownloadProgress: View {
    let download: Download
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.m) {
            ProgressView(value: download.of > 0 ? Double(download.done) / Double(download.of) : 0).frame(width: 90)
            Text(verbatim: model.t(download.text)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
        }
    }
}

/// A question inside the window (instead of an alert window, which would close the menu window): its text, a field
/// where it needs one (a name, a password – twice where it must agree), Cancel and its answer
struct ConfirmationOverlay: View {
    let confirmation: AppModel.Confirmation

    var body: some View {
        ZStack {
            BrandTokens.Colors.scrim.ignoresSafeArea()
                .onTapGesture { confirmation.answer(nil) }
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
                Text(verbatim: confirmation.title).font(.headline).fixedSize(horizontal: false, vertical: true)
                if !confirmation.text.isEmpty {
                    Text(verbatim: confirmation.text).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                if let input = confirmation.input {
                    field(input, prompt: "")
                    if let repeated = confirmation.repeated { field(repeated, prompt: String(localized: "Repeat")) }
                }
                HStack {
                    Spacer()
                    Button("Cancel") { confirmation.answer(nil) }.keyboardShortcut(.cancelAction)
                    Button(confirmation.button, role: confirmation.destructive ? .destructive : nil) { confirmation.answer(0) }
                        .keyboardShortcut(.defaultAction)
                        .tint(confirmation.destructive ? BrandStyle.red : nil)
                        .disabled(!confirmation.canAnswer)
                }
            }
            .padding(BrandStyle.margin)
            .frame(maxWidth: 400)
            .background(BrandStyle.base, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.sheet, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: BrandTokens.Radius.sheet, style: .continuous).strokeBorder(BrandStyle.cardBorder))
            .shadow(radius: 20)
        }
    }

    @ViewBuilder private func field(_ input: AppModel.TextInput, prompt: String) -> some View {
        @Bindable var input = input
        Group {
            if confirmation.secure {
                SecureField(prompt, text: $input.text)
            } else {
                TextField(prompt, text: $input.text)
            }
        }
        .textFieldStyle(.roundedBorder)
        .onSubmit { if confirmation.canAnswer { confirmation.answer(0) } }
    }
}
