// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import SwiftUI
import UniformTypeIdentifiers
import Foundation
import Observation
import ServiceManagement
import OmaflakeKit

/// App state. Everything about VMs, images, recipes, jobs and the Doctor comes from omaflake's Rust core – the app
/// starts `omaf serve` (`CoreClient`, docs/api.md), loads the lists once and then follows the server's events; it holds
/// only what the UI shows and what the user is doing (pages, filters, questions, the activity list). VMs started via
/// the CLI appear on their own, and quitting the app leaves running VMs untouched.
@MainActor @Observable
final class AppModel {
    let client: CoreClient
    /// What `initialize` answered (data folder, the core's version); nil while the core is not running
    private(set) var session: InitializeResult?
    /// Why the core is not running (it is started again on its own)
    private(set) var coreProblem: String?

    // MARK: What the core reports

    /// The VMs (`vms.list`, `vms.changed`)
    private(set) var rows: [Vm] = []
    private(set) var unreadableVMs: [Unreadable] = []
    private(set) var images: [VmImage] = []
    private(set) var unreadableImages: [Unreadable] = []
    private(set) var recipeList = RecipeList()
    private(set) var doctor = DoctorReport()
    private(set) var checkingPrerequisites = false
    /// The server's jobs in the order they came (builds, downloads, imports, …)
    private(set) var jobs: [Job] = []
    /// Their log lines (`job.log`), at most 5000 per job
    private(set) var jobLogs: [String: [String]] = [:]
    private(set) var sources: [Source] = []
    private(set) var settingsInfo = SettingsInfo()
    /// What the remote sources offer – fetched only when the Recipes tab asks (never in the background)
    private(set) var available = AvailableRecipes()
    private(set) var catalogsLoaded = false
    private(set) var loadingCatalogs = false
    /// Running macOS guests Virtualization.framework allows (`vms.limits`, plan BM16)
    private(set) var macMax = 2
    /// VMs with an action of this app in progress (besides the server's own `busy`)
    private var pendingActions: Set<String> = []

    var recipes: [Recipe] { recipeList.recipes }
    var unreadableRecipes: [Unreadable] { recipeList.unreadable }
    var idLengths: RecipeList.IDLengths { recipeList.idLengths }

    func recipe(_ id: String) -> Recipe? { recipes.first { $0.id == id } }
    func source(_ id: String?) -> Source? { id.flatMap { id in sources.first { $0.id == id } } }
    func isBusy(_ vm: Vm) -> Bool { vm.busy || pendingActions.contains(vm.id) }

    /// Recipes the app shows: lab recipes only in developer mode
    var visibleRecipes: [Recipe] { developerMode ? recipes : recipes.filter { !$0.lab } }

    /// One row per recipe line (plan B111): its newest version names it; built ones first, each kind by name
    var recipeRows: [Recipe] {
        Dictionary(grouping: visibleRecipes, by: \.line).compactMap { _, versions in
            versions.first { r in r.versions.first?.id == r.id } ?? versions.first
        }
        .sorted { ($0.image == nil ? 1 : 0, $0.kind.rawValue, $0.displayName) < ($1.image == nil ? 1 : 0, $1.kind.rawValue, $1.displayName) }
    }

    /// Every built image, grouped by recipe (macOS, then NixOS, by name), newest first within a recipe (plan B93)
    var allImages: [VmImage] {
        images.sorted {
            ($0.kind.rawValue, RecipeText.name($0.recipe), $0.recipe, $1.created)
                < ($1.kind.rawValue, RecipeText.name($1.recipe), $1.recipe, $0.created)
        }
    }

    /// The images of a recipe's line, newest first
    func images(ofLine line: String) -> [VmImage] {
        images.filter { $0.line == line }.sorted { $0.created > $1.created }
    }

    /// The recipe evaluates to something else than when the image was built (nil: not known)
    func isOutdated(_ image: VmImage) -> Bool? {
        if let outdated = image.outdated { return outdated }
        guard image.current, let r = recipes.first(where: { $0.image == image.name }) else { return nil }
        return r.outdated
    }

    /// "dev (words)" – a recipe as its rows name it
    func recipeName(_ id: String) -> String {
        guard let r = recipe(id) else { return RecipeText.name(id) }
        return r.displayName + (r.distinction.map { " (\($0))" } ?? "")
    }

    // MARK: Settings (config.json through the core)

    /// Developer mode: shows the lab recipes
    var developerMode: Bool {
        get { settingsInfo.settings.developerMode }
        set {
            settingsInfo.settings.developerMode = newValue
            setSetting("developerMode", newValue ? .bool(true) : .null)
        }
    }

    /// Keep older versions of a recipe's line (plan B111); off: they go unless an image uses them – the core prunes
    /// them when the setting goes off and tells how many with a `message`
    var keepRecipeVersions: Bool {
        get { settingsInfo.settings.keepRecipeVersions }
        set {
            settingsInfo.settings.keepRecipeVersions = newValue
            setSetting("keepRecipeVersions", newValue ? .bool(true) : .null)
        }
    }

    /// System notifications while the menu window is closed
    var notifications: Bool {
        get { settingsInfo.settings.notifications }
        set {
            settingsInfo.settings.notifications = newValue
            setSetting("notifications", newValue ? .null : .bool(false))
        }
    }

    var checkForUpdates: Bool {
        get { settingsInfo.settings.checkForUpdates }
        set {
            settingsInfo.settings.checkForUpdates = newValue
            setSetting("checkForUpdates", newValue ? .null : .bool(false)) { [weak self] in await self?.checkForUpdate() }
        }
    }

    /// `settings.set` with one key (`.null` = back to the default); `then` runs once it is stored
    private func setSetting(_ key: String, _ value: JSONValue, then: (@MainActor () async throws -> Void)? = nil) {
        Task {
            do {
                let settings: CoreSettings = try await client.call("settings.set", JSONValue.object([key: value]))
                settingsInfo.settings = settings
                try await then?()
            } catch { message = describe(error) }
        }
    }

    /// The data folder now (`settings.get` follows a move; `initialize` named the one at the start)
    var dataFolder: String {
        let now = settingsInfo.settings.dataFolder
        return !now.isEmpty ? now : session?.dataFolder ?? Self.defaultDataFolder.path
    }
    static var defaultDataFolder: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".omaflake", isDirectory: true)
    }
    var isDefaultDataFolder: Bool { settingsInfo.isDefaultDataFolder }

    /// A system notification, if they are on (and the menu window is closed – `Notifier`)
    func notify(_ title: String, _ body: String? = nil) {
        if notifications { Notifier.post(title, body) }
    }

    // Updates (spec §11.6, plan B68): only a check against the feed for now; installing comes with signed releases
    enum UpdateState: Equatable {
        case noFeed, checking, upToDate(Date), available(UpdateStatus.Release), failed(String), off
    }
    private(set) var updateState: UpdateState = .noFeed

    func checkForUpdate() async {
        guard checkForUpdates else { updateState = .off; return }
        updateState = .checking
        do {
            let status: UpdateStatus = try await client.call("updates.check")
            switch status.state {
            case .available:
                if let release = status.release {
                    updateState = .available(release)
                    message = String(localized: "omaflake \(release.version) is available")
                } else {
                    updateState = .upToDate(Date())
                }
            case .noFeed: updateState = .noFeed
            case .off: updateState = .off
            case .upToDate, .unknown: updateState = .upToDate(Date())
            }
        } catch {
            updateState = .failed(describe(error))
        }
    }

    /// The language the app shows ("de", "en") – for recipe descriptions (the core gets it at `initialize`)
    static var uiLanguage: String? { Bundle.main.preferredLocalizations.first.map { String($0.prefix(2)) } }

    // MARK: Pages, tabs, activity

    /// The image the New VM page creates from – set by an image row, the only way to it (plan B100)
    var newVMImage: String?
    /// The recipe the Build Image page builds – set by a recipe row or a build row, the only ways to it (plan B100)
    var buildRecipe: String?

    /// Reports what happened ("VM deleted", errors) – it lands in the activity tab and briefly in the footer line
    var message: String? {
        didSet {
            guard let message, !message.isEmpty else { return }
            let activity = Activity(date: Date(), text: message)
            activities.insert(activity, at: 0)
            if activities.count > 30 { activities.removeLast(activities.count - 30) }
            footer = activity
            if tab != .activity || !path.isEmpty { unseenActivity = true }
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(6))
                if self?.footer?.id == activity.id { self?.footer = nil }
            }
        }
    }
    /// The newest message for a few seconds, below the tabs
    private(set) var footer: Activity?
    /// Messages arrived since the activity tab was last open
    private(set) var unseenActivity = false

    /// Tabs of the main page (plan B91)
    enum HomeTab: String, CaseIterable {
        case vms, images, recipes, activity
    }
    /// What a list shows (plan B101): status (on/off, in use, built), guest kind, and per list a tag or a recipe.
    /// Kept per tab for the session; the filter button in the list's header sets it.
    struct ListFilter: Equatable {
        var status: Bool?
        var kind: GuestKind?
        var tag: String?
        var recipe: String?
        /// Recipes: also those for other architectures (plan B114)
        var otherSystems = false
        var isActive: Bool { status != nil || kind != nil || tag != nil || recipe != nil || otherSystems }
    }
    var filters: [HomeTab: ListFilter] = [:]

    var tab: HomeTab = .vms {
        didSet {
            UserDefaults.standard.set(tab.rawValue, forKey: "homeTab")
            if tab == .activity { unseenActivity = false }
        }
    }
    // Navigation inside the window (menu and app window share it): the open pages, the last one is shown; empty = the main page
    enum Page: Hashable {
        case doctor, settings, newVM, build, macBase, guide

        var title: LocalizedStringKey {
            switch self {
            case .doctor: "Doctor"
            case .settings: "Settings"
            case .newVM: "New VM"
            case .build: "Build Image"
            case .macBase: "Create macOS Base"
            case .guide: "Linux Builder Guide"
            }
        }
    }
    var path: [Page] = []
    /// Opens a page one level deeper; a page already open further up becomes the current one again
    func show(_ page: Page) {
        if let i = path.firstIndex(of: page) { path = Array(path[...i]) } else { path.append(page) }
    }
    func back() { if !path.isEmpty { path.removeLast() } }

    /// Recent Activity in the main window, newest first (this session only)
    private(set) var activities: [Activity] = []
    func clearActivities() {
        activities = []
        footer = nil
        unseenActivity = false
        message = nil
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
            launcher = CoreClient.processLauncher(executable: omaf)
        } else {
            launcher = { _ in throw APIError(message: "omaf not found – it belongs next to the app (Contents/MacOS/omaf); for development set OMAF_BIN or run cargo build") }
        }
        client = CoreClient(launcher: launcher, initialize: params)
        // The tab used last; the very first time (no VM yet: see `loadAll`) the recipes, where one starts
        if let saved = UserDefaults.standard.string(forKey: "homeTab").flatMap(HomeTab.init(rawValue:)) { tab = saved }
        // The first time, the window (menu or app window, plan B119) opens on the Doctor
        if Self.firstLaunch { path = [.doctor] }
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
        // At launch (once the core runs), then once a day
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            while !Task.isCancelled {
                await self?.checkForUpdate()
                try? await Task.sleep(for: .seconds(24 * 3600))
            }
        }
    }

    // MARK: Events

    private func handle(_ event: ServerEvent) {
        switch event {
        case .connected(let result):
            let restarted = session != nil || coreProblem != nil
            session = result
            if coreProblem != nil { message = String(localized: "omaflake's core runs again") }
            coreProblem = nil
            // A restarted server has lost its jobs (api.md §5); their logs stay in their files
            if restarted { jobs = []; jobLogs = [:] }
            Task { await loadAll(firstTime: !restarted) }
        case .disconnected(let reason):
            if coreProblem == nil { message = reason }
            coreProblem = reason
        case .vmsChanged(let list):
            apply(list)
        case .vmEnded(let name, let requested):
            // A VM that ended on its own (shut down inside, crashed, stopped with the CLI): message and notification
            if !requested {
                message = String(localized: "\(name) has ended")
                notify(String(localized: "VM ended"), String(localized: "\(name) is no longer running."))
            }
        case .vmReady(let name, let via):
            message = String(localized: "\(name) is ready")
            notify(String(localized: "VM ready"), via == .ssh ? String(localized: "\(name) is running and answers SSH.")
                   : String(localized: "\(name) is running; its omaflake agent reported in."))
        case .imagesChanged(let list):
            if list.images != images { images = list.images }
            if list.unreadable != unreadableImages { unreadableImages = list.unreadable }
        case .recipesChanged(let list):
            if list != recipeList { recipeList = list }
        case .jobChanged(let job):
            update(job)
        case .jobLog(let id, let lines):
            appendLog(id, lines)
        case .doctorChanged(let report):
            doctor = report
        case .message(let text, _):
            // Only what the app cannot tell from the other events (api.md §6): pruned recipe versions, a recipe list
            // that cannot be read, a VM created but not started
            message = text
        case .unknown:
            break
        case .undecodable(let method, let error):
            FileHandle.standardError.write(Data("omaflake: event \(method) not understood: \(error)\n".utf8))
        }
    }

    /// Everything the pages show, at the start and after the core was started again
    private func loadAll(firstTime: Bool) async {
        async let vms: VmList = client.call("vms.list")
        async let imageList: ImageList = client.call("images.list")
        async let recipes: RecipeList = client.call("recipes.list")
        async let settings: SettingsInfo = client.call("settings.get")
        async let sourceList: [Source] = client.call("sources.list")
        async let jobList: [Job] = client.call("jobs.list")
        async let limits: Limits = client.call("vms.limits")
        do {
            apply(try await vms)
            let i = try await imageList
            images = i.images
            unreadableImages = i.unreadable
            recipeList = try await recipes
            settingsInfo = try await settings
            sources = try await sourceList
            for job in try await jobList { update(job) }
            macMax = try await limits.macMax
        } catch {
            message = describe(error)
        }
        if firstTime, UserDefaults.standard.string(forKey: "homeTab") == nil, rows.isEmpty { tab = .recipes }
        // The Doctor's checks run at start, when the Doctor opens, on "Check Again", and in the core after actions
        // and right before a build – not while idle
        await checkAll()
    }

    private func apply(_ list: VmList) {
        if list.vms != rows { rows = list.vms }
        if list.unreadable != unreadableVMs { unreadableVMs = list.unreadable }
    }

    private func reloadVMs() async {
        if let list = try? await client.call("vms.list", as: VmList.self) { apply(list) }
    }

    // MARK: Jobs (api.md §5)

    /// What to say when a job the app started is done (its kind alone does not tell: an export's name, …)
    private var onDone: [String: (Job) -> Void] = [:]

    private func update(_ job: Job) {
        let old = jobs.first { $0.id == job.id }
        if let i = jobs.firstIndex(where: { $0.id == job.id }) { jobs[i] = job } else { jobs.append(job) }
        guard job.isDone, old.map({ !$0.isDone }) ?? true else { return }
        if let done = onDone.removeValue(forKey: job.id) { done(job) } else { finished(job) }
    }

    /// Follows a job the app started; `done` runs once it is finished, failed or cancelled
    private func track(_ job: Job, done: ((Job) -> Void)? = nil) {
        let known = jobs.first { $0.id == job.id }
        if known == nil { jobs.append(job) }
        guard let done else { return }
        // Its end may have come as an event before this answer
        if let known, known.isDone { done(known) } else { onDone[job.id] = done }
    }

    private func appendLog(_ id: String, _ lines: [String]) {
        var log = jobLogs[id] ?? []
        log.append(contentsOf: lines)
        if log.count > 5000 { log.removeFirst(log.count - 5000) }
        jobLogs[id] = log
    }

    /// A job's log; loaded from the core when this app has none of it yet
    func loadLog(_ job: Job) async {
        guard jobLogs[job.id] == nil, job.logLines > 0 else { return }
        if let log: JobLog = try? await client.call("jobs.log", ["id": job.id]) { jobLogs[job.id] = Array(log.lines.suffix(5000)) }
    }

    /// The messages and notifications for jobs that ended – the core sends none for them (api.md §6)
    private func finished(_ job: Job) {
        let name = job.recipe.map(RecipeText.name) ?? job.title
        switch (job.kind, job.state) {
        case (.build, .finished):
            message = String(localized: "Image \(name) ready")
            notify(String(localized: "Image ready"), String(localized: "\(name) can be used for new VMs."))
        case (.build, .failed):
            message = String(localized: "Build \(name): \(job.error ?? "")")
            notify(String(localized: "Build failed"), String(localized: "\(name) – the log is in the Images tab."))
        case (.build, .cancelled):
            message = String(localized: "Build of \(name) cancelled")
        case (.ipswDownload, .finished):
            message = job.result(as: IpswResult.self).map { String(localized: "macOS \($0.version) downloaded") }
                ?? String(localized: "macOS downloaded")
        case (.ipswDownload, .cancelled):
            message = String(localized: "Download paused – it continues from there")
        case (.macosInstall, .failed):
            message = String(localized: "macOS base: \(job.error ?? "")")
        case (.dataMove, .finished):
            Task { settingsInfo = (try? await client.call("settings.get")) ?? settingsInfo }
            message = String(localized: "Data folder: \(job.result(as: DataMoveResult.self)?.dataFolder ?? job.title)")
        case (let kind, .failed) where kind.isBuilderTask:
            message = (job.logFile.map { String(localized: "Failed – see \($0)") } ?? String(localized: "Failed"))
                + "\n" + String((job.error ?? "").suffix(400))
        case (.builderInstallNix, .finished):
            message = String(localized: "Nix installed")
        case (.builderSetup, .finished), (.builderApply, .finished):
            message = String(localized: "Linux builder settings applied")
        case (.builderCleanUp, .finished):
            message = job.result(as: MessageResult.self)?.message ?? String(localized: "Linux builder cleaned up")
        case (.recipeInstall, .finished):
            installed(job, id: job.recipe ?? job.title)
        case (.import, .finished):
            imported(job)
        case (_, .failed):
            message = "\(job.title): \(job.error ?? "")"
        default:
            break
        }
    }

    /// The recipe's waiting or running build
    func activeJob(for recipe: String) -> Job? { jobs.last { $0.kind == .build && $0.recipe == recipe && $0.isActive } }
    /// The recipe's latest build (also a finished one, for its log)
    func latestJob(for recipe: String) -> Job? { jobs.last { $0.kind == .build && $0.recipe == recipe } }
    var builds: [Job] { jobs.filter { $0.kind == .build } }
    var anyBuildRunning: Bool { builds.contains { $0.isActive } }

    private func activeJob(_ kind: Job.Kind) -> Job? { jobs.last { $0.kind == kind && $0.isActive } }
    /// The macOS base is being created (`macos.installBase`)
    var installing: Bool { activeJob(.macosInstall) != nil }
    /// The IPSW download (on its own, or while the base's creation downloads)
    var downloadJob: Job? {
        activeJob(.ipswDownload) ?? activeJob(.macosInstall).flatMap { ($0.progress?.of ?? 0) > 0 ? $0 : nil }
    }
    var downloading: Bool { downloadJob != nil }
    var downloadProgress: (done: Int64, total: Int64) { (downloadJob?.progress?.done ?? 0, downloadJob?.progress?.of ?? 0) }
    /// The log of the latest base creation
    var installLog: [String] { jobs.last { $0.kind == .macosInstall }.flatMap { jobLogs[$0.id] } ?? [] }
    /// The running Linux builder task (its title)
    var builderTask: String? { jobs.last { $0.kind.isBuilderTask && $0.isActive }?.title }
    var movingData: Bool { activeJob(.dataMove) != nil }
    var installingRecipes: Set<String> {
        Set(jobs.filter { $0.kind == .recipeInstall && $0.isActive }.compactMap(\.recipe))
    }

    /// Stops a job: a waiting one leaves the queue; a running one stops (a macOS build VM is forced off, a half-built
    /// image is deleted; a download pauses)
    func cancel(_ job: Job) {
        Task {
            do { update(try await client.call("jobs.cancel", ["id": job.id])) } catch { message = describe(error) }
        }
    }

    /// Hides a finished job's row (its log goes with it)
    func dismiss(_ job: Job) {
        jobs.removeAll { $0.id == job.id }
        jobLogs[job.id] = nil
        Task { let _: Ignored? = try? await client.call("jobs.dismiss", ["id": job.id]) }
    }

    // MARK: Recipes and sources (spec §11.3, plan B107)

    func revealRecipe(_ id: String) {
        if let folder = recipe(id)?.folder { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)]) }
    }

    /// Fetches the enabled remote sources' catalogs (opening the Recipes tab, or its refresh button)
    func refreshCatalogs() {
        guard !loadingCatalogs else { return }
        loadingCatalogs = true
        Task {
            do { available = try await client.call("recipes.available") } catch { message = describe(error) }
            catalogsLoaded = true
            loadingCatalogs = false
        }
    }

    var offeredRecipes: [CatalogRecipe] { available.offered.filter { developerMode || !$0.lab } }
    var suggestedRecipes: [CatalogRecipe] { available.suggested }
    var recipeUpdates: [AvailableRecipes.Update] { available.updates }
    /// The catalog entry of an installed recipe when its source has a newer version
    func update(for id: String) -> CatalogRecipe? { available.updates.first { $0.id == id }?.recipe }

    /// Downloads a recipe (with what it requires) from its source; also updates
    func installRecipe(_ recipe: CatalogRecipe) {
        guard !installingRecipes.contains(recipe.id) else { return }
        Task {
            do {
                let job: Job = try await client.call("recipes.install", ["id": recipe.id, "source": recipe.source])
                track(job) { job in
                    self.finished(job)
                    if job.state == .finished { self.refreshCatalogs() }
                }
            } catch { message = describe(error) }
        }
    }

    /// "Installed …" or "… is up to date" for a finished `recipe-install` job
    private func installed(_ job: Job, id: String) {
        let written = job.result(as: InstallResult.self)?.installed ?? []
        message = written.isEmpty ? String(localized: "\(RecipeText.qualified(id)) is up to date")
            : String(localized: "Installed \(written.map(RecipeText.qualified).joined(separator: ", "))")
    }

    func installSuggested() {
        Task {
            do {
                let started: [Job] = try await client.call("recipes.installSuggested")
                for job in started { track(job) }
            } catch { message = describe(error) }
        }
    }

    /// Removes every version of a line that can go (plan B111)
    func removeAllVersions(_ line: String, name: String) {
        Task { @MainActor in
            guard await confirm(String(localized: "Remove all versions of “\(name)”?"),
                                String(localized: "Versions images are built from stay while those images are there; a recipe in a local folder stays in its folder. Images and VMs stay."),
                                button: String(localized: "Remove All"), destructive: true) else { return }
            do {
                let result: LineRemoval = try await client.call("recipes.removeLine", ["line": line])
                let removed = result.removed.count, kept = result.kept.count
                message = kept == 0 ? String(localized: "\(name): \(removed) versions removed")
                    : String(localized: "\(name): \(removed) versions removed, \(kept) kept")
            } catch { message = describe(error) }
        }
    }

    func removeRecipe(_ version: Recipe.Version) {
        Task { @MainActor in
            let label = version.label
            guard await confirm(String(localized: "Remove the recipe version “\(label)”?"),
                                String(localized: "Its files are deleted; images built from it and their VMs stay. You can install it again from its source."),
                                button: String(localized: "Remove"), destructive: true) else { return }
            do {
                let _: Ignored = try await client.call("recipes.remove", ["id": version.id])
                message = String(localized: "\(label) removed")
            } catch { message = describe(error) }
        }
    }

    /// Checks a new source (with its token for a private repository) and keeps it; the result or the error comes back
    func addSource(_ input: String, isPrivate: Bool, token: String) async -> Result<String, APIError> {
        struct Params: Encodable, Sendable { let input: String; let `private`: Bool; let token: String? }
        do {
            let added: SourceAdded = try await client.call("sources.add", Params(input: input, private: isPrivate, token: isPrivate ? token : nil))
            await reloadSources()
            // The note names the source already ("acme/recipes: 3 recipes")
            return .success(added.note.isEmpty ? added.source.displayName : added.note)
        } catch let e as APIError { return .failure(e) } catch { return .failure(APIError(message: error.localizedDescription)) }
    }

    private func reloadSources() async {
        if let list: [Source] = try? await client.call("sources.list") { sources = list }
        refreshCatalogs()
    }

    func setSource(_ id: String, enabled: Bool) {
        struct Params: Encodable, Sendable { let id: String; let enabled: Bool }
        Task {
            do {
                sources = try await client.call("sources.setEnabled", Params(id: id, enabled: enabled))
                refreshCatalogs()
            } catch { message = describe(error) }
        }
    }

    func moveSource(_ id: String, up: Bool) {
        struct Params: Encodable, Sendable { let id: String; let up: Bool }
        Task {
            do { sources = try await client.call("sources.move", Params(id: id, up: up)) } catch { message = describe(error) }
        }
    }

    func removeSource(_ id: String) {
        Task { @MainActor in
            guard let s = source(id), await confirm(String(localized: "Remove the source “\(s.displayName)”?"),
                String(localized: "Recipes installed from it stay until you remove them; its token goes from the Keychain."),
                button: String(localized: "Remove"), destructive: true) else { return }
            do {
                let _: Ignored = try await client.call("sources.remove", ["id": id])
                await reloadSources()
            } catch { message = describe(error) }
        }
    }

    // MARK: VMs

    /// Runs an action on a VM; while it runs the VM's row shows it as busy. `work` returns the message to show.
    private func perform(_ name: String, _ work: @escaping @MainActor () async throws -> String?) {
        guard !pendingActions.contains(name) else { return }
        pendingActions.insert(name)
        Task {
            do {
                if let note = try await work() { message = note }
            } catch {
                message = "\(name): \(describe(error))"
            }
            pendingActions.remove(name)
            await reloadVMs()
        }
    }

    /// An error as the app shows it: the core's message, or the app's own text for a kind it has one for
    func describe(_ error: Error) -> String {
        guard let e = error as? APIError else { return error.localizedDescription }
        switch e.kind {
        case .vmRunning?: return String(localized: "running – stop it first")
        case .macLimit?: return macLimitReason ?? e.message
        default: return e.message
        }
    }

    /// Running macOS guests; Virtualization.framework allows `macMax` (plan BM16).
    var runningMacs: [String] { rows.filter { $0.isMac && $0.isRunning }.map(\.id) }

    /// Why a macOS guest cannot start right now (shown before starting), nil if it can.
    var macLimitReason: String? {
        runningMacs.count < macMax ? nil
            : String(localized: "macOS allows at most \(macMax) macOS VMs at the same time – running: \(runningMacs.joined(separator: ", "))")
    }

    func canStart(_ row: Vm) -> Bool {
        row.status == .stopped && (!row.isMac || macLimitReason == nil) && startBlockers(row.kind).isEmpty
    }

    /// Without a window, or the UI's start (`auto`: a window, with 3D where QEMU can)
    func start(_ name: String, window: Bool) {
        if rows.first(where: { $0.id == name })?.isMac == true, let reason = macLimitReason {
            message = reason
            return
        }
        perform(name) { [client] in
            let _: Vm = try await client.call("vms.start", ["name": name, "display": window ? "auto" : "none"])
            return nil
        }
    }

    func stop(_ name: String) {
        perform(name) { [client] in
            let answer: StopAnswer = try await client.call("vms.stop", ["name": name])
            return "\(name): \(answer.result.rawValue)"
        }
    }

    func reset(_ name: String) {
        Task { @MainActor in
            guard await confirm(String(localized: "Reset “\(name)”?"), String(localized: "The current state of the VM is deleted and cannot be restored.")) else { return }
            perform(name) { [client] in
                let _: Ignored = try await client.call("vms.reset", ["name": name])
                return String(localized: "\(name) reset")
            }
        }
    }

    func upgrade(_ name: String) {
        Task { @MainActor in
            guard let row = rows.first(where: { $0.id == name }), let latest = row.latestImage else { return }
            guard await confirm(String(localized: "Upgrade “\(name)” to \(latest)?"), String(localized: "The current state is deleted and cannot be restored; settings are kept.")) else { return }
            perform(name) { [client] in
                // The newest image of its recipe – the one the row offered
                let vm: Vm = try await client.call("vms.upgrade", ["name": name, "image": latest])
                return String(localized: "\(name) now uses \(vm.image)")
            }
        }
    }

    func remove(_ name: String) {
        Task { @MainActor in
            let location = rows.first(where: { $0.id == name })?.location
            let text = location.map { String(localized: "The VM runs from \($0) – it is deleted there and cannot be restored. Its share folder is kept if it holds files. To keep it there, choose Stop Listing instead.") }
                ?? String(localized: "The VM and its disk are deleted and cannot be restored. Its share folder is kept if it holds files.")
            guard await confirm(String(localized: "Remove “\(name)”?"), text, button: String(localized: "Remove"), destructive: true) else { return }
            perform(name) { [client] in
                let _: Ignored = try await client.call("vms.remove", ["name": name])
                return String(localized: "\(name) deleted")
            }
        }
    }

    /// Creates a VM from exactly this image (plan B100); `sharePath`: a folder the user picked (nil = the default one)
    func create(name: String, image: String, settings: RecipeSettings, sharePath: String? = nil, startAfter: Bool) {
        struct Params: Encodable, Sendable {
            let name: String, image: String, settings: RecipeSettings, sharePath: String?, start: Bool
        }
        let start = startAfter && (settings.kind != .macos || macLimitReason == nil)
        let params = Params(name: name, image: image, settings: settings, sharePath: settings.share ? sharePath : nil, start: start)
        perform(name) { [client] in
            let vm: Vm = try await client.call("vms.create", params)
            return vm.isMac || vm.sshPort == nil ? String(localized: "\(name) created")
                : String(localized: "\(name) created (SSH port \(vm.sshPort ?? 0))")
        }
    }

    /// Sound, microphone, clipboard and camera of a VM; they take effect at its next start.
    func setSound(_ name: String, audio: Bool? = nil, microphone: Bool? = nil, clipboard: Bool? = nil, camera: Bool? = nil) {
        struct Params: Encodable, Sendable { let name: String; let audio, microphone, clipboard, camera: Bool? }
        let params = Params(name: name, audio: audio, microphone: microphone, clipboard: clipboard, camera: camera)
        perform(name) { [client] in
            let changed: VmSetAnswer = try await client.call("vms.set", params)
            return changed.takesEffectAtNextStart ? String(localized: "\(name): takes effect at the next start") : nil
        }
    }

    /// The first free VM name for a recipe (`dev`, `dev-2`, …)
    func nextFreeName(for recipe: String) async -> String {
        let result = try? await client.call("vms.nextName", ["recipe": recipe], as: NameAnswer.self)
        return result?.name ?? RecipeText.name(recipe)
    }

    /// The SSH command of a VM, as Terminal would run it
    func sshLine(_ name: String) async -> String? {
        try? await client.call("vms.ssh", ["name": name], as: SshAnswer.self).shellLine
    }

    /// Terminal.app with `ssh` to the VM (asks for automation permission the first time).
    func openTerminal(_ name: String) {
        Task {
            do {
                let ssh: SshAnswer = try await client.call("vms.ssh", ["name": name])
                runInTerminal(ssh.shellLine)
            } catch { message = describe(error) }
        }
    }

    /// Types `line` into a new Terminal window and runs it – the user sees what runs and answers prompts there.
    func runInTerminal(_ line: String) {
        let escaped = line.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let script = "tell application \"Terminal\"\nactivate\ndo script \"\(escaped)\"\nend tell"
        var error: NSDictionary?
        NSAppleScript(source: script)?.executeAndReturnError(&error)
        if let error { message = "Terminal: \(error[NSAppleScript.errorMessage] ?? error)" }
    }

    func revealShare(_ name: String) {
        Task {
            do {
                let share: PathAnswer = try await client.call("vms.share", ["name": name])
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: share.path)])
            } catch { message = describe(error) }
        }
    }

    // MARK: Images

    /// Queues a build of a recipe's image. macOS recipes need the guest user's password once (sudo in the build VM;
    /// kept by the core only until the build VM uses it).
    func build(_ recipe: String, password: String? = nil) {
        guard activeJob(for: recipe) == nil, self.recipe(recipe)?.runsHere != false else { return }
        struct Params: Encodable, Sendable { let recipe: String; let password: String? }
        Task {
            do { track(try await client.call("images.build", Params(recipe: recipe, password: password))) }
            catch { message = String(localized: "Build \(RecipeText.name(recipe)): \(describe(error))") }
        }
    }

    /// Deletes an image (refused while VMs use it).
    func removeImage(_ name: String) {
        Task { @MainActor in
            guard await confirm(String(localized: "Remove the image “\(name)”?"),
                          String(localized: "It is deleted and cannot be restored. VMs that use it must be removed first."),
                          button: String(localized: "Remove"), destructive: true) else { return }
            do {
                let _: Ignored = try await client.call("images.remove", ["name": name])
                message = String(localized: "Image \(name) deleted")
            } catch { message = describe(error) }
        }
    }

    // MARK: Doctor (spec §11.2)

    var prerequisites: [Prerequisite] { doctor.prerequisites }
    var nixosPreflight: [Prerequisite] { doctor.nixosPreflight }
    var macosPreflight: [Prerequisite] { doctor.macosPreflight }
    var latestRelease: MacOSRelease? { doctor.latestRelease }
    /// Items in the Doctor that need attention (warning sign on its icon)
    var doctorIssues: Int { doctor.issues }
    /// A macOS base exists (needed to build macOS images)
    var hasMacBase: Bool { doctor.hasMacBase }
    /// Guest user of the macOS base (images are built logging in as this user)
    var macBaseUser: String? { doctor.macBaseUser }

    /// What the Doctor reports against building images of a guest kind; empty = building is possible
    func buildBlockers(_ kind: GuestKind) -> [Prerequisite] { doctor.buildBlockers[kind] }
    /// What blocks starting VMs of a guest kind (NixOS needs QEMU; macOS has its own runner)
    func startBlockers(_ kind: GuestKind) -> [Prerequisite] { doctor.startBlockers[kind] }

    func checkAll() async {
        checkingPrerequisites = true
        do { doctor = try await client.call("doctor.check") } catch { message = describe(error) }
        checkingPrerequisites = false
    }

    /// The Doctor opens by itself on the first launch
    static var firstLaunch: Bool { !UserDefaults.standard.bool(forKey: "requirementsShown") }
    func markRequirementsShown() { UserDefaults.standard.set(true, forKey: "requirementsShown") }

    /// Path for nix or QEMU (a file) or the image flake (a folder), picked by the user.
    func choosePath(for item: Prerequisite.Item) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = item == .flake
        panel.canChooseFiles = item != .flake
        panel.treatsFilePackagesAsDirectories = true
        panel.showsHiddenFiles = true
        panel.message = item == .flake ? String(localized: "Folder with the flake.nix of your recipes")
            : item == .nix ? String(localized: "The nix binary") : String(localized: "The qemu-system-aarch64 binary")
        present(panel) { [self] url in
            Task {
                do { doctor = try await client.call("doctor.setPath", ["item": item.rawValue, "path": url.path]) }
                catch { message = describe(error) }
            }
        }
    }

    // IPSW download (part of the macOS preflight)

    func downloadIPSW() {
        guard !downloading else { return }
        Task {
            do { track(try await client.call("macos.downloadIpsw")) } catch { message = describe(error) }
        }
    }

    /// Pauses the download; it continues from there with the next download.
    func pauseDownload() { if let job = downloadJob { cancel(job) } }

    /// An IPSW file the user already has
    func chooseIPSW(_ done: @escaping (URL) -> Void) {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.message = String(localized: "A macOS restore image (.ipsw) from Apple")
        present(panel) { url in if url.pathExtension == "ipsw" { done(url) } }
    }

    // Linux builder (spec §11.5): each task asks for an administrator through macOS' own dialog

    /// Installs Nix with the official installer through the administrator dialog.
    func installNix() {
        Task { @MainActor in
            guard await confirm(String(localized: "Install Nix?"),
                          String(localized: "omaflake installs Nix with the official installer of the NixOS community (nix-installer). It creates the /nix volume, Nix's build users and its background service, and needs the internet. Uninstall later with /nix/nix-installer uninstall."),
                          button: String(localized: "Install")) else { return }
            startBuilderTask("builder.installNix")
        }
    }

    /// nix-darwin and the Linux builder in two stages: the default builder from the cache, then omaflake's settings
    /// (each asks for an administrator once).
    func setUpNixDarwin() {
        Task { @MainActor in
            guard builderTask == nil, await confirm(String(localized: "Set up nix-darwin and the Linux builder?"),
                          String(localized: "omaflake sets up nix-darwin with its Linux builder host (nix/hosts/linux-builder.nix) – needed to build NixOS images. nix-darwin then manages /etc/nix/nix.conf; the files there that the Nix installer wrote are kept as *.before-nix-darwin. First the standard builder, then omaflake's settings; macOS asks for an administrator twice. This takes several minutes."),
                          button: String(localized: "Set Up")) else { return }
            startBuilderTask("builder.setup")
        }
    }

    /// Applies the image flake's builder host (`darwin-rebuild switch …#linux-builder`).
    func applyBuilderSettings() {
        Task { @MainActor in
            let replaces = !doctor.builderSystemIsOmaflakes
            let text = replaces
                ? String(localized: "This Mac's nix-darwin system does not come from omaflake's image flake yet. Applying replaces your current nix-darwin configuration with omaflake's Linux builder host (nix/hosts/linux-builder.nix and your local/darwin.nix). Only continue if you set up nix-darwin for omaflake.")
                : String(localized: "Runs darwin-rebuild with omaflake's Linux builder host. This takes a few minutes; macOS asks for an administrator.")
            guard await confirm(String(localized: "Apply Linux builder settings?"), text,
                                button: replaces ? String(localized: "Replace and Apply") : String(localized: "Apply")) else { return }
            startBuilderTask("builder.apply", force: replaces)
        }
    }

    /// Removes unused store paths in the builder.
    func cleanUpBuilder() { startBuilderTask("builder.cleanUp") }

    private func startBuilderTask(_ method: String, force: Bool = false) {
        guard builderTask == nil else { return }
        Task {
            do {
                let job: Job = force ? try await client.call(method, ["force": true]) : try await client.call(method)
                track(job)
            } catch { message = describe(error) }
        }
    }

    // MARK: macOS base

    /// `omaf macos install` by click: download/install, clone, first boot – a job in the core
    func installBase(user: String, fullName: String, password: String, ipsw: URL? = nil) {
        guard !installing else { return }
        struct Params: Encodable, Sendable { let user, fullName, password: String; let ipsw: String? }
        Task {
            do { track(try await client.call("macos.installBase", Params(user: user, fullName: fullName, password: password, ipsw: ipsw?.path))) }
            catch { message = String(localized: "macOS base: \(describe(error))") }
        }
    }

    // MARK: Data folder

    /// Folder picked by the user for the data; a protected folder (Documents, Desktop) then asks for access once.
    func chooseDataFolder(_ done: @escaping (URL) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = String(localized: "Use as Data Folder")
        panel.message = String(localized: "Empty folder for omaflake's images, instances and bases")
        present(panel, done)
    }

    /// Moves the data (`dataFolder.move`, a job); all VMs must be stopped.
    func moveDataFolder(to target: URL) {
        Task { @MainActor in
            guard !movingData else { return }
            guard rows.allSatisfy({ $0.status == .stopped }) else {
                message = String(localized: "Stop all VMs before moving the data folder.")
                return
            }
            guard await confirm(String(localized: "Move the data folder to \(target.path)?"),
                          String(localized: "Images, instances and bases are moved there (renamed, not copied). The settings stay in ~/.omaflake.")) else { return }
            do { track(try await client.call("dataFolder.move", ["target": target.path])) } catch { message = describe(error) }
        }
    }

    // MARK: Language

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

    // MARK: Guest values (spec §11.1)

    /// The Mac's values – shown as defaults
    var guestDefaults: GuestValues { settingsInfo.guestDefaults }
    var guestChoices: SettingsInfo.Choices { settingsInfo.choices }
    private var guestSave: Task<Void, Never>?

    /// Overrides from config.json; an empty field means "the Mac's value". Typing is stored a moment after the last key.
    var guestOverrides: GuestValues {
        get { settingsInfo.settings.guest }
        set {
            func clean(_ v: String?) -> String? { v.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 } }
            let cleaned = GuestValues(user: clean(newValue.user), fullName: clean(newValue.fullName), email: clean(newValue.email),
                                      timeZone: clean(newValue.timeZone), locale: clean(newValue.locale), keyboard: clean(newValue.keyboard))
            // The field keeps what is typed; what is stored is cleaned
            settingsInfo.settings.guest = newValue
            guestSave?.cancel()
            guestSave = Task {
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled else { return }
                let value = cleaned == GuestValues() ? JSONValue.null : ((try? JSONValue(cleaned)) ?? .null)
                do {
                    let _: CoreSettings = try await client.call("settings.set", JSONValue.object(["guest": value]))
                } catch { message = describe(error) }
            }
        }
    }

    // MARK: Launch at login

    var launchAtLogin: Bool {
        get { SMAppService.mainApp.status == .enabled }
        set {
            do {
                if newValue { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            } catch {
                message = String(localized: "Login item: \(error.localizedDescription)")
            }
        }
    }

    // MARK: Quit

    /// Quits the app; running VMs keep running, but the user is asked first.
    func quit() {
        Task { @MainActor in
            let running = rows.filter { $0.status == .running }.map(\.id)
            if !running.isEmpty {
                guard await confirm(String(localized: "Quit omaflake?"),
                              String(localized: "These VMs keep running without the app: \(running.joined(separator: ", ")). You can stop them later with omaf or when you open omaflake again."),
                              button: String(localized: "Quit")) else { return }
            }
            NSApp.terminate(nil)
        }
    }

    // MARK: Questions and panels

    /// A question inside omaflake's window – the menu window or the app window (an alert window would close the menu
    /// window, plan B59)
    struct Confirmation: Identifiable {
        let id = UUID()
        let title: String
        let text: String
        let button: String
        /// The answer removes something: the button is red
        var destructive = false
        /// More answers, left of the main button
        var others: [String] = []
        /// A text the answer needs (e.g. a new name), edited in the question
        var input: TextInput?
        /// nil = cancelled, 0 = the main button, 1… = `others`
        let answer: (Int?) -> Void
    }

    @Observable final class TextInput {
        var text: String
        init(text: String) { self.text = text }
    }
    var confirmation: Confirmation?

    /// Asks in the open window (menu or app window) and waits for the answer.
    private func confirm(_ title: String, _ text: String, button: String = String(localized: "Continue"), destructive: Bool = false) async -> Bool {
        await choose(title, text, button: button, destructive: destructive) == 0
    }

    /// Asks with several answers: nil = cancelled, 0 = `button`, 1… = `others`
    private func choose(_ title: String, _ text: String, button: String, others: [String] = [], destructive: Bool = false,
                        input: TextInput? = nil) async -> Int? {
        // Without an open window (a package double-clicked in the Finder) the question comes as an alert
        if !NSApp.windows.contains(where: { $0.isVisible && $0.canBecomeKey }) {
            NSApp.activate()
            let alert = NSAlert()
            alert.messageText = title
            alert.informativeText = text
            for b in [button] + others + [String(localized: "Cancel")] { alert.addButton(withTitle: b) }
            alert.buttons.last?.keyEquivalent = "\u{1b}"
            if destructive { alert.buttons.first?.hasDestructiveAction = true }
            let field = input.map { NSTextField(string: $0.text) }
            field?.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
            alert.accessoryView = field
            let i = alert.runModal().rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
            if let field { input?.text = field.stringValue }
            return i <= others.count ? i : nil
        }
        confirmation?.answer(nil)   // a newer question replaces an unanswered one
        return await withCheckedContinuation { cont in
            confirmation = Confirmation(title: title, text: text, button: button, destructive: destructive, others: others,
                                        input: input) { [weak self] i in
                self?.confirmation = nil
                cont.resume(returning: i)
            }
        }
    }

    /// Shows a file picker as a sheet on the key window – the menu window (a separate panel window would close it, plan
    /// B59/B69) or the app window (plan B119). Without a visible window (e.g. called early) it falls back to a modal panel.
    private func present(_ panel: NSOpenPanel, _ done: @escaping (URL) -> Void) {
        NSApp.activate()
        let window = NSApp.keyWindow ?? NSApp.windows.first { $0.isVisible && $0.canBecomeKey }
        if let window {
            panel.beginSheetModal(for: window) { response in
                if response == .OK, let url = panel.url { done(url) }
            }
        } else if panel.runModal() == .OK, let url = panel.url {
            done(url)
        }
    }

    /// Folder picked by the user – the only way omaflake touches protected folders like Documents or Desktop.
    func chooseShareFolder(_ done: @escaping (String) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = String(localized: "Share")
        panel.message = String(localized: "Folder to share with the VM")
        present(panel) { done($0.path) }
    }

    /// A folder of recipes as a source (Settings → Recipe Sources)
    func chooseRecipeFolder(_ done: @escaping (String) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.prompt = String(localized: "Use")
        panel.message = String(localized: "Folder with recipes (recipes/<author>/<kind>/<name>/)")
        present(panel) { done($0.path) }
    }

    // MARK: Packages (plan B108, B109): export, import (always a copy), external VMs

    enum PackageKind: String { case vm, image, recipe }

    /// A folder to export to (an external disk, a NAS, a local folder)
    private func chooseFolder(_ message: String, _ done: @escaping (URL) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = String(localized: "Export Here")
        panel.message = message
        present(panel, done)
    }

    private func export(_ kind: PackageKind, _ name: String, to folder: URL, shown: String) {
        struct Params: Encodable, Sendable { let kind: String; let name: String; let folder: String }
        Task {
            do {
                let job: Job = try await client.call("packages.export", Params(kind: kind.rawValue, name: name, folder: folder.path))
                track(job) { job in
                    guard job.state == .finished else { return self.finished(job) }
                    self.message = String(localized: "\(shown) exported to \(job.result(as: ExportResult.self)?.path ?? folder.path)")
                }
            } catch { message = "\(shown): \(describe(error))" }
        }
    }

    func exportVM(_ name: String) {
        chooseFolder(String(localized: "Where to put “\(name)” – an external disk, a NAS or a folder. It goes along with its user, SSH keys and data: keep it for yourself.")) { folder in
            self.export(.vm, name, to: folder, shown: name)
        }
    }

    func exportImage(_ name: String) {
        chooseFolder(String(localized: "Where to put the image – an external disk, a NAS or a folder. It carries the user it was built for: share recipes instead.")) { folder in
            self.export(.image, name, to: folder, shown: name)
        }
    }

    func exportRecipe(_ id: String) {
        chooseFolder(String(localized: "Where to put the recipe (with the recipes it needs) – to keep or to share")) { folder in
            self.export(.recipe, id, to: folder, shown: RecipeText.qualified(id))
        }
    }

    /// The open panel for packages of one kind: only those are offered, and what is chosen is checked by the core
    /// before the panel closes (a folder that only looks like a package is refused with the reason)
    private func choosePackages(_ kind: PackageKind, external: Bool = false, _ done: @escaping ([URL]) -> Void) {
        let panel = NSOpenPanel()
        let type = UTType(exportedAs: "app.omaflake.\(kind.rawValue)")
        // Recipes also come as plain folders (a checkout, a folder of recipe folders, one recipe folder)
        panel.allowedContentTypes = kind == .recipe ? [type, .folder] : [type]
        panel.canChooseDirectories = kind == .recipe
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = !external
        panel.prompt = external ? String(localized: "Run from There") : String(localized: "Import")
        panel.message = switch kind {
        case .vm: external ? String(localized: "A VM (.omaflake-vm) to run from where it is – nothing is copied")
            : String(localized: "A VM (.omaflake-vm) – it is copied into the data folder")
        case .image: String(localized: "An image (.omaflake-image) – it is copied into the data folder")
        case .recipe: String(localized: "Recipes (.omaflake-recipe) or a folder of recipes – copied into your recipes")
        }
        let checker = PackageChecker(kind: kind, external: external, client: client)
        panel.delegate = checker
        NSApp.activate()
        let finish = { (ok: Bool) in withExtendedLifetime(checker) { if ok { done(panel.urls) } } }
        if let window = NSApp.keyWindow ?? NSApp.windows.first(where: { $0.isVisible && $0.canBecomeKey }) {
            panel.beginSheetModal(for: window) { finish($0 == .OK) }
        } else {
            finish(panel.runModal() == .OK)
        }
    }

    /// The Import button of a tab
    func chooseImport(_ kind: PackageKind) {
        choosePackages(kind) { [weak self] urls in self?.openPackages(urls) }
    }

    /// The VMs tab's second button: a VM that runs from where its package is
    func chooseExternal() {
        choosePackages(.vm, external: true) { [weak self] urls in
            guard let self, let url = urls.first else { return }
            Task { @MainActor in
                struct Inspect: Encodable, Sendable { let path: String; let expect: String; let external: Bool }
                struct Use: Encodable, Sendable { let path: String; let confirmNetwork: Bool }
                let inspection = try? await self.client.call("packages.inspect", Inspect(path: url.path, expect: "vm", external: true),
                                                             as: PackageInspection.self)
                if inspection?.networkVolume == true {
                    guard await self.confirm(String(localized: "Run the VM over the network?"),
                                             String(localized: "\(url.lastPathComponent) is on a network volume. It runs from there, but slower – and it breaks when the connection does. Importing it (a copy into the data folder) avoids that."),
                                             button: String(localized: "Run from There")) else { return }
                }
                self.perform(url.lastPathComponent) { [client = self.client] in
                    let vm: Vm = try await client.call("vms.useExternal", Use(path: url.path, confirmNetwork: true))
                    return String(localized: "\(vm.name) runs from \(url.deletingLastPathComponent().path)")
                }
            }
        }
    }

    /// Stops listing a VM that runs from elsewhere; its package stays where it is
    func forget(_ name: String) {
        perform(name) { [client] in
            let _: Ignored = try await client.call("vms.forget", ["name": name])
            return String(localized: "\(name) is no longer listed – its package stays where it is")
        }
    }

    /// Packages opened in the Finder (double-click, dropped on the app icon) or chosen with an Import button
    func openPackages(_ urls: [URL]) {
        Task { @MainActor in
            for url in urls { await importPackage(url) }
        }
    }

    /// One import: what is here already is asked about first (plan B109) – a VM under another name, the same image or
    /// recipes replaced – otherwise it is copied in without a question
    private func importPackage(_ url: URL) async {
        struct Inspect: Encodable, Sendable { let path: String }
        struct Import: Encodable, Sendable { let path: String; let name: String?; let replace: Bool? }
        let inspection: PackageInspection
        do {
            inspection = try await client.call("packages.inspect", Inspect(path: url.path))
        } catch {
            message = "\(url.lastPathComponent): \(describe(error))"
            return
        }
        guard inspection.valid else {
            message = "\(url.lastPathComponent): \(inspection.reason ?? String(localized: "Not an omaflake package"))"
            return
        }
        var name: String?
        var replace: Bool?
        if let existing = inspection.conflict?.vmName {
            let input = TextInput(text: inspection.conflict?.suggestedName ?? "\(existing)-2")
            guard await choose(String(localized: "“\(existing)” is here already"),
                               String(localized: "You probably have this VM already. Under another name it becomes a second VM with its own identity (SSH port, macOS machine identity); its user, keys and data are the same."),
                               button: String(localized: "Import under This Name"), input: input) == 0 else { return }
            name = input.text.trimmingCharacters(in: .whitespaces)
        } else if let existing = inspection.conflict?.image {
            guard await confirm(String(localized: "The image is here already"),
                                String(localized: "\(existing) is here already – the same name means the same content. Replacing it copies it again."),
                                button: String(localized: "Replace")) else { return }
            replace = true
        }
        do {
            let job: Job = try await client.call("packages.import", Import(path: url.path, name: name, replace: replace))
            track(job)
        } catch {
            message = "\(url.lastPathComponent): \(describe(error))"
        }
    }

    /// What a finished import says
    private func imported(_ job: Job) {
        guard let result = job.result(as: ImportResult.self) else { return }
        let names = result.names
        switch result.kind {
        case .vm: message = String(localized: "VM \(names.joined()) imported")
        case .image: message = String(localized: "Image \(names.joined()) imported")
        case .recipe, .unknown:
            message = names.isEmpty ? String(localized: "These recipes are here already, unchanged – nothing to import")
                : String(localized: "Recipes imported: \(names.map(RecipeText.qualified).joined(separator: ", "))")
        }
    }
}

/// One entry of Recent Activity.
struct Activity: Identifiable, Equatable {
    let id = UUID()
    let date: Date
    let text: String
}

/// Recipe IDs as the app shows them where the core's recipe list has no entry (e.g. a removed recipe):
/// "omaflake.nixos.dev@velvet-tiger-orbit" → name "dev", qualified "omaflake/nixos/dev", words "velvet-tiger-orbit"
enum RecipeText {
    static func base(_ id: String) -> String { String(id.split(separator: "@", maxSplits: 1).first ?? Substring(id)) }
    static func name(_ id: String) -> String { String(base(id).split(separator: ".").last ?? Substring(id)) }
    static func qualified(_ id: String) -> String { base(id).replacingOccurrences(of: ".", with: "/") }
    static func words(_ id: String) -> String? { id.split(separator: "@", maxSplits: 1).dropFirst().first.map(String.init) }
}

/// The first `count` words of an omaflake ID ("vm:research:amber-lake-quiet", 1 → "vm:research:amber")
enum IDText {
    static func short(_ id: String, _ count: Int) -> String {
        guard let cut = id.lastIndex(where: { $0 == ":" || $0 == "@" }) else { return id }
        let words = id[id.index(after: cut)...].split(separator: "-").prefix(max(1, count)).joined(separator: "-")
        return String(id[...cut]) + words
    }
}

/// "12.3 GB"
func gigabytes(_ bytes: Int64) -> String { String(format: "%.1f GB", Double(bytes) / 1e9) }

/// Checks what is chosen in an import panel before the panel closes (plan B109): by content, through the core's
/// `packages.inspect` – the panel stays open with the reason when it is not what the tab imports
final class PackageChecker: NSObject, NSOpenSavePanelDelegate {
    let kind: AppModel.PackageKind
    let external: Bool
    let client: CoreClient

    init(kind: AppModel.PackageKind, external: Bool, client: CoreClient) {
        self.kind = kind
        self.external = external
        self.client = client
    }

    func panel(_ sender: Any, validate url: URL) throws {
        struct Params: Encodable, Sendable { let path: String; let expect: String; let external: Bool }
        let inspection: PackageInspection
        do {
            inspection = try client.callBlocking("packages.inspect", Params(path: url.path, expect: kind.rawValue, external: external),
                                                 as: PackageInspection.self)
        } catch {
            throw NSError(domain: "app.omaflake", code: 1, userInfo: [NSLocalizedDescriptionKey: "\(error)"])
        }
        guard inspection.valid else {
            throw NSError(domain: "app.omaflake", code: 1, userInfo: [
                NSLocalizedDescriptionKey: inspection.reason ?? String(localized: "Not an omaflake package"),
            ])
        }
    }
}
