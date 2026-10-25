// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import SwiftUI
import UniformTypeIdentifiers
import Foundation
import Observation
import ServiceManagement
import OmaflakeKit

/// App state. What every page shows – rows, lamps, badges, actions and whether they can be done, the questions they
/// ask, filters, form fields, the activity list – is the core's presentation model (docs/api.md §10, plan B121): the app
/// subscribes to the pages it shows (`views.subscribe`) and draws what `view.changed` brings. It keeps only what is its
/// own: the session, which tab and page are open, the filter and form values the user chose, the panels and questions
/// an action asks, notifications, launch at login and the language. Quitting leaves running VMs untouched.
@MainActor @Observable
final class AppModel {
    let client: CoreClient
    /// What `initialize` answered; nil while the core is not running
    private(set) var session: InitializeResult?
    /// Why the core is not running (it is started again on its own)
    private(set) var coreProblem: String?

    /// Texts of the views in the app's language (de.lproj, by the English source string as key)
    let renderer = ViewTextRenderer(translate: { Bundle.main.localizedString(forKey: $0, value: nil, table: nil) })

    /// A text of the views as the app shows it; `params`: values just asked for an action
    nonisolated func t(_ text: ViewText?, _ params: [String: String] = [:]) -> String {
        text.map { renderer.render($0, params: params) } ?? ""
    }

    // MARK: Pages from the core

    /// The newest model of every subscribed page, by the page and the params it was subscribed with
    private(set) var models: [PageRef: PageModel] = [:]
    /// The newest model of each page name – shown while the model for changed params (a filter, a form value) is on
    /// its way
    private var latest: [String: (params: JSONValue, model: PageModel)] = [:]

    var home: HomePage? { if case .home(let h)? = models[PageRef("home")] { h } else { nil } }
    var activity: ActivityPage? { if case .activity(let a)? = models[PageRef("activity")] { a } else { nil } }

    /// The model of a page: exactly for `ref`, else the newest one of that page whose params agree on `same`
    func model(_ ref: PageRef, same: [String] = []) -> PageModel? {
        if let m = models[ref] { return m }
        guard let l = latest[ref.page] else { return nil }
        for key in same where l.params[key] != ref.echoed[key] { return nil }
        return l.model
    }

    // MARK: Navigation

    /// Tabs of the main page (plan B91); the core's `home` page names them
    enum HomeTab: String, CaseIterable {
        case vms, images, recipes, looks, activity
    }

    var tab: HomeTab = .vms {
        didSet {
            UserDefaults.standard.set(tab.rawValue, forKey: "homeTab")
            markActivitySeen()
        }
    }

    /// The filter values chosen per list (plan B101) – the core applies them (`params.filter`)
    var filters: [HomeTab: [String: JSONValue]] = [:]

    func listRef(_ tab: HomeTab) -> PageRef {
        let f = (filters[tab] ?? [:]).filter { !$0.value.isNull && $0.value != .bool(false) }
        return PageRef(tab.rawValue, f.isEmpty ? .object([:]) : .object(["filter": .object(f)]))
    }

    /// The pages open one level deeper inside the window (menu or app window share it); empty = the main page
    var path: [PageRef] = [] {
        didSet { subscribe() }
    }

    /// Opens a page one level deeper; the same page already open further up becomes the current one again
    func show(_ ref: PageRef) {
        if let i = path.firstIndex(where: { $0.page == ref.page }) {
            path = Array(path[..<i]) + [ref]
        } else {
            path.append(ref)
        }
    }

    func back() {
        guard let last = path.popLast() else { return }
        formValues[last.page] = nil
    }

    // MARK: Forms (newVm, macBase, the new source in settings)

    /// What the user put into a form page's fields so far, by page and the fields' `valueParam` – the page is asked
    /// with them (`params.values`), so the core validates them and its action holds them
    private(set) var formValues: [String: [String: JSONValue]] = [:]
    private var resubscribe: Task<Void, Never>?

    /// The ref a page is subscribed with: its params plus the form values
    func fullRef(_ ref: PageRef) -> PageRef {
        guard let values = formValues[ref.page], !values.isEmpty else { return ref }
        var params = ref.echoed
        params.set(.object(values), at: "values")
        return PageRef(ref.page, params)
    }

    /// The user's value of a form field (nil: the page's); a field without `valueParam` keeps none
    func formValue(_ page: String, _ field: Field) -> JSONValue? {
        field.valueParam.flatMap { formValues[page]?[$0] }
    }

    func setFormValue(_ page: String, _ field: Field, _ value: JSONValue?, typing: Bool = false) {
        guard let key = field.valueParam else { return }
        setFormValue(page, key: key, value, typing: typing)
    }

    /// `values.<key>` of a form page (nil: back to the page's default); typed text is sent after a moment
    func setFormValue(_ page: String, key: String, _ value: JSONValue?, typing: Bool = false) {
        var values = formValues[page] ?? [:]
        values[key] = value
        formValues[page] = values
        resubscribe?.cancel()
        resubscribe = Task {
            if typing { try? await Task.sleep(for: .milliseconds(300)) }
            guard !Task.isCancelled else { return }
            subscribe()
        }
    }

    // MARK: Subscription

    private var subscribed: [PageRef] = []

    /// Subscribes to what the window shows: the header and tabs, the activity, every list (switching tabs is instant)
    /// and the open page
    func subscribe(force: Bool = false) {
        guard session != nil else { return }
        var pages = [PageRef("home"), PageRef("activity")] + HomeTab.allCases.filter { $0 != .activity }.map(listRef)
        for ref in path where ref.page != "guide" {
            let full = fullRef(ref)
            if !pages.contains(full) { pages.append(full) }
        }
        guard force || pages != subscribed else { return }
        subscribed = pages
        // Models of pages no longer subscribed go (the newest per page stays in `latest`)
        models = models.filter { pages.contains($0.key) }
        struct Params: Encodable, Sendable { let pages: [PageRef] }
        Task {
            do { let _: Ignored? = try await client.call("views.subscribe", Params(pages: pages)) }
            catch { toast(describe(error)) }
        }
    }

    // MARK: Activity, footer, notifications

    // Unseen activity is the app's own (docs/api.md §10.6): entry ids are stable and rise; the app remembers the highest
    // id it has shown (on the activity tab; what was there when it connected counts as shown) and entries above it are
    // unseen. A new server starts its ids at 1 again: when the newest id in the list is lower than the remembered one,
    // the mark is forgotten.

    /// The newest activity entry this app has announced (footer, notification); nil before the first list
    private var announced: Int?
    /// The highest entry id shown on the activity tab
    private var seen = 0
    var unseenActivity: Bool { (activity?.entries.first?.id ?? 0) > seen }

    /// A short message at the bottom of the main page for a few seconds (the newest activity, or the app's own)
    struct Footer: Identifiable, Equatable {
        let id = UUID()
        let text: String
    }
    private(set) var footer: Footer?

    func toast(_ text: String) {
        guard !text.isEmpty else { return }
        let f = Footer(text: text)
        footer = f
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(6))
            if self?.footer?.id == f.id { self?.footer = nil }
        }
    }

    private func markActivitySeen() {
        if tab == .activity && path.isEmpty { seen = activity?.entries.first?.id ?? seen }
    }

    /// New activity entries: in the footer, and as a system notification where the core words one
    private func announce(_ a: ActivityPage) {
        let newest = a.entries.first?.id ?? 0
        if newest < max(announced ?? 0, seen) {
            // A new server: its ids start again – forget the marks
            if announced != nil { announced = 0 }
            seen = 0
        }
        defer {
            announced = max(announced ?? 0, newest)
            markActivitySeen()
        }
        guard let announced else {
            // What happened before this app connected is history, not news
            seen = newest
            return
        }
        for entry in a.entries.reversed() where entry.id > announced {
            toast(t(entry.text))
            if a.notifications, let n = entry.notification { Notifier.post(t(n.title), t(n.body)) }
        }
    }

    // MARK: Job logs (the build page, the macOS base)

    /// Log lines of jobs (`job.log`), at most 5000 per job
    private(set) var jobLogs: [String: [String]] = [:]

    /// A job's log as its page names it: `jobs.log {id, from}` once, then the `job.log` events
    func log(_ ref: LogRef) -> [String] {
        if jobLogs[ref.job] == nil, ref.lines > 0 { loadLog(ref) }
        return jobLogs[ref.job] ?? []
    }

    private var loadingLogs: Set<String> = []

    private func loadLog(_ ref: LogRef) {
        guard !loadingLogs.contains(ref.job) else { return }
        loadingLogs.insert(ref.job)
        Task {
            struct Params: Encodable, Sendable { let id: String; let from: Int }
            if let log: JobLog = try? await client.call("jobs.log", Params(id: ref.job, from: ref.from)) {
                jobLogs[ref.job] = Array(log.lines.suffix(5000))
            }
            loadingLogs.remove(ref.job)
        }
    }

    private func appendLog(_ id: String, _ lines: [String]) {
        guard jobLogs[id] != nil || !loadingLogs.contains(id) else { return }
        var log = jobLogs[id] ?? []
        log.append(contentsOf: lines)
        if log.count > 5000 { log.removeFirst(log.count - 5000) }
        jobLogs[id] = log
    }

    // MARK: Start

    init() {
        // Tooltips almost at once instead of after about a second (AppKit's NSInitialToolTipDelay, this app only)
        UserDefaults.standard.register(defaults: ["NSInitialToolTipDelay": 150])
        let feed = ProcessInfo.processInfo.environment["OMAFLAKE_UPDATE_FEED"]
            ?? Bundle.main.object(forInfoDictionaryKey: "OMUpdateFeedURL") as? String
        let params = InitializeParams(client: "omaflake-macos", clientVersion: AppVersion.short, language: Self.uiLanguage,
                                      updateFeed: feed.flatMap { $0.isEmpty ? nil : $0 })
        let launcher: CoreClient.Launcher
        if let omaf = CoreClient.locateServer() {
            // The core service shared with the CLI and other interfaces; a private server if it cannot be reached (B120)
            launcher = CoreClient.automaticLauncher(executable: omaf)
        } else {
            launcher = { _ in throw APIError(message: "omaf not found – it belongs next to the app (Contents/MacOS/omaf); for development set OMAF_BIN or run cargo build") }
        }
        client = CoreClient(launcher: launcher, initialize: params, topics: ServerEvent.viewTopics)
        if let saved = UserDefaults.standard.string(forKey: "homeTab").flatMap(HomeTab.init(rawValue:)) { tab = saved }
        // The first time, the window (menu or app window, plan B119) opens on the Doctor
        if Self.firstLaunch { path = [PageRef("doctor")] }
        // Development aid (screenshots, checks without clicks): open on a page, `OMAFLAKE_PAGE=settings` with
        // `OMAFLAKE_PAGE_PARAMS` as JSON
        let env = ProcessInfo.processInfo.environment
        if let page = env["OMAFLAKE_PAGE"], !page.isEmpty {
            let params = env["OMAFLAKE_PAGE_PARAMS"].flatMap { try? JSONDecoder().decode(JSONValue.self, from: Data($0.utf8)) }
            path = [PageRef(page, params ?? .object([:]))]
        }
        // Packages opened in the Finder before the model existed, and from now on
        AppDelegate.open = { [weak self] urls in self?.openPackages(urls) }
        AppDelegate.terminate = { [weak self] in self?.client.stop() }
        if !AppDelegate.waiting.isEmpty {
            openPackages(AppDelegate.waiting)
            AppDelegate.waiting = []
        }
        let events = client.events
        Task { [weak self] in
            for await event in events { self?.handle(event) }
        }
        client.start()
        // The update check: at launch (once the core runs), then once a day; the core answers "off" when it is
        // switched off, and words what it found in the activity list and the Doctor
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            while !Task.isCancelled {
                if let client = self?.client { let _: JSONValue? = try? await client.call("updates.check") }
                try? await Task.sleep(for: .seconds(24 * 3600))
            }
        }
    }

    /// The language the app shows ("de", "en") – the core words recipe descriptions in it (`initialize`)
    static var uiLanguage: String? { Bundle.main.preferredLocalizations.first.map { String($0.prefix(2)) } }

    /// The Doctor opens by itself on the first launch
    static var firstLaunch: Bool { !UserDefaults.standard.bool(forKey: "requirementsShown") }
    func markRequirementsShown() { UserDefaults.standard.set(true, forKey: "requirementsShown") }

    // MARK: Events

    private func handle(_ event: ServerEvent) {
        switch event {
        case .connected(let result):
            let restarted = session != nil || coreProblem != nil
            if coreProblem != nil { toast(String(localized: "omaflake's core runs again")) }
            session = result
            coreProblem = nil
            // A restarted server has lost its jobs (api.md §5); their logs stay in their files
            if restarted { jobLogs = [:] }
            subscribe(force: true)
            // The Doctor's checks at the start (the core runs them after actions and before builds, not while idle)
            if !restarted { checkDoctor() }
        case .disconnected(let reason):
            if coreProblem == nil { toast(reason) }
            coreProblem = reason
        case .viewChanged(let page, let params, let model):
            let ref = PageRef(page, params)
            models[ref] = model
            latest[page] = (params, model)
            if case .activity(let a) = model { announce(a) }
            if case .home(let h) = model, UserDefaults.standard.string(forKey: "homeTab") == nil,
               let first = HomeTab(rawValue: h.firstTab) {
                tab = first
            }
        case .jobLog(let id, let lines):
            appendLog(id, lines)
        case .undecodable(let method, let error):
            FileHandle.standardError.write(Data("omaflake: event \(method) not understood: \(error)\n".utf8))
        default:
            break
        }
    }

    func checkDoctor() {
        Task { let _: JSONValue? = try? await client.call("doctor.check") }
    }

    /// An error as the app shows it: the core's message, in the app's language when it has a key (api.md §3)
    func describe(_ error: Error) -> String {
        guard let e = error as? APIError else { return error.localizedDescription }
        return e.text.map { t($0) } ?? e.message
    }

    // MARK: Actions (docs/api.md §10.3, §10.4)

    /// Actions whose call is under way (their buttons show it)
    private(set) var running: Set<String> = []

    static func key(_ a: ViewAction) -> String {
        let params = (try? String(decoding: JSONEncoder.api.encode(a.params), as: UTF8.self)) ?? ""
        return "\(a.method) \(params)"
    }

    func isRunning(_ a: ViewAction) -> Bool { running.contains(Self.key(a)) }

    /// A VM an action of this app works on right now (its row shows it before the core does)
    func isBusy(_ name: String) -> Bool {
        running.contains { $0.hasPrefix("vms.") && $0.contains("\"name\":\"\(name)\"") && !$0.hasPrefix("vms.set") }
    }

    /// Does an action: asks what it asks, then its question, then sends it. `form`: the page whose field values go
    /// into its params (a form page's own action).
    func run(_ a: ViewAction, form: PageRef? = nil) {
        guard a.enabled, !isRunning(a) else { return }
        Task { await perform(a, form: form) }
    }

    private func perform(_ a: ViewAction, form: PageRef?) async {
        var params = a.params
        if let form { params = withFormValues(params, form) }
        var shown: [String: String] = [:]
        var variants = [params]
        for ask in a.ask {
            guard let values = await askFor(ask), !values.isEmpty else { return }
            if ask.kind != .password { shown[ask.param] = values.compactMap(\.string).joined(separator: ", ") }
            variants = variants.flatMap { v in
                values.map { value in
                    var p = v
                    p.set(value, at: ask.param)
                    return p
                }
            }
        }
        for p in variants {
            await send(a, params: p, shown: shown, form: form)
        }
    }

    /// A form page's field values put into an action's params, each at its field's `param`
    private func withFormValues(_ params: JSONValue, _ form: PageRef) -> JSONValue {
        guard let values = formValues[form.page], !values.isEmpty else { return params }
        var p = params
        for field in fields(of: form) {
            if let param = field.param, let key = field.valueParam, let value = values[key] { p.set(value, at: param) }
        }
        return p
    }

    /// The model a deeper page shows: for its params and form values, else the newest of that page (same identity)
    func pageModel(_ ref: PageRef) -> PageModel? {
        model(fullRef(ref), same: Self.identity[ref.page] ?? [])
    }

    func fields(of ref: PageRef) -> [Field] {
        switch pageModel(ref) {
        case .newVm(let p)?: p.sections.flatMap(\.fields)
        case .macBase(let p)?: p.fields
        case .build(let p)?: p.fields ?? []
        case .credential(let p)?: p.fields
        case .settings(let p)?: p.sources.add.fields
        default: []
        }
    }

    /// The params that make a page another page (not form values)
    static let identity: [String: [String]] = ["newVm": ["image"], "build": ["recipe"], "credential": ["id"], "baseCredential": ["name"], "locale": ["id"]]

    private func send(_ a: ViewAction, params: JSONValue, shown: [String: String], form: PageRef?) async {
        var params = params
        if let c = a.confirm {
            let input = c.input.map { TextInput(text: $0.default) }
            guard await choose(t(c.title, shown), t(c.text, shown), button: t(c.button), destructive: c.destructive,
                               input: input) == 0 else { return }
            if let field = c.input, let input {
                params.set(.string(input.text.trimmingCharacters(in: .whitespaces)), at: field.param)
            }
        }
        if a.method.hasPrefix("ui.") {
            await uiVerb(a.method, params)
            return
        }
        let key = Self.key(a)
        running.insert(key)
        defer { running.remove(key) }
        do {
            let result: JSONValue? = try await client.call(a.method, params)
            switch a.then {
            case .reveal?: if let path = result?["path"]?.string { reveal(path) }
            case .terminal?: if let line = result?["shellLine"]?.string { runInTerminal(line) }
            case .back?: back()
            default: break
            }
            // A form that did its work starts over
            if let form, a.then != .back { formValues[form.page] = nil; subscribe() }
        } catch {
            toast(describe(error))
        }
    }

    /// What a UI does itself (§10.4)
    private func uiVerb(_ method: String, _ params: JSONValue) async {
        switch method {
        case "ui.show":
            guard let page = params["page"]?.string else { return }
            let pageParams = params["params"] ?? .object([:])
            // A guide the app cannot show itself (no file of it here) opens on omaflake.dev
            if page == "guide", GuideView.source(pageParams) == nil {
                if let url = pageParams["url"]?.string.flatMap(URL.init(string:)) { NSWorkspace.shared.open(url) }
                return
            }
            show(PageRef(page, pageParams))
        case "ui.back":
            back()
        case "ui.step":
            guard let page = params["page"]?.string else { return }
            await step(PageRef(page, params["params"] ?? .object([:])))
        case "ui.filter":
            guard let page = params["page"]?.string, let tab = HomeTab(rawValue: page) else { return }
            if case .object(let f)? = params["filter"] { filters[tab] = f } else { filters[tab] = nil }
            subscribe()
        case "ui.field":
            // `values.<valueParam>` of the page it names; `value: null` = back to the default (an asked value is in
            // `value` already)
            guard let page = params["page"]?.string, let key = params["valueParam"]?.string,
                  let value = params["value"] else { return }
            setFormValue(page, key: key, value.isNull ? nil : value)
        case "ui.reveal":
            if let path = params["path"]?.string { reveal(path) }
        case "ui.open":
            if let path = params["path"]?.string {
                NSWorkspace.shared.open(URL(fileURLWithPath: path))
            } else if let url = params["url"]?.string.flatMap(URL.init(string:)) {
                NSWorkspace.shared.open(url)
            }
        case "ui.terminal":
            if let line = params["line"]?.string { runInTerminal(line) }
        case "ui.quit":
            NSApp.terminate(nil)
        default:
            break
        }
    }

    /// A step, not a page: the core says what a package is and the one action that brings it in (`import`)
    private func step(_ ref: PageRef) async {
        do {
            let page: ImportPage = try await client.call("views.get", ref)
            if let action = page.action {
                await send(action, params: action.params, shown: [:], form: nil)
            } else {
                toast(t(page.reason))
            }
        } catch {
            toast(describe(error))
        }
    }

    /// Packages opened in the Finder (double-click, dropped on the app icon): the import step for each
    func openPackages(_ urls: [URL]) {
        Task { @MainActor in
            for url in urls { await step(PageRef("import", .object(["path": .string(url.path)]))) }
        }
    }

    /// A Settings field stored at once (`set`); typed text after a moment, an empty one back to the default
    func store(_ field: Field, _ value: JSONValue, typing: Bool = false) {
        guard let setter = field.set else { return }
        storing[field.id]?.cancel()
        storing[field.id] = Task {
            if typing { try? await Task.sleep(for: .milliseconds(500)) }
            guard !Task.isCancelled else { return }
            var params = setter.params
            var v = value
            if case .string(let s) = value, s.trimmingCharacters(in: .whitespaces).isEmpty { v = .null }
            params.set(v, at: setter.param)
            do { let _: JSONValue? = try await client.call(setter.method, params) } catch { toast(describe(error)) }
        }
    }
    private var storing: [String: Task<Void, Never>] = [:]

    func reveal(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    /// Types `line` into a new Terminal window and runs it – the user sees what runs and answers prompts there
    /// (asks for automation permission the first time)
    func runInTerminal(_ line: String) {
        let escaped = line.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let script = "tell application \"Terminal\"\nactivate\ndo script \"\(escaped)\"\nend tell"
        var error: NSDictionary?
        NSAppleScript(source: script)?.executeAndReturnError(&error)
        if let error { toast("Terminal: \(error[NSAppleScript.errorMessage] ?? error)") }
    }

    /// The header's power button and the app menu's Quit: the core's quit action, with its question
    func quit() {
        if let quit = home?.actions.first(where: { $0.method == "ui.quit" }) { run(quit) } else { NSApp.terminate(nil) }
    }

    // MARK: Asking (§10.3 Ask)

    /// A path field's panel (its `ask`): the chosen value becomes the field's value
    func choose(_ field: Field, on page: String) {
        guard let ask = field.ask else { return }
        Task {
            guard let value = await askFor(ask)?.first else { return }
            setFormValue(page, field, value)
        }
    }

    /// The values an ask gets from the user (several for `multiple`); nil: cancelled
    private func askFor(_ ask: Ask) async -> [JSONValue]? {
        switch ask.kind {
        case .text, .password:
            let secure = ask.kind == .password
            let input = TextInput(text: ask.default ?? "")
            let again = ask.repeat ? TextInput(text: "") : nil
            let prompt = ask.prompt.map { t($0) } ?? ""
            guard await choose(t(ask.label), prompt, button: ask.button.map { t($0) } ?? String(localized: "Continue"),
                               input: input, secure: secure, repeated: again) == 0 else { return nil }
            return [.string(input.text)]
        case .folder, .file, .package:
            let urls = await choosePaths(ask)
            return urls.map { $0.map { .string($0.path) } }
        case .unknown:
            return nil
        }
    }

    /// An open panel for an ask, as a sheet on the window
    private func choosePaths(_ ask: Ask) async -> [URL]? {
        let panel = NSOpenPanel()
        panel.message = ask.prompt.map { t($0) } ?? ""
        if let button = ask.button { panel.prompt = t(button) }
        panel.allowsMultipleSelection = ask.multiple
        var checker: PathChecker?
        switch ask.kind {
        case .folder:
            panel.canChooseDirectories = true
            panel.canChooseFiles = false
            panel.canCreateDirectories = true
        case .file:
            panel.canChooseDirectories = false
            panel.canChooseFiles = true
            panel.treatsFilePackagesAsDirectories = true
            panel.showsHiddenFiles = ask.accept == nil
            if let accept = ask.accept { checker = PathChecker(extension: accept) }
        default:
            // An omaflake package of one kind; recipes also come as plain folders (a checkout, a folder of recipes)
            let accept = ask.accept ?? "vm"
            let type = UTType(exportedAs: "app.omaflake.\(accept)")
            panel.allowedContentTypes = accept == "recipe" ? [type, .folder] : [type]
            panel.canChooseDirectories = accept == "recipe"
            panel.canChooseFiles = true
            checker = PathChecker(check: ask.check, client: client, renderer: renderer)
        }
        panel.delegate = checker
        let ok = await present(panel)
        return withExtendedLifetime(checker) { ok ? panel.urls : nil }
    }

    /// Shows a panel as a sheet on the key window – the menu window (a separate panel window would close it, plan
    /// B59/B69) or the app window (plan B119); without a visible window as a modal panel
    private func present(_ panel: NSOpenPanel) async -> Bool {
        NSApp.activate()
        if let window = NSApp.keyWindow ?? NSApp.windows.first(where: { $0.isVisible && $0.canBecomeKey }) {
            return await withCheckedContinuation { cont in
                panel.beginSheetModal(for: window) { cont.resume(returning: $0 == .OK) }
            }
        }
        return panel.runModal() == .OK
    }

    // MARK: Questions

    /// A question inside omaflake's window – the menu window or the app window (an alert window would close the menu
    /// window, plan B59)
    struct Confirmation: Identifiable {
        let id = UUID()
        let title: String
        let text: String
        let button: String
        /// The answer removes something: the button is red
        var destructive = false
        /// A text the answer needs (a new name, a password), edited in the question
        var input: TextInput?
        var secure = false
        /// The password once more – the answer needs both to agree
        var repeated: TextInput?
        /// nil = cancelled, 0 = the button
        let answer: (Int?) -> Void

        var canAnswer: Bool {
            guard let input else { return true }
            if let repeated { return !input.text.isEmpty && input.text == repeated.text }
            return !secure || !input.text.isEmpty
        }
    }

    @Observable final class TextInput {
        var text: String
        init(text: String) { self.text = text }
    }
    var confirmation: Confirmation?

    /// Asks in the open window and waits for the answer: nil = cancelled, 0 = `button`
    private func choose(_ title: String, _ text: String, button: String, destructive: Bool = false,
                        input: TextInput? = nil, secure: Bool = false, repeated: TextInput? = nil) async -> Int? {
        // Without an open window (a package double-clicked in the Finder) the question comes as an alert
        if !NSApp.windows.contains(where: { $0.isVisible && $0.canBecomeKey }) {
            NSApp.activate()
            let alert = NSAlert()
            alert.messageText = title
            alert.informativeText = text
            for b in [button, String(localized: "Cancel")] { alert.addButton(withTitle: b) }
            alert.buttons.last?.keyEquivalent = "\u{1b}"
            if destructive { alert.buttons.first?.hasDestructiveAction = true }
            func field(_ i: TextInput) -> NSTextField {
                let f = secure ? NSSecureTextField(string: i.text) : NSTextField(string: i.text)
                f.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
                return f
            }
            let fields = [input, repeated].compactMap { $0 }.map { ($0, field($0)) }
            if fields.count == 1 {
                alert.accessoryView = fields[0].1
            } else if fields.count == 2 {
                let stack = NSStackView(views: fields.map(\.1))
                stack.orientation = .vertical
                stack.frame = NSRect(x: 0, y: 0, width: 260, height: 56)
                alert.accessoryView = stack
            }
            let i = alert.runModal().rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
            for (input, f) in fields { input.text = f.stringValue }
            if let repeated, let input, input.text != repeated.text { return nil }
            return i == 0 ? 0 : nil
        }
        confirmation?.answer(nil)   // a newer question replaces an unanswered one
        return await withCheckedContinuation { cont in
            confirmation = Confirmation(title: title, text: text, button: button, destructive: destructive, input: input,
                                        secure: secure, repeated: repeated) { [weak self] i in
                self?.confirmation = nil
                cont.resume(returning: i)
            }
        }
    }

    // MARK: Language (the app's own)

    /// App language: "" = system language, otherwise "en" or "de" (English is the fallback for everything else).
    /// Stored as AppleLanguages in the app's own defaults; takes effect at the next launch.
    static let languages = ["en", "de"]
    private(set) var languageChanged = false

    var language: String {
        get {
            let domain = Bundle.main.bundleIdentifier.flatMap { UserDefaults.standard.persistentDomain(forName: $0) }
            return (domain?["AppleLanguages"] as? [String])?.first.flatMap { Self.languages.contains($0) ? $0 : nil } ?? ""
        }
        set {
            if newValue.isEmpty { UserDefaults.standard.removeObject(forKey: "AppleLanguages") }
            else { UserDefaults.standard.set([newValue], forKey: "AppleLanguages") }
            languageChanged = true
        }
    }

    /// Starts a new instance of the app and quits this one (VMs keep running).
    func relaunch() {
        let config = NSWorkspace.OpenConfiguration()
        config.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: config) { _, _ in
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }

    // MARK: Launch at login (the app's own)

    var launchAtLogin: Bool {
        get { SMAppService.mainApp.status == .enabled }
        set {
            do {
                if newValue { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            } catch {
                toast(String(localized: "Login item: \(error.localizedDescription)"))
            }
        }
    }
}

/// Checks what is chosen in an open panel before it closes: a package by the core's `check` (`packages.inspect`, plan
/// B109) – the panel stays open with the reason when it is not what is asked for – or a file by its extension
final class PathChecker: NSObject, NSOpenSavePanelDelegate {
    let check: Ask.Check?
    let client: CoreClient?
    let renderer: ViewTextRenderer
    let pathExtension: String?

    init(check: Ask.Check?, client: CoreClient, renderer: ViewTextRenderer) {
        self.check = check
        self.client = client
        self.renderer = renderer
        pathExtension = nil
    }

    init(extension ext: String) {
        check = nil
        client = nil
        renderer = .english
        pathExtension = ext
    }

    func panel(_ sender: Any, shouldEnable url: URL) -> Bool {
        guard let pathExtension else { return true }
        return url.hasDirectoryPath || url.pathExtension == pathExtension
    }

    func panel(_ sender: Any, validate url: URL) throws {
        if let pathExtension, url.pathExtension != pathExtension {
            throw NSError(domain: "app.omaflake", code: 1, userInfo: [NSLocalizedDescriptionKey: String(localized: "Not an omaflake package")])
        }
        guard let check, let client else { return }
        var params = check.params
        params.set(.string(url.path), at: check.param)
        let answer: JSONValue
        do {
            answer = try client.callBlocking(check.method, params, as: JSONValue.self)
        } catch {
            throw NSError(domain: "app.omaflake", code: 1, userInfo: [NSLocalizedDescriptionKey: "\(error)"])
        }
        if answer["valid"] == .bool(false) {
            throw NSError(domain: "app.omaflake", code: 1, userInfo: [
                NSLocalizedDescriptionKey: reason(answer["reason"]) ?? String(localized: "Not an omaflake package"),
            ])
        }
    }

    /// The refusal's `reason`, a text of the views, in the app's language
    private func reason(_ value: JSONValue?) -> String? {
        guard let value, let data = try? JSONEncoder.api.encode(value),
              let text = try? JSONDecoder.api.decode(ViewText.self, from: data) else { return nil }
        return renderer.render(text)
    }
}
