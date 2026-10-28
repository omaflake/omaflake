// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The look studio (plan B150): a look of your own – its name and the look it starts from, a preview drawn from its
// colours (a window with its border, a terminal, the desktop's bar), what is hard to read, then a section each for the
// colours, the terminal's sixteen (as a grid), the desktop, the editors and macOS. Colours are chosen with the system's
// colour picker. Every choice is a form value of the page; the core says what follows from it.

struct LookStudioView: View {
    let page: LookStudioPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            Form {
                if model.helpOn { Section { HelpNote(page.intro, model: model) }.listRowBackground(Color.clear) }
                Section {
                    ForEach(page.about) { FormField(field: $0, page: ref.page, model: model) }
                }
                if let preview = page.preview {
                    Section {
                        LookPreviewView(preview: preview).listRowInsets(EdgeInsets())
                        ForEach(Array(page.hints.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                    } header: { Text("Preview") }
                }
                ForEach(page.sections) { section in
                    Section {
                        if model.helpOn { HelpNote(section.help, model: model) }
                        if section.id == "terminal" {
                            TerminalGrid(fields: section.fields, ref: ref, model: model)
                        } else {
                            ForEach(section.fields) { FormField(field: $0, page: ref.page, model: model) }
                        }
                    } header: { Text(verbatim: model.t(section.title)) }
                }
            }
            .omaflakeForm()
            Divider()
            HStack {
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
            .padding(.horizontal, BrandStyle.margin).padding(.vertical, BrandTokens.Spacing.xl)
        }
    }
}

/// The terminal's sixteen colours: a row of eight and their bright ones, each a swatch that opens the colour picker
private struct TerminalGrid: View {
    let fields: [Field]
    let ref: PageRef
    let model: AppModel

    var body: some View {
        // The core lists them as normal 0, bright 0, normal 1, … – here a row each
        let normal = fields.filter { $0.id.hasPrefix("normal.") }
        let bright = fields.filter { $0.id.hasPrefix("bright.") }
        VStack(alignment: .leading, spacing: BrandTokens.Spacing.m) {
            row(normal)
            row(bright)
        }
        .padding(.vertical, BrandTokens.Spacing.xs)
    }

    private func row(_ list: [Field]) -> some View {
        HStack(spacing: BrandTokens.Spacing.m) {
            ForEach(list) { f in
                let value = (model.formValue(ref.page, f) ?? f.value).string ?? "#808080"
                ColorPicker(selection: Binding(get: { Color(hex: value) },
                                               set: { model.setFormValue(ref.page, f, .string($0.hexString)) }),
                            supportsOpacity: false) { EmptyView() }
                    .labelsHidden()
                    .help(Text(verbatim: model.t(f.label)))
            }
        }
    }
}

/// The look drawn: a window with the active border on the background, its text, a selection, a terminal with the
/// sixteen colours, the bar with the clock – enough to judge it before building an image
struct LookPreviewView: View {
    let preview: LookStudioPage.Preview

    private func c(_ hex: String) -> Color { Color(hex: hex) }

    var body: some View {
        let border = preview.border.map(c)
        VStack(spacing: 0) {
            // The bar
            HStack(spacing: 6) {
                ForEach(0..<4, id: \.self) { i in
                    Text(verbatim: "\(i + 1)").font(.system(size: 10, weight: i == 0 ? .bold : .regular, design: .monospaced))
                        .foregroundStyle(i == 0 ? c(preview.accent) : c(preview.muted))
                }
                Spacer()
                Text(verbatim: "12:34").font(.system(size: 10, weight: .semibold, design: .monospaced)).foregroundStyle(c(preview.clock))
            }
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(c(preview.surface))
            HStack(alignment: .top, spacing: 10) {
                // The active window: an editor
                VStack(alignment: .leading, spacing: 3) {
                    Text(verbatim: "notes.md").font(.system(size: 11, weight: .semibold)).foregroundStyle(c(preview.text))
                    Text(verbatim: "# A look of my own").font(.system(size: 10, design: .monospaced)).foregroundStyle(c(preview.accent))
                    Text(verbatim: "Selected text").font(.system(size: 10, design: .monospaced)).foregroundStyle(c(preview.text))
                        .padding(.horizontal, 2).background(c(preview.selection))
                    Text(verbatim: "Secondary text").font(.system(size: 10)).foregroundStyle(c(preview.subtext))
                    Text(verbatim: "-- a comment").font(.system(size: 10, design: .monospaced)).foregroundStyle(c(preview.muted))
                    Text(verbatim: "error: it failed").font(.system(size: 10, design: .monospaced)).foregroundStyle(c(preview.urgent))
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .background(c(preview.background), in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(
                    LinearGradient(colors: border.count > 1 ? border : [border.first ?? .gray, border.first ?? .gray],
                                   startPoint: .topLeading, endPoint: .bottomTrailing), lineWidth: 2))
                // Another window: the terminal
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 4) {
                        Text(verbatim: "~/work").foregroundStyle(c(preview.normal[4]))
                        Text(verbatim: "❯").foregroundStyle(c(preview.normal[2]))
                        Text(verbatim: "ls").foregroundStyle(c(preview.text))
                    }
                    .font(.system(size: 10, design: .monospaced))
                    ForEach([preview.normal, preview.bright], id: \.self) { row in
                        HStack(spacing: 2) {
                            ForEach(Array(row.enumerated()), id: \.offset) { _, hex in
                                RoundedRectangle(cornerRadius: 2).fill(c(hex)).frame(width: 13, height: 13)
                            }
                        }
                    }
                }
                .padding(8)
                .background(c(preview.background), in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(c(preview.inactiveBorder), lineWidth: 2))
            }
            .padding(10)
        }
        .background(c(preview.surface).opacity(0.6))
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .padding(BrandTokens.Spacing.m)
    }
}
