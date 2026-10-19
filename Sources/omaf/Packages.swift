// SPDX-License-Identifier: AGPL-3.0-or-later
import ArgumentParser
import Foundation
import OmaflakeCore

// MARK: - Packages (plan B108): export, import, run from another place

func folderURL(_ path: String) -> URL {
    URL(fileURLWithPath: (path as NSString).expandingTildeInPath, isDirectory: true).standardizedFileURL
}

let printLine: @Sendable (String) -> Void = { print($0) }

struct Export: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Copy a VM, an image or a recipe as a package into a folder (an external disk, a NAS, a local folder).",
        discussion: """
        A VM goes back to the same person (it carries their user, keys and data); recipes are what people share.
        Packages are folders the Finder shows as one file – copying one by hand is just as good.
        """,
        subcommands: [ExportVM.self, ExportImage.self, ExportRecipe.self])
}

struct ExportVM: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "vm", abstract: "A stopped VM → <folder>/<name>.omaflake-vm (NixOS: with its image).")
    @Argument var name: String
    @Argument(help: "Target folder") var folder: String
    @Flag(help: "macOS: also carry the image (needed to reset the VM where no image is)") var withImage = false

    func run() throws {
        let env = Env()
        let transfer = Transfer(try env.backends())
        try transfer.exportVM(name, to: folderURL(folder), withImage: withImage, log: printLine)
    }
}

struct ExportImage: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "image", abstract: "An image → <folder>/<name>.omaflake-image.")
    @Argument(help: "Image name as listed by omaf images") var name: String
    @Argument(help: "Target folder") var folder: String

    func run() throws {
        let env = Env()
        try Transfer(try env.backends()).exportImage(name, to: folderURL(folder), log: printLine)
    }
}

struct ExportRecipe: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "recipe", abstract: "A recipe and those it requires → <folder>/<id>.omaflake-recipe.")
    @Argument(help: "Recipe: its name, <kind>/<name> or <author>/<kind>/<name>") var recipe: String
    @Argument(help: "Target folder") var folder: String

    func run() throws {
        let env = Env()
        let settings = Settings.load(env.paths)
        let library = RecipeLibrary(paths: env.paths)
        let id = try RecipeID.resolve(recipe, among: library.available(settings).map(\.id))
        let dest = try library.exportRecipe(id, to: folderURL(folder), settings: settings)
        print("Exported to \(dest.path)")
    }
}

struct Import: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Import a package (.omaflake-vm, .omaflake-image, .omaflake-recipe) or a folder of recipes – always as a copy into the data folder.",
        discussion: """
        A VM comes back as the same VM; when its name is taken, import it under another --name (a second VM with its
        own SSH port and macOS machine identity). The same image or recipes that are here already are replaced only
        with --replace. The source is never changed. To run a VM from where it is instead: omaf external add.
        """)
    @Argument(help: "Package or recipe folder") var path: String
    @Option(help: "VM: import it under this name (a second VM with its own identity)") var name: String?
    @Flag(help: "Image or recipes: replace what is here already") var replace = false

    func run() throws {
        let env = Env()
        let result = try Transfer(try env.backends()).importPackage(folderURL(path), name: name, replace: replace, log: printLine)
        for note in result.notes { print(note) }
        if result.kind == .recipe {
            print(result.names.isEmpty ? "These recipes are here already, unchanged – nothing to import"
                  : "Recipes: \(result.names.map(RecipeID.qualified).joined(separator: ", "))")
        }
    }
}

struct External: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "VMs that run from where their package is (an external disk, a stick, a NAS, a folder).",
        subcommands: [ExternalList.self, ExternalAdd.self, ExternalRemove.self], defaultSubcommand: ExternalList.self)
}

struct ExternalList: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "list", abstract: "The external VMs and where they are.")

    func run() throws {
        let env = Env()
        let paths = Settings.load(file: env.paths.configFile).externalVMs ?? []
        if paths.isEmpty { print("No external VMs. Add one with: omaf external add <package>") }
        for p in paths {
            let there = FileManager.default.fileExists(atPath: (p as NSString).appendingPathComponent("instance.json"))
            print("\(Package.name(of: URL(fileURLWithPath: p), kind: .vm) ?? p)  \(p)\(there ? "" : "  (not there)")")
        }
    }
}

struct ExternalAdd: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "add", abstract: "Run a VM package from where it is – nothing is copied (export it with omaf export vm first).")
    @Argument(help: "The .omaflake-vm package") var path: String

    func run() throws {
        let env = Env()
        let url = folderURL(path)
        let name = try Transfer(try env.backends()).useExternal(url)
        print("\(name) runs from \(url.path)")
        if Transfer.isOnNetworkVolume(url) { print("Note: on a network volume – slower, and the VM breaks when the connection does.") }
    }
}

struct ExternalRemove: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "remove", abstract: "Stop listing an external VM (also while its place is not there); its package stays.")
    @Argument var name: String

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        if let c = try? backends.store.load(name), try backends.backend(for: c).status(c) != .stopped {
            throw VMError("\"\(name)\" is running – first run: omaf stop \(name)")
        }
        try backends.store.forget(name)
        print("\(name) is no longer listed; its package stays at its place.")
    }
}
