// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

@main
struct OmaflakeApp: App {
    @State private var model = AppModel()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    /// Menu bar, window or both (plan B119); `SettingsView` applies a change
    @AppStorage(AppFrame.key) private var frame: AppFrame = .menuBar

    var body: some Scene {
        MenuBarExtra(isInserted: Binding(get: { frame.showsMenuBar }, set: { _ in })) {
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

        // The same pages in a regular window with a Dock icon (plan B119). Opened at launch when the choice includes
        // it; no state restoration, so the choice alone decides what opens.
        Window("omaflake", id: AppFrame.windowID) {
            MainView(model: model, inWindow: true)
                .environment(model)
                .preferredColorScheme(.dark)
                .tint(BrandStyle.cyan)
                .buttonBorderShape(.roundedRectangle(radius: BrandStyle.radius))
                .containerBackground(for: .window) { BrandStyle.background }
                .background(MainWindowHook())
        }
        .defaultSize(width: 760, height: 720)
        .windowResizability(.contentMinSize)
        .defaultLaunchBehavior(frame.showsWindow ? .presented : .suppressed)
        .restorationBehavior(.disabled)
        .commands { MainCommands(model: model) }

        // A recipe's keys – of an image or a VM through its recipe – by module and mode (plan B147)
        Window(String(localized: "Keys"), id: KeysView.windowID) {
            KeysView(model: model)
                .environment(model)
                .preferredColorScheme(.dark)
                .tint(BrandStyle.cyan)
                .containerBackground(for: .window) { BrandStyle.background }
        }
        .defaultSize(width: 600, height: 720)
        .restorationBehavior(.disabled)
        .defaultLaunchBehavior(.suppressed)
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

    /// Opens the app window (set by `MainView` once it is shown somewhere; used for a click on the Dock icon)
    static var openMain: (() -> Void)?
    /// Opens the keys window (set by `MainView`)
    static var openKeys: (() -> Void)?

    func applicationWillTerminate(_ notification: Notification) { Self.terminate?() }

    /// Closing the window keeps omaflake running, like a normal Mac app (it quits with Cmd+Q or the power button)
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    /// A click on the Dock icon with no window open opens the app window again
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag, AppFrame.current.showsWindow { Self.openMain?() }
        return true
    }

    func applicationWillFinishLaunching(_ notification: Notification) { takeOpenEvents() }
    func applicationDidFinishLaunching(_ notification: Notification) {
        // The app starts as `LSUIElement` (no Dock icon); with the window it becomes a regular app (plan B119)
        if AppFrame.current.showsWindow {
            AppFrame.current.applyActivationPolicy()
            NSApp.activate()
        }
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


// MARK: - Main window (menu bar or app window)

struct MainView: View {
    @Bindable var model: AppModel
    /// In the app window (resizable) rather than the menu bar window (fixed size)
    var inWindow = false
    @AppStorage(AppFrame.key) private var frame: AppFrame = .menuBar
    @Environment(\.openWindow) private var openWindow

    /// Pages open one level deeper inside this window (no extra windows: those closed the menu window, plan B42)
    /// One fixed size for every page, so the window neither grows nor jumps when navigating
    static let size = CGSize(width: 500, height: 600)

    var body: some View {
        Group {
            if let page = model.path.last {
                VStack(alignment: .leading, spacing: 0) {
                    PageHeader(title: title(page), model: model)
                    Divider()
                    PageView(ref: page, model: model)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                }
            } else if inWindow {
                // The app window: a sidebar with the two groups, the open tab next to it (plan B147)
                VStack(spacing: 0) {
                    header.padding([.horizontal, .top], BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.xl)
                    HStack(alignment: .top, spacing: 0) {
                        HomeSidebar(model: model).frame(width: Self.sidebarWidth)
                        Divider()
                        content.padding(.top, BrandTokens.Spacing.s)
                    }
                }
            } else {
                // The menu bar window is narrow: the group above the tabs of the group
                VStack(spacing: 0) {
                    header.padding([.horizontal, .top], BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.xl)
                    HomeGroupSwitch(model: model).padding(.horizontal, BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.m)
                    HomeTabBar(model: model).padding(.horizontal, BrandStyle.margin).padding(.bottom, BrandTokens.Spacing.xxl)
                    content
                }
            }
        }
        .frame(minWidth: inWindow ? Self.size.width + Self.sidebarWidth : Self.size.width,
               idealWidth: inWindow ? Self.size.width + Self.sidebarWidth : Self.size.width,
               maxWidth: inWindow ? .infinity : Self.size.width,
               minHeight: Self.size.height, idealHeight: Self.size.height, maxHeight: inWindow ? .infinity : Self.size.height)
        .overlay { if let c = model.confirmation { ConfirmationOverlay(confirmation: c) } }
        .onAppear {
            AppDelegate.openMain = { [openWindow] in openWindow(id: AppFrame.windowID); NSApp.activate() }
            AppDelegate.openKeys = { [openWindow] in openWindow(id: KeysView.windowID); NSApp.activate() }
        }
    }

    static let sidebarWidth: CGFloat = 170

    /// The open tab: a list in a scroll view, or a form that scrolls itself (credentials, locales, profiles)
    @ViewBuilder private var content: some View {
        if model.tab.isForm {
            HomeList(tab: model.tab, model: model).id(model.tab)
                .overlay(alignment: .bottom) { FooterMessage(model: model) }
        } else {
            ScrollView { HomeList(tab: model.tab, model: model).padding([.horizontal, .bottom], BrandStyle.margin).background(OverlayScrollers()) }
                .id(model.tab)   // each tab opens at its top
                .overlay(alignment: .bottom) { FooterMessage(model: model) }
        }
    }

    /// The page's title: the core's (the guide's in its params)
    private func title(_ ref: PageRef) -> String {
        if ref.page == "guide" { return model.t(ref.echoed["title"]?.viewText) }
        switch model.pageModel(ref) {
        case .vm(let p)?: return model.t(p.title)
        case .image(let p)?: return model.t(p.title)
        case .doctor(let p)?: return model.t(p.title)
        case .settings(let p)?: return model.t(p.title)
        case .credentials(let p)?: return model.t(p.title)
        case .credential(let p)?: return model.t(p.title)
        case .newVm(let p)?: return model.t(p.title)
        case .build(let p)?: return model.t(p.title)
        case .macBase(let p)?: return model.t(p.title)
        default: return ""
        }
    }

    private var header: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            Image(nsImage: Brand.menuBarImage())
            Wordmark()
            Spacer()
            if frame == .both {
                if inWindow {
                    Image(systemName: "menubar.arrow.up.rectangle").foregroundStyle(.secondary)
                        .help("omaflake is in the menu bar too – Settings → Show omaflake in")
                } else {
                    IconButton(icon: "macwindow", help: String(localized: "Open in Window")) { openWindow(id: AppFrame.windowID); NSApp.activate() }
                }
            }
            ForEach(model.home?.actions ?? []) { ActionIcon(action: $0, model: model) }
        }
    }
}

/// Title row of a page: back to where the user came from (also Esc).
struct PageHeader: View {
    let title: String
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
            Text(verbatim: title).font(.headline)
            Spacer()
        }
        .padding(.horizontal, BrandStyle.margin - 6).padding(.top, BrandStyle.margin - 6).padding(.bottom, BrandTokens.Spacing.l)
    }
}

/// Tabs of the main page with what needs attention there (the core's badges: running VMs, a build running or failed,
/// recipes changed since their image) and the app's own dot for unseen activity
struct HomeTabBar: View {
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.s) {
            ForEach((model.home?.tabs ?? []).filter { $0.group == model.group }) { tab in
                if let id = AppModel.HomeTab(rawValue: tab.id) { button(id, tab) }
            }
        }
        .padding(BrandTokens.Spacing.xs)
        .background(BrandStyle.card, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous).strokeBorder(BrandStyle.cardBorder))
        .frame(minHeight: 34)
    }

    private func button(_ id: AppModel.HomeTab, _ tab: HomePage.Tab) -> some View {
        let selected = model.tab == id
        return Button { model.tab = id } label: {
            HStack(spacing: 5) {
                Image(systemName: Icons.symbol(tab.icon)?.main ?? "circle").font(.system(size: BrandTokens.Icon.tab, weight: .medium))
                Text(verbatim: model.t(tab.label)).font(.callout.weight(selected ? .semibold : .regular)).lineLimit(1)
                badge(tab.badge)
                if id == .activity && model.unseenActivity {
                    Circle().fill(BrandStyle.cyan).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot).help("New messages")
                }
            }
            .foregroundStyle(selected ? .primary : .secondary)
            .frame(maxWidth: .infinity).frame(height: 26)
            .background(selected ? Color.primary.opacity(BrandTokens.Opacity.selected) : .clear,
                        in: RoundedRectangle(cornerRadius: BrandTokens.Radius.tab, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder private func badge(_ b: HomePage.TabBadge?) -> some View {
        if let b {
            let help = Text(verbatim: model.t(b.help))
            switch b.kind {
            case "count": CountBadge(count: b.count ?? 0, color: b.tone.color).help(help)
            case "working": ProgressView().controlSize(.mini).help(help)
            case "failed": Image(systemName: "xmark.circle.fill").font(.caption2).foregroundStyle(b.tone.color).help(help)
            default: Circle().fill(b.tone.color).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot).help(help)
            }
        }
    }
}

/// The menu bar window's choice between the groups (plan B147): what runs, and the workshop
struct HomeGroupSwitch: View {
    let model: AppModel

    var body: some View {
        HStack(spacing: BrandTokens.Spacing.l) {
            ForEach(model.home?.groups ?? []) { g in
                let selected = model.group == g.id
                Button { model.selectGroup(g.id) } label: {
                    HStack(spacing: BrandTokens.Spacing.s) {
                        Text(verbatim: model.t(g.title)).font(.caption.weight(selected ? .semibold : .regular))
                        if g.attention && !selected {
                            Circle().fill(BrandTokens.Colors.warning).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot)
                        }
                    }
                    .foregroundStyle(selected ? BrandStyle.cyan : .secondary)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            Spacer()
        }
    }
}

/// The app window's sidebar (plan B147): the groups with their tabs and what needs attention there
struct HomeSidebar: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xxl) {
                ForEach(model.home?.groups ?? []) { g in
                    VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                        Text(verbatim: model.t(g.title).uppercased()).font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
                            .padding(.horizontal, BrandTokens.Spacing.l)
                        ForEach((model.home?.tabs ?? []).filter { $0.group == g.id }) { tab in
                            if let id = AppModel.HomeTab(rawValue: tab.id) { item(id, tab) }
                        }
                    }
                }
            }
            .padding(BrandTokens.Spacing.l)
        }
    }

    private func item(_ id: AppModel.HomeTab, _ tab: HomePage.Tab) -> some View {
        let selected = model.tab == id
        return Button { model.tab = id } label: {
            HStack(spacing: BrandTokens.Spacing.l) {
                Image(systemName: Icons.symbol(tab.icon)?.main ?? "circle").font(.system(size: BrandTokens.Icon.tab, weight: .medium))
                    .frame(width: 18)
                Text(verbatim: model.t(tab.label)).font(.callout.weight(selected ? .semibold : .regular)).lineLimit(1)
                Spacer(minLength: 0)
                HomeTabBadge(badge: tab.badge, model: model)
                if id == .activity && model.unseenActivity {
                    Circle().fill(BrandStyle.cyan).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot).help("New messages")
                }
            }
            .foregroundStyle(selected ? .primary : .secondary)
            .padding(.horizontal, BrandTokens.Spacing.l).frame(height: 28)
            .background(selected ? Color.primary.opacity(BrandTokens.Opacity.selected) : .clear,
                        in: RoundedRectangle(cornerRadius: BrandTokens.Radius.tab, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// A tab's badge: a count, a spinner, a failure, a dot
struct HomeTabBadge: View {
    let badge: HomePage.TabBadge?
    let model: AppModel

    var body: some View {
        if let b = badge {
            let help = Text(verbatim: model.t(b.help))
            switch b.kind {
            case "count": CountBadge(count: b.count ?? 0, color: b.tone.color).help(help)
            case "working": ProgressView().controlSize(.mini).help(help)
            case "failed": Image(systemName: "xmark.circle.fill").font(.caption2).foregroundStyle(b.tone.color).help(help)
            default: Circle().fill(b.tone.color).frame(width: BrandTokens.Size.dot, height: BrandTokens.Size.dot).help(help)
            }
        }
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
                    Text(verbatim: footer.text).font(.caption).lineLimit(2).multilineTextAlignment(.leading)
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
