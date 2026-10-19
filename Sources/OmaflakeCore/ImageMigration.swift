// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

extension ImageStore {
    /// Older layouts (before image.json, 2026-10-17): NixOS images were symlinks into the Nix store and recipes were
    /// called "minimal", "mac-minimal", "nixos-minimal", … Turns them into image folders with image.json (recipe
    /// "omaflake.<kind>.<name>"), renames `current/` links and points instances at the new paths (qcow2 overlays are
    /// rebased). Then everything becomes a package (plan B108, `migratePackages`). Idempotent and cheap when there is
    /// nothing to do; returns what it changed.
    @discardableResult
    public func migrateLegacy() -> [String] {
        let fm = FileManager.default
        var notes: [String] = []
        if let flake = Settings.migrateRecipesFolder(paths) { notes.append("Image flake moved to \(flake) (plan B92, B107)") }
        let names = (try? fm.contentsOfDirectory(atPath: paths.images.path)) ?? []
        for name in names where name != "current" && !name.hasPrefix(".") && Package.kind(of: URL(fileURLWithPath: name)) == nil {
            let entry = paths.images.appendingPathComponent(name)
            guard Self.readInfo(entry) == nil else { continue }
            let legacy = Self.recipe(ofImage: name)
            let revision = Self.legacyRevision(name, recipe: legacy)
            // When the old image was made (not when it was migrated); the link itself, not its store target
            let created = (try? fm.attributesOfItem(atPath: entry.path)[.creationDate] as? Date) ?? Date()
            if let dest = try? fm.destinationOfSymbolicLink(atPath: entry.path), dest.hasPrefix("/nix/store/") {
                // NixOS: the symlink becomes a folder with result → store path (registered as a GC root again)
                let parked = paths.images.appendingPathComponent(".legacy-\(name)")
                do {
                    try fm.moveItem(at: entry, to: parked)
                    try fm.createDirectory(at: entry, withIntermediateDirectories: true)
                    try Shell.check("nix", ["build", dest, "--out-link", entry.appendingPathComponent("result").path])
                    try Self.writeInfo(Info(recipe: Self.legacyRecipeID(legacy, kind: .nixos), kind: .nixos, source: "",
                                            revision: revision, storePath: dest, created: created), to: entry)
                    try? Disposal.delete([parked])   // only the old symlink; the store path has its new GC root
                    notes.append("image \(name): folder with image.json")
                } catch {
                    // Put the old link back so nothing is lost
                    if !fm.fileExists(atPath: entry.appendingPathComponent("result").path) {
                        try? fm.removeItem(at: entry)   // the empty folder just created
                        try? fm.moveItem(at: parked, to: entry)
                    }
                    notes.append("image \(name): not migrated (\(error))")
                }
            } else if fm.fileExists(atPath: MacVMBundle(entry).config.path) {
                try? Self.writeInfo(Info(recipe: Self.legacyRecipeID(legacy, kind: .macos), kind: .macos, source: "",
                                         revision: revision, storePath: nil, created: created), to: entry)
                notes.append("image \(name): image.json")
            }
        }
        // current/<legacy recipe> → current/<recipe ID>
        for link in (try? fm.contentsOfDirectory(atPath: paths.currentImages.path)) ?? [] where RecipeID.parts(link) == nil {
            let old = paths.currentImages.appendingPathComponent(link)
            guard let dest = try? fm.destinationOfSymbolicLink(atPath: old.path),
                  let info = Self.readInfo(paths.images.appendingPathComponent((dest as NSString).lastPathComponent))
                    ?? Self.readInfo(paths.imageDir(Self.imageName(ofLink: dest))) else { continue }
            let new = paths.currentImages.appendingPathComponent(info.recipe)
            if fm.fileExists(atPath: new.path) || (try? fm.destinationOfSymbolicLink(atPath: new.path)) != nil {
                try? Disposal.delete([old])
            } else {
                try? fm.moveItem(at: old, to: new)
            }
            notes.append("current/\(link) → \(info.recipe)")
        }
        let qemuImg = (try? QEMURuntime.locate(paths).1) ?? "qemu-img"
        notes += migratePackages(qemuImg: qemuImg)
        // Instances: recipe ID; an overlay that still names its image by an absolute path gets the relative one
        let store = InstanceStore(paths: paths, qemuImg: qemuImg)
        for var c in store.list() {
            var changed = false
            var note: String?
            if RecipeID.parts(c.recipe) == nil {
                c.recipe = Self.legacyRecipeID(c.recipe, kind: c.guestKind)
                changed = true
                note = "instance \(c.name): \(c.recipe)"
            }
            let ip = paths.instance(c.name)
            if c.guestKind == .nixos, fm.fileExists(atPath: ip.disk.path), !DataFolder.isRunning(c, paths: paths),
               let backing = Self.backingFile(of: ip.disk, qemuImg: qemuImg), backing.hasPrefix("/"),
               let image = store.backingImage(c) {
                let relative = Package.relativePath(from: ip.dir, to: image)
                if (try? Shell.check(qemuImg, ["rebase", "-u", "-F", "qcow2", "-b", relative, ip.disk.path], cwd: ip.dir)) != nil {
                    c.imagePath = image.path
                    changed = true
                    notes.append("instance \(c.name): overlay on \(relative)")
                }
            }
            if changed, (try? store.save(c)) != nil, let note { notes.append(note) }
        }
        notes += migrateRecipeIDs(store)
        return notes
    }

    /// Plan B110/B111: recipe IDs carry their content hash as 12 words and images their recipe's source (the line).
    /// Downloaded recipes move to `recipes/<id>/`; images, `current/` links and VMs of older IDs (none, or fewer
    /// words) get the ID and source of the recipe there is now – the one whose words they start with, else the first of
    /// that base; an image built from an older state is then outdated by its store path, as before. Recipes that are
    /// gone stay as they are. VMs get an identity of 12 words (a shorter one is extended, so it starts the same).
    func migrateRecipeIDs(_ store: InstanceStore) -> [String] {
        let fm = FileManager.default
        var notes: [String] = []
        let library = RecipeLibrary(paths: paths)
        func isFull(_ id: String) -> Bool { RecipeID.words(id).map { OmaflakeID.isWords($0, full: true) } ?? false }
        // Downloaded and imported recipes: recipes/<author>/<kind>/<name>/ or recipes/<shorter ID>/ → recipes/<id>/
        var manifest = library.installed()
        for (old, entry) in manifest where !isFull(old) {
            guard let (author, kind, name) = RecipeRef.parts(RecipeID.base(old)) else { continue }
            let folder = RecipeID.words(old) == nil
                ? paths.recipes.appendingPathComponent("\(author)/\(kind.rawValue)/\(name)", isDirectory: true)
                : paths.recipes.appendingPathComponent(old, isDirectory: true)
            guard let hash = try? RecipeLibrary.contentHash(folder) else { continue }
            let id = RecipeID.make(base: RecipeID.base(old), contentHash: hash)
            guard let target = library.folder(id) else { continue }
            if !fm.fileExists(atPath: target.path) {
                guard (try? fm.moveItem(at: folder, to: target)) != nil else { continue }
            }
            manifest[old] = nil
            manifest[id] = entry
            notes.append("recipe \(RecipeID.qualified(old)): \(OmaflakeID.recipe(id, words: 3))")
        }
        if manifest != library.installed() { try? library.saveManifest(manifest) }
        let available = library.available(Settings.load(paths))
        func current(_ recipe: String) -> (id: String, source: String)? {
            if let e = available.first(where: { $0.id == recipe }) { return (e.id, e.source) }
            let base = RecipeID.base(recipe)
            if let words = RecipeID.words(recipe) {
                return available.first { RecipeID.base($0.id) == base && RecipeID.words($0.id)?.hasPrefix(words) == true }
                    .map { ($0.id, $0.source) }
            }
            return available.first { RecipeID.base($0.id) == base }.map { ($0.id, $0.source) }
        }
        // Images
        for image in list() where image.info.recipeSource == nil || !isFull(image.recipe) {
            guard let now = current(image.recipe) else { continue }
            var info = image.info
            info.recipe = now.id
            info.recipeSource = now.source
            if (try? Self.writeInfo(info, to: paths.imageDir(image.name))) != nil {
                notes.append("image \(image.name): \(OmaflakeID.recipe(now.id, words: 3))")
            }
        }
        // current/<old ID> → current/<line>
        for link in (try? fm.contentsOfDirectory(atPath: paths.currentImages.path)) ?? [] where !link.contains("~") {
            let url = paths.currentImages.appendingPathComponent(link)
            guard let dest = try? fm.destinationOfSymbolicLink(atPath: url.path) else { continue }
            let name = Self.imageName(ofLink: dest)
            guard (try? image(named: name)) != nil, (try? setCurrent(imageName: name)) != nil else { continue }
            try? fm.removeItem(at: url)
        }
        // VMs: the recipe of their image, else the one there is now; an identity of 12 words
        for var c in store.list() where !isFull(c.recipe) || c.identity.map({ !OmaflakeID.isWords($0, full: true) }) ?? true {
            let before = c
            if !isFull(c.recipe), let id = (try? image(named: c.image)).map(\.recipe).flatMap({ isFull($0) ? $0 : nil }) ?? current(c.recipe)?.id {
                c.recipe = id
            }
            let have = c.identity?.split(separator: "-").map(String.init) ?? []
            if have.count != OmaflakeID.length {
                let more = OmaflakeID.randomWords().split(separator: "-").map(String.init)
                c.identity = (have + more).prefix(OmaflakeID.length).joined(separator: "-")
            }
            if c != before, (try? store.save(c)) != nil { notes.append("instance \(c.name): \(OmaflakeID.vm(c, words: 3))") }
        }
        notes += renameImages(store)
        return notes
    }

    /// Plan B110/B111: image packages are named `<base>@<12 words of the store path>` – older names are renamed; the VMs on
    /// them follow (their `image`, and NixOS overlays their relative backing file). An image a running VM uses waits.
    func renameImages(_ store: InstanceStore) -> [String] {
        let fm = FileManager.default
        var notes: [String] = []
        for image in list() where RecipeID.words(image.name).map({ !OmaflakeID.isWords($0, full: true) }) ?? true {
            let new = Self.imageName(recipe: image.recipe, storePath: image.info.storePath ?? image.name)
            let users = store.list().filter { $0.image == image.name }
            guard !fm.fileExists(atPath: paths.imageDir(new).path),
                  !users.contains(where: { DataFolder.isRunning($0, paths: paths) }) else { continue }
            let wasCurrent = currentName(line: image.info.line) == image.name
            guard (try? fm.moveItem(at: paths.imageDir(image.name), to: paths.imageDir(new))) != nil else { continue }
            if wasCurrent { try? setCurrent(imageName: new) }
            let qemuImg = (try? QEMURuntime.locate(paths).1) ?? "qemu-img"
            for var c in users {
                c.image = new
                let ip = paths.instance(c.name)
                let embedded = ip.embeddedImage.appendingPathComponent("disk.qcow2")
                if c.guestKind == .nixos, !fm.fileExists(atPath: embedded.path), fm.fileExists(atPath: ip.disk.path) {
                    let backing = paths.imageDir(new).appendingPathComponent("disk.qcow2")
                    c.imagePath = backing.path
                    _ = try? Shell.check(qemuImg, ["rebase", "-u", "-F", "qcow2", "-b", Package.relativePath(from: ip.dir, to: backing),
                                                   ip.disk.path], cwd: ip.dir)
                } else if c.guestKind == .macos {
                    c.imagePath = paths.imageDir(new).path
                }
                try? store.save(c)
            }
            notes.append("image \(image.name) → \(new)")
        }
        return notes
    }

    /// The backing file a qcow2 names (as written in it), nil without one
    static func backingFile(of disk: URL, qemuImg: String) -> String? {
        guard let json = try? Shell.check(qemuImg, ["info", "-U", "--output=json", disk.path]),
              let obj = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else { return nil }
        return obj["backing-filename"] as? String
    }

    /// Plan B108: image folders → `<name>.omaflake-image` (NixOS: the qcow2 is copied out of the Nix store, the GC
    /// root goes), `current/` links follow, instance folders → `<name>.omaflake-vm` with their runtime files moved to
    /// `run/<name>/`. Skipped as a whole while a VM of the old layout runs (its files are open) – next start then.
    func migratePackages(qemuImg: String) -> [String] {
        let fm = FileManager.default
        var notes: [String] = []
        let oldInstances = ((try? fm.contentsOfDirectory(atPath: paths.instances.path)) ?? [])
            .filter { !$0.hasPrefix(".") && Package.kind(of: URL(fileURLWithPath: $0)) == nil }
            .map { paths.instances.appendingPathComponent($0, isDirectory: true) }
            .filter { fm.fileExists(atPath: $0.appendingPathComponent("instance.json").path) }
        let running = oldInstances.filter {
            let ip = InstancePaths(dir: $0)
            return livePid(file: ip.pidFile, executable: "qemu") != nil || livePid(file: ip.runnerPid, executable: "omaf-runner") != nil
        }
        guard running.isEmpty else {
            return ["packages: not yet – stop \(running.map(\.lastPathComponent).joined(separator: ", ")) first"]
        }
        // Images
        for name in ((try? fm.contentsOfDirectory(atPath: paths.images.path)) ?? []).sorted()
        where name != "current" && !name.hasPrefix(".") && Package.kind(of: URL(fileURLWithPath: name)) == nil {
            let old = paths.images.appendingPathComponent(name, isDirectory: true)
            guard let info = Self.readInfo(old) else { continue }
            let new = paths.imageDir(name)
            guard !fm.fileExists(atPath: new.path) else { continue }
            do {
                if info.kind == .nixos {
                    let result = old.appendingPathComponent("result")
                    try Self.copyDisk(fromStore: result.resolvingSymlinksInPath(), into: old)
                    try fm.moveItem(at: old, to: new)
                    try? Disposal.delete([new.appendingPathComponent("result")])   // only the GC root
                } else {
                    try fm.moveItem(at: old, to: new)
                }
                notes.append("image \(name): package")
            } catch {
                notes.append("image \(name): not a package yet (\(error))")
            }
        }
        // current/<recipe> → ../<name>.omaflake-image
        for link in (try? fm.contentsOfDirectory(atPath: paths.currentImages.path)) ?? [] {
            let url = paths.currentImages.appendingPathComponent(link)
            guard let dest = try? fm.destinationOfSymbolicLink(atPath: url.path),
                  Package.kind(of: URL(fileURLWithPath: dest)) == nil else { continue }
            let name = (dest as NSString).lastPathComponent
            if fm.fileExists(atPath: paths.imageDir(name).path) {
                try? fm.removeItem(at: url)
                try? fm.createSymbolicLink(atPath: url.path, withDestinationPath: "../\(name).\(Package.image)")
            }
        }
        // Instances
        for old in oldInstances {
            let name = old.lastPathComponent
            let new = paths.instances.appendingPathComponent("\(name).\(Package.vm)", isDirectory: true)
            guard !fm.fileExists(atPath: new.path) else { continue }
            do {
                let run = paths.run.appendingPathComponent(name, isDirectory: true)
                try fm.createDirectory(at: run, withIntermediateDirectories: true)
                for file in InstancePaths.runtimeNames {
                    let from = old.appendingPathComponent(file)
                    let isLink = (try? fm.destinationOfSymbolicLink(atPath: from.path)) != nil
                    guard isLink || fm.fileExists(atPath: from.path) else { continue }
                    try? Disposal.delete([run.appendingPathComponent(file)])
                    try fm.moveItem(at: from, to: run.appendingPathComponent(file))
                }
                try fm.moveItem(at: old, to: new)
                notes.append("instance \(name): package")
            } catch {
                notes.append("instance \(name): not a package yet (\(error))")
            }
        }
        return notes
    }

    /// "minimal", "nixos-minimal", "mac-minimal", "macos-minimal" → "omaflake.<kind>.minimal"; IDs stay as they are
    static func legacyRecipeID(_ recipe: String, kind: GuestKind) -> String {
        if RecipeID.parts(recipe) != nil { return recipe }
        var name = recipe
        for prefix in kind == .macos ? ["macos-", "mac-"] : ["nixos-"] where name.hasPrefix(prefix) {
            name = String(name.dropFirst(prefix.count))
            break
        }
        return RecipeID.make(author: "omaflake", kind: kind, name: name)
    }

    /// "<recipe>-<rev>[-dirty]-<hash>" → "<rev>[-dirty]"
    static func legacyRevision(_ name: String, recipe: String) -> String {
        guard name.hasPrefix(recipe + "-") else { return "unknown" }
        var rest = name.dropFirst(recipe.count + 1).split(separator: "-").map(String.init)
        if rest.count > 1 { rest.removeLast() }   // store hash
        return rest.joined(separator: "-")
    }
}
