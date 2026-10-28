// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

// The recipe studio (plan B147): a recipe of your own put together like a kit, from top to bottom – its name, the
// modules in it with what is missing or does not fit (and the one to add), the catalog to find more (search and
// filters), its look and keys, the VM. Every choice is a form value of the page; the core says what follows from it.

struct RecipeStudioView: View {
    let page: RecipeStudioPage
    let ref: PageRef
    let model: AppModel

    var body: some View {
        Form {
            // The explanation in a box of its own above the form, with the help switch on (plan B148)
            if model.helpOn { Section { HelpNote(page.intro, model: model) }.listRowBackground(Color.clear) }
            Section {
                ForEach(page.about) { FormField(field: $0, page: ref.page, model: model) }
            } header: { Text(verbatim: model.t(page.titles.about)) }

            Section {
                ForEach(Array(page.recipe.hints.enumerated()), id: \.offset) { HintRow(hint: $1, model: model) }
                if let empty = page.recipe.empty {
                    Text(verbatim: model.t(empty)).font(.callout).foregroundStyle(.secondary)
                }
                ForEach(page.recipe.rows) { StudioModuleRow(module: $0, ref: ref, model: model) }
            } header: {
                HStack {
                    Text(verbatim: model.t(page.recipe.title))
                    Spacer()
                    if let keys = page.recipe.keys { ActionPlainButton(action: keys, model: model) }
                }
            }

            Section {
                FormField(field: page.catalog.search, page: ref.page, model: model)
                ForEach(page.catalog.filters) { FormField(field: $0, page: ref.page, model: model) }
                if let empty = page.catalog.empty {
                    Text(verbatim: model.t(empty)).font(.callout).foregroundStyle(.secondary)
                }
                ForEach(page.catalog.rows) { StudioModuleRow(module: $0, ref: ref, model: model) }
            } header: {
                HStack {
                    Text(verbatim: model.t(page.catalog.title))
                    Spacer()
                    Text(verbatim: model.t(page.catalog.count)).foregroundStyle(.secondary)
                }
            }

            Section {
                ForEach(page.dress) { FormField(field: $0, page: ref.page, model: model) }
            } header: { Text(verbatim: model.t(page.titles.dress)) }

            Section {
                ForEach(page.vm) { FormField(field: $0, page: ref.page, model: model) }
            } header: { Text(verbatim: model.t(page.titles.vm)) }

            HStack {
                Spacer()
                ForEach(page.actions) { ActionPlainButton(action: $0, model: model, form: ref, isDefault: $0.primary) }
            }
        }
        .formStyle(.grouped)
    }
}

/// A module in the recipe or in the catalog: its name, what it is, its badges, and Add or Remove
private struct StudioModuleRow: View {
    let module: RecipeStudioPage.Module
    let ref: PageRef
    let model: AppModel

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: BrandTokens.Spacing.l) {
            VStack(alignment: .leading, spacing: BrandTokens.Spacing.xs) {
                HStack(spacing: BrandTokens.Spacing.m) {
                    Text(verbatim: module.title).font(.callout.weight(.medium))
                    ForEach(module.badges) { BadgeView(badge: $0, model: model) }
                }
                Text(verbatim: model.t(module.subtitle)).font(.caption).foregroundStyle(.secondary)
                if !module.description.isEmpty {
                    Text(verbatim: module.description).font(.caption).foregroundStyle(.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: BrandTokens.Spacing.l)
            ActionPlainButton(action: module.action, model: model, isDefault: false)
        }
        .padding(.vertical, BrandTokens.Spacing.xs)
        .help(module.reference)
    }
}
