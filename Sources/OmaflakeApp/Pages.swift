// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The pages one level deeper (New VM, Build Image, Doctor, macOS base, Settings) as the core's pages say; the guide
// shows the docs' file the core names (bundled, offline).

struct PageView: View {
    let ref: PageRef
    let model: AppModel

    var body: some View {
        if ref.page == "guide" {
            GuideView(params: ref.echoed)
        } else {
            switch model.pageModel(ref) {
            case .newVm(let p)?: NewVMView(page: p, ref: ref, model: model)
            case .build(let p)?: BuildView(page: p, ref: ref, model: model)
            case .credentials(let p)?: CredentialsView(page: p, model: model)
            case .credential(let p)?: CredentialView(page: p, ref: ref, model: model)
            case .studio(let p)?: RecipeStudioView(page: p, ref: ref, model: model)
            case .keymapStudio(let p)?: KeymapStudioView(page: p, ref: ref, model: model)
            case .doctor(let p)?: DoctorView(page: p, model: model)
            case .macBase(let p)?: MacBaseView(page: p, ref: ref, model: model)
            case .settings(let p)?: SettingsView(page: p, ref: ref, model: model)
            case .error(let e)?:
                Label(e.text.map { model.t($0) } ?? e.message, systemImage: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning)
                    .padding(BrandStyle.margin)
            default:
                ProgressView().controlSize(.small).padding(.top, 40)
            }
        }
    }
}

// MARK: - Fields

extension JSONValue {
    /// A text of the views held in a field's value (an `info` field)
    var viewText: ViewText? {
        guard case .object = self, let data = try? JSONEncoder.api.encode(self) else { return nil }
        return try? JSONDecoder.api.decode(ViewText.self, from: data)
    }
}

/// A form field of a page: its value is the user's (kept by the app, `params.values`) or the page's
struct FormField: View {
    let field: Field
    /// The form page it belongs to
    let page: String
    let model: AppModel

    private var value: JSONValue { model.formValue(page, field) ?? field.value }

    private func set(_ v: JSONValue?, typing: Bool = false) { model.setFormValue(page, field, v, typing: typing) }

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
            control
            if let error = field.error {
                Text(verbatim: model.t(error)).font(.caption).foregroundStyle(BrandStyle.red)
            }
        }
        .disabled(!field.enabled)
        .help(Text(verbatim: field.enabled ? "" : model.t(field.reason)))
    }

    @ViewBuilder private var control: some View {
        let label = model.t(field.label)
        switch field.kind {
        case .toggle:
            Toggle(label, isOn: Binding(get: { value.bool ?? false }, set: { set(.bool($0)) }))
        case .text:
            TextField(label, text: Binding(get: { value.string ?? "" }, set: { set(.string($0), typing: true) }),
                      prompt: field.placeholder.map { Text(verbatim: model.t($0)) })
        case .number:
            let n = Int(value.double ?? 0)
            // The label names the value ("CPU cores: 4"): drawn with the value as it is now
            let shown = field.label.args.count == 1 && field.label.key != nil
                ? model.t(ViewText(key: field.label.key!, args: [.int(Int64(n))], text: field.label.text)) : label
            Stepper(shown, value: Binding(get: { n }, set: { set(.number(Double($0))) }),
                    in: (field.min ?? 0)...(field.max ?? Int.max), step: field.step ?? 1)
        case .choice:
            Picker(label, selection: Binding(get: { value }, set: { set($0) })) {
                ForEach(Array(field.choices.enumerated()), id: \.offset) { _, c in
                    Text(verbatim: model.t(c.label)).tag(c.value)
                }
            }
        case .path:
            LabeledContent(label) {
                HStack {
                    let path = value.string
                    Text(verbatim: path.map { ($0 as NSString).abbreviatingWithTildeInPath }
                         ?? field.placeholder.map { (model.t($0) as NSString).abbreviatingWithTildeInPath } ?? "")
                        .lineLimit(1).truncationMode(.middle).foregroundStyle(.secondary)
                    if let ask = field.ask { Button(model.t(ask.label)) { model.choose(field, on: page) } }
                    if path != nil, let reset = field.help { Button(model.t(reset)) { set(nil) } }
                }
            }
        case .info:
            LabeledContent(label, value: model.t(value.viewText ?? ViewText(verbatim: value.string ?? "")))
        case .unknown:
            EmptyView()
        }
    }
}

/// A Settings field stored at once (`set`); a typed value after a moment
struct StoredField: View {
    let field: Field
    let model: AppModel
    @State private var text: String?

    var body: some View {
        let label = model.t(field.label)
        Group {
            switch field.kind {
            case .toggle:
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Toggle(label, isOn: Binding(get: { field.value.bool ?? false }, set: { model.store(field, .bool($0)) }))
                    if let help = field.help {
                        Text(verbatim: model.t(help)).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
            case .text:
                TextField(label, text: Binding(get: { text ?? field.value.string ?? "" }, set: {
                    text = $0
                    model.store(field, .string($0), typing: true)
                }), prompt: field.placeholder.map { Text(verbatim: model.t($0)) })
            case .choice:
                Picker(label, selection: Binding(get: { field.value }, set: { model.store(field, $0) })) {
                    let choices = field.choices.map { (value: $0.value, name: GuestNames.name(field.id, $0, model)) }
                    if let first = choices.first, first.value.isNull {
                        Text(verbatim: first.name).tag(first.value)
                        Divider()
                    }
                    ForEach(Array(choices.filter { !$0.value.isNull }
                        .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }.enumerated()), id: \.offset) { _, c in
                        Text(verbatim: c.name).tag(c.value)
                    }
                }
            default:
                FormField(field: field, page: "settings", model: model)
            }
        }
        .disabled(!field.enabled)
    }
}

/// Names of the guest choices in the app's language (Foundation's names; the lists and plain values come from the core)
enum GuestNames {
    /// "Deutsch (Deutschland)" for "de_DE.UTF-8"; a keyboard ("ch (de_CH)") after its locale; time zones as they are
    @MainActor static func name(_ field: String, _ choice: Choice, _ model: AppModel) -> String {
        let label = model.t(choice.label)
        func locale(_ id: String) -> String? {
            Locale.current.localizedString(forIdentifier: id.components(separatedBy: ".").first ?? id)
        }
        if choice.value.isNull {
            // "Like this Mac – <value>"
            guard case .string(let mac)? = choice.label.args.first, let key = choice.label.key else { return label }
            let named = field == "locale" ? locale(mac) ?? mac : field == "timeZone" ? mac.replacingOccurrences(of: "_", with: " ") : mac
            return model.t(ViewText(key: key, args: [.string(named)], text: label))
        }
        switch field {
        case "locale": return choice.value.string.flatMap(locale) ?? label
        case "keyboard":
            guard let open = label.firstIndex(of: "("), let close = label.lastIndex(of: ")") else { return label }
            return locale(String(label[label.index(after: open)..<close])) ?? label
        default: return label
        }
    }
}

// MARK: - New VM

struct NewVMView: View {
    let page: NewVmPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        Form {
            ForEach(Array(page.sections.enumerated()), id: \.offset) { i, section in
                Section {
                    if i == 0 { header }
                    ForEach(section.fields) { FormField(field: $0, page: ref.page, model: model) }
                    if i == page.sections.count - 1 {
                        ForEach(Array(page.warnings.enumerated()), id: \.offset) { _, w in
                            Text(verbatim: model.t(w)).font(.caption).foregroundStyle(BrandTokens.Colors.warning)
                        }
                    }
                } header: {
                    if let title = section.title { Text(verbatim: model.t(title)) }
                }
            }
            HStack {
                Spacer()
                ForEach(page.actions) { a in
                    ActionPlainButton(action: a, model: model, form: ref, isDefault: a.primary)
                }
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }

    /// What it is created from: exactly this image (plan B100)
    @ViewBuilder private var header: some View {
        HStack(spacing: BrandTokens.Spacing.xl) {
            KindIcon(badge: page.kind, model: model).frame(width: 16)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                Text(verbatim: page.image).fontWeight(.medium).lineLimit(1).truncationMode(.middle).help(page.image)
                Text(verbatim: model.t(page.subtitle)).font(.caption).foregroundStyle(.secondary)
            }
        }
        if let text = page.description {
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.m) {
                if page.lab { Image(systemName: "flask").foregroundStyle(BrandStyle.yellow) }
                Text(text).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        if let blockers = page.blockers { BlockersBanner(blockers: blockers, model: model) }
    }
}

// MARK: - Credentials

/// The Credentials page (plan B137): what a credential is, one line per credential with its actions, a new one
struct CredentialsView: View {
    let page: CredentialsPage
    let model: AppModel

    var body: some View {
        Form {
            Section {
                Text(verbatim: model.t(page.intro)).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                // As the other lists: lamp (in use), name and usage, the quick action; opened: its values and actions
                ForEach(page.rows) { r in
                    ExpandableRow {
                        HStack(spacing: BrandTokens.Spacing.xl) {
                            StatusLamp(state: .init(r.lamp.state)).help(Text(verbatim: model.t(r.lamp.help)))
                            Image(systemName: "key").frame(width: 16).foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                                HStack(spacing: BrandTokens.Spacing.m) {
                                    Text(verbatim: r.name).font(.callout.weight(.medium)).lineLimit(1)
                                    ForEach(r.badges) { BadgeView(badge: $0, model: model) }
                                }
                                Text(verbatim: model.t(r.subtitle)).font(.caption).foregroundStyle(.secondary)
                                    .lineLimit(1).help(r.usedBy.joined(separator: ", "))
                            }
                            Spacer(minLength: 4)
                            ForEach(r.actions) { ActionIcon(action: $0, model: model) }
                        }
                    } detail: {
                        DetailBox {
                            InfoLines(lines: r.detail.info, model: model)
                            DetailActions(actions: r.detail.actions, model: model)
                        }
                    }
                }
                HStack {
                    Spacer()
                    ActionPlainButton(action: page.add, model: model, isDefault: true)
                }
            } footer: {
                Text(verbatim: model.t(page.footer)).font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}

// MARK: - One credential

/// One credential's form (plan B137): what it is for, why only the name can change, the fields, Cancel and Save
struct CredentialView: View {
    let page: CredentialPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        Form {
            Section {
                Text(verbatim: model.t(page.intro)).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let locked = page.locked {
                    Label(model.t(locked), systemImage: "lock").font(.caption).foregroundStyle(BrandTokens.Colors.warning)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(page.fields) { FormField(field: $0, page: ref.page, model: model) }
            }
            HStack {
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
        }
        .formStyle(.grouped)
    }
}

// MARK: - Build image

struct BuildView: View {
    let page: BuildPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
            // What – the recipe with its guest kind, always the one it was opened for (plan B100) – and, on the same
            // line, go: build (a macOS recipe asks the password in a question) or cancel; other recipes can be queued
            HStack(spacing: BrandTokens.Spacing.l) {
                KindIcon(badge: page.kind, model: model).frame(width: 16)
                Text(verbatim: page.name).font(.headline)
                ForEach(page.badges) { BadgeView(badge: $0, model: model) }
                if let description = page.description {
                    Image(systemName: "info.circle").font(.caption).foregroundStyle(.secondary).help(description)
                }
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
            if let blockers = page.blockers { BlockersBanner(blockers: blockers, model: model) }
            // The credential and look of the image (plan B137): their values go into the build action
            if let fields = page.fields, !fields.isEmpty {
                Form { ForEach(fields) { FormField(field: $0, page: ref.page, model: model) } }
                    .formStyle(.grouped).scrollDisabled(true).frame(height: CGFloat(fields.count) * 44 + 24)
            }
            if let hint = page.macBase {
                HStack {
                    Image(systemName: "apple.logo").foregroundStyle(.secondary)
                    Text(verbatim: model.t(hint.text)).font(.caption).fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    if let a = hint.action { Button(model.t(a.label)) { model.run(a) } }
                }
            }
            // What the recipe is made of and what is wrong (plan B142)
            ForEach(Array((page.notes ?? []).enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
            // Where it stands
            if let status = page.status {
                Label(model.t(status), systemImage: "clock").font(.caption).foregroundStyle(.secondary)
            } else if let progress = page.progress {
                BuildProgressView(progress: progress, model: model)
            } else if let outcome = page.outcome {
                OutcomeView(outcome: outcome, model: model)
            } else if let newest = page.newest {
                Text(verbatim: model.t(newest)).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            }
            JobLogView(log: page.log, model: model)
        }
        .padding(BrandStyle.margin)
    }
}

/// A job's log as its page names it (the build, the macOS base): the lines, the note, "Full Log …"
struct JobLogView: View {
    let log: LogRef?
    let model: AppModel
    var minHeight: CGFloat? = nil

    var body: some View {
        LogView(lines: log.map { model.log($0) + ($0.note.map { [model.t($0)] } ?? []) } ?? [])
            .frame(minHeight: minHeight)
        if let full = log?.full {
            HStack {
                Spacer()
                Button(model.t(full.label)) { model.run(full) }.help(Text(verbatim: model.t(full.help)))
            }
        }
    }
}

/// How the build ended, said plainly above its log
struct OutcomeView: View {
    let outcome: BuildPage.Outcome
    let model: AppModel

    var body: some View {
        HStack(alignment: .top, spacing: BrandTokens.Spacing.l) {
            Image(systemName: Icons.symbol(outcome.icon)?.main ?? "circle").foregroundStyle(outcome.tone.color)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                Text(verbatim: model.t(outcome.title)).font(.callout.weight(.semibold))
                if let detail = outcome.detail {
                    Text(verbatim: model.t(detail)).font(.caption).foregroundStyle(.secondary)
                        .lineLimit(outcome.state == .failed ? 3 : 1).truncationMode(.middle).textSelection(.enabled)
                }
            }
            Spacer()
            if let a = outcome.action { Button(model.t(a.label)) { model.run(a) } }
        }
        .padding(BrandTokens.Spacing.cardPadding)
        .background(BrandStyle.card, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.toast))
    }
}

/// Step "2/4", what it does, packages done of announced, a bar over all steps; the running time and the time since
/// the last output, counted here every second
struct BuildProgressView: View {
    let progress: BuildPage.Progress
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: BrandTokens.Spacing.l) {
                ProgressView().controlSize(.small)   // spinning: the build is alive
                Text(verbatim: model.t(progress.step))
                    .font(.system(.caption, design: .monospaced).weight(.bold)).foregroundStyle(BrandStyle.cyan)
                Text(verbatim: model.t(progress.title)).font(.caption.weight(.medium))
                Spacer()
                if let count = progress.count {
                    Text(verbatim: model.t(count))
                        .font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
                        .help("Packages built or downloaded")
                }
            }
            ProgressView(value: progress.fraction).progressViewStyle(.linear).tint(BrandStyle.cyan)
            if let started = progress.started {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    let quiet = Int(context.date.timeIntervalSince(progress.lastOutput ?? started))
                    let long = quiet > progress.quietWarning
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text("Running for \(Self.duration(context.date.timeIntervalSince(started)))")
                        Text(verbatim: "·")
                        Text(quiet < 3 ? String(localized: "output just now") : String(localized: "last output \(Self.duration(Double(quiet))) ago"))
                            .foregroundStyle(long ? BrandTokens.Colors.warning : .secondary)
                            .help(Text(verbatim: long ? model.t(progress.quietHelp) : ""))
                    }
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
        }
    }

    static func duration(_ seconds: Double) -> String {
        let s = Int(seconds)
        return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60) : String(format: "%d:%02d", s / 60, s % 60)
    }
}

// MARK: - Doctor

struct DoctorView: View {
    let page: DoctorPage
    let model: AppModel

    var body: some View {
        Form {
            Section {
                // One block: "omaflake" with the app's and the core's version, then the update
                ForEach(Array(page.versions.enumerated()), id: \.offset) { _, line in
                    LabeledContent(model.t(line.label)) {
                        Text(verbatim: model.t(line.value)).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                if let problem = model.coreProblem {
                    Label(problem, systemImage: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning).font(.callout)
                        .fixedSize(horizontal: false, vertical: true)
                }
                LabeledContent(model.t(page.update.label)) {
                    HStack(spacing: BrandTokens.Spacing.l) {
                        if page.update.busy {
                            ProgressView().controlSize(.small)
                        } else {
                            Text(verbatim: model.t(page.update.text))
                                .foregroundStyle(page.update.tone == .normal ? AnyShapeStyle(.secondary) : AnyShapeStyle(page.update.tone.color))
                                .lineLimit(1).help(Text(verbatim: model.t(page.update.text)))
                        }
                        ForEach(page.update.actions) { a in Button(model.t(a.label)) { model.run(a) } }
                    }
                    .font(.callout)
                }
            }
            Section {
                HStack(alignment: .top) {
                    Text(verbatim: model.t(page.intro))
                        .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    if page.checking { ProgressView().controlSize(.small) }
                    ForEach(page.actions) { a in Button(model.t(a.label)) { model.run(a) }.disabled(!a.enabled) }
                }
            }
            ForEach(page.sections) { section in
                Section(model.t(section.title)) {
                    ForEach(section.items) { RequirementRow(item: $0, model: model) }
                }
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
        .onAppear {
            model.markRequirementsShown()
            model.checkDoctor()
        }
    }
}

/// One check: a single line (status, title, what was found). Rows with something to do get a chevron on the far
/// right that opens a set-off sub-line with the hint on the left and the actions right-aligned; problems and running
/// work start open (the core's `expanded`). Buttons never sit in the first line.
struct RequirementRow: View {
    let item: DoctorPage.Item
    let model: AppModel
    @State private var userOpen: Bool?

    private var expanded: Bool { item.expandable && (userOpen ?? item.expanded) }

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
                Image(systemName: item.status == .ok ? "checkmark.circle.fill" : item.status == .limited ? "exclamationmark.circle.fill" : "xmark.circle.fill")
                    .foregroundStyle(item.status == .ok ? BrandTokens.Colors.success : item.status == .limited ? BrandTokens.Colors.warning : .red)
                Text(verbatim: model.t(item.title)).lineLimit(1).layoutPriority(1)
                Spacer(minLength: 12)
                if let detail = item.detail {
                    // No text selection here: it would swallow the click that opens the row
                    let d = model.t(detail)
                    Text(verbatim: d).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle).help(d)
                }
                Group {
                    if item.expandable {
                        Button {
                            withAnimation(.easeInOut(duration: 0.15)) { userOpen = !expanded }
                        } label: {
                            Image(systemName: "chevron.right")
                                .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                                .rotationEffect(.degrees(expanded ? 90 : 0))
                                .frame(width: 16, height: 12).contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    } else {
                        Color.clear
                    }
                }
                .frame(width: 16)
            }
            .contentShape(Rectangle())
            .onTapGesture { if item.expandable { withAnimation(.easeInOut(duration: 0.15)) { userOpen = !expanded } } }
            if expanded {
                HStack(alignment: .center, spacing: BrandTokens.Spacing.l) {
                    if let hint = item.hint {
                        Text(verbatim: model.t(hint)).font(.caption).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 12)
                    if let task = item.task {
                        ProgressView().controlSize(.small)
                        Text(verbatim: model.t(task)).font(.caption).foregroundStyle(.secondary)
                    }
                    if let download = item.download { DownloadProgress(download: download, model: model) }
                    ForEach(item.actions) { ActionPlainButton(action: $0, model: model) }
                }
                .controlSize(.small)
                .padding(.horizontal, BrandTokens.Spacing.xl).padding(.vertical, BrandTokens.Spacing.m)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
                .padding(.leading, 26)
            }
        }
    }
}

// MARK: - macOS base

/// Creating the macOS base: IPSW (Apple's or a file), the credential of its guest user, progress and log
struct MacBaseView: View {
    let page: MacBasePage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
            Text(verbatim: model.t(page.intro))
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let base = page.base {
                Label(model.t(base.text), systemImage: base.tone == .success ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                    .foregroundStyle(base.tone.color).fixedSize(horizontal: false, vertical: true)
            }
            GroupBox {
                HStack {
                    Image(systemName: "shippingbox").foregroundStyle(.secondary)
                    Text(verbatim: model.t(page.ipsw.text)).lineLimit(1).truncationMode(.middle)
                    Spacer()
                    if let download = page.ipsw.download { DownloadProgress(download: download, model: model) }
                    ForEach(page.ipsw.actions) { ActionPlainButton(action: $0, model: model) }
                }
            } label: { Text(verbatim: model.t(page.ipsw.label)) }
            // The credential the guest user is created from (plan B139) – no password to type
            Form { ForEach(page.fields) { FormField(field: $0, page: ref.page, model: model) } }
                .formStyle(.grouped).scrollDisabled(true).frame(height: CGFloat(page.fields.count) * 64 + 24)
            HStack {
                if page.installing {
                    ProgressView().controlSize(.small)
                    Text(verbatim: model.t(page.actions.first { $0.id == "create" }?.reason)).foregroundStyle(.secondary)
                }
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
            JobLogView(log: page.log, model: model, minHeight: 160)
        }
        .padding(16)
        .onAppear { model.checkDoctor() }
    }
}

// MARK: - Settings

struct SettingsView: View {
    let page: SettingsPage
    let ref: PageRef
    @Bindable var model: AppModel
    @AppStorage(AppFrame.key) private var frame: AppFrame = .menuBar
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        Form {
            Section(model.t(page.general.title)) {
                // The app's own: its frame, launch at login, its language
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Picker("Show omaflake in", selection: $frame) {
                        ForEach(AppFrame.allCases) { Text($0.title).tag($0) }
                    }
                    .onChange(of: frame) { _, new in new.apply(openWindow: openWindow, dismissWindow: dismissWindow) }
                    Text("Menu bar: a window at the omaflake icon, no Dock icon. Window: a regular window with a Dock icon. Both: the menu bar icon and the window.")
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                Toggle("Launch at Login", isOn: $model.launchAtLogin)
                ForEach(page.general.fields) { StoredField(field: $0, model: model) }
                Picker("Language", selection: $model.language) {
                    Text("System").tag("")
                    Text(verbatim: "English").tag("en")
                    Text(verbatim: "Deutsch").tag("de")
                }
                if model.languageChanged {
                    HStack {
                        Text("Restart omaflake to apply the language.").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Restart Now") { model.relaunch() }
                    }
                }
            }
            Section {
                ForEach(page.guest.fields) { StoredField(field: $0, model: model) }
            } header: {
                Text(verbatim: model.t(page.guest.title))
            } footer: {
                if let footer = page.guest.footer {
                    Text(verbatim: model.t(footer)).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            Section(model.t(page.dataFolder.title)) {
                LabeledContent("Location") {
                    Text(verbatim: (page.dataFolder.location as NSString).abbreviatingWithTildeInPath)
                        .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                }
                HStack {
                    Spacer()
                    if page.dataFolder.moving { ProgressView().controlSize(.small) }
                    ForEach(page.dataFolder.actions) { a in
                        Button(model.t(a.label)) { model.run(a) }.disabled(!a.enabled)
                            .help(Text(verbatim: a.enabled ? "" : model.t(a.reason)))
                    }
                }
                Text(verbatim: model.t(page.dataFolder.note))
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }

}

// MARK: - Sources

/// The recipe sources, a tab of the catalog (plan B147; before it a section of the settings)
struct SourcesView: View {
    let page: SourcesPage
    let ref: PageRef
    @Bindable var model: AppModel

    /// Recipe Sources (spec §11.3, plan B107): in their order (an earlier source wins), and a new one
    var body: some View {
        Form { section }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
    }

    private var section: some View {
        Section {
            ForEach(page.sources.rows) { s in
                HStack(spacing: BrandTokens.Spacing.l) {
                    Image(systemName: Icons.symbol(s.icon)?.main ?? "folder").frame(width: 16).foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 1) {
                        HStack(spacing: BrandTokens.Spacing.s) {
                            Text(verbatim: s.name).lineLimit(1).truncationMode(.middle).help(s.source.location)
                            ForEach(s.badges) { b in
                                BadgeView(badge: b, model: model)
                            }
                        }
                        Text(verbatim: model.t(s.subtitle)).font(.caption2).foregroundStyle(.secondary)
                    }
                    Spacer()
                    ForEach(s.actions) { a in
                        if a.on != nil {
                            Toggle("", isOn: Binding(get: { a.on ?? false }, set: { _ in model.run(a) })).labelsHidden()
                                .toggleStyle(.switch).controlSize(.mini)
                        } else {
                            ActionIcon(action: a, model: model)
                        }
                    }
                }
            }
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
                ForEach(page.sources.add.fields) { field in
                    if field.kind == .text {
                        HStack {
                            TextField(model.t(field.label), text: Binding(
                                get: { (model.formValue(ref.page, field) ?? field.value).string ?? "" },
                                set: { model.setFormValue(ref.page, field, .string($0), typing: true) }),
                                      prompt: Text(verbatim: model.t(field.label)))
                                .textFieldStyle(.roundedBorder).labelsHidden()
                            if let ask = field.ask { Button(model.t(ask.label)) { model.choose(field, on: ref.page) } }
                        }
                    } else {
                        FormField(field: field, page: ref.page, model: model)
                    }
                }
                HStack {
                    Spacer()
                    ActionPlainButton(action: page.sources.addAction, model: model, form: ref,
                                      isDefault: page.sources.addAction.primary)
                }
            }
        } header: {
            Text(verbatim: model.t(page.sources.title))
        } footer: {
            Text(verbatim: model.t(page.sources.footer))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
    }
}
