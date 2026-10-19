// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import SwiftUI
import UniformTypeIdentifiers
import Foundation
import Observation
import ServiceManagement
import OmaflakeCore

/// App state. All knowledge about VMs comes from ~/.omaflake (OmaflakeCore) – the app only holds a
/// snapshot and refreshes it every 2 s. VMs started via the CLI thus appear automatically,
/// and quitting the app leaves running VMs untouched (QEMU runs in its own session).
@MainActor @Observable
final class AppModel {
    struct Row: Identifiable, Equatable {
        var id: String { config.name }
        var config: InstanceConfig
        var status: VMStatus
        var latestImage: String?
        /// macOS guests: NAT address while running
        var address: String?
        /// What the VM's omaflake agent last reported (NixOS guests, spec §11.9)
        var agent: AgentState?
        /// The package's place when it runs from outside the data folder (an external disk, a NAS, a folder; plan B108)
        var location: String?
        var upgradable: Bool { latestImage != nil && latestImage != config.image }
        var isMac: Bool { config.guestKind == .macos }
    }

    /// One recipe in the Images list: its current image (nil = not built yet) and the VMs using any of its images.
    struct ImageRow: Identifiable, Equatable {
        var id: String { recipe }
        /// Recipe ID ("nixos-minimal")
        var recipe: String
        /// Name shown ("minimal")
        var name: String
        /// Author when it is not omaflake's preset (shown next to the name)
        var author: String?
        var kind: GuestKind
        /// Development recipe (only in developer mode), and what the recipe is for
        var lab = false
        var description: String?
        var image: String?
        var instances: [String]
        /// VMs built on the current image (it cannot be removed while there are any)
        var instancesOnCurrent: [String]
        /// The source when another source has a recipe of the same author, kind and name (plan B110)
        var distinction: String?
        /// The line's versions, newest first (plan B111) – shown as sub-rows, also when there is one
        var versions: [RecipeLibrary.Version] = []
        var line: String = ""
    }

    /// One row per recipe line (plan B111) – its newest version names it, built ones first, each kind sorted by name
    var imageRows: [ImageRow] {
        let visible = visibleRecipes
        let byLine = Dictionary(grouping: visible.keys, by: line(of:))
        return byLine.compactMap { line, ids -> ImageRow? in
            let versions = recipeVersions[line] ?? []
            guard let name = versions.first(where: { visible[$0.id] != nil })?.id ?? ids.sorted().first, let d = visible[name] else { return nil }
            return ImageRow(recipe: name, name: RecipeID.displayName(name, d), author: d.author == "omaflake" ? nil : d.author,
                     kind: d.kind, lab: d.isLab, description: d.description(in: Self.uiLanguage), image: currentImages[name],
                     instances: rows.filter { self.line(of: $0.config.recipe) == line }.map(\.id),
                     instancesOnCurrent: rows.filter { $0.config.image == currentImages[name] }.map(\.id),
                     distinction: recipeDistinctions[name], versions: versions, line: line)
        }.sorted { ($0.image == nil ? 1 : 0, $0.kind.rawValue, $0.name) < ($1.image == nil ? 1 : 0, $1.kind.rawValue, $1.name) }
    }

    /// Recipes the app shows: lab recipes only in developer mode
    var visibleRecipes: [String: RecipeDefaults] { developerMode ? recipes : recipes.filter { !$0.value.isLab } }

    /// Developer mode (config.json `developerMode`): shows the lab recipes
    var developerMode: Bool = false {
        didSet {
            guard developerMode != oldValue else { return }
            var s = Settings.load(paths)
            s.developerMode = developerMode ? true : nil
            do { try s.save(paths) } catch { message = "\(error)" }
        }
    }

    // Updates (spec §11.6, plan B68): only a check against the feed for now; installing comes with signed releases
    enum UpdateState: Equatable {
        case noFeed, checking, upToDate(Date), available(UpdateFeed), failed(String), off
    }
    private(set) var updateState: UpdateState = .noFeed

    /// System notifications while the menu window is closed (config.json `notifications`, nil = on)
    /// Keep older versions of a recipe's line (plan B111); off: they go unless an image uses them
    var keepRecipeVersions: Bool = false {
        didSet {
            guard keepRecipeVersions != oldValue else { return }
            var s = Settings.load(paths)
            s.keepRecipeVersions = keepRecipeVersions ? true : nil
            do { try s.save(paths) } catch { message = "\(error)" }
            let paths = self.paths
            Task {
                if !keepRecipeVersions {
                    let removed = await Task.detached { RecipeLibrary(paths: paths).pruneVersions() }.value
                    if !removed.isEmpty { message = String(localized: "Older recipe versions removed: \(removed.count)") }
                }
                await loadRecipes()
            }
        }
    }

    var notifications: Bool = true {
        didSet {
            guard notifications != oldValue else { return }
            var s = Settings.load(paths)
            s.notifications = notifications ? nil : false
            do { try s.save(paths) } catch { message = "\(error)" }
        }
    }

    /// A system notification, if they are on (and the menu window is closed – `Notifier`)
    func notify(_ title: String, _ body: String? = nil) {
        if notifications { Notifier.post(title, body) }
    }

    var checkForUpdates: Bool = true {
        didSet {
            guard checkForUpdates != oldValue else { return }
            var s = Settings.load(paths)
            s.checkForUpdates = checkForUpdates ? nil : false
            do { try s.save(paths) } catch { message = "\(error)" }
            Task { await checkForUpdate() }
        }
    }

    func checkForUpdate() async {
        guard checkForUpdates else { updateState = .off; return }
        guard let feed = UpdateCheck.feedURL() else { updateState = .noFeed; return }
        updateState = .checking
        do {
            if let release = try await UpdateCheck.newer(than: AppVersion.short, feed: feed) {
                updateState = .available(release)
                message = String(localized: "omaflake \(release.version) is available")
            } else {
                updateState = .upToDate(Date())
            }
        } catch {
            updateState = .failed(error.localizedDescription)
        }
    }

    /// The language the app shows ("de", "en") – for recipe descriptions
    static var uiLanguage: String? { Bundle.main.preferredLocalizations.first.map { String($0.prefix(2)) } }

    /// The folder of a recipe – in its local source folder, or where it was downloaded to (plan B107)
    func recipeFile(_ id: String) -> URL? {
        RecipeLibrary(paths: paths).available(Settings.load(paths)).first { $0.id == id }?.folder
    }

    func revealRecipe(_ id: String) {
        if let file = recipeFile(id) { NSWorkspace.shared.activateFileViewerSelecting([file]) }
    }

    /// All versions (images) of a recipe's line, newest first
    func versions(_ recipe: String) -> [ImageStore.Image] { imageVersions[line(of: recipe)] ?? [] }

    /// A recipe's line (plan B110): its source and base – images of a line replace each other
    func line(of recipe: String) -> String {
        if let source = recipeOrigins[recipe] { return RecipeID.lineKey(base: recipe, source: source) }
        return imageVersions.values.lazy.flatMap { $0 }.first { $0.recipe == recipe }?.info.line
            ?? RecipeID.lineKey(base: recipe, source: "")
    }

    /// What tells a recipe apart from others of its author, kind and name ("source", "words" or both), nil if nothing
    func distinction(_ recipe: String) -> String? { recipeDistinctions[recipe] }
    /// Every built image, grouped by recipe (macOS, then NixOS, by name), newest first within a recipe (plan B93)
    var allImages: [ImageStore.Image] {
        imageVersions.values.flatMap { $0 }.sorted {
            ($0.kind.rawValue, RecipeID.name(ofID: $0.recipe), $0.recipe, $1.info.created)
                < ($1.kind.rawValue, RecipeID.name(ofID: $1.recipe), $1.recipe, $0.info.created)
        }
    }
    /// All images per line, newest first – observed state (read from disk in `refresh`), so the version lists
    /// follow a delete at once (plan B62)
    private(set) var imageVersions: [String: [ImageStore.Image]] = [:]

    /// Deletes an image (refused while VMs use it).
    func removeImage(_ name: String) {
        Task { @MainActor in
            guard await confirm(String(localized: "Remove the image “\(name)”?"),
                          String(localized: "It is deleted and cannot be restored. VMs that use it must be removed first."),
                          button: String(localized: "Remove"), destructive: true) else { return }
            let paths = self.paths
            Task {
                do {
                    try await Task.detached {
                        let backends = try Backends(paths: paths)
                        try ImageStore(paths: paths).remove(name, instances: backends.store)
                    }.value
                    message = String(localized: "Image \(name) deleted")
                } catch {
                    message = error.localizedDescription
                }
                refresh()
                await loadRecipes()   // its recipe version may have gone (plan B111)
            }
        }
    }

    /// The image the New VM page creates from – set by an image row, the only way to it (plan B100)
    var newVMImage: String?
    /// The recipe the Build Image page builds – set by a recipe row or a build row, the only ways to it (plan B100)
    var buildRecipe: String?

    private(set) var rows: [Row] = []
    private(set) var recipes: [String: RecipeDefaults] = [:]
    private(set) var currentImages: [String: String] = [:]
    /// A macOS base exists (needed to build macOS images)
    private(set) var hasMacBase = false
    /// Guest user of the macOS base (images are built logging in as this user)
    var macBaseUser: String? { macosPreflight.first { $0.item == .macBase }?.baseUser }
    private(set) var busy: Set<String> = []
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
        var isActive: Bool { status != nil || kind != nil || tag != nil || recipe != nil }
    }
    var filters: [HomeTab: ListFilter] = [:]

    var tab: HomeTab = .vms {
        didSet {
            UserDefaults.standard.set(tab.rawValue, forKey: "homeTab")
            if tab == .activity { unseenActivity = false }
        }
    }
    // Navigation inside the menu window: the open pages, the last one is shown; empty = the main page
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
    private(set) var qemuHasGL = false

    // Image builds: a queue, one build per guest kind at a time – NixOS and macOS run side by side (plan B60)
    private(set) var builds: [BuildJob] = []

    /// The recipe's waiting or running build
    func activeJob(for recipe: String) -> BuildJob? { builds.last { $0.recipe == recipe && $0.isActive } }
    /// The recipe's latest build (also a finished one, for its log)
    func latestJob(for recipe: String) -> BuildJob? { builds.last { $0.recipe == recipe } }
    var anyBuildRunning: Bool { builds.contains { $0.isActive } }

    // macOS base installation (omaf macos install)
    private(set) var installLog: [String] = []
    private(set) var installing = false

    private(set) var paths = Paths.standard()
    private(set) var movingData = false

    init() {
        // Tooltips almost at once instead of after about a second (AppKit's NSInitialToolTipDelay, this app only)
        UserDefaults.standard.register(defaults: ["NSInitialToolTipDelay": 150])
        developerMode = Settings.load(paths).developerMode ?? false
        checkForUpdates = Settings.load(paths).checkForUpdates ?? true
        notifications = Settings.load(paths).notifications ?? true
        keepRecipeVersions = Settings.load(paths).keepRecipeVersions ?? false
        // Older layouts (before image.json, before packages – plan B108); first, so the lists below see the result
        ImageStore(paths: paths).migrateLegacy()
        // The tab used last; the very first time (no VM yet) the recipes, where one starts
        tab = UserDefaults.standard.string(forKey: "homeTab").flatMap(HomeTab.init(rawValue:))
            ?? (((try? Backends(paths: paths))?.store.list().isEmpty ?? true) ? .recipes : .vms)
        // At launch, then once a day
        Task {
            while !Task.isCancelled {
                await checkForUpdate()
                try? await Task.sleep(for: .seconds(24 * 3600))
            }
        }
        // The first time, the menu window opens on the Doctor
        if Self.firstLaunch { path = [.doctor] }
        Task { await loadRecipes() }
        // Packages opened in the Finder before the model existed, and from now on
        AppDelegate.open = { [weak self] urls in self?.openPackages(urls) }
        if !AppDelegate.waiting.isEmpty {
            openPackages(AppDelegate.waiting)
            AppDelegate.waiting = []
        }
        // The Doctor's checks run at start, when the Doctor opens, on "Check Again", after every action and right
        // before a build – not while idle (nothing changes there that a check would need to catch)
        Task { await checkAll() }
        Task {
            while !Task.isCancelled {
                refresh()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    // MARK: Reading

    func refresh() {
        defer {
            // Word counts follow images and VMs coming and going (recipe versions: loadRecipes)
            let lengths = OmaflakeID.Lengths(recipes: recipeVersions.values.flatMap { $0 }.map(\.id),
                                             images: imageVersions.values.flatMap { $0 }.map(\.name), vms: rows.map(\.config))
            if lengths != idLengths { idLengths = lengths }
        }
        guard let backends = try? Backends(paths: paths) else { return }
        let store = backends.store
        let images = ImageStore(paths: paths)
        var current: [String: String] = [:]
        let versions = Dictionary(grouping: images.list(), by: \.info.line)
            .mapValues { $0.sorted { $0.info.created > $1.info.created } }
        if versions != imageVersions { imageVersions = versions }
        // A recipe's image is the newest of its line: there is no default image to pick any more – a VM is created from
        // the image it is started from (plan B100); "upgrade" moves a VM to the newest image of its recipe's line (B110)
        for recipe in Set(store.list().map(\.recipe)).union(recipes.keys) {
            current[recipe] = versions[line(of: recipe)]?.first?.name
        }
        if current != currentImages { currentImages = current }
        hasMacBase = MacImageBuilder.newestBase(paths) != nil
        let vz = try? backends.vz
        let new = store.list().map { c in
            let backend = try? backends.backend(for: c)
            let status = backend?.status(c) ?? .stopped
            let address = c.guestKind == .macos && status == .running ? vz?.address(c) : nil
            let agent = status == .running ? backend?.agentState(c) : nil
            let location = store.isExternal(c.name) ? paths.instance(c.name).dir.path : nil
            return Row(config: c, status: status, latestImage: current[c.recipe], address: address, agent: agent, location: location)
        }
        // A VM that ended on its own (shut down inside, crashed, stopped with the CLI): message and notification
        for old in rows where old.status == .running {
            guard let now = new.first(where: { $0.id == old.id }), now.status == .stopped else { continue }
            if stopRequested.remove(old.id) == nil {
                message = String(localized: "\(old.id) has ended")
                notify(String(localized: "VM ended"), String(localized: "\(old.id) is no longer running."))
            }
        }
        for name in stopRequested where new.first(where: { $0.id == name })?.status != .running { stopRequested.remove(name) }
        if new != rows { rows = new }
        ensureClipboardAgents()
        reloadRecipesIfChanged()
    }

    /// The recipe list and the images' freshness follow edits of the recipes: every refresh compares the newest
    /// modification date in the flake folder and the guest settings; `nix eval` runs only after a change (plan B73, B82)
    private var flakeStamp: Date?
    private var flakeStampChecked = Date.distantPast
    private func reloadRecipesIfChanged() {
        guard Date().timeIntervalSince(flakeStampChecked) > 5, let flake = try? Settings.load(paths).flakePath() else { return }
        flakeStampChecked = Date()
        // The flake, the guest values, the downloaded recipes and the enabled local recipe folders (plan B107)
        let folders = Settings.load(paths).recipeSources.filter { $0.isEnabled && $0.kind == .folder }
            .map { URL(fileURLWithPath: $0.location) }
        let stamp = Self.newestChange(in: [URL(fileURLWithPath: flake), paths.guest, paths.recipes] + folders)
        guard stamp != flakeStamp else { return }
        let first = flakeStamp == nil
        flakeStamp = stamp
        Task {
            if !first { await loadRecipes() }
            await checkFreshness()
        }
    }

    static func newestChange(in folders: [URL]) -> Date {
        var newest = Date.distantPast
        for folder in folders {
            guard let e = FileManager.default.enumerator(at: folder, includingPropertiesForKeys: [.contentModificationDateKey],
                                                         options: [.skipsHiddenFiles]) else { continue }
            for case let url as URL in e {
                if url.lastPathComponent.hasPrefix("result") { e.skipDescendants(); continue }
                if let d = try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate, d > newest { newest = d }
            }
        }
        return newest
    }

    /// Store path a build would give now, per recipe with images – a version with another one is older than its recipe
    private(set) var expectedStorePaths: [String: String] = [:]
    private(set) var checkingFreshness = false
    private var freshnessAgain = false

    func checkFreshness() async {
        guard !checkingFreshness else { freshnessAgain = true; return }
        checkingFreshness = true
        let paths = self.paths, recipes = self.recipes
        let result = await Task.detached { () -> [String: String] in
            guard let flake = try? Settings.load(paths).flakePath() else { return [:] }
            return ImageStore(paths: paths).expectedStorePaths(recipes, flake: flake)
        }.value
        expectedStorePaths = result
        checkingFreshness = false
        if freshnessAgain { freshnessAgain = false; await checkFreshness() }
    }

    /// The recipe changed since its current image was built
    func isOutdated(_ recipe: String) -> Bool {
        guard let expected = expectedStorePaths[recipe], let current = versions(recipe).first(where: { $0.name == currentImages[recipe] })
        else { return false }
        return current.info.storePath != expected
    }

    /// macOS guests get their clipboard agent over SSH once they answer (darwin-guest.nix). Checked at every refresh, so
    /// it also comes after an app restart or a first attempt that found no SSH yet (plan B64). Remembered per runner
    /// process: a restarted VM gets it again.
    private var clipboardAgentStarted: [String: Int32] = [:]
    private var clipboardAgentPending: Set<String> = []

    private func ensureClipboardAgents() {
        let paths = self.paths
        for row in rows where row.isMac && row.status == .running && row.config.clipboardOn && row.config.network != .none {
            let name = row.id
            guard !clipboardAgentPending.contains(name) else { continue }
            clipboardAgentPending.insert(name)
            let known = clipboardAgentStarted[name]
            Task {
                let result = await Task.detached { () -> Int32? in
                    guard let vz = try? Backends(paths: paths).vz, let c = try? vz.store.load(name),
                          let pid = vz.pid(c) else { return nil }
                    if pid == known { return pid }   // this run has it already
                    guard let host = vz.address(c), Ports.sshBanner(host: host, port: 22) != nil,
                          vz.startClipboardAgent(c, host: host) else { return nil }
                    return pid
                }.value
                if let result { clipboardAgentStarted[name] = result }
                clipboardAgentPending.remove(name)
            }
        }
    }

    func loadRecipes() async {
        let paths = self.paths
        do {
            let result = try await Task.detached {
                let settings = Settings.load(paths)
                let flake = try settings.flakePath()
                // Without QEMU, macOS guests still work; NixOS starts then fail with the backend's message
                let gl = (try? QEMURuntime.locate(paths).0.capabilities().gl) ?? false
                let library = RecipeLibrary(paths: paths)
                let origins = Dictionary(library.available(settings).map { ($0.id, $0.source) }, uniquingKeysWith: { a, _ in a })
                return (try ImageStore(paths: paths).recipeDefaults(flake: flake), gl, origins, library.installed(),
                        library.distinctions(settings), library.versions(settings), OmaflakeID.Lengths.current(paths))
            }.value
            recipes = result.0
            qemuHasGL = result.1
            recipeOrigins = result.2
            installedRecipes = result.3
            recipeDistinctions = result.4
            recipeVersions = result.5
            idLengths = result.6
        } catch {
            message = "\(error)"
        }
    }

    // MARK: Recipe sources (spec §11.3, plan B107)

    /// The sources in their order (config.json; none set = omaflake/recipes)
    var sources: [RecipeSource] { Settings.load(paths).recipeSources }
    /// Where each available recipe comes from: RecipeSource.id
    private(set) var recipeOrigins: [String: String] = [:]
    /// What tells recipes of the same author, kind and name apart (plan B110)
    private(set) var recipeDistinctions: [String: String] = [:]
    /// Every line's versions (plan B111)
    private(set) var recipeVersions: [String: [RecipeLibrary.Version]] = [:]
    /// How many words omaflake IDs show, per kind (plan B111)
    private(set) var idLengths = OmaflakeID.Lengths()
    /// Recipes downloaded from remote sources
    private(set) var installedRecipes: [String: RecipeLibrary.Installed] = [:]
    /// The remote sources' catalogs, fetched only when the Recipes tab asks (never in the background)
    private(set) var catalogs: [String: RecipeCatalog] = [:]
    private(set) var catalogErrors: [String: String] = [:]
    private(set) var loadingCatalogs = false
    private(set) var installingRecipes: Set<String> = []

    func source(_ id: String) -> RecipeSource? { sources.first { $0.id == id } }

    /// Fetches the enabled remote sources' catalogs (opening the Recipes tab, or its refresh button)
    func refreshCatalogs() {
        guard !loadingCatalogs else { return }
        let remote = sources.filter { $0.isEnabled && $0.kind != .folder }
        guard !remote.isEmpty else { catalogs = [:]; catalogErrors = [:]; return }
        loadingCatalogs = true
        Task {
            var got: [String: RecipeCatalog] = [:], errors: [String: String] = [:]
            for s in remote {
                do { got[s.id] = try await SourceClient(s).catalog() } catch { errors[s.id] = error.localizedDescription }
            }
            catalogs = got
            catalogErrors = errors
            loadingCatalogs = false
        }
    }

    /// Recipes the remote catalogs offer that are not available yet (first source wins), and the newest catalog entry
    /// of installed ones (for updates)
    var offeredRecipes: [(recipe: RecipeCatalog.Recipe, source: RecipeSource)] {
        // By ID (author, kind, name and content): what is here already is not offered, also not from another source
        var seen = Set(recipes.keys)
        // An older version of a recipe of this source is updated in its details, not offered again
        let lines = Set(recipeOrigins.map { RecipeID.lineKey(base: $0.key, source: $0.value) })
        var out: [(RecipeCatalog.Recipe, RecipeSource)] = []
        for s in sources where s.isEnabled && s.kind != .folder {
            for r in catalogs[s.id]?.recipes ?? [] where developerMode || !(r.lab ?? false) {
                guard !lines.contains(RecipeID.lineKey(base: r.id, source: s.id)),
                      seen.insert(RecipeID.make(base: r.id, contentHash: r.contentHash)).inserted else { continue }
                out.append((r, s))
            }
        }
        return out
    }

    /// The catalog entry of an installed recipe when its source has a newer version
    func update(for id: String) -> (recipe: RecipeCatalog.Recipe, source: RecipeSource)? {
        guard let entry = installedRecipes[id], let s = source(entry.source),
              let r = catalogs[s.id]?.recipes.first(where: { $0.id == RecipeID.base(id) }), r.version != entry.version else { return nil }
        return (r, s)
    }

    /// Downloads a recipe (with what it requires) from its source; also updates
    func installRecipe(_ id: String, from source: RecipeSource) {
        guard let catalog = catalogs[source.id], !installingRecipes.contains(id) else { return }
        installingRecipes.insert(id)
        let paths = self.paths
        Task {
            do {
                let written = try await RecipeLibrary(paths: paths).install(id, catalog: catalog, client: SourceClient(source))
                message = written.isEmpty ? String(localized: "\(RecipeID.qualified(id)) is up to date")
                    : String(localized: "Installed \(written.map(RecipeID.qualified).joined(separator: ", "))")
            } catch { message = error.localizedDescription }
            installingRecipes.remove(id)
            await loadRecipes()
            await checkFreshness()
        }
    }

    /// The official recipes (by omaflake from omaflake/recipes, no lab ones) – suggested at the first start
    var suggestedRecipes: [(recipe: RecipeCatalog.Recipe, source: RecipeSource)] {
        offeredRecipes.filter { $0.source.isOfficial && $0.recipe.author == "omaflake" && !($0.recipe.lab ?? false) }
    }

    func installSuggested() {
        for (r, s) in suggestedRecipes { installRecipe(r.id, from: s) }
    }

    /// Removes every version of a line that can go (plan B111)
    func removeAllVersions(_ line: String, name: String) {
        Task { @MainActor in
            guard await confirm(String(localized: "Remove all versions of “\(name)”?"),
                                String(localized: "Versions images are built from stay while those images are there; a recipe in a local folder stays in its folder. Images and VMs stay."),
                                button: String(localized: "Remove All"), destructive: true) else { return }
            let paths = self.paths
            let (removed, kept) = await Task.detached { RecipeLibrary(paths: paths).removeLine(line) }.value
            message = kept.isEmpty ? String(localized: "\(name): \(removed.count) versions removed")
                : String(localized: "\(name): \(removed.count) versions removed, \(kept.count) kept")
            await loadRecipes()
        }
    }

    func removeRecipe(_ id: String) {
        Task { @MainActor in
            let label = OmaflakeID.recipe(id, words: idLengths.recipe)
            guard await confirm(String(localized: "Remove the recipe version “\(label)”?"),
                                String(localized: "Its files are deleted; images built from it and their VMs stay. You can install it again from its source."),
                                button: String(localized: "Remove"), destructive: true) else { return }
            do {
                try RecipeLibrary(paths: paths).remove(id)
                message = String(localized: "\(label) removed")
            } catch { message = error.localizedDescription }
            await loadRecipes()
        }
    }

    // Sources: changed in Settings; the recipe list follows

    private func saveSources(_ list: [RecipeSource]) {
        var s = Settings.load(paths)
        s.sources = list
        do { try s.save(paths) } catch { message = "\(error)" }
        Task { await loadRecipes(); refreshCatalogs() }
    }

    /// Checks a new source (with its token for a private repository) and keeps it; the result or the error comes back
    func addSource(_ input: String, isPrivate: Bool, token: String) async -> Result<String, VMError> {
        do {
            let s = try RecipeSource.parse(input, isPrivate: isPrivate)
            guard !sources.contains(where: { $0.id == s.id }) else {
                return .failure(VMError(String(localized: "\(s.displayName) is already a source")))
            }
            let note = try await SourceClient(s, token: s.isPrivateRepository ? token : nil).verify()
            if s.isPrivateRepository { try SourceTokens.set(token, for: s) }
            saveSources(sources + [s])
            return .success("\(s.displayName): \(note)")
        } catch let e as VMError { return .failure(e) } catch { return .failure(VMError(error.localizedDescription)) }
    }

    func setSource(_ id: String, enabled: Bool) {
        saveSources(sources.map { var s = $0; if s.id == id { s.enabled = enabled ? nil : false }; return s })
    }

    func moveSource(_ id: String, up: Bool) {
        var list = sources
        guard let i = list.firstIndex(where: { $0.id == id }) else { return }
        let j = up ? i - 1 : i + 1
        guard list.indices.contains(j) else { return }
        list.swapAt(i, j)
        saveSources(list)
    }

    func removeSource(_ id: String) {
        Task { @MainActor in
            guard let s = source(id), await confirm(String(localized: "Remove the source “\(s.displayName)”?"),
                String(localized: "Recipes installed from it stay until you remove them; its token goes from the Keychain."),
                button: String(localized: "Remove"), destructive: true) else { return }
            try? SourceTokens.set(nil, for: s)
            saveSources(sources.filter { $0.id != id })
        }
    }

    // MARK: Actions (blocking OmaflakeCore calls in the background)

    /// Runs `work` for an instance in the background; errors end up in `message`.
    /// `locked`: hold the instance lock during `work` – only for existing instances, the lock file lives in their folder.
    private func perform(_ name: String, locked: Bool = true, _ work: @escaping @Sendable (Backends, Paths) throws -> String?) {
        guard !busy.contains(name) else { return }
        busy.insert(name)
        let paths = self.paths
        Task {
            do {
                let note = try await Task.detached {
                    let backends = try Backends(paths: paths)
                    let lock = locked ? try InstanceLock(paths.instance(name)) : nil
                    defer { withExtendedLifetime(lock) {} }
                    return try work(backends, paths)
                }.value
                if let note { message = note }
            } catch {
                message = "\(name): \(error)"
            }
            busy.remove(name)
            refresh()
            await checkAll()
        }
    }

    /// `perform` for an existing instance, with the backend of its guest kind.
    private func performOn(_ name: String,
                           _ work: @escaping @Sendable (any VMBackend, InstanceConfig, Paths) throws -> String?) {
        perform(name) { backends, paths in
            let c = try backends.store.load(name)
            return try work(try backends.backend(for: c), c, paths)
        }
    }

    /// Running macOS guests; Virtualization.framework allows `VZBackend.maxRunning` (plan BM16).
    var runningMacs: [String] { rows.filter { $0.isMac && $0.status == .running }.map(\.id) }

    /// Why a macOS guest cannot start right now (shown before starting), nil if it can.
    var macLimitReason: String? {
        runningMacs.count < VZBackend.maxRunning ? nil
            : String(localized: "macOS allows at most \(VZBackend.maxRunning) macOS VMs at the same time – running: \(runningMacs.joined(separator: ", "))")
    }

    func canStart(_ row: Row) -> Bool {
        row.status == .stopped && (!row.isMac || macLimitReason == nil) && startBlockers(row.config.guestKind).isEmpty
    }

    func start(_ name: String, window: Bool) {
        if rows.first(where: { $0.id == name })?.isMac == true, let reason = macLimitReason {
            message = reason
            return
        }
        let gl = qemuHasGL
        performOn(name) { backend, c, _ in
            let display: DisplayMode = !window ? .none
                : c.guestKind == .nixos && gl && c.display != .window ? .gpu : .window
            _ = try backend.start(c, options: StartOptions(display: display))
            return nil
        }
        whenReachable(name)
    }

    /// Once the VM's agent says hello (NixOS, spec §11.9) or SSH answers, the VM is ready: a message and a notification
    /// ("VM ready", plan B91). macOS guests are also named after the instance then (VZBackend.waitForSSH, plan BM28).
    /// Runs without the instance lock, so the instance stays usable meanwhile. VMs without network and without an
    /// agent can't be reached – no message.
    private func whenReachable(_ name: String) {
        let paths = self.paths
        Task.detached {
            // Creating (clone, overlay) comes before the start: wait for the VM to run, up to two minutes
            guard let backends = try? Backends(paths: paths) else { return }
            var running: (InstanceConfig, any VMBackend)?
            for _ in 0..<60 {
                try? await Task.sleep(for: .seconds(2))
                if let c = try? backends.store.load(name), let backend = try? backends.backend(for: c), backend.status(c) == .running {
                    running = (c, backend)
                    break
                }
            }
            guard let (c, backend) = running, c.network != .none || c.guestKind == .nixos else { return }
            guard let ready = backend.waitForReady(c, timeout: 300), backend.status(c) == .running else { return }
            await MainActor.run {
                self.message = String(localized: "\(name) is ready")
                self.notify(String(localized: "VM ready"), ready == .ssh
                            ? String(localized: "\(name) is running and answers SSH.")
                            : String(localized: "\(name) is running; its omaflake agent reported in."))
            }
        }
    }

    /// VMs the app was asked to shut down – their end is no news
    private var stopRequested: Set<String> = []

    func stop(_ name: String) {
        stopRequested.insert(name)
        performOn(name) { backend, c, _ in "\(name): \(try backend.stop(c, timeout: 60).rawValue)" }
    }

    func reset(_ name: String) {
        Task { @MainActor in
            guard await confirm(String(localized: "Reset “\(name)”?"), String(localized: "The current state of the VM is deleted and cannot be restored.")) else { return }
            performOn(name) { backend, c, _ in
                guard backend.status(c) == .stopped else { throw VMError(String(localized: "running – stop it first")) }
                try backend.reset(c)
                return String(localized: "\(name) reset")
            }
        }
    }

    func upgrade(_ name: String) {
        Task { @MainActor in
            guard let row = rows.first(where: { $0.id == name }), let latest = row.latestImage else { return }
            guard await confirm(String(localized: "Upgrade “\(name)” to \(latest)?"), String(localized: "The current state is deleted and cannot be restored; settings are kept.")) else { return }
            performOn(name) { backend, c, paths in
                guard backend.status(c) == .stopped else { throw VMError(String(localized: "running – stop it first")) }
                // The newest image of its recipe – the one the row offered
                let image = try ImageStore(paths: paths).image(named: latest)
                try backend.upgrade(c, to: image)
                return String(localized: "\(name) now uses \(image.name)")
            }
        }
    }

    func remove(_ name: String) {
        Task { @MainActor in
            let location = rows.first(where: { $0.id == name })?.location
            let text = location.map { String(localized: "The VM runs from \($0) – it is deleted there and cannot be restored. Its share folder is kept if it holds files. To keep it there, choose Stop Listing instead.") }
                ?? String(localized: "The VM and its disk are deleted and cannot be restored. Its share folder is kept if it holds files.")
            guard await confirm(String(localized: "Remove “\(name)”?"), text, button: String(localized: "Remove"), destructive: true) else { return }
            performOn(name) { backend, c, _ in
                guard backend.status(c) == .stopped else { throw VMError(String(localized: "running – stop it first")) }
                try backend.remove(c)
                return String(localized: "\(name) deleted")
            }
        }
    }

    /// `sharePath`: a folder the user picked in the open panel (nil = the default share folder).
    /// Creates a VM from exactly this image (not "the recipe's current one", plan B100).
    func create(name: String, image imageName: String, settings: RecipeDefaults, sharePath: String? = nil, startAfter: Bool) {
        let startAfter = startAfter && (settings.kind == .nixos || macLimitReason == nil)
        // No lock yet: the instance folder (and its lock file) is created by `create`
        perform(name, locked: false) { backends, paths in
            let backend = try backends.backend(for: settings.kind)
            let image = try ImageStore(paths: paths).image(named: imageName)
            var c = try backend.create(name: name, recipe: image.recipe, image: image, settings: settings)
            if settings.share, let sharePath {
                c.sharePath = sharePath
                try backends.store.save(c)
            }
            let gl = settings.kind == .nixos && (try? backends.qemu.runtime.capabilities().gl) == true
            if startAfter {
                let lock = try InstanceLock(paths.instance(name))
                defer { withExtendedLifetime(lock) {} }
                _ = try backend.start(c, options: StartOptions(display: gl ? .gpu : .window))
            }
            return c.guestKind == .macos ? String(localized: "\(name) created") : String(localized: "\(name) created (SSH port \(c.sshPort))")
        }
        if startAfter { whenReachable(name) }
    }

    /// Sound, microphone, clipboard and camera of an instance; they take effect at its next start.
    func setSound(_ name: String, audio: Bool? = nil, microphone: Bool? = nil, clipboard: Bool? = nil, camera: Bool? = nil) {
        perform(name) { backends, _ in
            var c = try backends.store.load(name)
            if let audio { c.audio = audio }
            if let microphone { c.microphone = microphone }
            if let clipboard { c.clipboard = clipboard }
            if let camera { c.camera = camera }
            try backends.store.save(c)
            let running = (try? backends.backend(for: c))?.status(c) == .running
            return running ? String(localized: "\(name): takes effect at the next start") : nil
        }
    }

    /// Terminal.app with `ssh` to the instance (asks for automation permission the first time).
    func openTerminal(_ name: String) {
        guard let c = rows.first(where: { $0.id == name })?.config,
              let backend = try? Backends(paths: paths).backend(for: c) else { return }
        runInTerminal(SSHCommand.shellLine(backend.sshArguments(c, command: [])))
    }

    /// Types `line` into a new Terminal window and runs it – the user sees what runs and answers prompts there.
    func runInTerminal(_ line: String) {
        let escaped = line.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let script = "tell application \"Terminal\"\nactivate\ndo script \"\(escaped)\"\nend tell"
        var error: NSDictionary?
        NSAppleScript(source: script)?.executeAndReturnError(&error)
        if let error { message = "Terminal: \(error[NSAppleScript.errorMessage] ?? error)" }
    }

    // MARK: Requirements (spec §11.2)

    private(set) var prerequisites: [Prerequisite] = []
    private(set) var checkingPrerequisites = false

    /// Something every feature needs is missing
    var requirementsMissing: Bool { prerequisites.contains { $0.required && $0.status == .missing } }

    // Preflight for building images (Doctor)
    private(set) var nixosPreflight: [Prerequisite] = []
    private(set) var macosPreflight: [Prerequisite] = []
    private(set) var latestRelease: MacOSRelease?

    /// Items in the Doctor that need attention (warning sign on its icon)
    var doctorIssues: Int {
        (prerequisites + nixosPreflight + macosPreflight).filter {
            ($0.status == .missing && $0.item != .git) || ($0.item == .macBase && $0.status == .limited)
        }.count
    }

    /// What the Doctor reports against building images of a guest kind; empty = building is possible.
    /// Before the first check has finished nothing is blocked.
    func buildBlockers(_ kind: GuestKind) -> [Prerequisite] {
        let needed: Set<Prerequisite.Item> = kind == .nixos
            ? [.nix, .flakes, .nixDarwin, .linuxBuilder, .flake, .builderRunning, .flakeEval, .nixSpace]
            : [.host, .nix, .flakes, .flake, .sshKey, .flakeEval, .macBase, .macSpace]
        return (prerequisites + nixosPreflight + macosPreflight).filter { p in
            needed.contains(p.item) && (p.status == .missing || (p.item == .macBase && p.status != .ok))
        }
    }

    /// What blocks starting VMs of a guest kind (NixOS needs QEMU; macOS has its own runner)
    func startBlockers(_ kind: GuestKind) -> [Prerequisite] {
        kind == .nixos ? prerequisites.filter { $0.item == .qemu && $0.status == .missing } : []
    }

    func checkAll() async {
        checkingPrerequisites = true
        let paths = self.paths
        let result = await Task.detached { () -> ([Prerequisite], [Prerequisite], [Prerequisite], MacOSRelease?) in
            let pre = Prerequisites(paths: paths)
            let latest = (try? VZBackend.locateRunner(paths)).flatMap { MacOSRelease.latest(runner: $0) }
            return (pre.check(), pre.nixosPreflight(), pre.macosPreflight(latest: latest), latest)
        }.value
        (prerequisites, nixosPreflight, macosPreflight, latestRelease) = result
        checkingPrerequisites = false
        // The recipes may have changed too (e.g. macUser in the flake's settings)
        await loadRecipes()
    }

    // IPSW download (part of the macOS preflight)
    private(set) var downloading = false
    private(set) var downloadProgress: (done: Int64, total: Int64) = (0, 0)
    private var downloader: IPSWDownloader?

    func downloadIPSW() {
        guard !downloading, let release = latestRelease else { return }
        downloading = true
        let paths = self.paths
        let model = self
        Task {
            do {
                try await Task.detached {
                    let installer = MacBaseInstaller(paths: paths, vz: try Backends(paths: paths).vz)
                    try installer.downloadIPSW(release, started: { d in Task { @MainActor in model.downloader = d } },
                                               progress: { done, total in Task { @MainActor in model.downloadProgress = (done, total) } },
                                               log: { _ in })
                }.value
                message = String(localized: "macOS \(release.version) downloaded")
            } catch {
                message = error.localizedDescription
            }
            downloading = false
            downloader = nil
            await checkAll()
        }
    }

    // Linux builder maintenance (needs admin rights: macOS' administrator dialog)
    private(set) var builderTask: String?

    /// Applies the image flake's builder host (`darwin-rebuild switch …#linux-builder`).
    func applyBuilderSettings(confirmed: Bool = false) {
        Task { @MainActor in
            let maintenance = BuilderMaintenance(paths: paths)
            let replaces = !maintenance.systemIsOmaflakes && !confirmed
            let text = replaces
                ? String(localized: "This Mac's nix-darwin system does not come from omaflake's image flake yet. Applying replaces your current nix-darwin configuration with omaflake's Linux builder host (nix/hosts/linux-builder.nix and your local/darwin.nix). Only continue if you set up nix-darwin for omaflake.")
                : String(localized: "Runs darwin-rebuild with omaflake's Linux builder host. This takes a few minutes; macOS asks for an administrator.")
            if !confirmed {
                guard await confirm(String(localized: "Apply Linux builder settings?"), text,
                                    button: replaces ? String(localized: "Replace and Apply") : String(localized: "Apply")) else { return }
            }
            guard builderTask == nil else { return }
            builderTask = String(localized: "Building Linux builder settings …")
            Task {
                do {
                    // Build as the user (root may not read the flake in ~/Documents), activate as root
                    let system = try await Task.detached { try maintenance.buildSystem() }.value
                    builderTask = nil
                    runAsAdmin(String(localized: "Applying Linux builder settings …"),
                               prompt: String(localized: "omaflake wants to apply the Linux builder settings (darwin-rebuild)."),
                               done: String(localized: "Linux builder settings applied"), waitForBuilder: true) {
                        BuilderMaintenance.activateCommand(system: system)
                    }
                } catch {
                    builderTask = nil
                    message = error.localizedDescription
                }
            }
        }
    }

    // Setup by click (spec §11.5): Nix, then nix-darwin with the Linux builder

    /// Installs Nix with the official installer through the administrator dialog.
    func installNix() {
        Task { @MainActor in
            guard await confirm(String(localized: "Install Nix?"),
                          String(localized: "omaflake installs Nix with the official installer of the NixOS community (nix-installer). It creates the /nix volume, Nix's build users and its background service, and needs the internet. Uninstall later with /nix/nix-installer uninstall."),
                          button: String(localized: "Install")) else { return }
            runAsAdmin(String(localized: "Installing Nix …"),
                       prompt: String(localized: "omaflake wants to install Nix."),
                       done: String(localized: "Nix installed")) { BuilderMaintenance.installNixCommand }
        }
    }

    /// nix-darwin and the Linux builder in two stages: the default builder from the cache, then omaflake's settings
    /// (each asks for an administrator once).
    func setUpNixDarwin() {
        Task { @MainActor in
            guard builderTask == nil, await confirm(String(localized: "Set up nix-darwin and the Linux builder?"),
                          String(localized: "omaflake sets up nix-darwin with its Linux builder host (nix/hosts/linux-builder.nix) – needed to build NixOS images. nix-darwin then manages /etc/nix/nix.conf; the files there that the Nix installer wrote are kept as *.before-nix-darwin. First the standard builder, then omaflake's settings; macOS asks for an administrator twice. This takes several minutes."),
                          button: String(localized: "Set Up")) else { return }
            let maintenance = BuilderMaintenance(paths: paths)
            builderTask = String(localized: "Building nix-darwin (stage 1 of 2) …")
            Task {
                do {
                    let first = try await Task.detached { try maintenance.buildSystem(host: "linux-builder-bootstrap") }.value
                    builderTask = nil
                    runAsAdmin(String(localized: "Activating nix-darwin (stage 1 of 2) …"),
                               prompt: String(localized: "omaflake wants to set up nix-darwin and the Linux builder."),
                               done: String(localized: "nix-darwin set up – applying omaflake's builder settings next"),
                               waitForBuilder: true, then: { self.applyBuilderSettings(confirmed: true) }) {
                        BuilderMaintenance.firstActivationCommand(system: first)
                    }
                } catch {
                    builderTask = nil
                    message = error.localizedDescription
                }
            }
        }
    }

    /// Removes unused store paths in the builder.
    func cleanUpBuilder() {
        runAsAdmin(String(localized: "Cleaning up the Linux builder …"),
                   prompt: String(localized: "omaflake wants to clean up the Linux builder's disk."),
                   done: nil) { BuilderMaintenance.cleanUpCommand }
    }

    private func runAsAdmin(_ title: String, prompt: String, done: String?, waitForBuilder: Bool = false,
                            then next: (@MainActor () -> Void)? = nil,
                            _ command: @escaping @Sendable () throws -> String) {
        guard builderTask == nil else { return }
        builderTask = title
        let maintenance = BuilderMaintenance(paths: paths)
        Task {
            var succeeded = false
            do {
                let output = try await Task.detached {
                    try AdminShell.run(try maintenance.rootScript(try command()), prompt: prompt)
                }.value
                succeeded = true
                if waitForBuilder {
                    // Activation restarts the builder VM; check again only once it answers (up to 2 minutes)
                    builderTask = String(localized: "Waiting for the Linux builder …")
                    let paths = self.paths
                    await Task.detached {
                        let pre = Prerequisites(paths: paths)
                        for _ in 0..<60 where pre.builderRunning().status != .ok { usleep(2_000_000) }
                    }.value
                }
                // Clean-up: "before" and "after" lines of df (size, used, avail, …)
                if let done { message = done } else {
                    let lines = output.split(separator: "\n").filter { $0.hasPrefix("/dev/") }
                    let free = lines.map { String($0.split(separator: " ", omittingEmptySubsequences: true).dropFirst(3).first ?? "?") }
                    message = free.count == 2 ? String(localized: "Linux builder: \(free[0]) → \(free[1]) free")
                                              : String(localized: "Linux builder cleaned up")
                }
            } catch is AdminShell.Cancelled {
            } catch {
                message = String(localized: "Failed – see \(maintenance.log.path)") + "\n" + String(error.localizedDescription.suffix(400))
            }
            builderTask = nil
            await checkAll()
            if succeeded { next?() }
        }
    }

    /// Pauses the download; it continues from there with the next download.
    func pauseDownload() { downloader?.cancel() }

    /// An IPSW file the user already has
    func chooseIPSW(_ done: @escaping (URL) -> Void) {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.message = String(localized: "A macOS restore image (.ipsw) from Apple")
        present(panel) { url in if url.pathExtension == "ipsw" { done(url) } }
    }

    /// Shows a file picker as a sheet on the menu window: a separate panel window would close it (plan B59/B69).
    /// Without a visible menu window (e.g. called early) it falls back to a modal panel.
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

    /// The Requirements window opens by itself on the first launch
    static var firstLaunch: Bool { !UserDefaults.standard.bool(forKey: "requirementsShown") }
    func markRequirementsShown() { UserDefaults.standard.set(true, forKey: "requirementsShown") }

    func checkPrerequisites() async {
        checkingPrerequisites = true
        let paths = self.paths
        prerequisites = await Task.detached { Prerequisites(paths: paths).check() }.value
        checkingPrerequisites = false
    }

    /// Path for nix or QEMU (a file) or the image flake (a folder), picked by the user.
    func choosePath(for item: Prerequisite.Item) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = item == .flake
        panel.canChooseFiles = item != .flake
        panel.treatsFilePackagesAsDirectories = true
        panel.showsHiddenFiles = true
        panel.message = item == .flake ? String(localized: "Folder with the flake.nix of your recipes")
            : item == .nix ? String(localized: "The nix binary") : String(localized: "The qemu-system-aarch64 binary")
        present(panel) { [self] url in setPath(url, for: item) }
    }

    private func setPath(_ url: URL, for item: Prerequisite.Item) {
        var s = Settings.load(paths)
        switch item {
        case .nix: s.nix = url.path
        case .qemu: s.qemu = url.path
        case .flake: s.flake = url.path
        default: return
        }
        do { try s.save(paths) } catch { message = "\(error)" }
        Task {
            await checkAll()
            await loadRecipes()
        }
    }

    func revealShare(_ name: String) {
        guard let c = rows.first(where: { $0.id == name })?.config, let dir = try? paths.prepareShare(c) else { return }
        NSWorkspace.shared.activateFileViewerSelecting([dir])
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

    // MARK: Image build

    /// Queues a build of a recipe's image. macOS recipes need the guest user's password once (sudo in the build VM;
    /// kept in memory only until the build starts). Starts at once unless a build of the same guest kind runs.
    func build(_ recipe: String, password: String? = nil) {
        guard activeJob(for: recipe) == nil, let kind = recipes[recipe]?.kind else { return }
        let job = BuildJob(recipe: recipe, kind: kind, password: password, logs: paths.logs)
        builds.removeAll { $0.recipe == recipe && !$0.isActive }   // the recipe's previous log goes
        builds.append(job)
        if builds.contains(where: { $0 !== job && $0.kind == kind && $0.isActive }) {
            job.append(String(localized: "Waiting – another \(kind == .macos ? "macOS" : "NixOS") build runs first"))
        }
        schedule()
    }

    /// Starts the next waiting build of each guest kind that has none running.
    private func schedule() {
        for kind in GuestKind.allCases where !builds.contains(where: { $0.kind == kind && $0.isRunning }) {
            if let next = builds.first(where: { $0.kind == kind && $0.state == .queued }) { start(next) }
        }
    }

    private func start(_ job: BuildJob) {
        job.state = .checking
        job.started = Date()
        job.append(String(localized: "Checking …"))
        Task {
            // Right before the build: e.g. the Linux builder may have stopped since the last check
            await checkAll()
            if let first = buildBlockers(job.kind).first {
                job.finish(.failed, String(localized: "Doctor: \(RequirementText.title(first.item)) – fix it there first"))
                message = String(localized: "Doctor: \(RequirementText.title(first.item)) – fix it there first")
                schedule()
                return
            }
            if job.state == .cancelled { schedule(); return }
            runBuild(job)
        }
    }

    private func runBuild(_ job: BuildJob) {
        let recipe = job.recipe, kind = job.kind, password = job.password
        job.password = nil
        job.state = .running
        job.append(kind == .macos ? "omaf build \(recipe)" : "nix build .#images.\(recipe)")
        let paths = self.paths
        let token = Cancellation()
        job.cancellation = token
        Task {
            do {
                let image = try await Task.detached {
                    try Cancellation.$current.withValue(token) {
                    let flake = try Settings.load(paths).flakePath()
                    let log: @Sendable (String) -> Void = { line in Task { @MainActor in job.append(line) } }
                    if kind == .macos {
                        // The flake as it is now (the app's recipe list may be older, e.g. after a settings change)
                        let user = try ImageStore(paths: paths).recipeDefaults(flake: flake)[recipe]?.user ?? NSUserName()
                        guard let password, !password.isEmpty else { throw VMError(String(localized: "Password of \(user) needed (sudo, once)")) }
                        let builder = MacImageBuilder(paths: paths, vz: try Backends(paths: paths).vz)
                        return try builder.build(.init(recipe: recipe, flake: flake, user: user, password: password), log: log)
                    }
                    return try ImageStore(paths: paths).build(recipe: recipe, flake: flake, log: log)
                    }
                }.value
                job.append(String(localized: "✓ Image \(OmaflakeID.image(image, words: 3)) ready"))
                // Retention: current + one before + those in use (plan B52)
                let removed = await Task.detached { () -> [String] in
                    guard let used = try? Set(Backends(paths: paths).store.list().map(\.image)) else { return [] }
                    return ImageStore(paths: paths).prune(recipe: recipe, used: used)
                }.value
                for name in removed { job.append(String(localized: "Removed older image \(name)")) }
                Task { await self.loadRecipes(); await self.checkFreshness() }   // a kept version, older ones gone (B111)
                job.finish(.finished, nil)
                message = String(localized: "Image \(RecipeID.name(ofID: recipe)) ready")
                notify(String(localized: "Image ready"), String(localized: "\(RecipeID.name(ofID: recipe)) can be used for new VMs."))
            } catch {
                job.append("✗ \(error)")
                job.finish(token.isCancelled ? .cancelled : .failed, nil)
                message = token.isCancelled ? String(localized: "Build of \(RecipeID.name(ofID: recipe)) cancelled")
                    : String(localized: "Build \(RecipeID.name(ofID: recipe)): \(error.localizedDescription)")
                if !token.isCancelled {
                    notify(String(localized: "Build failed"), String(localized: "\(RecipeID.name(ofID: recipe)) – the log is in the Images tab."))
                }
            }
            refresh()
            schedule()
            await checkAll()
        }
    }

    /// Stops a build: a waiting one leaves the queue; a running one stops its step, a macOS build VM is forced off,
    /// the half-built folder is deleted.
    func cancel(_ job: BuildJob) {
        switch job.state {
        case .queued, .checking:
            job.password = nil
            job.finish(.cancelled, String(localized: "Removed from the queue"))
            schedule()
        case .running:
            guard let token = job.cancellation, !token.isCancelled else { return }
            job.append(String(localized: "Cancelling …"))
            token.cancel()
        default: break
        }
    }

    /// Hides a finished build's row (its log goes with it)
    func dismiss(_ job: BuildJob) { builds.removeAll { $0 === job && !$0.isActive } }

    // MARK: macOS base

    /// `omaf macos install` by click: download/install, clone, first boot in a window (blocks until it is shut down).
    func installBase(user: String, fullName: String, password: String, ipsw: URL? = nil) {
        guard !installing else { return }
        installing = true
        installLog = ["omaf macos install (user \(user))"]
        let paths = self.paths
        let model = self
        Task {
            do {
                let outcome = try await Task.detached {
                    let installer = MacBaseInstaller(paths: paths, vz: try Backends(paths: paths).vz)
                    return try installer.install(.init(ipsw: ipsw, user: user, fullName: fullName, password: password),
                                                 started: { d in Task { @MainActor in model.downloader = d; model.downloading = true } },
                                                 progress: { done, total in Task { @MainActor in model.downloadProgress = (done, total) } }) { line in
                        Task { @MainActor in model.appendInstallLog(line) }
                    }
                }.value
                switch outcome {
                case .existing(let base):
                    appendInstallLog(String(localized: "✓ \(base.lastPathComponent) exists already"))
                case .created(let base, _):
                    appendInstallLog(String(localized: "✓ \(base.lastPathComponent) is ready – build a macOS image next (Build Image …)."))
                }
            } catch {
                appendInstallLog("✗ \(error)")
                message = String(localized: "macOS base: \(error.localizedDescription)")
            }
            installing = false
            downloading = false
            downloader = nil
            refresh()
            await checkAll()
        }
    }

    private func appendInstallLog(_ line: String) {
        installLog.append(line)
        if installLog.count > 5000 { installLog.removeFirst(installLog.count - 5000) }
    }

    // MARK: Data folder

    var isDefaultDataFolder: Bool {
        paths.root.standardizedFileURL.path == Paths.defaultHome.standardizedFileURL.path
    }

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

    /// Moves the data (`DataFolder.move`); all VMs must be stopped.
    func moveDataFolder(to target: URL) {
        Task { @MainActor in
            guard !movingData else { return }
            guard rows.allSatisfy({ $0.status == .stopped }) else {
                message = String(localized: "Stop all VMs before moving the data folder.")
                return
            }
            guard await confirm(String(localized: "Move the data folder to \(target.path)?"),
                          String(localized: "Images, instances and bases are moved there (renamed, not copied). The settings stay in ~/.omaflake.")) else { return }
            movingData = true
            let from = paths
            Task {
                do {
                    let new = try await Task.detached { try DataFolder.move(from: from, to: target) }.value
                    paths = new
                    message = String(localized: "Data folder: \(new.root.path)")
                } catch {
                    message = "\(error)"
                }
                movingData = false
                refresh()
                await loadRecipes()
                await checkAll()
            }
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
    let guestDefaults = GuestSettings.mac()

    /// Bumped on every change, so views reading `guestOverrides` (stored in config.json) refresh
    private var guestRevision = 0

    /// Overrides from config.json; an empty field means "the Mac's value"
    var guestOverrides: GuestOverrides {
        get {
            _ = guestRevision
            return Settings.load(paths).guest ?? GuestOverrides()
        }
        set {
            var s = Settings.load(paths)
            func clean(_ v: String?) -> String? { v.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 } }
            s.guest = GuestOverrides(user: clean(newValue.user), fullName: clean(newValue.fullName), email: clean(newValue.email),
                                     timeZone: clean(newValue.timeZone), locale: clean(newValue.locale), keyboard: clean(newValue.keyboard))
            do { try s.save(paths) } catch { message = "\(error)" }
            guestRevision += 1
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

    // MARK: Helpers

    /// A question inside the menu window (an alert window would close the menu window, plan B59)
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

    /// Asks in the menu window and waits for the answer.
    private func confirm(_ title: String, _ text: String, button: String = String(localized: "Continue"), destructive: Bool = false) async -> Bool {
        await choose(title, text, button: button, destructive: destructive) == 0
    }

    /// Asks with several answers: nil = cancelled, 0 = `button`, 1… = `others`
    private func choose(_ title: String, _ text: String, button: String, others: [String] = [], destructive: Bool = false,
                        input: TextInput? = nil) async -> Int? {
        // Without the menu window (a package double-clicked in the Finder) the question comes as an alert
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

    // MARK: Packages (plan B108, B109): export, import (always a copy), external VMs

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

    func exportVM(_ name: String) {
        chooseFolder(String(localized: "Where to put “\(name)” – an external disk, a NAS or a folder. It goes along with its user, SSH keys and data: keep it for yourself.")) { folder in
            self.perform(name) { backends, _ in
                let dest = try Transfer(backends).exportVM(name, to: folder)
                return String(localized: "\(name) exported to \(dest.path)")
            }
        }
    }

    func exportImage(_ name: String) {
        chooseFolder(String(localized: "Where to put the image – an external disk, a NAS or a folder. It carries the user it was built for: share recipes instead.")) { folder in
            self.perform(name, locked: false) { backends, _ in
                let dest = try Transfer(backends).exportImage(name, to: folder)
                return String(localized: "\(name) exported to \(dest.path)")
            }
        }
    }

    func exportRecipe(_ id: String) {
        chooseFolder(String(localized: "Where to put the recipe (with the recipes it needs) – to keep or to share")) { folder in
            self.perform(id, locked: false) { _, paths in
                let dest = try RecipeLibrary(paths: paths).exportRecipe(id, to: folder, settings: Settings.load(paths))
                return String(localized: "\(RecipeID.qualified(id)) exported to \(dest.path)")
            }
        }
    }

    /// The open panel for packages of one kind: only those are offered, and what is chosen is checked before the
    /// panel closes (a folder that only looks like a package is refused with the reason)
    private func choosePackages(_ kind: Package.Kind, external: Bool = false, _ done: @escaping ([URL]) -> Void) {
        let panel = NSOpenPanel()
        let type = UTType(exportedAs: "app.omaflake.\(kind == .vm ? "vm" : kind == .image ? "image" : "recipe")")
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
        let checker = PackageChecker(kind: kind, external: external, paths: paths)
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
    func chooseImport(_ kind: Package.Kind) {
        choosePackages(kind) { [weak self] urls in self?.openPackages(urls) }
    }

    /// The VMs tab's second button: a VM that runs from where its package is
    func chooseExternal() {
        choosePackages(.vm, external: true) { [weak self] urls in
            guard let self, let url = urls.first else { return }
            Task { @MainActor in
                if Transfer.isOnNetworkVolume(url) {
                    guard await self.confirm(String(localized: "Run the VM over the network?"),
                                             String(localized: "\(url.lastPathComponent) is on a network volume. It runs from there, but slower – and it breaks when the connection does. Importing it (a copy into the data folder) avoids that."),
                                             button: String(localized: "Run from There")) else { return }
                }
                self.perform(url.lastPathComponent, locked: false) { backends, _ in
                    let name = try Transfer(backends).useExternal(url)
                    return String(localized: "\(name) runs from \(url.deletingLastPathComponent().path)")
                }
            }
        }
    }

    /// Stops listing a VM that runs from elsewhere; its package stays where it is
    func forget(_ name: String) {
        perform(name, locked: false) { backends, _ in
            try backends.store.forget(name)
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
        let paths = self.paths
        let conflict: Transfer.Conflict
        do {
            conflict = try await Task.detached {
                try Transfer(paths: paths, qemuImg: "qemu-img", runner: nil).conflict(url)
            }.value
        } catch {
            message = "\(url.lastPathComponent): \(error)"
            return
        }
        var name: String?
        var replace = false
        switch conflict {
        case .none: break
        case .vmName(let existing):
            let input = TextInput(text: nextFreeName(existing))
            guard await choose(String(localized: "“\(existing)” is here already"),
                               String(localized: "You probably have this VM already. Under another name it becomes a second VM with its own identity (SSH port, macOS machine identity); its user, keys and data are the same."),
                               button: String(localized: "Import under This Name"), input: input) == 0 else { return }
            name = input.text.trimmingCharacters(in: .whitespaces)
        case .image(let existing):
            guard await confirm(String(localized: "The image is here already"),
                                String(localized: "\(existing) is here already – the same name means the same content. Replacing it copies it again."),
                                button: String(localized: "Replace")) else { return }
            replace = true
        }
        let (newName, replacing) = (name, replace)
        let kind = Transfer.kind(of: url)
        perform(url.lastPathComponent, locked: false) { backends, _ in
            let r = try Transfer(backends).importPackage(url, name: newName, replace: replacing)
            switch r.kind {
            case .vm: return String(localized: "VM \(r.names.joined()) imported")
            case .image: return String(localized: "Image \(r.names.joined()) imported")
            case .recipe:
                return r.names.isEmpty ? String(localized: "These recipes are here already, unchanged – nothing to import")
                    : String(localized: "Recipes imported: \(r.names.map(RecipeID.qualified).joined(separator: ", "))")
            }
        }
        if kind == .recipe {
            // perform runs detached; the recipes list reads the installed recipes once it is done
            while busy.contains(url.lastPathComponent) { try? await Task.sleep(for: .milliseconds(200)) }
            await loadRecipes()
        }
    }

    /// "research" → "research-2", "research-3", … (the first free VM name)
    private func nextFreeName(_ name: String) -> String {
        let names = Set(rows.map(\.id))
        var n = 2
        while names.contains("\(name)-\(n)") { n += 1 }
        return "\(name)-\(n)"
    }

    func nextFreeName(for recipe: String) -> String {
        let recipe = RecipeID.displayName(recipe, recipes[recipe])
        let names = Set(rows.map(\.id))
        if !names.contains(recipe) { return recipe }
        var n = 2
        while names.contains("\(recipe)-\(n)") { n += 1 }
        return "\(recipe)-\(n)"
    }
}

/// One entry of Recent Activity.
struct Activity: Identifiable, Equatable {
    let id = UUID()
    let date: Date
    let text: String
}

/// One image build in the queue: state, log, progress (plan B60)
@MainActor @Observable
final class BuildJob: Identifiable {
    enum State: Equatable { case queued, checking, running, finished, failed, cancelled }

    let id = UUID()
    let recipe: String
    let kind: GuestKind
    /// macOS: the guest user's password, only until the build starts
    var password: String?
    var state: State = .queued
    private(set) var log: [String] = []
    private let tracker: BuildProgressTracker
    private(set) var progress: BuildProgress
    var started: Date?
    private(set) var lastOutput: Date?
    var cancellation: Cancellation?

    /// Every line, also those left out of `log` (BuildLog.isNoise): <data>/logs/image-build-<recipe>.log
    let fullLog: URL?
    private var fullLogHandle: FileHandle?
    private(set) var hiddenLines = 0

    init(recipe: String, kind: GuestKind, password: String?, logs: URL? = nil) {
        self.recipe = recipe
        self.kind = kind
        self.password = password
        tracker = BuildProgressTracker(kind: kind)
        progress = tracker.progress
        fullLog = logs?.appendingPathComponent("image-build-\(recipe).log")
        if let fullLog, let logs {
            try? FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
            FileManager.default.createFile(atPath: fullLog.path, contents: nil)
            fullLogHandle = try? FileHandle(forWritingTo: fullLog)
        }
    }

    var isActive: Bool { [.queued, .checking, .running].contains(state) }
    var isRunning: Bool { [.checking, .running].contains(state) }
    var canCancel: Bool { isActive && cancellation?.isCancelled != true }

    func append(_ line: String) {
        if tracker.feed(line) { progress = tracker.progress }
        lastOutput = Date()
        fullLogHandle?.write(Data((line + "\n").utf8))
        if BuildLog.isNoise(line) { hiddenLines += 1; return }
        log.append(line)
        if log.count > 5000 { log.removeFirst(log.count - 5000) }
    }

    func finish(_ state: State, _ line: String?) {
        if let line { append(line) }
        if hiddenLines > 0, let fullLog {
            append(String(localized: "(\(hiddenLines) routine lines of the disk image tool hidden – full log: \(fullLog.path))"))
        }
        try? fullLogHandle?.close()
        fullLogHandle = nil
        self.state = state
        cancellation = nil
    }
}


/// Checks what is chosen in an import panel before the panel closes (plan B109): by content, not only by extension –
/// the panel stays open with the reason when it is not what the tab imports
final class PackageChecker: NSObject, NSOpenSavePanelDelegate {
    let kind: Package.Kind
    let external: Bool
    let paths: Paths

    init(kind: Package.Kind, external: Bool, paths: Paths) {
        self.kind = kind
        self.external = external
        self.paths = paths
    }

    func panel(_ sender: Any, validate url: URL) throws {
        do {
            switch kind {
            case .vm:
                guard Package.kind(of: url) == .vm else { throw VMError(String(localized: "Not a VM package (.omaflake-vm)")) }
                let c = try Transfer.readConfig(url)
                let image = url.appendingPathComponent(Transfer.embeddedImageName).appendingPathComponent("disk.qcow2")
                if external, c.guestKind == .nixos, !FileManager.default.fileExists(atPath: image.path) {
                    throw VMError(String(localized: "\(c.name) does not carry its image – export it again to run it from there"))
                }
            case .image:
                _ = try ImageStore(paths: paths).image(at: url)
            case .recipe:
                _ = try RecipeLibrary(paths: paths).recipes(in: url)
            }
        } catch {
            throw NSError(domain: "app.omaflake", code: 1, userInfo: [NSLocalizedDescriptionKey: "\(error)"])
        }
    }
}
