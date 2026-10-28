// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import AppKit
import OmaflakeKit

// The key binding studio (plan B147, B148): a key binding set of your own – its name and the set it starts from, the
// modules it covers as chips, then a card per module that opens to its keys by the mode of its program (the way there
// as key caps). The rows only show; "Change …" opens the key editor in a sheet: record a key or put it together from
// modifiers and a key, choose where it leads afterwards, see what it overlaps. Every choice is a form value of the
// page; the core says what follows from it.

struct KeymapStudioView: View {
    let page: KeymapStudioPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            Form {
                if model.helpOn { Section { HelpNote(page.intro, model: model) }.listRowBackground(Color.clear) }
                Section {
                    ForEach(page.about) { FormField(field: $0, page: ref.page, model: model) }
                }
                Section {
                    ModuleChips(chips: page.modules, model: model)
                    FormField(field: page.search, page: ref.page, model: model)
                    ForEach(Array(page.hints.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                } header: {
                    HStack {
                        Text("Modules")
                        Spacer()
                        Text(verbatim: model.t(page.changed)).foregroundStyle(.secondary)
                    }
                }
                ForEach(page.cards) { ModuleCard(card: $0, model: model) }
            }
            .omaflakeForm()
            Divider()
            HStack {
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary && page.editor == nil) }
            }
            .padding(.horizontal, BrandStyle.margin).padding(.vertical, BrandTokens.Spacing.xl)
        }
        .sheet(isPresented: Binding(get: { page.editor != nil }, set: { shown in
            if !shown, let e = page.editor { model.run(e.cancel) }
        })) {
            if let e = page.editor { KeyEditorSheet(editor: e, ref: ref, model: model) }
        }
    }
}

/// The modules the set covers (a click takes one out) and a menu to add another
private struct ModuleChips: View {
    let chips: [KeymapStudioPage.ModuleChip]
    let model: AppModel

    var body: some View {
        ChipFlow(spacing: BrandTokens.Spacing.s) {
            ForEach(chips.filter(\.on)) { c in
                Button { model.run(c.action) } label: {
                    HStack(spacing: BrandTokens.Spacing.xs) {
                        Text(verbatim: c.title)
                        Image(systemName: "xmark").font(.caption2.weight(.bold))
                    }
                    .font(.caption.weight(.semibold)).foregroundStyle(BrandStyle.cyan)
                    .padding(.horizontal, BrandTokens.Spacing.l).padding(.vertical, 3)
                    .background(BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill), in: Capsule())
                    .overlay(Capsule().strokeBorder(BrandStyle.cyan.opacity(0.6)))
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .help(Text(verbatim: model.t(c.action.label)))
            }
            let off = chips.filter { !$0.on }
            if !off.isEmpty {
                Menu {
                    ForEach(off) { c in Button(c.title) { model.run(c.action) } }
                } label: {
                    Label("Add Module", systemImage: "plus").font(.caption)
                }
                .menuStyle(.borderlessButton).fixedSize()
            }
        }
        .padding(.vertical, BrandTokens.Spacing.xs)
    }
}

/// A module's keys: a section that opens, its modes inside – each opening too – with the way there as caps
private struct ModuleCard: View {
    let card: KeymapStudioPage.Card
    let model: AppModel
    @State private var open: Bool?

    var body: some View {
        Section {
            DisclosureGroup(isExpanded: Binding(get: { open ?? card.open }, set: { open = $0 })) {
                ForEach(Array(card.groups.enumerated()), id: \.offset) { _, g in ModeGroup(group: g, model: model) }
            } label: {
                HStack(spacing: BrandTokens.Spacing.l) {
                    Text(verbatim: card.title).font(.headline)
                    Text(verbatim: model.t(card.count)).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    if card.changed > 0 {
                        Text(String(format: String(localized: "%lld changed"), card.changed)).font(.caption).foregroundStyle(BrandStyle.cyan)
                    }
                    if card.conflicts > 0 {
                        Label(String(format: String(localized: "%lld overlap"), card.conflicts), systemImage: "exclamationmark.triangle.fill")
                            .font(.caption).foregroundStyle(BrandTokens.Colors.warning)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture { withAnimation(.snappy) { open = !(open ?? card.open) } }
            }
        }
        .onChange(of: card.open) { open = nil }
    }
}

private struct ModeGroup: View {
    let group: KeymapStudioPage.Group
    let model: AppModel
    @State private var open: Bool?

    var body: some View {
        DisclosureGroup(isExpanded: Binding(get: { open ?? group.open }, set: { open = $0 })) {
            ForEach(Array(group.rows.enumerated()), id: \.offset) { _, r in KeyRow(row: r, model: model) }
        } label: {
            HStack(spacing: BrandTokens.Spacing.m) {
                Text(verbatim: model.t(group.title)).foregroundStyle(BrandStyle.cyan)
                if group.start { Text("start").font(.caption2).foregroundStyle(.secondary) }
                Text(verbatim: "\(group.rows.count)").font(.caption).foregroundStyle(.tertiary)
                Spacer()
                ForEach(Array(group.way.enumerated()), id: \.offset) { i, step in
                    if i > 0 { Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary) }
                    KeyCaps(chords: step)
                }
            }
            .contentShape(Rectangle())
            .onTapGesture { withAnimation(.snappy) { open = !(open ?? group.open) } }
        }
        .onChange(of: group.open) { open = nil }
    }
}

/// One action, read only: its keys as caps, what it is, where it leads; Change … and Undo
private struct KeyRow: View {
    let row: KeymapStudioPage.Row
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
                KeyAlternatives(keys: row.caps).frame(minWidth: 110, alignment: .leading)
                Text(verbatim: model.t(row.title)).foregroundStyle(row.own ? .secondary : .primary)
                if row.changed {
                    Circle().fill(BrandStyle.cyan).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot)
                        .help(String(localized: "Differs from the module's own"))
                }
                if row.own {
                    Text("own").font(.caption2).foregroundStyle(.secondary)
                        .help(String(localized: "The program's own key – key bindings do not change it"))
                }
                Spacer(minLength: BrandTokens.Spacing.l)
                if let after = row.after {
                    Label(model.t(after), systemImage: row.backToStart ? "arrow.uturn.backward" : "arrow.right")
                        .font(.caption).foregroundStyle(row.backToStart ? Color.secondary : BrandStyle.cyan)
                }
                if let reset = row.reset { ActionPlainButton(action: reset, model: model) }
                if let edit = row.edit { ActionPlainButton(action: edit, model: model) }
            }
            if let conflict = row.conflict {
                Label(model.t(conflict), systemImage: "exclamationmark.triangle.fill").font(.caption)
                    .foregroundStyle(BrandTokens.Colors.warning)
            }
        }
        .help(row.does ?? "")
    }
}

// MARK: - The key editor

/// The sheet that changes one key (plan B148): record it, or put it together from modifiers and a key – then use it
/// as the key or as one more; where it leads afterwards; what it overlaps; the module's own key to go back to
private struct KeyEditorSheet: View {
    let editor: KeymapStudioPage.Editor
    let ref: PageRef
    let model: AppModel

    enum Way: String, CaseIterable { case record, compose }
    @State private var way: Way = .record
    /// The key recorded or put together, not used yet ("Ctrl+Shift+g", "Ctrl+x Ctrl+f")
    @State private var pending: String?
    @State private var modifiers: Set<String> = []
    @StateObject private var recorder = KeyRecorder()

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
            // What is changed, and where
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
                Text(verbatim: model.t(editor.title)).font(.title3.weight(.semibold))
                HStack(spacing: BrandTokens.Spacing.m) {
                    Text(verbatim: model.t(editor.context)).font(.callout).foregroundStyle(.secondary)
                    if !editor.way.isEmpty {
                        ForEach(Array(editor.way.enumerated()), id: \.offset) { i, step in
                            Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary)
                            KeyCaps(chords: step)
                        }
                    }
                }
            }

            // The keys now, each to take out
            VStack(spacing: BrandTokens.Spacing.l) {
                if editor.caps.isEmpty {
                    Text("No key yet").font(.callout).foregroundStyle(.secondary)
                }
                HStack(spacing: BrandTokens.Spacing.l) {
                    ForEach(Array(editor.caps.enumerated()), id: \.offset) { i, key in
                        if i > 0 { Text("or").font(.callout).foregroundStyle(.tertiary) }
                        HStack(spacing: BrandTokens.Spacing.xs) {
                            KeyCaps(chords: key, size: 17)
                            if editor.remove.indices.contains(i), editor.caps.count > 1 {
                                Button { model.run(editor.remove[i]) } label: {
                                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                                }
                                .buttonStyle(.plain)
                                .help(Text(verbatim: model.t(editor.remove[i].label)))
                            }
                        }
                    }
                }
                HStack(spacing: BrandTokens.Spacing.s) {
                    Text("The module's own:").font(.caption).foregroundStyle(.secondary)
                    KeyAlternatives(keys: editor.ownCaps)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(BrandTokens.Spacing.xl)
            .background(BrandTokens.Colors.inset, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.card))

            // A new key: recorded or put together
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xl) {
                Picker("", selection: $way) {
                    Label("Record", systemImage: "record.circle").tag(Way.record)
                    Label("Put Together", systemImage: "square.grid.3x3").tag(Way.compose)
                }
                .pickerStyle(.segmented).labelsHidden()
                switch way {
                case .record: recordView
                case .compose: composeView
                }
                if let pending {
                    HStack(spacing: BrandTokens.Spacing.l) {
                        Text("New:").font(.callout).foregroundStyle(.secondary)
                        KeyCaps(chords: caps(of: pending), size: 15)
                        Spacer()
                        Button("Use as the Key") { use(pending, adding: false) }.keyboardShortcut(.defaultAction)
                        Button("Add as Another") { use(pending, adding: true) }
                    }
                }
            }

            // Where it leads afterwards
            if let after = editor.after { AfterChoice(field: after, ref: ref, model: model) }

            ForEach(Array(editor.conflicts.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
            if let does = editor.does, model.helpOn {
                HelpNote(ViewText(verbatim: String(format: String(localized: "What it does: %@"), does)), model: model)
            }

            HStack {
                ActionPlainButton(action: editor.default, model: model)
                Spacer()
                Button(model.t(editor.cancel.label)) { model.run(editor.cancel) }.keyboardShortcut(.cancelAction)
                ActionPlainButton(action: editor.apply, model: model, isDefault: pending == nil)
            }
        }
        .padding(BrandStyle.margin + 4)
        .frame(width: 520)
        .onDisappear { recorder.stop() }
    }

    // MARK: Recording

    private var recordView: some View {
        VStack(spacing: BrandTokens.Spacing.l) {
            Button {
                if recorder.recording { recorder.stop() } else {
                    pending = nil
                    recorder.start(sequences: editor.sequences) { pending = $0 }
                }
            } label: {
                HStack(spacing: BrandTokens.Spacing.m) {
                    Image(systemName: recorder.recording ? "stop.circle.fill" : "record.circle")
                        .font(.system(size: 18)).foregroundStyle(BrandStyle.red)
                    Text(recorder.recording ? "Press the keys … (click to stop)" : "Record a Key")
                }
                .padding(.horizontal, BrandTokens.Spacing.xxl).padding(.vertical, BrandTokens.Spacing.l)
                .background(BrandStyle.red.opacity(recorder.recording ? 0.18 : 0.08), in: Capsule())
                .overlay(Capsule().strokeBorder(BrandStyle.red.opacity(0.5)))
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            if recorder.recording {
                if recorder.live.isEmpty {
                    Text("Waiting for a key").font(.caption).foregroundStyle(.secondary)
                } else {
                    KeyCaps(chords: caps(of: recorder.live), size: 15)
                }
            } else {
                Text("Keys the Mac keeps for itself (Cmd+Tab, Cmd+Space, Ctrl with arrows) do not reach the app – put those together instead.")
                    .font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity)
    }

    // MARK: Putting together

    private var composeView: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.l) {
            HStack(spacing: BrandTokens.Spacing.s) {
                ForEach(editor.modifiers) { m in
                    let on = modifiers.contains(m.value)
                    Button {
                        if on { modifiers.remove(m.value) } else { modifiers.insert(m.value) }
                        if let p = pending, let last = p.split(separator: "+").last { compose(String(last)) }
                    } label: {
                        Text(verbatim: m.label).font(.callout.weight(on ? .semibold : .regular))
                            .frame(minWidth: 52).padding(.vertical, 4)
                            .foregroundStyle(on ? BrandStyle.cyan : .primary)
                            .background(on ? BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill) : BrandTokens.Colors.inset,
                                        in: RoundedRectangle(cornerRadius: 6))
                            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(on ? BrandStyle.cyan.opacity(0.6) : BrandTokens.Colors.windowBorder))
                    }
                    .buttonStyle(.plain)
                }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
                    ForEach(Array(editor.keyGroups.enumerated()), id: \.offset) { _, g in
                        Text(verbatim: model.t(g.title)).font(.caption).foregroundStyle(.secondary)
                        ChipFlow(spacing: BrandTokens.Spacing.xs) {
                            ForEach(g.keys) { k in
                                Button { compose(k.value) } label: {
                                    Text(verbatim: k.label).font(.system(size: 12, weight: .medium, design: .rounded))
                                        .frame(minWidth: 22).padding(.horizontal, 5).padding(.vertical, 2)
                                        .background(BrandTokens.Colors.inset, in: RoundedRectangle(cornerRadius: 4))
                                        .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(BrandTokens.Colors.windowBorder))
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            }
            .frame(height: 170)
        }
    }

    /// The modifiers chosen and the key, as written ("Super+Shift+g")
    private func compose(_ key: String) {
        let order = ["Super", "Ctrl", "Alt", "Shift"]
        pending = (order.filter { modifiers.contains($0) } + [key]).joined(separator: "+")
    }

    private func use(_ key: String, adding: Bool) {
        let keys = adding ? editor.keys + [key] : [key]
        model.setFormValue(ref.page, key: "draftText", nil)
        model.setFormValue(ref.page, key: "draft", .array(keys.map { .string($0) }))
        pending = nil
    }

    /// A key as written, as caps with this Mac's names (as the core names them)
    private func caps(of key: String) -> [[String]] {
        let names = Dictionary(uniqueKeysWithValues: editor.modifiers.map { ($0.value, $0.label) }
            + editor.keyGroups.flatMap(\.keys).map { ($0.value, $0.label) })
        return key.split(separator: " ").map { chord in
            chord.split(separator: "+", omittingEmptySubsequences: false).map { names[String($0)] ?? String($0) }
        }
    }
}

/// Where the key leads afterwards: stay or back to the start mode as a switch, the other modes in a menu
private struct AfterChoice: View {
    let field: Field
    let ref: PageRef
    let model: AppModel

    var body: some View {
        let value = model.formValue(ref.page, field) ?? field.value
        let first = Array(field.choices.prefix(2))
        let others = Array(field.choices.dropFirst(2))
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
            Text(verbatim: model.t(field.label)).font(.callout.weight(.medium))
            HStack(spacing: BrandTokens.Spacing.s) {
                ForEach(Array(first.enumerated()), id: \.offset) { _, c in
                    segment(model.t(c.label), on: c.value == value) { model.setFormValue(ref.page, field, c.value) }
                }
                if !others.isEmpty {
                    let chosen = others.first { $0.value == value }
                    Menu {
                        ForEach(Array(others.enumerated()), id: \.offset) { _, c in
                            Button(model.t(c.label)) { model.setFormValue(ref.page, field, c.value) }
                        }
                    } label: {
                        Text(verbatim: chosen.map { model.t($0.label) } ?? String(localized: "Another Mode …"))
                    }
                    .menuStyle(.borderlessButton).fixedSize()
                    .padding(.horizontal, BrandTokens.Spacing.l).padding(.vertical, 4)
                    .background(chosen != nil ? BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill) : .clear, in: RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(chosen != nil ? BrandStyle.cyan.opacity(0.6) : BrandTokens.Colors.windowBorder))
                }
            }
        }
    }

    private func segment(_ label: String, on: Bool, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(verbatim: label).font(.callout.weight(on ? .semibold : .regular))
                .foregroundStyle(on ? BrandStyle.cyan : .primary)
                .padding(.horizontal, BrandTokens.Spacing.l).padding(.vertical, 4)
                .background(on ? BrandStyle.cyan.opacity(BrandTokens.Opacity.tintFill) : .clear, in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(on ? BrandStyle.cyan.opacity(0.6) : BrandTokens.Colors.windowBorder))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// Records a key while on: the next key pressed with its modifiers ("Ctrl+g"), and – where sequences are allowed – the
/// chords that follow within a moment ("Ctrl+x Ctrl+f"). The keys go to it, not to the app. Keys macOS keeps for
/// itself never arrive (OPEN: which ones exactly depends on the user's shortcuts).
@MainActor final class KeyRecorder: ObservableObject {
    @Published private(set) var recording = false
    /// What is pressed so far, as written
    @Published private(set) var live = ""
    private var monitor: Any?
    private var chords: [String] = []
    private var finish: Task<Void, Never>?
    private var done: ((String) -> Void)?
    private var sequences = false

    func start(sequences: Bool, done: @escaping (String) -> Void) {
        stop()
        self.sequences = sequences
        self.done = done
        chords = []
        live = ""
        recording = true
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [weak self] event in
            guard let self else { return event }
            self.handle(event)
            return nil
        }
    }

    func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        finish?.cancel()
        if recording, !chords.isEmpty { done?(chords.joined(separator: " ")) }
        recording = false
        done = nil
    }

    private func handle(_ event: NSEvent) {
        let mods = Self.modifiers(event.modifierFlags)
        if event.type == .flagsChanged {
            live = (chords + (mods.isEmpty ? [] : [mods.joined(separator: "+") + "+"])).joined(separator: " ")
            return
        }
        guard let key = Self.name(event) else { return }
        chords.append((mods + [key]).joined(separator: "+"))
        live = chords.joined(separator: " ")
        finish?.cancel()
        if !sequences || chords.count >= 3 {
            stop()
        } else {
            // A moment for the next chord of a sequence, then it is done
            finish = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(1200))
                guard !Task.isCancelled else { return }
                self?.stop()
            }
        }
    }

    /// The modifiers as omaflake writes them: ⌘ is a guest's Super
    static func modifiers(_ f: NSEvent.ModifierFlags) -> [String] {
        var out: [String] = []
        if f.contains(.command) { out.append("Super") }
        if f.contains(.control) { out.append("Ctrl") }
        if f.contains(.option) { out.append("Alt") }
        if f.contains(.shift) { out.append("Shift") }
        return out
    }

    /// The key's name as omaflake writes it ("g", "Return", "F5"); nil for one it has no name for
    static func name(_ event: NSEvent) -> String? {
        let named: [UInt16: String] = [
            36: "Return", 76: "Return", 48: "Tab", 49: "Space", 51: "Backspace", 53: "Escape",
            123: "Left", 124: "Right", 125: "Down", 126: "Up", 116: "PageUp", 121: "PageDown",
            122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6", 98: "F7", 100: "F8", 101: "F9", 109: "F10",
            103: "F11", 111: "F12",
        ]
        if let n = named[event.keyCode] { return n }
        guard let c = event.charactersIgnoringModifiers?.lowercased(), c.count == 1,
              let ch = c.first, !ch.isWhitespace, ch.isASCII else { return nil }
        return c == "+" ? "Plus" : c
    }
}
