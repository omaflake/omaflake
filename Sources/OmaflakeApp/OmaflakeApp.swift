// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

@main
struct OmaflakeApp: App {
    @State private var model = AppModel()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        MenuBarExtra {
            MainView(model: model)
                .environment(model)
                // omaflake's look: dark brand background, cyan accent (brand/README.md)
                .preferredColorScheme(.dark)
                .tint(BrandStyle.cyan)
                .buttonBorderShape(.roundedRectangle(radius: BrandStyle.radius))
                .background(BrandStyle.background.clipShape(RoundedRectangle(cornerRadius: BrandStyle.windowRadius, style: .continuous)))
                .overlay(RoundedRectangle(cornerRadius: BrandStyle.windowRadius, style: .continuous)
                    .strokeBorder(BrandTokens.Colors.windowBorder, lineWidth: BrandTokens.Size.windowBorderWidth).allowsHitTesting(false))
                .background(RoundedWindow())
        } label: {
            Image(nsImage: Brand.menuBarImage())
        }
        .menuBarExtraStyle(.window)
    }
}

/// Packages opened in the Finder (double-click, dropped on the app icon): handed to the model once it is there.
/// The "open documents" Apple event is taken directly: with only a menu bar scene, SwiftUI's own handler swallows it
/// and `application(_:open:)` is never called (seen on macOS 27).
@MainActor final class AppDelegate: NSObject, NSApplicationDelegate {
    static var open: (([URL]) -> Void)?
    static var waiting: [URL] = []
    /// Ends the core's server when the app quits (running VMs keep running)
    static var terminate: (() -> Void)?

    func applicationWillTerminate(_ notification: Notification) { Self.terminate?() }

    func applicationWillFinishLaunching(_ notification: Notification) { takeOpenEvents() }
    func applicationDidFinishLaunching(_ notification: Notification) {
        takeOpenEvents()
        // Once more after SwiftUI has set up its scenes, in case it installs its handler then (OPEN, plan B108)
        DispatchQueue.main.async { [weak self] in self?.takeOpenEvents() }
    }

    private func takeOpenEvents() {
        NSAppleEventManager.shared().setEventHandler(self, andSelector: #selector(openDocuments(_:withReplyEvent:)),
                                                     forEventClass: AEEventClass(kCoreEventClass), andEventID: AEEventID(kAEOpenDocuments))
    }

    @objc func openDocuments(_ event: NSAppleEventDescriptor, withReplyEvent reply: NSAppleEventDescriptor) {
        guard let list = event.paramDescriptor(forKeyword: keyDirectObject) else { return }
        let urls = list.numberOfItems == 0 ? [list.fileURLValue].compactMap { $0 }
            : (1...list.numberOfItems).compactMap { list.atIndex($0)?.fileURLValue }
        received(urls)
    }

    func application(_ application: NSApplication, open urls: [URL]) { received(urls) }

    private func received(_ urls: [URL]) {
        guard !urls.isEmpty else { return }
        if let open = Self.open { open(urls) } else { Self.waiting += urls }
    }
}

// MARK: - Main window (menu bar)

struct MainView: View {
    @Bindable var model: AppModel

    /// Pages open one level deeper inside this window (no extra windows: those closed the menu window, plan B42)
    /// One fixed size for every page, so the window neither grows nor jumps when navigating
    static let size = CGSize(width: 500, height: 600)

    var body: some View {
        Group {
            if let page = model.path.last {
                VStack(alignment: .leading, spacing: 0) {
                    PageHeader(page: page, model: model)
                    Divider()
                    pageView(page)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                }
            } else {
                VStack(spacing: 0) {
                    header.padding([.horizontal, .top], BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.xl)
                    HomeTabBar(model: model).padding(.horizontal, BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.xxl)
                    ScrollView { home.padding([.horizontal, .bottom], BrandStyle.margin).background(OverlayScrollers()) }
                        .id(model.tab)   // each tab opens at its top
                        .overlay(alignment: .bottom) { FooterMessage(model: model) }
                }
            }
        }
        .frame(width: Self.size.width, height: Self.size.height)
        .overlay { if let c = model.confirmation { ConfirmationOverlay(confirmation: c) } }
    }

    @ViewBuilder private func pageView(_ page: AppModel.Page) -> some View {
        switch page {
        case .doctor: DoctorView(model: model)
        case .settings: SettingsView(model: model)
        case .newVM: NewVMView(model: model)
        case .build: BuildView(model: model)
        case .macBase: MacBaseView(model: model)
        case .guide: GuideView(file: "linux-builder")
        }
    }

    /// The open tab's list (plan B91): VMs, images (what is built), recipes (what can be built), activity.
    /// The header's filter (plan B101) narrows VMs, images and recipes.
    @ViewBuilder private var home: some View {
        let f = model.filters[model.tab] ?? AppModel.ListFilter()
        switch model.tab {
        case .vms:
            let rows = model.rows.filter {
                (f.status == nil || f.status == ($0.status == .running)) && (f.kind == nil || f.kind == $0.kind)
            }
            ListSection(title: "Virtual Machines", count: rows.count, of: model.rows.count,
                        filter: AnyView(FilterMenu(model: model, tab: .vms, on: "Running", off: "Stopped"))) {
                HStack(spacing: BrandTokens.Spacing.xxs) {
                    IconButton(icon: "square.and.arrow.down", help: "Import a VM – a copy into the data folder") { model.chooseImport(.vm) }
                    IconButton(icon: "externaldrive.badge.plus", help: "Run a VM from where it is – an external disk, a NAS or a folder; nothing is copied") {
                        model.chooseExternal()
                    }
                }
            } content: {
                if model.rows.isEmpty {
                    EmptyHint(icon: "desktopcomputer", text: "No VMs yet – create one from an image.")
                } else if rows.isEmpty {
                    FilteredOut(model: model)
                }
                ForEach(rows) { row in
                    InstanceRow(row: row, busy: model.isBusy(row), model: model)
                }
                ForEach(model.unreadableVMs, id: \.self) { UnreadableHint(name: $0.name, reason: $0.reason) }
            }
        case .images:
            // Every built image on its own row (plan B93); builds that wait, run or failed on top
            let all = model.allImages
            let used = Set(model.rows.map(\.image))
            let images = all.filter {
                (f.status == nil || f.status == used.contains($0.name)) && (f.kind == nil || f.kind == $0.kind)
                    && (f.recipe == nil || f.recipe == $0.recipe)
            }
            let jobs = model.builds.filter {
                $0.state != .finished && (f.kind == nil || f.kind == $0.guestKind) && (f.recipe == nil || f.recipe == $0.recipe)
                    && f.status != true
            }
            let recipes = Array(Set(all.map(\.recipe))).sorted { RecipeText.name($0) < RecipeText.name($1) }
            ListSection(title: "Images", count: images.count, of: all.count,
                        filter: AnyView(FilterMenu(model: model, tab: .images, on: "In use", off: "Not in use",
                                                   recipes: recipes.map { ($0, model.recipeName($0)) }))) {
                HStack(spacing: BrandTokens.Spacing.xxs) {
                    IconButton(icon: "square.and.arrow.down", help: "Import an image – a copy into the data folder") { model.chooseImport(.image) }
                }
            } content: {
                if all.isEmpty && jobs.isEmpty {
                    EmptyHint(icon: "shippingbox", text: "No images yet – build one from a recipe.")
                } else if images.isEmpty && jobs.isEmpty {
                    FilteredOut(model: model)
                }
                ForEach(jobs) { job in
                    BuildJobRow(job: job, model: model, standalone: true) {
                        model.buildRecipe = job.recipe
                        show("build")
                    }
                }
                ForEach(images) { image in
                    ImageRowView(image: image, model: model) {
                        model.newVMImage = image.name
                        show("new")
                    }
                }
                ForEach(model.unreadableImages, id: \.self) { UnreadableHint(name: $0.name, reason: $0.reason) }
            }
        case .recipes:
            let all = model.recipeRows
            let rows = all.filter {
                (f.otherSystems || $0.runsHere) && (f.status == nil || f.status == ($0.image != nil)) && (f.kind == nil || f.kind == $0.kind)
                    && (f.tag == nil || $0.tags.contains(f.tag!))
            }
            let tags = Array(Set(all.flatMap(\.tags))).sorted()
            // What the remote sources offer and is not installed (plan B107); the status filter "with image" hides them
            let offered = model.offeredRecipes.filter {
                (f.otherSystems || $0.runsHere) && f.status != true && (f.kind == nil || f.kind == $0.kind) && (f.tag == nil || $0.tags.contains(f.tag!))
            }
            ListSection(title: "Recipes", count: rows.count, of: all.count,
                        filter: AnyView(FilterMenu(model: model, tab: .recipes, on: "With image", off: "Without image", tags: tags,
                                                   architectures: true))) {
                HStack(spacing: BrandTokens.Spacing.xxs) {
                    if model.loadingCatalogs {
                        ProgressView().controlSize(.small).frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight)
                    } else {
                        IconButton(icon: "arrow.clockwise", help: "Look for new recipes in the sources") { model.refreshCatalogs() }
                    }
                    IconButton(icon: "square.and.arrow.down", help: "Import recipes – a copy into your recipes (sources to use as they are: Settings)") {
                        model.chooseImport(.recipe)
                    }
                }
            } content: {
                if all.isEmpty && offered.isEmpty {
                    EmptyHint(icon: "book.closed", text: model.loadingCatalogs ? "Looking into the recipe sources …"
                              : "No recipes – add a recipe source in Settings.")
                } else if rows.isEmpty && offered.isEmpty {
                    FilteredOut(model: model)
                }
                let updates = model.recipeUpdates
                if !updates.isEmpty { RecipeUpdates(updates: updates, model: model) }
                if all.isEmpty, !model.suggestedRecipes.isEmpty {
                    // First start: omaflake ships no recipes – the official ones are suggested
                    HStack(spacing: BrandTokens.Spacing.l) {
                        Image(systemName: "checkmark.seal.fill").foregroundStyle(BrandStyle.cyan)
                        Text("Start with the official recipes?").font(.callout)
                        Spacer()
                        ActionButton(icon: "arrow.down.circle", title: "Install suggested") { model.installSuggested() }
                    }
                    .padding(BrandTokens.Spacing.cardPadding)
                }
                ForEach(rows) { recipe in
                    RecipeRowView(recipe: recipe, model: model) {
                        model.buildRecipe = recipe.id
                        show("build")
                    }
                }
                ForEach(offered) { recipe in
                    OfferedRecipeRow(recipe: recipe, model: model)
                }
                ForEach(model.unreadableRecipes, id: \.self) { UnreadableHint(name: RecipeText.qualified($0.name), reason: $0.reason) }
                ForEach(model.available.errors.sorted { $0.key < $1.key }, id: \.key) { id, error in
                    HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning)
                        Text(verbatim: "\(model.source(id)?.displayName ?? id): \(error)").font(.caption)
                            .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(BrandTokens.Spacing.cardPadding)
                }
            }
            .onAppear { if !model.catalogsLoaded { model.refreshCatalogs() } }
        case .activity:
            ListSection(title: "Activity", count: model.activities.count) {
                IconButton(icon: "trash", help: "Clear History") { model.clearActivities() }
            } content: {
                if model.activities.isEmpty {
                    EmptyHint(icon: "clock", text: "Nothing happened yet in this session.")
                }
                ForEach(model.activities) { ActivityRow(activity: $0) }
            }
        }
    }

    private var header: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Image(nsImage: Brand.menuBarImage())
            Wordmark()
            Spacer()
            IconButton(icon: "stethoscope", help: "Doctor – requirements and preflight", badge: model.doctorIssues) { model.show(.doctor) }
            IconButton(icon: "gearshape", help: "Settings") { model.show(.settings) }
            IconButton(icon: "folder", help: "Show Data Folder in Finder") { NSWorkspace.shared.open(URL(fileURLWithPath: model.dataFolder)) }
            IconButton(icon: "power", help: "Quit omaflake") { model.quit() }
        }
    }

    private func show(_ id: String) {
        model.show(id == "new" ? .newVM : .build)
    }
}

/// Title row of a page: back to where the user came from (also Esc).
struct PageHeader: View {
    let page: AppModel.Page
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.m) {
            Button { model.back() } label: {
                Image(systemName: "chevron.left").font(.system(size: BrandTokens.Icon.toolbar, weight: .semibold))
                    .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .disabled(model.confirmation != nil)
            .help("Back")
            Text(page.title).font(.headline)
            Spacer()
        }
        .padding(.horizontal, BrandStyle.margin - 6).padding(.top, BrandStyle.margin - 6).padding(.bottom, BrandTokens.Spacing.l)
    }
}

/// Tabs of the main page with what needs attention: running VMs, a build running or failed, recipes changed since
/// their image, new messages.
struct HomeTabBar: View {
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.s) {
            tab(.vms, "VMs", icon: "desktopcomputer") {
                let running = model.rows.filter { $0.status == .running }.count
                if running > 0 { CountBadge(count: running, color: BrandTokens.Colors.success) }
            }
            tab(.images, "Images", icon: "shippingbox") {
                if model.anyBuildRunning {
                    ProgressView().controlSize(.mini).help("A build is running")
                } else if model.builds.contains(where: { $0.state == .failed }) {
                    Image(systemName: "xmark.circle.fill").font(.caption2).foregroundStyle(BrandStyle.red).help("A build failed")
                }
            }
            tab(.recipes, "Recipes", icon: "book.closed") {
                if model.recipeRows.contains(where: { $0.image != nil && $0.outdated }) {
                    Circle().fill(BrandTokens.Colors.warning).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot).help("A recipe changed since its image was built")
                }
            }
            tab(.activity, "Activity", icon: "clock") {
                if model.unseenActivity { Circle().fill(BrandStyle.cyan).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot).help("New messages") }
            }
        }
        .padding(BrandTokens.Spacing.xs)
        .background(BrandStyle.card, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous).strokeBorder(BrandStyle.cardBorder))
    }

    private func tab(_ tab: AppModel.HomeTab, _ title: LocalizedStringKey, icon: String,
                     @ViewBuilder badge: () -> some View) -> some View {
        let selected = model.tab == tab
        return Button { model.tab = tab } label: {
            HStack(spacing: 5) {
                Image(systemName: icon).font(.system(size: BrandTokens.Icon.tab, weight: .medium))
                Text(title).font(.callout.weight(selected ? .semibold : .regular)).lineLimit(1)
                badge()
            }
            .foregroundStyle(selected ? .primary : .secondary)
            .frame(maxWidth: .infinity).frame(height: 26)
            .background(selected ? Color.primary.opacity(BrandTokens.Opacity.selected) : .clear,
                        in: RoundedRectangle(cornerRadius: BrandTokens.Radius.tab, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// A small number in a capsule (e.g. running VMs on the tab)
struct CountBadge: View {
    let count: Int
    let color: Color

    var body: some View {
        Text(verbatim: "\(count)").font(.caption2.weight(.bold).monospacedDigit()).foregroundStyle(.black)
            .padding(.horizontal, 5).frame(minWidth: 16, minHeight: 14)
            .background(color, in: Capsule())
    }
}

/// The newest message for a few seconds at the bottom of the main page; a click opens the activity tab.
struct FooterMessage: View {
    let model: AppModel

    var body: some View {
        if let footer = model.footer, model.tab != .activity {
            Button { model.tab = .activity } label: {
                HStack(spacing: BrandTokens.Spacing.l) {
                    Image(systemName: "info.circle").foregroundStyle(BrandStyle.cyan)
                    Text(footer.text).font(.caption).lineLimit(2).multilineTextAlignment(.leading)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, BrandTokens.Spacing.xxl).padding(.vertical, BrandTokens.Spacing.l)
                .background(BrandStyle.base, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous).strokeBorder(BrandStyle.cyan.opacity(BrandTokens.Opacity.noticeBorder)))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Show Activity")
            .padding(.horizontal, BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.xl)
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .id(footer.id)
        }
    }
}

/// One recipe (recipe): what it is for, whether its image is up to date, and building it.
struct RecipeRowView: View {
    let recipe: Recipe
    let model: AppModel
    let build: () -> Void

    var body: some View {
        ExpandableRow {
            header
        } detail: {
            detail
        }
    }

    private var header: some View {
        HStack(spacing: BrandTokens.Spacing.xl) {
            StatusLamp(state: lampState).help(lampHelp)
            GuestKindIcon(kind: recipe.kind).frame(width: 16)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                HStack(spacing: BrandTokens.Spacing.s) {
                    Text(recipe.displayName).fontWeight(.medium)
                    if recipe.author != "omaflake", !recipe.author.isEmpty {
                        Text(verbatim: "· \(recipe.author)").font(.caption).foregroundStyle(.secondary)
                    }
                    // Only when another recipe has the same author, kind and name (plan B110)
                    if let d = recipe.distinction {
                        Text(verbatim: "· \(d)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    if recipe.lab {
                        Image(systemName: "flask").font(.caption).foregroundStyle(BrandStyle.yellow).help("Development recipe (developer mode)")
                    }
                    if !recipe.runsHere { ArchitectureBadge(systems: recipe.systems) }
                }
                if !recipe.tags.isEmpty { TagList(tags: recipe.tags) }
            }
            Spacer()
            // Its folder in the Finder – to look at it, or to change or remove it by hand
            IconButton(icon: "folder", help: "Show in Finder") { model.revealRecipe(recipe.id) }
                .disabled(recipe.folder == nil)
            let blockers = model.buildBlockers(recipe.kind)
            if !recipe.runsHere {
                IconButton(icon: "shippingbox", help: "Not for this Mac – it is built for \(recipe.systems.joined(separator: ", "))",
                           subIcon: "hammer.fill", subIconColor: BrandStyle.cyan) {}
                    .disabled(true)
            } else if !blockers.isEmpty {
                DoctorWarning(blockers: blockers)
            } else {
                IconButton(icon: "shippingbox", help: recipe.image == nil ? "Build image" : "Build a new version of this image",
                           subIcon: "hammer.fill", subIconColor: BrandStyle.cyan, action: build)
                    .disabled(model.activeJob(for: recipe.id) != nil)
            }
        }
    }

    /// The whole description, the defaults a new VM gets from it, its images and versions
    private var detail: some View {
        let images = model.images(ofLine: recipe.line)
        return DetailBox {
            if let description = recipe.description {
                Text(description).font(.caption).fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                InfoLine(label: "Resources", value: Text(verbatim: "\(recipe.cpus) CPU · \(recipe.memoryGB) GB RAM · \(recipe.diskGB) GB"))
                InfoLine(label: "Architectures", value: Text(verbatim: recipe.systems.joined(separator: ", ")))
                InfoLine(label: "Network", value: Text(verbatim: recipe.network.rawValue))
                InfoLine(label: "New VMs", value: Text(verbatim: defaults(recipe.settings)))
                InfoLine(label: "Images", value: images.isEmpty ? Text("not built yet")
                         : Text("\(images.count) built, the newest \(images[0].created.formatted(date: .abbreviated, time: .shortened))"))
                if recipe.source == "import" {
                    InfoLine(label: "Source", value: Text("imported (a package or a folder)"))
                } else if let origin = recipe.source {
                    let s = model.source(origin)
                    InfoLine(label: "Source", value: Text(verbatim: s?.displayName ?? origin)
                             + (s?.kind == .folder ? Text(verbatim: " · ") + Text("used as it is in the folder") : Text(verbatim: "")),
                             full: s?.location ?? origin)
                }
                if let license = recipe.license { InfoLine(label: "License", value: Text(verbatim: license)) }
            }
            .textSelection(.enabled)
            // The line's versions (plan B111), always as sub-rows – so the words, export and remove are always in the
            // same place; exported recipes are what people share (plan B108)
            VStack(spacing: 0) {
                ForEach(Array(recipe.versions.enumerated()), id: \.element.id) { i, v in
                    if i > 0 { Divider() }
                    RecipeVersionRow(version: v, model: model)
                }
            }
            .background(BrandTokens.Colors.inset, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.small, style: .continuous))
            HStack(spacing: BrandTokens.Spacing.l) {
                if let u = model.update(for: recipe.id) {
                    ActionButton(icon: "arrow.down.circle", title: "Update") { model.installRecipe(u) }
                }
                Spacer()
                ActionButton(icon: "trash", title: "Remove All Versions …", color: BrandStyle.red) {
                    model.removeAllVersions(recipe.line, name: recipe.displayName)
                }
                .disabled(!recipe.versions.contains { !$0.isLive && $0.images.isEmpty })
            }
        }
    }

    /// The switches a new VM starts with, in words
    private func defaults(_ d: RecipeSettings) -> String {
        var on: [String] = []
        if d.clipboard { on.append(String(localized: "Clipboard")) }
        if d.audio { on.append(String(localized: "Sound")) }
        if d.microphone { on.append(String(localized: "Microphone")) }
        if d.camera, d.kind == .nixos { on.append(String(localized: "Camera")) }
        if d.share { on.append(String(localized: "Share folder")) }
        if d.ephemeral { on.append(String(localized: "Ephemeral")) }
        return on.isEmpty ? "–" : on.joined(separator: " · ")
    }

    private var lampState: StatusLamp.State {
        if model.activeJob(for: recipe.id) != nil { return .working }
        if recipe.image == nil { return .off }
        return recipe.outdated ? .attention : .on
    }

    private var lampHelp: LocalizedStringKey {
        switch lampState {
        case .working: "being built – see Images"
        case .off: "not built yet"
        case .attention: "Recipe changed – build a new version"
        case .on: "Built – its image is in Images"
        }
    }
}

/// One version of a recipe's line (plan B111): its words (as many as tell the versions apart, the whole omaflake ID as
/// tooltip), where it is, the images built from it; export, and remove when nothing keeps it
struct RecipeVersionRow: View {
    let version: Recipe.Version
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            let words = String(IDText.short("@" + (RecipeText.words(version.id) ?? "?"), model.idLengths.recipe).dropFirst())
            Text(verbatim: words).font(.caption.monospaced()).help(Text(verbatim: version.id))
                .textSelection(.enabled)
            Text(verbatim: state).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            Spacer()
            IconButton(icon: "square.and.arrow.up", help: "Export …") { model.exportRecipe(version.id) }
            IconButton(icon: "trash", help: LocalizedStringKey(removeHelp), iconColor: BrandStyle.red) { model.removeRecipe(version) }
                .disabled(version.isLive || !version.images.isEmpty)
        }
        .padding(.horizontal, BrandTokens.Spacing.l)
        .padding(.vertical, BrandTokens.Spacing.xs)
    }

    private var state: String {
        var parts: [String] = []
        if version.isLive { parts.append(String(localized: "as it is in the folder")) }
        else if let added = version.added { parts.append(added.formatted(date: .abbreviated, time: .shortened)) }
        if version.isNewest { parts.append(String(localized: "newest")) }
        if !version.images.isEmpty { parts.append(String(localized: "images: \(version.images.count)")) }
        return parts.joined(separator: " · ")
    }

    private var removeHelp: String {
        if version.isLive { return String(localized: "Lives in a local folder – change or remove it there") }
        if !version.images.isEmpty { return String(localized: "Kept while images built from it are there") }
        return String(localized: "Remove this version …")
    }
}

/// Settings → Recipe Sources (spec §11.3, plan B107): where recipes come from, in their order (an earlier source wins
/// when two have the same recipe). Add: a GitHub repository (with "Private repository": a fine-grained token, checked
/// by the core and kept in the Keychain), the HTTPS URL of a catalog.json, or a folder.
struct RecipeSourcesSection: View {
    let model: AppModel
    @State private var input = ""
    @State private var isPrivate = false
    @State private var token = ""
    @State private var checking = false
    @State private var result: Result<String, APIError>?

    /// The input names a GitHub repository ("owner/repo", a github.com URL) – only those can be private. The core
    /// checks the input itself; this only decides whether the switch shows.
    private var looksLikeGitHub: Bool {
        let t = input.trimmingCharacters(in: .whitespaces)
        if t.contains("github.com/") { return true }
        guard !t.hasPrefix("/"), !t.hasPrefix("~"), !t.contains("://") else { return false }
        return t.split(separator: "/").count == 2
    }

    var body: some View {
        Section {
            let list = model.sources
            ForEach(Array(list.enumerated()), id: \.element.id) { i, s in
                HStack(spacing: BrandTokens.Spacing.l) {
                    Image(systemName: s.kind == .folder ? "folder" : s.kind == .github ? "chevron.left.forwardslash.chevron.right" : "globe")
                        .frame(width: 16).foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 1) {
                        HStack(spacing: BrandTokens.Spacing.s) {
                            Text(verbatim: s.displayName).lineLimit(1).truncationMode(.middle).help(s.location)
                            if s.official { Image(systemName: "checkmark.seal.fill").font(.caption).foregroundStyle(BrandStyle.cyan).help("Official recipes") }
                            if s.private {
                                Image(systemName: !s.hasToken ? "lock.trianglebadge.exclamationmark" : "lock.fill")
                                    .font(.caption).foregroundStyle(.secondary)
                                    .help(!s.hasToken ? "Private – no token in the Keychain" : "Private – token in the Keychain")
                            }
                        }
                        Text(s.kind == .folder ? "used as it is – changes count at once" : s.kind == .github ? "GitHub" : "HTTPS")
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                    Spacer()
                    IconButton(icon: "arrow.up", help: "Earlier: wins when two sources have the same recipe") { model.moveSource(s.id, up: true) }
                        .disabled(i == 0)
                    Toggle("", isOn: Binding(get: { s.enabled }, set: { model.setSource(s.id, enabled: $0) })).labelsHidden()
                        .toggleStyle(.switch).controlSize(.mini)
                    IconButton(icon: "trash", help: "Remove this source", iconColor: BrandStyle.red) { model.removeSource(s.id) }
                }
            }
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
                HStack {
                    TextField("GitHub repository, HTTPS URL or folder", text: $input)
                        .textFieldStyle(.roundedBorder)
                        .onChange(of: input) { result = nil }
                    Button("Folder …") { model.chooseRecipeFolder { input = $0 } }
                }
                if looksLikeGitHub {
                    Toggle("Private repository", isOn: $isPrivate)
                    if isPrivate {
                        SecureField("Fine-grained token (Contents: Read-only on this repository)", text: $token)
                            .textFieldStyle(.roundedBorder)
                        Text("The token is checked against the repository and kept in your Keychain – never in a file.")
                            .font(.caption2).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                HStack {
                    switch result {
                    case .success(let note)?: Label(note, systemImage: "checkmark.circle.fill").font(.caption).foregroundStyle(BrandTokens.Colors.success)
                    case .failure(let e)?: Label(e.message, systemImage: "xmark.octagon.fill").font(.caption)
                        .foregroundStyle(BrandStyle.red).fixedSize(horizontal: false, vertical: true)
                    case nil: EmptyView()
                    }
                    Spacer()
                    if checking { ProgressView().controlSize(.small) }
                    Button("Check and Add") {
                        checking = true
                        let (i, p, t) = (input, isPrivate && looksLikeGitHub, token)
                        Task {
                            result = await model.addSource(i, isPrivate: p, token: t)
                            checking = false
                            if case .success = result { input = ""; token = ""; isPrivate = false }
                        }
                    }
                    .disabled(input.isEmpty || checking || (isPrivate && looksLikeGitHub && token.isEmpty))
                }
            }
        } header: {
            Text("Recipe Sources")
        } footer: {
            Text("omaflake ships no recipes; it reads them from these sources. Recipes from other sources than omaflake/recipes are not reviewed by the omaflake team – their Nix code runs on this Mac when you build them.")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A recipe a remote source offers that is not installed: its lamp is off, the action downloads it (plan B107)
struct OfferedRecipeRow: View {
    let recipe: CatalogRecipe
    let model: AppModel

    private var source: Source? { model.source(recipe.source) }
    private var sourceName: String { source?.displayName ?? recipe.source }
    /// Official: by omaflake *and* from omaflake/recipes – an author name alone is not enough
    private var official: Bool { source?.official == true && recipe.author == "omaflake" }

    var body: some View {
        ExpandableRow {
            HStack(spacing: BrandTokens.Spacing.xl) {
                StatusLamp(state: model.installingRecipes.contains(recipe.id) ? .working : .off)
                    .help(model.installingRecipes.contains(recipe.id) ? "Downloading …" : "Not installed")
                GuestKindIcon(kind: recipe.kind).frame(width: 16)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    HStack(spacing: BrandTokens.Spacing.s) {
                        Text(verbatim: recipe.name).fontWeight(.medium).foregroundStyle(.secondary)
                        Text(verbatim: "· \(recipe.author)").font(.caption).foregroundStyle(.secondary)
                        if official {
                            Image(systemName: "checkmark.seal.fill").font(.caption).foregroundStyle(BrandStyle.cyan).help("Official recipe")
                        }
                        if recipe.lab {
                            Image(systemName: "flask").font(.caption).foregroundStyle(BrandStyle.yellow).help("Development recipe (developer mode)")
                        }
                        if !recipe.runsHere { ArchitectureBadge(systems: recipe.systems) }
                    }
                    if !recipe.tags.isEmpty { TagList(tags: recipe.tags) }
                }
                Spacer()
                IconButton(icon: "arrow.down.circle", help: "Install from \(sourceName)") { model.installRecipe(recipe) }
                    .disabled(model.installingRecipes.contains(recipe.id))
            }
        } detail: {
            DetailBox {
                Text(recipe.description).font(.caption).fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                    InfoLine(label: "ID", value: Text(verbatim: recipe.qualified))
                    InfoLine(label: "Source", value: Text(verbatim: sourceName) + (official ? Text(verbatim: " · ") + Text("official") : Text(verbatim: "")))
                    InfoLine(label: "Architectures", value: Text(verbatim: recipe.systems.joined(separator: ", ")))
                    if let license = recipe.license { InfoLine(label: "License", value: Text(verbatim: license)) }
                    if !recipe.requires.isEmpty {
                        InfoLine(label: "Requires", value: Text(verbatim: recipe.requires.joined(separator: ", ")))
                    }
                    InfoLine(label: "Files", value: Text("\(recipe.files.count) files, \(ByteCountFormatter.string(fromByteCount: recipe.files.reduce(0) { $0 + $1.size }, countStyle: .file))"))
                    if let homepage = recipe.homepage, !homepage.isEmpty { InfoLine(label: "Homepage", value: Text(verbatim: homepage), full: homepage) }
                }
                .textSelection(.enabled)
                if !official {
                    Text("Not reviewed by the omaflake team – its Nix code runs on this Mac when you build it.")
                        .font(.caption2).foregroundStyle(BrandTokens.Colors.warning).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

/// A VM, image or recipe this omaflake cannot read – too old, too new or damaged (plan B114)
struct UnreadableHint: View {
    let name: String
    let reason: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning)
            Text(verbatim: "\(name): \(reason)").font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
        }
        .padding(BrandTokens.Spacing.cardPadding)
    }
}

/// Newer versions of downloaded recipes, found when the sources are looked into (plan B111, B114): taken over – the
/// older one goes unless an image uses it – or added next to it when older versions are kept
struct RecipeUpdates: View {
    let updates: [AvailableRecipes.Update]
    let model: AppModel

    var body: some View {
        let keep = model.keepRecipeVersions
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(spacing: BrandTokens.Spacing.l) {
                Image(systemName: "arrow.down.circle.fill").foregroundStyle(BrandStyle.cyan)
                Text("Newer recipe versions in your sources").font(.callout)
                Spacer()
                if updates.count > 1 {
                    ActionButton(icon: "arrow.down.circle", title: keep ? "Add All" : "Update All") {
                        for u in updates { model.installRecipe(u.recipe) }
                    }
                }
            }
            ForEach(updates, id: \.id) { u in
                HStack(spacing: BrandTokens.Spacing.l) {
                    GuestKindIcon(kind: u.recipe.kind).frame(width: 16)
                    Text(verbatim: RecipeText.qualified(u.id)).font(.caption)
                    Text(verbatim: "· \(model.source(u.recipe.source)?.displayName ?? u.recipe.source)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    Spacer()
                    ActionButton(icon: "arrow.down.circle", title: keep ? "Add Version" : "Update") {
                        model.installRecipe(u.recipe)
                    }
                    .disabled(model.installingRecipes.contains(u.recipe.id))
                }
            }
            Text(keep ? "The new version is added; the older ones stay (Settings: keep older recipe versions)."
                      : "The new version takes over; the older one goes unless an image is built from it.")
                .font(.caption2).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .padding(BrandTokens.Spacing.cardPadding)
    }
}

/// The architectures of a recipe that does not run on this Mac (plan B114), e.g. "x86_64"
struct ArchitectureBadge: View {
    let systems: [String]

    var body: some View {
        Text(verbatim: systems.map { String($0.split(separator: "-").first ?? Substring($0)) }.joined(separator: ", "))
            .font(.caption2).foregroundStyle(BrandTokens.Colors.warning).lineLimit(1).fixedSize()
            .padding(.horizontal, 5).padding(.vertical, 1)
            .background(BrandTokens.Colors.warning.opacity(BrandTokens.Opacity.tintFill), in: Capsule())
            .help("Not for this Mac – built for \(systems.joined(separator: ", "))")
    }
}

/// A recipe's tags as small capsules in one line; those that don't fit become "+n" (plan B97)
struct TagList: View {
    let tags: [String]

    var body: some View {
        // The longest prefix of the tags that fits, the rest counted
        ViewThatFits(in: .horizontal) {
            ForEach((1...tags.count).reversed(), id: \.self) { n in
                HStack(spacing: BrandTokens.Spacing.xs) {
                    ForEach(tags.prefix(n), id: \.self) { chip($0) }
                    if n < tags.count { Text(verbatim: "+\(tags.count - n)").font(.caption2).foregroundStyle(.tertiary) }
                }
            }
        }
        .help(tags.joined(separator: ", "))
    }

    private func chip(_ tag: String) -> some View {
        Text(verbatim: tag).font(.caption2).foregroundStyle(.secondary).lineLimit(1).fixedSize()
            .padding(.horizontal, 5).padding(.vertical, 1)
            .background(Color.primary.opacity(0.07), in: Capsule())
    }
}

/// Every list row works the same (plan B94): a click on the row – not on its buttons – opens its details below,
/// the chevron on the right shows whether it is open.
struct ExpandableRow<Header: View, Detail: View>: View {
    @ViewBuilder let header: Header
    @ViewBuilder let detail: Detail
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: BrandTokens.Spacing.xl) {
                header
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
                    .frame(width: 14)
            }
            .padding(.horizontal, BrandTokens.Spacing.xl).padding(.vertical, 7)
            .contentShape(Rectangle())
            .onTapGesture { withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() } }
            if expanded {
                detail
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.leading, 36).padding(.trailing, BrandTokens.Spacing.xl).padding(.bottom, BrandTokens.Spacing.xl)
                    .transition(.opacity)
            }
        }
    }
}

/// The box that holds a row's details: one set-off area, the same for VMs, images and recipes
struct DetailBox<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.l) { content }
            .padding(BrandTokens.Spacing.cardPadding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(BrandTokens.Opacity.subtleFill), in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
    }
}

/// "Label  value" with a fixed label column; long values are shortened in the middle, the whole one is the tooltip
struct InfoLine: View {
    let label: LocalizedStringKey
    let value: Text
    var full: String? = nil

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.xl) {
            Text(label).foregroundStyle(.secondary).lineLimit(1).frame(width: 80, alignment: .trailing)
            value.lineLimit(1).truncationMode(.middle)
                .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                .help(full.map { Text(verbatim: $0) } ?? Text(verbatim: ""))
        }
        .font(.caption)
    }
}

/// An action in a row's details: symbol and name in a thin frame; red for removing (macOS would draw a system
/// button's title in the label colour, whatever its role)
struct ActionButton: View {
    let icon: String
    let title: LocalizedStringKey
    var color: Color = .primary
    let action: () -> Void
    @Environment(\.isEnabled) private var enabled
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: BrandTokens.Spacing.s) {
                Image(systemName: icon).font(BrandTokens.Fonts.buttonIcon)
                Text(title).font(.caption).lineLimit(1)
            }
            .foregroundStyle(color)
            .padding(.horizontal, BrandTokens.Spacing.l).frame(height: 22)
            .background(hovering && enabled ? color.opacity(BrandTokens.Opacity.selected) : .clear,
                        in: RoundedRectangle(cornerRadius: BrandTokens.Radius.small, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: BrandTokens.Radius.small, style: .continuous)
                .strokeBorder(color == .primary ? BrandStyle.cardBorder : color.opacity(BrandTokens.Opacity.tintedBorder)))
            .opacity(enabled ? 1 : BrandTokens.Opacity.disabled)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

/// A switch as a small chip: icon and name, cyan while on
struct FeatureChip: View {
    let icon: String
    let title: LocalizedStringKey
    let isOn: Bool
    var help: LocalizedStringKey = ""
    let toggle: () -> Void
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: BrandTokens.Spacing.s) {
                Image(systemName: icon).font(BrandTokens.Fonts.buttonIcon)
                Text(title).font(.caption).lineLimit(1)
            }
            .foregroundStyle(isOn ? .primary : .secondary)
            .padding(.horizontal, BrandTokens.Spacing.l).frame(height: 22)
            .background(isOn ? BrandStyle.cyan.opacity(BrandTokens.Opacity.toggleOnFill) : .clear, in: Capsule())
            .overlay(Capsule().strokeBorder(isOn ? BrandStyle.cyan.opacity(BrandTokens.Opacity.toggleOnBorder) : BrandStyle.cardBorder))
            .opacity(enabled ? 1 : 0.45)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help(help)
    }
}

/// What a VM has switched on, at a glance in its row: clipboard, sound, microphone (orange: it listens), camera,
/// ephemeral, network other than shared. Off means no icon – a missing microphone shows by its absence.
struct FeatureIcons: View {
    let vm: Vm

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.s) {
            if vm.clipboard { icon("doc.on.clipboard", "Shared clipboard on") }
            if vm.audio { icon("speaker.wave.2.fill", "Sound on") }
            if vm.microphone { icon("mic.fill", "Microphone on", BrandTokens.Colors.warning) }
            if vm.camera && vm.kind == .nixos { icon("camera.fill", "Camera on", BrandTokens.Colors.warning) }
            if vm.ephemeral { icon("sparkles", "Ephemeral instance") }
            if vm.network == .isolated { icon("lock.shield", "Network isolated") }
            if vm.network == .none { icon("wifi.slash", "No network") }
        }
    }

    private func icon(_ name: String, _ help: LocalizedStringKey, _ color: Color = .secondary) -> some View {
        Image(systemName: name).font(.system(size: BrandTokens.Icon.badge, weight: .semibold)).foregroundStyle(color).help(help)
    }
}

/// Section of the main window: title, count ("3/6" while filtered) and its tools on the right (filter, folder,
/// clear), rows in a rounded box.
struct ListSection<Accessory: View, Content: View>: View {
    let title: LocalizedStringKey
    let count: Int
    /// All rows before the filter (nil = not filtered)
    var of: Int? = nil
    /// The list's filter: right next to the title and count, apart from the actions
    var filter: AnyView? = nil
    /// Actions (import, refresh, …) on the right
    @ViewBuilder let accessory: Accessory
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(alignment: .firstTextBaseline) {
                Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                if let of, of != count {
                    Text(verbatim: "\(count)/\(of)").font(.caption).foregroundStyle(BrandStyle.cyan)
                } else if count > 0 {
                    Text(verbatim: "\(count)").font(.caption).foregroundStyle(.tertiary)
                }
                if let filter { filter.frame(height: 24) }
                Spacer()
                accessory.frame(height: 24)
            }
            VStack(spacing: 0) { content }
                .background(BrandStyle.card, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous).strokeBorder(BrandStyle.cardBorder))
        }
    }
}

/// The filter of a list (plan B101): status, guest kind, and a tag (recipes) or a recipe (images). The symbol is
/// filled while a filter is set.
struct FilterMenu: View {
    let model: AppModel
    let tab: AppModel.HomeTab
    let on: LocalizedStringKey
    let off: LocalizedStringKey
    var tags: [String] = []
    var recipes: [(id: String, name: String)] = []
    /// Offer "Other architectures" (the recipes list, plan B114)
    var architectures = false

    private var filter: Binding<AppModel.ListFilter> {
        Binding(get: { model.filters[tab] ?? AppModel.ListFilter() }, set: { model.filters[tab] = $0 })
    }

    var body: some View {
        Menu {
            Picker("Status", selection: filter.status) {
                Text("All").tag(Bool?.none)
                Text(on).tag(Bool?.some(true))
                Text(off).tag(Bool?.some(false))
            }
            .pickerStyle(.inline)
            Picker("System", selection: filter.kind) {
                Text("All").tag(GuestKind?.none)
                Label("NixOS", systemImage: "snowflake").tag(GuestKind?.some(.nixos))
                Label("macOS", systemImage: "apple.logo").tag(GuestKind?.some(.macos))
            }
            .pickerStyle(.inline)
            if !tags.isEmpty {
                Picker("Tag", selection: filter.tag) {
                    Text("All").tag(String?.none)
                    ForEach(tags, id: \.self) { Text(verbatim: $0).tag(String?.some($0)) }
                }
                .pickerStyle(.inline)
            }
            if !recipes.isEmpty {
                Picker("Recipe", selection: filter.recipe) {
                    Text("All").tag(String?.none)
                    ForEach(recipes, id: \.id) { Text(verbatim: $0.name).tag(String?.some($0.id)) }
                }
                .pickerStyle(.inline)
            }
            if architectures {
                Divider()
                Toggle("Other Architectures", isOn: filter.otherSystems)
            }
            Divider()
            Button("Reset Filter") { model.filters[tab] = nil }.disabled(!filter.wrappedValue.isActive)
        } label: {
            Image(systemName: filter.wrappedValue.isActive ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                .font(BrandTokens.Fonts.toolbarIcon)
                .foregroundStyle(filter.wrappedValue.isActive ? BrandStyle.cyan : .primary)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight)
        .help("Filter")
    }
}

/// A list whose filter leaves nothing: say so, with a way back
struct FilteredOut: View {
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Image(systemName: "line.3.horizontal.decrease.circle").foregroundStyle(.tertiary)
            Text("Nothing matches the filter.").font(.callout).foregroundStyle(.secondary)
            Spacer()
            Button("Reset Filter") { model.filters[model.tab] = nil }.controlSize(.small)
        }
        .padding(BrandTokens.Spacing.cardPadding)
    }
}

/// Small borderless icon button with a hover highlight and an optional count badge.
struct IconButton: View {
    let icon: String
    let help: LocalizedStringKey
    var badge = 0
    /// A small symbol at the bottom right (e.g. "arrow.clockwise": does it again)
    var subIcon: String? = nil
    var subIconColor: Color = .secondary
    /// Colour of the main symbol (nil = the usual label colour)
    var iconColor: Color? = nil
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: icon)
                .font(BrandTokens.Fonts.toolbarIcon)
                .foregroundStyle(iconColor.map(AnyShapeStyle.init) ?? AnyShapeStyle(.primary))
                .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight)
                .background(hovering ? Color.primary.opacity(BrandTokens.Opacity.hover) : .clear, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
                .overlay(alignment: .bottomTrailing) {
                    if let subIcon {
                        Image(systemName: subIcon)
                            .font(.system(size: BrandTokens.Icon.subIcon, weight: .heavy)).foregroundStyle(subIconColor)
                            .padding(1.5).background(Circle().fill(BrandStyle.base))
                            .offset(x: 1, y: 1)
                    }
                }
                .overlay(alignment: .topTrailing) {
                    if badge > 0 {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: BrandTokens.Icon.subBadge, weight: .bold)).foregroundStyle(BrandTokens.Colors.warning)
                            .background(Circle().fill(.background).padding(-1))
                            .offset(x: 1, y: -1)
                    }
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(help)
    }
}

/// Puts the enclosing scroll view on overlay scrollers: with a mouse attached macOS uses legacy scrollers, which take
/// width as soon as the content gets taller than the window – the whole list would jump sideways. Overlay scrollers
/// float above the content and can still be dragged. macOS resets the style when the preference changes, so it is set again.
struct OverlayScrollers: NSViewRepresentable {
    final class Probe: NSView {
        override init(frame: NSRect) {
            super.init(frame: frame)
            // Selector observers are removed with the view (macOS 10.11+)
            NotificationCenter.default.addObserver(self, selector: #selector(styleChanged),
                                                   name: NSScroller.preferredScrollerStyleDidChangeNotification, object: nil)
        }

        required init?(coder: NSCoder) { fatalError("not used") }

        @objc private func styleChanged() { DispatchQueue.main.async { self.apply() } }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            apply()
        }

        override func layout() {
            super.layout()
            apply()
        }

        func apply() {
            guard let scroll = enclosingScrollView, scroll.scrollerStyle != .overlay else { return }
            scroll.scrollerStyle = .overlay
        }

    }

    func makeNSView(context: Context) -> Probe { Probe() }
    func updateNSView(_ view: Probe, context: Context) { view.apply() }
}

/// Orange warning sign that opens the Doctor; its tooltip names what the Doctor reports.
struct DoctorWarning: View {
    let blockers: [Prerequisite]
    @Environment(AppModel.self) private var nav

    var body: some View {
        Button {
            nav.show(.doctor)
        } label: {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: BrandTokens.Icon.toolbar)).foregroundStyle(BrandTokens.Colors.warning)
                .frame(width: BrandTokens.Size.iconButtonWidth, height: BrandTokens.Size.iconButtonHeight).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(Text(DoctorWarning.text(blockers)))
    }

    static func text(_ blockers: [Prerequisite]) -> String {
        String(localized: "Open the Doctor first: \(blockers.map { RequirementText.title($0.item) }.joined(separator: ", "))")
    }
}

struct EmptyHint: View {
    let icon: String
    let text: LocalizedStringKey

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Image(systemName: icon).foregroundStyle(.tertiary)
            Text(text).font(.callout).foregroundStyle(.secondary)
            Spacer()
        }
        .padding(BrandTokens.Spacing.cardPadding)
    }
}

/// One line of Recent Activity: time and what happened.
struct ActivityRow: View {
    let activity: Activity

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
            Text(activity.date, format: .dateTime.hour().minute()).font(.caption.monospacedDigit()).foregroundStyle(.tertiary)
            Text(activity.text).font(.caption).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, BrandTokens.Spacing.xl).padding(.vertical, BrandTokens.Spacing.m)
    }
}

/// The status lamp that opens every row (plan B99), one meaning per colour everywhere: green = on / active / built,
/// grey = off / not active / not built, orange = needs attention, cyan pulsing = something is happening.
struct StatusLamp: View {
    enum State: Equatable { case on, off, attention, working }
    let state: State
    @State private var pulse = false

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 8, height: 8)
            .shadow(color: state == .off ? .clear : color.opacity(0.7), radius: state == .off ? 0 : 3)
            .opacity(state == .working && pulse ? 0.35 : 1)
            .frame(width: 16, height: 18)
            .onAppear { startPulse() }
            .onChange(of: state) { startPulse() }
    }

    private var color: Color {
        switch state {
        case .on: BrandTokens.Colors.success
        case .off: Color.secondary.opacity(0.35)
        case .attention: BrandTokens.Colors.warning
        case .working: BrandStyle.cyan
        }
    }

    private func startPulse() {
        guard state == .working else { pulse = false; return }
        withAnimation(.easeInOut(duration: 0.8).repeatForever(autoreverses: true)) { pulse = true }
    }
}

struct InstanceRow: View {
    let row: Vm
    let busy: Bool
    let model: AppModel

    var body: some View {
        ExpandableRow {
            header
        } detail: {
            InstanceDetail(row: row, busy: busy, model: model)
        }
    }

    private var header: some View {
        HStack(spacing: BrandTokens.Spacing.xl) {
            StatusLamp(state: busy ? .working : row.status == .running ? .on : .off)
                .help(busy ? "Starting or stopping …" : row.status == .running ? "Running" : "Stopped")
            GuestKindIcon(kind: row.kind).frame(width: 16)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                HStack(spacing: BrandTokens.Spacing.m) {
                    Text(row.name).fontWeight(.medium)
                    if let location = row.location {
                        Image(systemName: "externaldrive").font(.caption).foregroundStyle(.secondary)
                            .help(Text(verbatim: (location as NSString).abbreviatingWithTildeInPath))
                    }
                    FeatureIcons(vm: row)
                    if row.upgradable {
                        Image(systemName: "arrow.up.circle.fill").font(.caption).foregroundStyle(BrandTokens.Colors.warning).help("Newer image available")
                    }
                }
                Text(verbatim: ([model.recipeName(row.recipe), "\(row.cpus) CPU", "\(row.memoryGB) GB"] + [ssh].compactMap { $0 })
                    .joined(separator: " · "))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            // The share folder is there whether or not the VM runs
            if row.share {
                IconButton(icon: "folder", help: "Show Share Folder") { model.revealShare(row.id) }
            }
            if busy {
                ProgressView().controlSize(.small)
            } else if row.status == .running {
                // Same places as the two start icons: the terminal takes headless start's place, stop the window start's
                IconButton(icon: "terminal", help: "Open Terminal with SSH") { model.openTerminal(row.id) }
                IconButton(icon: "stop.fill", help: "Shut down", iconColor: BrandStyle.red) { model.stop(row.id) }
            } else if !model.startBlockers(row.kind).isEmpty {
                DoctorWarning(blockers: model.startBlockers(row.kind))
            } else {
                let limit = row.isMac ? model.macLimitReason : nil
                IconButton(icon: "terminal", help: LocalizedStringKey(limit ?? String(localized: "Start Headless")), subIcon: "play.fill", subIconColor: BrandStyle.cyan) {
                    model.start(row.id, window: false)
                }
                .disabled(!model.canStart(row))
                IconButton(icon: "desktopcomputer", help: LocalizedStringKey(limit ?? String(localized: "Start in Window")), subIcon: "play.fill", subIconColor: BrandStyle.cyan) {
                    model.start(row.id, window: true)
                }
                .disabled(!model.canStart(row))
            }
        }
    }

    /// NixOS: forwarded port on 127.0.0.1; macOS: NAT address while running (the guest kind is the icon in front)
    private var ssh: String? {
        guard row.isMac else { return row.sshPort.map { "SSH \($0)" } }
        return row.address.map { "SSH \($0)" }
    }
}

/// A VM's details (plan B94): its switches as chips, what it is built from and how to reach it, and the actions that
/// were in its "…" menu – reset, switch to a newer image, remove.
struct InstanceDetail: View {
    let row: Vm
    let busy: Bool
    let model: AppModel
    /// The SSH command as the core gives it (`vms.ssh`)
    @State private var sshCommand: String?

    var body: some View {
        let c = row
        DetailBox {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.s) {
                HStack(spacing: BrandTokens.Spacing.m) {
                    FeatureChip(icon: "doc.on.clipboard", title: "Clipboard", isOn: c.clipboard, help: "Shared Clipboard") {
                        model.setSound(row.id, clipboard: !c.clipboard)
                    }
                    Divider().frame(height: 14)
                    FeatureChip(icon: "speaker.wave.2.fill", title: "Sound", isOn: c.audio) {
                        model.setSound(row.id, audio: !c.audio)
                    }
                    FeatureChip(icon: "mic.fill", title: "Microphone", isOn: c.microphone) {
                        model.setSound(row.id, audio: c.microphone ? nil : true, microphone: !c.microphone)
                    }
                    FeatureChip(icon: "camera.fill", title: "Camera", isOn: c.camera && !row.isMac,
                                help: row.isMac ? "The camera for macOS guests comes with a signed release" : "") {
                        model.setSound(row.id, camera: !c.camera)
                    }
                    .disabled(row.isMac)   // macOS guests: camera extension after M12 (plan)
                }
                if row.status == .running {
                    Text("Changes take effect at the next start.").font(.caption2).foregroundStyle(.secondary)
                }
            }
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                InfoLine(label: "Image", value: row.upgradable
                         ? Text(verbatim: c.image + " · ") + Text("newer image available").foregroundColor(BrandTokens.Colors.warning)
                         : Text(verbatim: c.image), full: c.image)
                InfoLine(label: "omaflake ID", value: Text(verbatim: IDText.short(c.omaflakeId, model.idLengths.vm)), full: c.omaflakeId)
                InfoLine(label: "Recipe", value: Text(verbatim: model.recipe(c.recipe)?.label ?? RecipeText.qualified(c.recipe)),
                         full: c.recipe)
                InfoLine(label: "Resources", value: Text(verbatim: "\(c.cpus) CPU · \(c.memoryGB) GB RAM · \(c.diskGB) GB"))
                InfoLine(label: "Network", value: Text(verbatim: c.network.rawValue))
                let ssh = sshCommand ?? (row.isMac && row.address == nil ? String(localized: "address while running") : "…")
                InfoLine(label: "SSH", value: Text(verbatim: ssh), full: ssh)
                if c.share, let dir = c.sharePath {
                    InfoLine(label: "Share folder", value: Text(verbatim: (dir as NSString).abbreviatingWithTildeInPath), full: dir)
                }
                if c.ephemeral {
                    InfoLine(label: "Ephemeral", value: Text("changes are discarded on power-off"))
                }
                InfoLine(label: "Created", value: Text(c.created.formatted(date: .abbreviated, time: .shortened)))
                if let location = row.location {
                    InfoLine(label: "Runs from", value: Text(verbatim: (location as NSString).abbreviatingWithTildeInPath), full: location)
                }
                if let agent = row.agent, let hello = agent.hello {
                    InfoLine(label: "Agent", value: agent.agentIsOlder
                             ? Text(verbatim: "\(hello.agent) · ") + Text("older than this omaflake – rebuild the image").foregroundColor(BrandTokens.Colors.warning)
                             : Text(verbatim: "\(hello.agent) · \(hello.osName ?? hello.os ?? "")"))
                    if let status = agent.status {
                        let line = Self.guestLine(status)
                        InfoLine(label: "Guest", value: Text(verbatim: line), full: line)
                    }
                }
            }
            .textSelection(.enabled)
            // Export (a snapshot, plan B108) on the left; what changes or ends the VM on the right
            HStack(spacing: BrandTokens.Spacing.l) {
                ActionButton(icon: "square.and.arrow.up", title: "Export …") { model.exportVM(row.id) }
                Spacer()
                if row.location != nil {
                    ActionButton(icon: "eye.slash", title: "Stop Listing") { model.forget(row.id) }
                }
                if row.upgradable {
                    ActionButton(icon: "arrow.up.circle", title: "Upgrade to New Image …") { model.upgrade(row.id) }
                }
                ActionButton(icon: "arrow.counterclockwise", title: "Reset …") { model.reset(row.id) }
                ActionButton(icon: "trash", title: "Remove …", color: BrandStyle.red) { model.remove(row.id) }
            }
            .disabled(row.status == .running || busy)
            .help(row.status == .running ? "Shut the VM down first" : "")
        }
        // macOS guests have an address only while running
        .task(id: "\(row.status.rawValue) \(row.address ?? "")") {
            if row.isMac && row.address == nil { sshCommand = nil; return }
            sshCommand = await model.sshLine(row.id)
        }
    }

    /// Load, free memory and disk, addresses – as the agent reported them (plan B96)
    static func guestLine(_ s: AgentStatus) -> String {
        let bytes = { (n: UInt64) in ByteCountFormatter.string(fromByteCount: Int64(clamping: n), countStyle: .memory) }
        var parts: [String] = []
        if let load = s.load?.first { parts.append(String(localized: "load \(load.formatted(.number.precision(.fractionLength(2))))")) }
        if let m = s.memory { parts.append(String(localized: "\(bytes(m.available)) of \(bytes(m.total)) RAM free")) }
        if let d = s.disk { parts.append(String(localized: "\(bytes(d.free)) of \(bytes(d.total)) disk free")) }
        if let ipv4 = s.addresses?.filter({ $0.family == "ipv4" }).map(\.address), !ipv4.isEmpty { parts.append(ipv4.joined(separator: ", ")) }
        if let users = s.users, !users.isEmpty { parts.append(users.joined(separator: ", ")) }
        return parts.joined(separator: " · ")
    }
}

/// One built image: its full name, the recipe it comes from, when it was built, the VMs on it and whether the recipe
/// has changed since (plan B93). "New VM" creates a VM from exactly this image (plan B100).
struct ImageRowView: View {
    let image: VmImage
    let model: AppModel
    let newVM: () -> Void
    /// The recipe evaluates to something else now (nil: not known)
    private var outdated: Bool? { model.isOutdated(image) }
    private var usedBy: [String] { image.instances }

    var body: some View {
        ExpandableRow {
            row
        } detail: {
            ImageVersionInfo(version: image, outdated: outdated, usedBy: usedBy, model: model,
                             export: { model.exportImage(image.name) }) {
                model.removeImage(image.name)
            }
        }
    }

    private var row: some View {
        HStack(spacing: BrandTokens.Spacing.xl) {
            // Green: at least one VM uses this image (plan B100)
            StatusLamp(state: usedBy.isEmpty ? .off : .on)
                .help(usedBy.isEmpty ? "No VM uses this image" : "Used by VMs")
            GuestKindIcon(kind: image.kind).frame(width: 16)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                HStack(spacing: BrandTokens.Spacing.m) {
                    Text(verbatim: image.name).font(.callout.weight(.medium)).lineLimit(1).truncationMode(.middle)
                        .help(image.name)
                    if outdated == true {
                        Text("outdated").font(.caption2.weight(.semibold)).foregroundStyle(BrandTokens.Colors.warning)
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .overlay(Capsule().strokeBorder(BrandTokens.Colors.warning.opacity(BrandTokens.Opacity.tintedBorder)))
                            .help("Built from an earlier state of the recipe – a new build would differ")
                    }
                }
                HStack(spacing: BrandTokens.Spacing.s) {
                    Text("recipe \(model.recipeName(image.recipe))")
                    Text(verbatim: "·")
                    Text(image.created.formatted(date: .abbreviated, time: .shortened))
                    Text(verbatim: "·")
                    // How many VMs – their names could fill the row (they are in the tooltip and the details)
                    Group {
                        if usedBy.isEmpty {
                            Text("no VM")
                        } else {
                            HStack(spacing: BrandTokens.Spacing.xs) {
                                Image(systemName: "desktopcomputer")
                                Text(usedBy.count == 1 ? "1 VM" : "\(usedBy.count) VMs")
                            }
                            .help(usedBy.joined(separator: ", "))
                        }
                    }
                }
                .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            // The image's folder in the data folder
            IconButton(icon: "folder", help: "Show in Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: image.path)])
            }
            let startBlockers = model.startBlockers(image.kind)
            if !startBlockers.isEmpty {
                DoctorWarning(blockers: startBlockers)
            } else {
                IconButton(icon: "desktopcomputer", help: "New VM from this image", subIcon: "plus", subIconColor: BrandStyle.cyan, action: newVM)
            }
        }
    }
}

/// Build details of an image version, shown under its row (inline: a popover could close the menu window, B59).
struct ImageVersionInfo: View {
    let version: VmImage
    let outdated: Bool?
    let usedBy: [String]
    let model: AppModel
    let export: () -> Void
    /// "Remove …" at the bottom, as in a VM's details (not while VMs use the image)
    let remove: () -> Void

    var body: some View {
        let info = version
        // Fixed label column, values on one line: long paths and store names must not widen the window's content
        DetailBox {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                InfoLine(label: "omaflake ID", value: Text(verbatim: info.label), full: info.omaflakeId)
                InfoLine(label: "Recipe", value: Text(verbatim: model.recipe(info.recipe)?.label ?? RecipeText.qualified(info.recipe)),
                         full: info.recipe)
                InfoLine(label: "Built", value: Text(info.created.formatted(date: .long, time: .standard)))
                InfoLine(label: "Recipe state", value: self.revision(info.revision))
                InfoLine(label: "Status", value: status)
                InfoLine(label: "Used by", value: usedBy.isEmpty ? Text("no VM") : Text(verbatim: usedBy.joined(separator: ", ")),
                         full: usedBy.joined(separator: ", "))
                InfoLine(label: "Size", value: info.sizeBytes.map { Text(verbatim: ByteCountFormatter.string(fromByteCount: $0, countStyle: .file)) }
                         ?? Text("unknown"))
                if let source = info.source {
                    InfoLine(label: "Source", value: Text(verbatim: (source as NSString).abbreviatingWithTildeInPath), full: source)
                }
                if let store = info.storePath {
                    InfoLine(label: info.kind == .macos ? "System" : "Disk image",
                             value: Text(verbatim: (store as NSString).lastPathComponent), full: store)
                }
            }
            .textSelection(.enabled)
            HStack {
                ActionButton(icon: "square.and.arrow.up", title: "Export …", action: export)
                Spacer()
                ActionButton(icon: "trash", title: "Remove …", color: BrandStyle.red, action: remove)
                    .disabled(!usedBy.isEmpty)
            }
            // On the row, not the button: a disabled button shows no tooltip
            .contentShape(Rectangle())
            .help(usedBy.isEmpty ? "" : "Used by VMs – remove or upgrade them first")
        }
    }

    /// "abc123-dirty": built with uncommitted changes; "nogit": the recipes folder is no git repository
    private func revision(_ rev: String) -> Text {
        if rev == "nogit" { return Text("no git revision") }
        if rev.hasSuffix("-dirty") { return Text(verbatim: String(rev.dropLast(6)) + " ") + Text("(with uncommitted changes)") }
        return Text(verbatim: rev)
    }

    private var status: Text {
        switch outdated {
        case true?: Text("built from an older state of the recipe").foregroundColor(BrandTokens.Colors.warning)
        case false?: Text("matches the recipe")
        case nil: Text(verbatim: "–")
        }
    }
}

/// Guest kind at a glance: snowflake for NixOS, Apple logo for macOS.
struct GuestKindIcon: View {
    let kind: GuestKind

    var body: some View {
        Image(systemName: kind == .macos ? "apple.logo" : "snowflake")
            .foregroundStyle(.secondary)
            .help(kind == .macos ? String(localized: "macOS guest") : String(localized: "NixOS guest"))
    }
}

// MARK: - New VM

struct NewVMView: View {
    let model: AppModel
    /// The image this page creates from (set by the image row that opened it, plan B100) and its recipe
    @State private var imageName = ""
    @State private var recipe = ""
    @State private var name = ""
    @State private var settings = RecipeSettings()
    @State private var startAfter = true
    @State private var sharePath: String?
    /// Recipe whose settings are in `settings`, and the name suggested for it
    @State private var loadedRecipe = ""
    @State private var suggestedName = ""

    var body: some View {
        Form {
            Section {
                // What it is created from: exactly this image
                HStack(spacing: BrandTokens.Spacing.xl) {
                    GuestKindIcon(kind: settings.kind).frame(width: 16)
                    VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                        Text(verbatim: imageName.isEmpty ? "–" : imageName).fontWeight(.medium).lineLimit(1).truncationMode(.middle)
                            .help(imageName)
                        Text("recipe \(model.recipeName(recipe))").font(.caption).foregroundStyle(.secondary)
                    }
                }
                RecipeDescription(recipe: model.recipe(recipe))
                if !model.startBlockers(settings.kind).isEmpty { DoctorBanner(blockers: model.startBlockers(settings.kind)) }
                TextField("Name", text: $name)
                Toggle("Ephemeral (changes are discarded on shutdown)", isOn: $settings.ephemeral)
            }
            Section("Resources") {
                Stepper("CPU cores: \(settings.cpus)", value: $settings.cpus, in: 1...10)
                Stepper("RAM: \(settings.memoryGB) GB", value: $settings.memoryGB, in: 1...48)
                if isMac {
                    // The clone keeps the image's disk (APFS resize in the guest: OPEN, plan BM27)
                    LabeledContent("Disk", value: String(localized: "as the image"))
                } else {
                    Stepper("Disk: \(settings.diskGB) GB", value: $settings.diskGB, in: 10...500, step: 10)
                }
                Toggle("Sound from the VM", isOn: $settings.audio)
                Toggle("Microphone in the VM", isOn: $settings.microphone)
                    .disabled(!settings.audio)
                Toggle("Shared clipboard", isOn: $settings.clipboard)
                if !isMac {
                    Toggle("Camera in the VM", isOn: $settings.camera)
                }
                Picker("Network", selection: $settings.network) {
                    // macOS guests have no isolated mode (plan BM27)
                    ForEach(NetworkMode.allCases.filter { !isMac || $0 != .isolated }, id: \.self) { Text($0.rawValue).tag($0) }
                }
            }
            Section("Share") {
                Toggle("Share a Mac folder with the VM", isOn: $settings.share)
                if settings.share {
                    LabeledContent("Folder") {
                        HStack {
                            Text(sharePath.map { ($0 as NSString).abbreviatingWithTildeInPath }
                                 ?? "\((model.dataFolder as NSString).abbreviatingWithTildeInPath)/share/\(name.isEmpty ? "<name>" : name)")
                                .lineLimit(1).truncationMode(.middle).foregroundStyle(.secondary)
                            Button("Choose …") { model.chooseShareFolder { sharePath = $0 } }
                            if sharePath != nil { Button("Default") { sharePath = nil } }
                        }
                    }
                }
            }
            Section {
                Toggle("Start afterwards", isOn: $startAfter)
                if isMac, startAfter, let reason = model.macLimitReason {
                    Text("\(reason) – the VM is created but not started.").font(.caption).foregroundStyle(BrandTokens.Colors.warning)
                }
            }
            HStack {
                Spacer()
                Button("Cancel") { model.back() }
                Button("Create") {
                    model.create(name: name, image: imageName, settings: settings, sharePath: sharePath, startAfter: startAfter)
                    model.back()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(imageName.isEmpty || name.isEmpty)
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
        .task(id: model.newVMImage) { await select(model.newVMImage) }
    }

    private var isMac: Bool { settings.kind == .macos }

    /// Loads the image's recipe settings (the recipe's defaults for a new VM); the name follows the recipe
    private func select(_ image: String?) async {
        guard let image, let f = model.images.first(where: { $0.name == image })?.recipe else { return }
        imageName = image
        recipe = f
        guard f != loadedRecipe else { return }
        if let r = model.recipe(f) { settings = r.settings }
        loadedRecipe = f
        // A name the user typed stays; the suggested one follows the image
        if name.isEmpty || name == suggestedName {
            name = await model.nextFreeName(for: f)
            suggestedName = name
        }
    }
}

// MARK: - Build image

/// What the Doctor reports against an action, with a button to it.
struct DoctorBanner: View {
    let blockers: [Prerequisite]
    @Environment(AppModel.self) private var nav

    var body: some View {
        HStack(alignment: .top, spacing: BrandTokens.Spacing.xl) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning)
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                Text("Not possible yet – the Doctor reports:").fontWeight(.medium)
                ForEach(blockers) { p in
                    Text(verbatim: "• \(RequirementText.title(p.item)): \(RequirementText.hint(p))").font(.caption)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer()
            Button("Open Doctor …") { nav.show(.doctor) }
        }
        .padding(BrandTokens.Spacing.cardPadding)
        .background(BrandTokens.Colors.warning.opacity(BrandTokens.Opacity.tintFill), in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
    }
}

/// Shown where a macOS recipe needs a base that does not exist yet.
struct MacBaseHint: View {
    @Environment(AppModel.self) private var nav

    var body: some View {
        HStack {
            Image(systemName: "apple.logo").foregroundStyle(.secondary)
            Text("No macOS base yet – it is needed for macOS images and is created from Apple's macOS once.")
                .font(.caption).fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button("Create macOS Base …") { nav.show(.macBase) }
        }
    }
}

struct BuildView: View {
    let model: AppModel
    @State private var recipe = ""
    @State private var password = ""

    private var r: Recipe? { model.recipe(recipe) }
    private var isMac: Bool { r?.kind == .macos }
    private var blockers: [Prerequisite] { r.map { model.buildBlockers($0.kind) } ?? [] }
    /// The chosen recipe's build (running, waiting or the last one with its log)
    private var job: Job? { model.latestJob(for: recipe) }

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
            // 1. What: the recipe with its guest kind
            HStack(spacing: BrandTokens.Spacing.l) {
                // Always the recipe it was opened for – from its row in Recipes or its build in Images (plan B100)
                GuestKindIcon(kind: r?.kind ?? .nixos).frame(width: 16)
                Text(recipe.isEmpty ? "–" : model.recipeName(recipe)).font(.headline)
                RecipeBadges(lab: r?.lab ?? false, description: r?.description)
            }
            // 2. Go: password (macOS) and start/cancel – other recipes can be queued meanwhile
            HStack(spacing: BrandTokens.Spacing.l) {
                if isMac && job?.isActive != true {
                    // sudo in the build VM, once; kept only until the build starts
                    SecureField("Password of \(model.macBaseUser ?? r?.user ?? NSUserName())", text: $password)
                        .textFieldStyle(.roundedBorder)
                } else {
                    Spacer()
                }
                if let job, job.isActive {
                    Button(job.state == .queued ? "Remove from Queue" : "Cancel", role: .cancel) { model.cancel(job) }
                } else {
                    Button(model.builds.contains { $0.isActive && $0.guestKind == r?.kind } ? "Add to Queue" : "Build") {
                        model.build(recipe, password: isMac ? password : nil)
                        password = ""
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(recipe.isEmpty || !blockers.isEmpty || (isMac && (password.isEmpty || !model.hasMacBase)))
                }
            }
            if !blockers.isEmpty && job?.isActive != true { DoctorBanner(blockers: blockers) }
            // 3. Where it stands
            if let job, job.isActive {
                if job.state == .queued {
                    Label("Waiting in the queue", systemImage: "clock").font(.caption).foregroundStyle(.secondary)
                } else {
                    BuildProgressView(progress: job.progress ?? Job.Progress(), started: job.started, lastOutput: job.lastOutput)
                }
            } else if let job, job.isDone {
                BuildOutcome(job: job, model: model)
            } else if let newest = r?.image {
                Text("newest image: \(newest)").font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            }
            LogView(lines: (job.flatMap { model.jobLogs[$0.id] } ?? [])
                    + (job.map { $0.isDone && $0.hiddenLines > 0 ? [String(localized: "(\($0.hiddenLines) routine lines of the disk image tool hidden – full log: \($0.logFile ?? ""))")] : [] } ?? []))
            if let full = job?.logFile {
                HStack {
                    Spacer()
                    Button("Full Log …") { NSWorkspace.shared.open(URL(fileURLWithPath: full)) }
                        .help("Every line, also the routine ones left out above")
                }
            }
        }
        .padding(BrandStyle.margin)
        .task(id: job?.id) { if let job { await model.loadLog(job) } }
        .onAppear { recipe = model.buildRecipe ?? "" }
        .onChange(of: model.buildRecipe) { _, f in if let f { recipe = f } }
    }
}

/// How the recipe's last build ended, said plainly above its log – without it a finished build only showed its Build
/// button again and the log simply stopped
struct BuildOutcome: View {
    let job: Job
    let model: AppModel

    private var image: String? { job.result(as: BuildResult.self)?.image }

    var body: some View {
        HStack(alignment: .top, spacing: BrandTokens.Spacing.l) {
            switch job.state {
            case .finished:
                Image(systemName: "checkmark.circle.fill").foregroundStyle(BrandStyle.cyan)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Text("Image ready").font(.callout.weight(.semibold))
                    if let image {
                        Text(verbatim: model.images.first { $0.name == image }?.label ?? image)
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    }
                }
                Spacer()
                if let image, model.images.contains(where: { $0.name == image }) {
                    Button("New VM …") {
                        model.newVMImage = image
                        model.show(.newVM)
                    }
                }
            case .failed:
                Image(systemName: "xmark.octagon.fill").foregroundStyle(BrandStyle.red)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Text("Build failed").font(.callout.weight(.semibold))
                    if let error = job.error {
                        Text(verbatim: error).font(.caption).foregroundStyle(.secondary).lineLimit(3).textSelection(.enabled)
                    }
                }
                Spacer()
            case .cancelled:
                Image(systemName: "minus.circle").foregroundStyle(.secondary)
                Text("Build cancelled").font(.callout.weight(.semibold))
                Spacer()
            default:
                EmptyView()
            }
        }
        .padding(BrandTokens.Spacing.cardPadding)
        .background(BrandStyle.card, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.toast))
    }
}

/// Monospaced log that follows its last line; a real text view, so several lines can be selected and copied.
/// Step "2 / 4", what it does, packages done of announced, a bar over all steps
struct BuildProgressView: View {
    let progress: Job.Progress
    var started: Date?
    var lastOutput: Date?

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: BrandTokens.Spacing.l) {
                ProgressView().controlSize(.small)   // spinning: the build is alive
                Text(verbatim: "\(progress.step)/\(progress.total)")
                    .font(.system(.caption, design: .monospaced).weight(.bold)).foregroundStyle(BrandStyle.cyan)
                Text(LocalizedStringKey(progress.title)).font(.caption.weight(.medium))
                Spacer()
                if progress.of > 0 {
                    Text(verbatim: "\(min(progress.done, progress.of))/\(progress.of)")
                        .font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
                        .help("Packages built or downloaded")
                }
            }
            ProgressView(value: progress.fraction).progressViewStyle(.linear).tint(BrandStyle.cyan)
            // Running time and the last output, every second: long steps (disk image, copying) stay quiet for a while
            if let started {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    let quiet = Int(context.date.timeIntervalSince(lastOutput ?? started))
                    HStack(spacing: BrandTokens.Spacing.m) {
                        Text("Running for \(Self.duration(context.date.timeIntervalSince(started)))")
                        Text(verbatim: "·")
                        Text(quiet < 3 ? String(localized: "output just now") : String(localized: "last output \(Self.duration(Double(quiet))) ago"))
                            .foregroundStyle(quiet > 120 ? BrandTokens.Colors.warning : .secondary)
                            .help(quiet > 120 ? String(localized: "Some steps print nothing for minutes (writing the disk image, copying); Cancel stops the build") : "")
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

/// The build log as a terminal: near-black, monospaced, coloured by what a line is
struct LogView: View {
    let lines: [String]

    var body: some View {
        LogTextView(text: lines.joined(separator: "\n"))
            .background(BrandTokens.Colors.console, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous).strokeBorder(BrandStyle.cyan.opacity(BrandTokens.Opacity.consoleBorder)))
    }
}

struct LogTextView: NSViewRepresentable {
    let text: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        scroll.drawsBackground = false
        let tv = scroll.documentView as! NSTextView
        tv.isEditable = false
        tv.isSelectable = true
        tv.drawsBackground = false
        tv.font = .monospacedSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        tv.textContainerInset = NSSize(width: 6, height: 6)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let tv = scroll.documentView as? NSTextView, tv.string != text else { return }
        // Follow new lines only while the user is at the end (reading further up stays put)
        let atEnd = scroll.documentVisibleRect.maxY >= tv.bounds.maxY - 24
        if text.hasPrefix(tv.string) {
            tv.textStorage?.append(Self.colored(String(text.dropFirst(tv.string.count)), font: tv.font))
        } else {
            tv.textStorage?.setAttributedString(Self.colored(text, font: tv.font))
        }
        if atEnd { tv.scrollToEndOfDocument(nil) }
    }

    /// Colours per line: errors red, omaflake's own steps cyan, Nix building/copying green, the rest grey
    static func colored(_ text: String, font: NSFont?) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let red = NSColor(red: 1, green: 0.36, blue: 0.36, alpha: 1), cyan = NSColor(red: 0, green: 0.9, blue: 1, alpha: 1)
        let green = NSColor(red: 0.45, green: 0.95, blue: 0.55, alpha: 1), grey = NSColor(white: 0.62, alpha: 1)
        let yellow = NSColor(red: 0.96, green: 1, blue: 0.23, alpha: 1)
        let parts = text.split(separator: "\n", omittingEmptySubsequences: false)
        for (i, part) in parts.enumerated() {
            let line = String(part), lower = line.lowercased()
            let nixStep = ["building ", "copying path", "these ", "this "].contains { line.hasPrefix($0) }
            let derivationOutput = line.contains("> ") || line.hasPrefix("  /nix/store")
            let color: NSColor = lower.contains("error") || lower.contains("failed") || line.hasPrefix("✗") ? red
                : line.hasPrefix("warning:") ? yellow
                : nixStep ? green
                : derivationOutput ? grey
                : cyan   // omaflake's own messages
            out.append(NSAttributedString(string: line + (i < parts.count - 1 ? "\n" : ""),
                                          attributes: [.font: font as Any, .foregroundColor: color]))
        }
        return out
    }
}

// MARK: - Doctor

struct DoctorView: View {
    let model: AppModel

    var body: some View {
        Form {
            Section {
                LabeledContent("omaflake") {
                    Text(AppVersion.current).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                if let core = model.session?.omafVersion {
                    LabeledContent("Core") {
                        Text(verbatim: core).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                if let problem = model.coreProblem {
                    Label(problem, systemImage: "exclamationmark.triangle.fill").foregroundStyle(BrandTokens.Colors.warning).font(.callout)
                        .fixedSize(horizontal: false, vertical: true)
                }
                UpdateRow(model: model)
            }
            Section {
                HStack(alignment: .top) {
                    Text("omaflake uses what your Mac already has. Set up missing parts here or enter their path.")
                        .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    if model.checkingPrerequisites { ProgressView().controlSize(.small) }
                    Button("Check Again") { Task { await model.checkAll() } }.disabled(model.checkingPrerequisites)
                }
            }
            Section("Requirements") {
                ForEach(model.prerequisites) { RequirementRow(p: $0, model: model) }
            }
            Section("NixOS – before building images") {
                ForEach(model.nixosPreflight) { RequirementRow(p: $0, model: model) }
            }
            Section("macOS – before building images") {
                ForEach(model.macosPreflight) { RequirementRow(p: $0, model: model) }
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
        .onAppear {
            model.markRequirementsShown()
            Task { await model.checkAll() }
        }
    }
}

// MARK: - macOS base

/// Own window for creating the macOS base: IPSW (Apple's or a file), the guest user, progress and log.
struct MacBaseView: View {
    let model: AppModel
    @State private var user = NSUserName()
    @State private var fullName = NSFullUserName()
    @State private var password = ""
    @State private var repeated = ""
    @State private var ipsw: URL?

    private var ipswItem: Prerequisite? { model.macosPreflight.first { $0.item == .ipsw } }
    private var baseItem: Prerequisite? { model.macosPreflight.first { $0.item == .macBase } }

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
            Text("Creates your macOS base from the IPSW: your guest user, automatic login, SSH with your keys from ~/.ssh. No manual steps; a missing IPSW is downloaded first.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let baseItem, baseItem.status == .ok {
                Label(baseItem.detail, systemImage: "checkmark.circle.fill").foregroundStyle(BrandTokens.Colors.success)
            } else if let baseItem, baseItem.status == .limited {
                Label(RequirementText.hint(baseItem), systemImage: "exclamationmark.circle.fill").foregroundStyle(BrandTokens.Colors.warning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            GroupBox {
                HStack {
                    Image(systemName: "shippingbox").foregroundStyle(.secondary)
                    Text(ipsw?.lastPathComponent ?? ipswItem?.detail ?? "…").lineLimit(1).truncationMode(.middle)
                    Spacer()
                    if model.downloading {
                        DownloadProgress(model: model)
                        Button("Pause") { model.pauseDownload() }
                    } else {
                        if ipsw != nil { Button("Use Apple's") { ipsw = nil } }
                        Button("Choose IPSW …") { model.chooseIPSW { ipsw = $0 } }
                    }
                }
            } label: { Text("macOS from Apple (IPSW)") }
            GroupBox {
                Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                    GridRow { Text("Guest user"); TextField("", text: $user).labelsHidden() }
                    GridRow { Text("Full name"); TextField("", text: $fullName).labelsHidden() }
                    GridRow { Text("Password"); SecureField("", text: $password).labelsHidden() }
                    GridRow { Text("Repeat"); SecureField("", text: $repeated).labelsHidden() }
                }
                .textFieldStyle(.roundedBorder)
                .disabled(model.installing)
                .padding(BrandTokens.Spacing.s)
            } label: { Text("Guest user") }
            HStack {
                if model.installing { ProgressView().controlSize(.small); Text("running …").foregroundStyle(.secondary) }
                Spacer()
                Button("Create Base") {
                    model.installBase(user: user, fullName: fullName, password: password, ipsw: ipsw)
                    password = ""
                    repeated = ""
                }
                .keyboardShortcut(.defaultAction)
                .disabled(model.installing || model.downloading || user.isEmpty || password.isEmpty || password != repeated
                          || baseItem?.status == .ok)
            }
            LogView(lines: model.installLog).frame(minHeight: 160)
        }
        .padding(16)
        .onAppear {
            // Suggest the macOS recipes' guest user, so base and images agree from the start
            if let recipeUser = model.recipes.first(where: { $0.kind == .macos })?.user, user == NSUserName() { user = recipeUser }
            Task { await model.checkAll() }
        }
    }
}

/// "12.3 / 26.6 GB" with a small bar while the IPSW downloads.
struct DownloadProgress: View {
    let model: AppModel

    var body: some View {
        let (done, total) = model.downloadProgress
        HStack(spacing: BrandTokens.Spacing.m) {
            ProgressView(value: total > 0 ? Double(done) / Double(total) : 0).frame(width: 90)
            Text(verbatim: total > 0 ? "\(gigabytes(done)) / \(gigabytes(total))" : "…")
                .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
        }
    }
}

/// One check: a single line (status, title, what was found). Rows with actions get a chevron on the far right that
/// opens a set-off sub-line with the hint on the left and the actions right-aligned; rows without actions keep that
/// space free so all lines end evenly. Problems and running work start open. Buttons never sit in the first line.
struct RequirementRow: View {
    let p: Prerequisite
    let model: AppModel
    @Environment(AppModel.self) private var nav

    @State private var userOpen: Bool?

    /// Something to do in the sub-line: actions or a hint
    private var hasSubline: Bool {
        p.status != .ok || p.settablePath || busy || builderActions
    }
    /// Apply/clean up need nix-darwin and a builder (the row is ok then)
    private var builderActions: Bool { p.item == .linuxBuilder && p.status == .ok }
    /// A download or base creation of this row is running
    private var busy: Bool {
        (p.item == .ipsw && model.downloading) || (p.item == .macBase && model.installing)
            || (p.item == .linuxBuilder && model.builderTask != nil)
    }
    private var expanded: Bool { hasSubline && (userOpen ?? (p.status != .ok || busy)) }

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
                Image(systemName: p.status == .ok ? "checkmark.circle.fill" : p.status == .limited ? "exclamationmark.circle.fill" : "xmark.circle.fill")
                    .foregroundStyle(p.status == .ok ? BrandTokens.Colors.success : p.status == .limited ? BrandTokens.Colors.warning : .red)
                Text(RequirementText.title(p.item)).lineLimit(1).layoutPriority(1)
                Spacer(minLength: 12)
                // Paths and versions as found; for a missing item the hint in the sub-line says it all
                if p.status != .missing || [.flakeEval, .nixSpace, .macSpace, .ipsw].contains(p.item) {
                    // No text selection here: it would swallow the click that opens the row
                    Text(RequirementText.detail(p)).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle).help(RequirementText.detail(p))
                }
                Group {
                    if hasSubline {
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
            .onTapGesture { if hasSubline { withAnimation(.easeInOut(duration: 0.15)) { userOpen = !expanded } } }
            if expanded {
                HStack(alignment: .center, spacing: BrandTokens.Spacing.l) {
                    if p.status != .ok {
                        Text(RequirementText.hint(p)).font(.caption).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 12)
                    actions
                }
                .controlSize(.small)
                .padding(.horizontal, BrandTokens.Spacing.xl).padding(.vertical, BrandTokens.Spacing.m)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
                .padding(.leading, 26)
            }
        }
    }

    @ViewBuilder private var actions: some View {
        // Nix and nix-darwin: set up by omaflake through the administrator dialog (spec §11.5); others in Terminal
        if p.item == .nix && p.status == .missing {
            Button("Install …") { model.installNix() }.disabled(model.builderTask != nil)
                .help(p.install ?? "")
        } else if p.item == .nixDarwin && p.status == .missing {
            Button("Set Up …") { model.setUpNixDarwin() }
                .disabled(model.builderTask != nil || model.prerequisites.contains { $0.item == .nix && $0.status == .missing })
        } else if let command = p.install {
            Button("Install …") { model.runInTerminal(command) }.help(command)
        }
        if builderActions {
            if let task = model.builderTask {
                ProgressView().controlSize(.small)
                Text(task).font(.caption).foregroundStyle(.secondary)
            }
            Button("Clean Up …") { model.cleanUpBuilder() }.disabled(model.builderTask != nil)
                .help("Removes unused data from the Linux builder's disk (asks for an administrator)")
            Button("Apply Settings …") { model.applyBuilderSettings() }.disabled(model.builderTask != nil)
                .help("darwin-rebuild switch with omaflake's Linux builder host (asks for an administrator)")
        }
        if [.linuxBuilder, .builderRunning].contains(p.item) {
            Button("Guide") { model.show(.guide) }
        }
        if p.settablePath {
            Button("Set Path …") { model.choosePath(for: p.item) }
        }
        if p.item == .ipsw {
            if model.downloading {
                DownloadProgress(model: model)
                Button("Pause") { model.pauseDownload() }
            } else {
                Button(p.status == .limited ? "Continue Download" : "Download") { model.downloadIPSW() }
                    .disabled(model.latestRelease == nil)
            }
        }
        if p.item == .macBase {
            if model.installing { ProgressView().controlSize(.small) }
            Button(model.installing ? "Show …" : "Create …") { model.show(.macBase) }
        }
    }
}

/// Localized names and hints (the details – paths, versions – come from OmaflakeCore as they are).
enum RequirementText {

    /// Paths and versions as found; space and recipe counts in the UI language
    static func detail(_ p: Prerequisite) -> String {
        if let free = p.freeBytes, let needed = p.neededBytes {
            return String(localized: "\(gigabytes(free)) free, about \(gigabytes(needed)) needed")
        }
        if p.item == .flakeEval, p.status == .ok {
            return p.detail.replacingOccurrences(of: " recipes (", with: " " + String(localized: "recipes") + " (")
        }
        return p.detail
    }

    static func title(_ item: Prerequisite.Item) -> String {
        switch item {
        case .host: String(localized: "Mac with Apple silicon, macOS 27")
        case .nix: String(localized: "Nix")
        case .flakes: String(localized: "Nix flakes")
        case .nixDarwin: String(localized: "nix-darwin")
        case .linuxBuilder: String(localized: "Linux builder (NixOS images)")
        case .qemu: String(localized: "QEMU (NixOS guests)")
        case .flake: String(localized: "Recipes (your flake)")
        case .sshKey: String(localized: "SSH key")
        case .git: String(localized: "git (optional)")
        case .builderRunning: String(localized: "Linux builder answers")
        case .flakeEval: String(localized: "Recipes evaluate")
        case .nixSpace: String(localized: "Space in the Nix store")
        case .ipsw: String(localized: "macOS from Apple (IPSW)")
        case .macBase: String(localized: "macOS base")
        case .macSpace: String(localized: "Space for macOS")
        case .unknown: String(localized: "Other check")
        }
    }

    /// Hint for a check; the base/recipe user mismatch names both users
    static func hint(_ p: Prerequisite) -> String {
        if p.item == .macBase, let base = p.baseUser, let expected = p.expectedUser {
            return String(localized: "The base's user \(base) differs from macUser \(expected) in your image flake's settings – set macUser to \(base) in nix/local/settings.nix, or create the base again with \(expected).")
        }
        return p.item == .unknown ? p.hint : hint(p.item, p.status)
    }

    static func hint(_ item: Prerequisite.Item, _ status: Prerequisite.Status) -> String {
        switch item {
        case .host: status == .limited ? String(localized: "macOS 27 is needed for automatic macOS bases.")
            : String(localized: "omaflake needs a Mac with Apple silicon.")
        case .nix: String(localized: "Install Nix, or enter the path to the nix binary.")
        case .flakes: String(localized: "Add “nix-command flakes” to experimental-features (nix-darwin: nix.settings.experimental-features).")
        case .nixDarwin: String(localized: "Set up nix-darwin – it provides the Linux builder for NixOS images.")
        case .linuxBuilder: String(localized: "Enable nix.linux-builder in your nix-darwin configuration (see the guide).")
        case .qemu: status == .limited ? String(localized: "Works headless and in 2D – without 3D and without automatic window resolution; omaflake's QEMU adds both.")
            : String(localized: "Needed for NixOS guests: install omaflake's QEMU, or enter a path.")
        case .flake: String(localized: "Choose the folder with your recipes (flake.nix).")
        case .sshKey: String(localized: "Create a key – used for SSH into VMs, image builds and macOS bases.")
        case .git: String(localized: "Optional: without it, image names show “nogit” instead of the flake's revision.")
        case .builderRunning: String(localized: "The Linux builder VM does not answer – it is started by nix-darwin (launchd: org.nixos.linux-builder).")
        case .flakeEval: String(localized: "Your recipes do not evaluate – see the message.")
        case .nixSpace, .macSpace: String(localized: "Not enough free space – free some up first.")
        case .ipsw: status == .limited ? String(localized: "Partly downloaded – continues where it stopped.")
            : String(localized: "Download macOS from Apple, or choose an IPSW file you already have.")
        case .macBase: status == .limited ? String(localized: "Unfinished – Create Base continues it; use the same password as before.")
            : String(localized: "Create it once from the IPSW – about 5 minutes, no manual steps.")
        case .unknown: ""
        }
    }
}

// MARK: - Settings

struct SettingsView: View {
    @Bindable var model: AppModel

    var body: some View {
        Form {
            RecipeSourcesSection(model: model)
            Section("General") {
                Toggle("Launch at Login", isOn: $model.launchAtLogin)
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Toggle("Notifications", isOn: $model.notifications)
                    Text("Only while the menu window is closed: VM ready, image built or failed, VM ended. macOS asks once whether omaflake may send them.")
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Toggle("Check for updates", isOn: $model.checkForUpdates)
                    Text("Once a day – the only network access omaflake makes on its own.")
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Toggle("Keep older recipe versions", isOn: $model.keepRecipeVersions)
                    Text("When a newer version of a recipe comes (an update, an import, a change in a local folder after a build). Off: only the newest stays – and every version an image is built from, as long as the image is there.")
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxs) {
                    Toggle("Developer Mode", isOn: $model.developerMode)
                    Text("Shows the lab recipes – omaflake's development recipes with test app and debug tools, and recipes others mark with lab = true.")
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
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
                GuestField(label: "User", value: $model.guestOverrides.user, placeholder: model.guestDefaults.user ?? "")
                GuestField(label: "Full name", value: $model.guestOverrides.fullName, placeholder: model.guestDefaults.fullName ?? "")
                GuestField(label: "E-mail (git)", value: $model.guestOverrides.email, placeholder: "")
                let choices = model.guestChoices
                GuestPicker(label: "Language and region", value: $model.guestOverrides.locale, mac: model.guestDefaults.locale ?? "",
                            options: choices.locales, name: { GuestNames.locale($0) })
                GuestPicker(label: "Keyboard", value: $model.guestOverrides.keyboard, mac: model.guestDefaults.keyboard ?? "",
                            options: choices.keyboards.map(\.code), name: { GuestNames.keyboard($0, choices.keyboards) })
                GuestPicker(label: "Time zone", value: $model.guestOverrides.timeZone, mac: model.guestDefaults.timeZone ?? "",
                            options: choices.timeZones, name: { $0.replacingOccurrences(of: "_", with: " ") })
            } header: {
                Text("Guest user in the VMs")
            } footer: {
                Text("Empty fields and \"Like this Mac\" use your Mac's values. The lists offer only what both NixOS and macOS VMs support. Your SSH keys from ~/.ssh are added automatically. macOS VMs always use the user of the macOS base. A developer flake's local/settings.nix overrides these values.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Section("Data Folder") {
                LabeledContent("Location") {
                    Text((model.dataFolder as NSString).abbreviatingWithTildeInPath)
                        .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                }
                HStack {
                    Spacer()
                    if model.movingData { ProgressView().controlSize(.small) }
                    Button("Default (~/.omaflake)") { model.moveDataFolder(to: AppModel.defaultDataFolder) }
                        .disabled(model.movingData || model.isDefaultDataFolder)
                    Button("Move …") { model.chooseDataFolder { model.moveDataFolder(to: $0) } }
                        .disabled(model.movingData)
                }
                Text("By default omaflake keeps everything in ~/.omaflake and needs no access to Documents or Desktop. A folder there asks for access once.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }
}

/// Names of the guest choices in the app's language (Foundation's names; the lists come from the core)
enum GuestNames {
    /// "Deutsch (Deutschland)" for "de_DE.UTF-8"
    static func locale(_ locale: String) -> String {
        let id = locale.components(separatedBy: ".").first ?? locale
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }

    /// "Deutsch (Deutschland)" for keyboard "de" (named after its locale)
    static func keyboard(_ code: String, _ keyboards: [SettingsInfo.Choices.Keyboard]) -> String {
        guard let locale = keyboards.first(where: { $0.code == code })?.locale else { return code }
        return Locale.current.localizedString(forIdentifier: locale) ?? code
    }
}

/// A guest value: empty = the Mac's value (shown as the placeholder)
struct GuestField: View {
    let label: LocalizedStringKey
    @Binding var value: String?
    let placeholder: String

    var body: some View {
        TextField(label, text: Binding(get: { value ?? "" }, set: { value = $0 }), prompt: Text(verbatim: placeholder))
    }
}

/// A guest value from a list: "Like this Mac" (nil) or one of `options`, sorted by their shown names
struct GuestPicker: View {
    let label: LocalizedStringKey
    @Binding var value: String?
    let mac: String
    let options: [String]
    let name: (String) -> String

    var body: some View {
        Picker(label, selection: $value) {
            Text("Like this Mac – \(name(mac))").tag(String?.none)
            Divider()
            ForEach(options.sorted { name($0).localizedStandardCompare(name($1)) == .orderedAscending }, id: \.self) {
                Text(name($0)).tag(String?.some($0))
            }
        }
    }
}

// MARK: - Guides (bundled, offline)

/// The app's version from its Info.plist ("0.1 (1)").
enum AppVersion {
    static var short: String { Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0" }
    static var current: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let short = info["CFBundleShortVersionString"] as? String ?? "dev"
        let build = info["CFBundleVersion"] as? String
        return build.map { "\(short) (\($0))" } ?? short
    }
}

/// A guide from docs/, shipped in the app (Resources/Guides/<file>.md, scripts/bundle-app.sh) – no network needed.
struct GuideView: View {
    let file: String

    private var blocks: [MarkdownBlock] {
        guard let url = Bundle.main.url(forResource: file, withExtension: "md", subdirectory: "Guides"),
              let text = try? String(contentsOf: url, encoding: .utf8) else { return [] }
        return MarkdownBlock.parse(text)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xl) {
                if blocks.isEmpty {
                    Text("This guide is part of the app bundle (scripts/bundle-app.sh).").foregroundStyle(.secondary)
                }
                ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in block.view }
            }
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(BrandStyle.margin)
        }
    }
}

/// The few Markdown blocks the guides use: headings, paragraphs, list items, code blocks.
enum MarkdownBlock {
    case heading(Int, String), paragraph(String), item(String), code(String)

    static func parse(_ text: String) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var para: [String] = [], code: [String]? = nil, item: [String]? = nil
        func flush() {
            if !para.isEmpty { blocks.append(.paragraph(para.joined(separator: " "))); para = [] }
            if let i = item { blocks.append(.item(i.joined(separator: " "))); item = nil }
        }
        for raw in text.components(separatedBy: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                if let c = code { blocks.append(.code(c.joined(separator: "\n"))); code = nil } else { flush(); code = [] }
                continue
            }
            if code != nil {
                // Code inside list items is indented in the source
                code!.append(raw.hasPrefix("  ") ? String(raw.drop { $0 == " " }.isEmpty ? "" : raw.dropFirst(min(2, raw.prefix { $0 == " " }.count))) : raw)
                continue
            }
            if line.isEmpty { flush(); continue }
            if line.hasPrefix("#") {
                flush()
                let level = line.prefix { $0 == "#" }.count
                blocks.append(.heading(level, line.dropFirst(level).trimmingCharacters(in: .whitespaces)))
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                flush()
                item = [String(line.dropFirst(2))]
            } else if item != nil {
                item!.append(line)
            } else {
                para.append(line)
            }
        }
        flush()
        if let c = code { blocks.append(.code(c.joined(separator: "\n"))) }
        return blocks
    }

    /// Inline Markdown (bold, code, links); links into the repository have no target offline and stay plain text
    static func inline(_ s: String) -> AttributedString {
        var a = (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
        for run in a.runs where run.link != nil && !(run.link!.scheme?.hasPrefix("http") ?? false) {
            a[run.range].link = nil
        }
        return a
    }

    @ViewBuilder var view: some View {
        switch self {
        case .heading(let level, let text):
            Text(Self.inline(text)).font(level == 1 ? .title : level == 2 ? .title2 : .title3).bold().padding(.top, level == 1 ? 0 : 8)
        case .paragraph(let text):
            Text(Self.inline(text)).fixedSize(horizontal: false, vertical: true)
        case .item(let text):
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.m) {
                Text("•")
                Text(Self.inline(text)).fixedSize(horizontal: false, vertical: true)
            }
        case .code(let text):
            Text(text).font(.system(.callout, design: .monospaced))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(BrandTokens.Spacing.cardPadding)
                .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
        }
    }
}

// MARK: - Brand look

/// omaflake's colours (brand/README.md, brand/generate.py)
/// The app's shorthands for the design tokens (brand/tokens.toml → BrandTokens.swift, plan B119)
enum BrandStyle {
    static let red = BrandTokens.Palette.red
    static let cyan = BrandTokens.Palette.cyan
    static let yellow = BrandTokens.Palette.yellow
    static let base = BrandTokens.Colors.window
    static let card = BrandTokens.Colors.card
    /// The window's corners, close to macOS' own windows with a toolbar (Finder, Safari) – Apple documents no value,
    /// 24 pt is matched by eye (plan B44)
    static let windowRadius = BrandTokens.Radius.window
    /// Distance of the content from the window's edge
    static let margin = BrandTokens.Spacing.margin
    /// Cards and buttons
    static let radius = BrandTokens.Radius.card
    static let cardBorder = BrandTokens.Colors.cardBorder

    /// Near-black with a faint red and cyan glow from the corners, like the hero mark's RGB split
    static var background: some View {
        ZStack {
            base
            RadialGradient(colors: [BrandTokens.Colors.glowRed, .clear], center: .topLeading, startRadius: 0,
                           endRadius: BrandTokens.Glow.redRadius)
            RadialGradient(colors: [BrandTokens.Colors.glowCyan, .clear], center: .bottomTrailing, startRadius: 0,
                           endRadius: BrandTokens.Glow.cyanRadius)
        }
    }
}

/// "omaflake" in Menlo Bold with the hero's red/cyan offset
struct Wordmark: View {
    var body: some View {
        let text = Text(verbatim: "omaflake").font(BrandTokens.Fonts.wordmark)
        ZStack {
            text.foregroundStyle(BrandStyle.red).offset(x: -1, y: 0)
            text.foregroundStyle(BrandStyle.cyan).offset(x: 1, y: 0)
            text.foregroundStyle(.white)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "omaflake"))
    }
}

/// Gives the menu window omaflake's corner radius: the system's own background (larger radius) is hidden and the
/// window made clear, so the brand background's rounded shape is the window's shape.
struct RoundedWindow: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { WindowHook() }
    func updateNSView(_ nsView: NSView, context: Context) {}

    final class WindowHook: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            Notifier.menuWindow = window
            window.isOpaque = false
            window.backgroundColor = .clear
            // The panel's frame view carries a visual effect view with the system's rounded mask
            func hideEffects(_ view: NSView) {
                for sub in view.subviews {
                    if let effect = sub as? NSVisualEffectView, !(effect.isDescendant(of: self)) { effect.isHidden = true }
                    hideEffects(sub)
                }
            }
            if let frame = window.contentView?.superview {
                hideEffects(frame)
                // The frame view also draws the system's border with its own corners: clip it to ours (our own
                // hairline in MainView replaces it)
                frame.wantsLayer = true
                frame.layer?.cornerRadius = BrandStyle.windowRadius
                frame.layer?.cornerCurve = .continuous
                frame.layer?.masksToBounds = true
            }
            // The shadow follows the window's opaque pixels – recompute it once our background is drawn
            DispatchQueue.main.async { [weak window] in window?.invalidateShadow() }
        }
    }
}

/// Next to a recipe's name: a flask for development (lab) recipes, an "i" with the description as tooltip
struct RecipeBadges: View {
    let lab: Bool
    let description: String?

    var body: some View {
        if lab {
            Image(systemName: "flask").font(.caption).foregroundStyle(BrandStyle.yellow).help("Development recipe (developer mode)")
        }
        if let description {
            Image(systemName: "info.circle").font(.caption).foregroundStyle(.secondary).help(description)
        }
    }
}

/// A recipe's description below a picker
struct RecipeDescription: View {
    let recipe: Recipe?

    var body: some View {
        if let text = recipe?.description {
            HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.m) {
                if recipe?.lab == true { Image(systemName: "flask").foregroundStyle(BrandStyle.yellow) }
                Text(text).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A question inside the menu window (instead of an alert window, which would close it)
struct ConfirmationOverlay: View {
    let confirmation: AppModel.Confirmation

    var body: some View {
        ZStack {
            BrandTokens.Colors.scrim.ignoresSafeArea()
                .onTapGesture { confirmation.answer(nil) }
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
                Text(confirmation.title).font(.headline).fixedSize(horizontal: false, vertical: true)
                Text(confirmation.text).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if let input = confirmation.input {
                    @Bindable var input = input
                    TextField("", text: $input.text).textFieldStyle(.roundedBorder).onSubmit { confirmation.answer(0) }
                }
                HStack {
                    Spacer()
                    Button("Cancel") { confirmation.answer(nil) }.keyboardShortcut(.cancelAction)
                    ForEach(Array(confirmation.others.enumerated()), id: \.offset) { i, title in
                        Button(title) { confirmation.answer(i + 1) }
                    }
                    Button(confirmation.button, role: confirmation.destructive ? .destructive : nil) { confirmation.answer(0) }
                        .keyboardShortcut(.defaultAction)
                        .tint(confirmation.destructive ? BrandStyle.red : nil)
                }
            }
            .padding(BrandStyle.margin)
            .frame(maxWidth: 400)
            .background(BrandStyle.base, in: RoundedRectangle(cornerRadius: BrandTokens.Radius.sheet, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: BrandTokens.Radius.sheet, style: .continuous).strokeBorder(BrandStyle.cardBorder))
            .shadow(radius: 20)
        }
    }
}

/// Under an image row: its build waiting, running (step, packages, bar) or failed; a click opens the build page
struct BuildJobRow: View {
    let job: Job
    let model: AppModel
    /// A row of its own in the image list (with the recipe's name) rather than below a recipe
    var standalone = false
    let open: () -> Void

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Group {
                switch job.state {
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
                    if standalone {
                        Text(verbatim: job.recipe.map(model.recipeName) ?? job.title).font(.callout.weight(.medium))
                        GuestKindIcon(kind: job.guestKind ?? .nixos).font(.caption2).foregroundStyle(.secondary)
                    }
                    Text(title).font(.caption.weight(.medium))
                    if job.isRunning, let p = job.progress, p.of > 0 {
                        Text(verbatim: "\(min(p.done, p.of))/\(p.of)")
                            .font(.caption2.monospaced()).foregroundStyle(.secondary)
                    }
                }
                if job.isRunning, let p = job.progress {
                    ProgressView(value: p.fraction).progressViewStyle(.linear).tint(BrandStyle.cyan).controlSize(.mini)
                }
            }
            Spacer()
            if job.isActive {
                IconButton(icon: "xmark.circle", help: job.state == .queued ? "Remove from the queue" : "Cancel build") { model.cancel(job) }
            } else {
                IconButton(icon: "xmark", help: "Hide") { model.dismiss(job) }
            }
            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.secondary).frame(width: 14)
        }
        .padding(.leading, standalone ? 10 : 36).padding(.trailing, BrandTokens.Spacing.xl).padding(.vertical, standalone ? 7 : 5)
        .contentShape(Rectangle())
        .onTapGesture(perform: open)
        .help("Show the build and its log")
    }

    private var title: String {
        switch job.state {
        case .queued: String(localized: "Waiting to build")
        case .checking: String(localized: "Checking …")
        case .running: job.progress.map { "\($0.step)/\($0.total) " + String(localized: String.LocalizationValue($0.title)) } ?? job.title
        case .failed: String(localized: "Build failed – show log")
        case .cancelled: String(localized: "Build cancelled")
        case .finished: String(localized: "Built")
        case .unknown: job.title
        }
    }
}

/// Doctor: is there a newer omaflake? (spec §11.6)
struct UpdateRow: View {
    let model: AppModel

    var body: some View {
        LabeledContent("Updates") {
            HStack(spacing: BrandTokens.Spacing.l) {
                switch model.updateState {
                case .noFeed: Text("No update feed yet (comes with the first release)").foregroundStyle(.secondary)
                case .off: Text("Check switched off (Settings)").foregroundStyle(.secondary)
                case .checking: ProgressView().controlSize(.small)
                case .upToDate(let date):
                    Text("Up to date – checked \(date.formatted(date: .omitted, time: .shortened))").foregroundStyle(.secondary)
                case .available(let release):
                    Text("\(release.version) available").foregroundStyle(BrandStyle.cyan)
                    Button("Download …") {
                        if let url = URL(string: release.page ?? release.url) { NSWorkspace.shared.open(url) }
                    }
                case .failed(let error): Text("Check failed: \(error)").foregroundStyle(BrandTokens.Colors.warning).lineLimit(1).help(error)
                }
                if model.updateState != .checking && model.updateState != .noFeed && model.updateState != .off {
                    Button("Check Now") { Task { await model.checkForUpdate() } }
                }
            }
            .font(.callout)
        }
    }
}
