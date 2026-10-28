// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The app's building blocks: how rows, buttons, lamps and lists look (design tokens, BrandStyle). What they show
// comes from the core's pages (docs/api.md §10); these only draw it.

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

/// An action in a row's details: symbol and name in a thin frame; red for removing (macOS would draw a system
/// button's title in the label colour, whatever its role)
struct ActionButton: View {
    let icon: String
    let title: String
    var color: Color = .primary
    let action: () -> Void
    @Environment(\.isEnabled) private var enabled
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: BrandTokens.Spacing.s) {
                Image(systemName: icon).font(BrandTokens.Fonts.buttonIcon)
                Text(verbatim: title).font(.caption).lineLimit(1)
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
    let title: String
    let isOn: Bool
    var help: String = ""
    let toggle: () -> Void
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: BrandTokens.Spacing.s) {
                Image(systemName: icon).font(BrandTokens.Fonts.buttonIcon)
                Text(verbatim: title).font(.caption).lineLimit(1)
            }
            .foregroundStyle(isOn ? .primary : .secondary)
            .padding(.horizontal, BrandTokens.Spacing.l).frame(height: 22)
            .background(isOn ? BrandStyle.cyan.opacity(BrandTokens.Opacity.toggleOnFill) : .clear, in: Capsule())
            .overlay(Capsule().strokeBorder(isOn ? BrandStyle.cyan.opacity(BrandTokens.Opacity.toggleOnBorder) : BrandStyle.cardBorder))
            .opacity(enabled ? 1 : 0.45)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help(Text(verbatim: help))
    }
}

/// Section of the main window: title, count ("3/6" while filtered) and its tools on the right (filter, folder,
/// clear), rows in a rounded box.
struct ListSection<Accessory: View, Content: View>: View {
    let title: String
    let count: Int
    /// All rows before the filter (nil = not filtered)
    var of: Int? = nil
    /// The list's filter: right next to the title and count, apart from the actions
    var filter: AnyView? = nil
    /// Actions (import, refresh, …) on the right
    /// Between the title and the list: a catalog tab's search and chips (plan B148)
    var below: AnyView? = nil
    @ViewBuilder let accessory: Accessory
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            HStack(alignment: .firstTextBaseline) {
                Text(verbatim: title).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                if let of, of != count {
                    Text(verbatim: "\(count)/\(of)").font(.caption).foregroundStyle(BrandStyle.cyan)
                } else if count > 0 {
                    Text(verbatim: "\(count)").font(.caption).foregroundStyle(.tertiary)
                }
                if let filter { filter.frame(height: 24) }
                Spacer()
                accessory.frame(height: 24)
            }
            if let below { below }
            VStack(spacing: 0) { content }
                .background(BrandStyle.card, in: RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: BrandStyle.radius, style: .continuous).strokeBorder(BrandStyle.cardBorder))
        }
    }
}

/// Small borderless icon button with a hover highlight and an optional count badge.
struct IconButton: View {
    let icon: String
    let help: String
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
        .help(Text(verbatim: help))
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

/// A guide from docs/ (`ui.show guide {file, title, url, path?}`): the file the core names (`path`, omaflake.app's
/// Resources/Guides, scripts/bundle-app.sh), else the one bundled with this app – no network needed. Without either
/// the app opens `url` instead (AppModel).
struct GuideView: View {
    let params: JSONValue

    /// The guide's Markdown file: `path`, else `file` in the app's bundle
    static func source(_ params: JSONValue) -> URL? {
        if let path = params["path"]?.string, FileManager.default.fileExists(atPath: path) {
            return URL(fileURLWithPath: path)
        }
        return params["file"]?.string.flatMap { Bundle.main.url(forResource: $0, withExtension: "md", subdirectory: "Guides") }
    }

    private var blocks: [MarkdownBlock] {
        guard let url = Self.source(params), let text = try? String(contentsOf: url, encoding: .utf8) else { return [] }
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
