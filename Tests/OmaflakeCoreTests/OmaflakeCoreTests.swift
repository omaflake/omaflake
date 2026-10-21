// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import CryptoKit
import Foundation
import Testing
@testable import OmaflakeCore

/// Temporary ~/.omaflake replacement per test
func tempPaths() throws -> Paths {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("vmtest-\(UUID().uuidString.prefix(8))")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return Paths(root: dir)
}

let devDefaults = RecipeDefaults(cpus: 8, memoryGB: 16, diskGB: 100, network: .shared, share: true, ephemeral: false)

func config(_ name: String = "t1", network: NetworkMode = .shared, ephemeral: Bool = false, share: Bool = true) -> InstanceConfig {
    var d = devDefaults
    d.network = network
    d.ephemeral = ephemeral
    d.share = share
    return InstanceConfig(name: name, recipe: "dev", image: "dev-abc-12345678", imagePath: "/img.qcow2",
                          system: "aarch64-linux", settings: d, sshPort: 2230, created: Date(timeIntervalSince1970: 0))
}

@Suite struct RecipeDefaultsTests {
    @Test func decodesFlakeJSON() throws {
        // Output of `nix eval --json .#lib.recipeDefaults` (shortened, 2026-10-16)
        let json = #"{"lab":{"systems":["aarch64-linux"],"cpus":4,"disk_gb":60,"ephemeral":true,"memory_gb":8,"network":"isolated","share":false}}"#
        let d = try JSONDecoder().decode([String: RecipeDefaults].self, from: Data(json.utf8))
        #expect(d["lab"] == RecipeDefaults(cpus: 4, memoryGB: 8, diskGB: 60, network: .isolated, share: false, ephemeral: true))
    }

    @Test func decodesKind() throws {
        let json = #"{"omaflake.macos.minimal":{"kind":"macos","systems":["aarch64-darwin"],"author":"omaflake","name":"minimal","cpus":4,"disk_gb":60,"ephemeral":false,"memory_gb":8,"network":"shared","share":true,"user":"me"}}"#
        let d = try JSONDecoder().decode([String: RecipeDefaults].self, from: Data(json.utf8))
        #expect(d["omaflake.macos.minimal"]?.kind == .macos && d["omaflake.macos.minimal"]?.author == "omaflake")
        #expect(d["omaflake.macos.minimal"]?.user == "me")
    }
}

@Suite struct QEMUCommandTests {
    let ip = InstancePaths(dir: URL(fileURLWithPath: "/vm/instances/t1"))

    func args(_ c: InstanceConfig, display: DisplayMode = .none, share: String? = "/vm/share/t1") -> [String] {
        QEMUCommand(firmware: "/fw.fd", paths: ip, sharePath: share, display: display).arguments(for: c)
    }

    /// Values after an option, e.g. values(a, "-netdev")
    func values(_ a: [String], _ opt: String) -> [String] {
        a.indices.filter { a[$0] == opt && $0 + 1 < a.count }.map { a[$0 + 1] }
    }

    @Test func basics() {
        let a = args(config())
        #expect(values(a, "-accel") == ["hvf"])
        #expect(values(a, "-smp") == ["8"])
        #expect(values(a, "-m") == ["16G"])
        #expect(values(a, "-drive") == [
            "if=pflash,format=raw,readonly=on,file=/fw.fd",
            "if=pflash,format=raw,file=/vm/instances/t1/efivars.fd",
            "if=none,id=hd0,format=qcow2,file=/vm/instances/t1/disk.qcow2",
        ])
        #expect(values(a, "-device").contains("virtio-blk-pci,drive=hd0,bootindex=0,addr=0x2"))
        #expect(values(a, "-qmp") == ["unix:/vm/instances/t1/qmp.sock,server=on,wait=off"])
        #expect(values(a, "-pidfile") == ["/vm/instances/t1/qemu.pid"])
        #expect(!a.contains("-snapshot"))
        #expect(values(a, "-display") == ["none"])
    }

    @Test func networkModes() {
        #expect(values(args(config(network: .shared)), "-netdev") == ["user,id=n0,hostfwd=tcp:127.0.0.1:2230-:22"])
        #expect(values(args(config(network: .isolated)), "-netdev") == ["user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:2230-:22"])
        let none = args(config(network: .none))
        #expect(values(none, "-netdev").isEmpty)
        #expect(values(none, "-nic") == ["none"])
    }

    @Test func ephemeralUsesSnapshot() {
        #expect(args(config(ephemeral: true)).contains("-snapshot"))
    }

    @Test func shareOnlyWhenEnabled() {
        #expect(values(args(config(share: true)), "-fsdev") == ["local,id=fs0,path=/vm/share/t1,security_model=mapped-xattr"])
        #expect(values(args(config(share: false)), "-fsdev").isEmpty)
    }

    @Test func gpuMode() {
        let a = QEMUCommand(firmware: "/fw.fd", paths: ip, sharePath: nil, display: .gpu, gl: .es,
                            capabilities: QEMUCapabilities(gl: true, guestOwner: false)).arguments(for: config())
        #expect(values(a, "-display") == ["cocoa,gl=es,zoom-to-fit=on"])
        #expect(values(a, "-device").contains("virtio-gpu-gl-pci,addr=0x5"))
        #expect(!values(a, "-device").contains { $0.hasPrefix("virtio-gpu-pci") })
    }

    @Test func shareWithGuestOwnerPatch() {
        let a = QEMUCommand(firmware: "/fw.fd", paths: ip, sharePath: "/vm/share/t1", display: .none,
                            capabilities: QEMUCapabilities(gl: true, guestOwner: true)).arguments(for: config())
        #expect(values(a, "-fsdev") == ["local,id=fs0,path=/vm/share/t1,security_model=none,guest_owner_uid=1000,guest_owner_gid=100"])
    }

    @Test func initialResolution() {
        // MacBook Pro 14": 1512×982 points, Retina 2 → 2240×1260 (16:9, whole points)
        let r = QEMUCommand.initialResolution(screenPoints: (1512, 982), scale: 2)
        #expect(r.width == 2240 && r.height == 1260)
        // 5K display with 2560×1440 points → 3840×2160
        let k = QEMUCommand.initialResolution(screenPoints: (2560, 1440), scale: 2)
        #expect(k.width == 3840 && k.height == 2160)
        var cmd = QEMUCommand(firmware: "/fw.fd", paths: ip, sharePath: nil, display: .gpu,
                              capabilities: QEMUCapabilities(gl: true, guestOwner: false))
        cmd.initialResolution = k
        #expect(values(cmd.arguments(for: config()), "-device").contains("virtio-gpu-gl-pci,xres=3840,yres=2160,addr=0x5"))
    }

    @Test func windowMode() {
        let a = args(config(), display: .window)
        #expect(values(a, "-display") == ["cocoa,zoom-to-fit=on"])
        #expect(values(a, "-device").contains("usb-tablet"))
    }

    @Test func clipboardPort() {
        var c = config()
        #expect(values(args(c), "-device").contains("virtserialport,chardev=clip0,name=org.omaflake.clipboard"))
        #expect(values(args(c), "-chardev").contains("socket,id=clip0,path=/vm/instances/t1/clip.sock,server=on,wait=off"))
        c.clipboard = false
        #expect(!values(args(c), "-device").contains { $0.contains("org.omaflake.clipboard") })
    }

    @Test func agentPortAlways() {
        var c = config(network: .none)
        c.clipboard = false
        #expect(values(args(c), "-device").contains("virtserialport,chardev=agent0,name=org.omaflake.agent"))
        #expect(values(args(c), "-chardev").contains("socket,id=agent0,path=/vm/instances/t1/agent.sock,server=on,wait=off"))
    }

    @Test func cameraPortOnlyWhenOn() {
        var c = config()
        #expect(!values(args(c), "-device").contains { $0.contains("org.omaflake.camera") })   // default off
        c.camera = true
        #expect(values(args(c), "-device").contains("virtserialport,chardev=cam0,name=org.omaflake.camera"))
        #expect(values(args(c), "-chardev").contains("socket,id=cam0,path=/vm/instances/t1/cam.sock,server=on,wait=off"))
    }

    @Test func soundAndMicrophone() {
        var c = config()
        #expect(values(args(c), "-audiodev") == ["coreaudio,id=snd0"])
        #expect(values(args(c), "-device").contains("hda-output,audiodev=snd0"))   // no input without the microphone
        c.microphone = true
        #expect(values(args(c), "-device").contains("hda-micro,audiodev=snd0"))
        c.audio = false
        #expect(values(args(c), "-audiodev").isEmpty && !c.microphoneOn)
    }

    @Test func pciSlotsDoNotDependOnTheWindow() {
        // The disk keeps its slot with and without a window (plan B32)
        func slot(_ a: [String], _ prefix: String) -> String? {
            values(a, "-device").first { $0.hasPrefix(prefix) }?.components(separatedBy: "addr=").last
        }
        for prefix in ["virtio-blk-pci", "virtio-net-pci", "virtio-rng-pci", "virtio-serial-pci"] {
            #expect(slot(args(config()), prefix) == slot(args(config(), display: .window), prefix))
        }
        let all = values(args(config(), display: .window), "-device").compactMap { $0.components(separatedBy: "addr=").dropFirst().first }
        #expect(Set(all).count == all.count)   // no slot used twice
    }
}

@Suite struct RecipeRenameTests {
    /// instance.json and image.json from before plan B92 say "flavor" (and have no format, plan B114)
    @Test func oldFlavorKeyIsRead() throws {
        let paths = try tempPaths()
        let store = InstanceStore(paths: paths, qemuImg: "/usr/bin/false")
        try FileManager.default.createDirectory(at: paths.instance("t1").dir, withIntermediateDirectories: true)
        try store.save(config("t1"))
        let file = paths.instance("t1").config
        let written = try String(contentsOf: file, encoding: .utf8)
        #expect(written.contains("\"recipe\""))
        try written.replacingOccurrences(of: "\"recipe\"", with: "\"flavor\"").replacingOccurrences(of: #""$schema" : "https://omaflake.dev/schema/instance/v1.json","#, with: "")
            .write(to: file, atomically: true, encoding: .utf8)
        let loaded = try store.load("t1")
        #expect(loaded.recipe == "dev")
        #expect(loaded == config("t1"))

        let dir = paths.images.appendingPathComponent("img")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let info = ImageStore.Info(recipe: "omaflake.nixos.minimal", kind: .nixos, system: "aarch64-linux", source: "/x", revision: "abc", storePath: nil)
        try ImageStore.writeInfo(info, to: dir)
        let json = ImageStore.infoFile(dir)
        let text = try String(contentsOf: json, encoding: .utf8).replacingOccurrences(of: "\"recipe\"", with: "\"flavor\"")
            .replacingOccurrences(of: #""$schema" : "https://omaflake.dev/schema/image/v1.json","#, with: "")
        try text.write(to: json, atomically: true, encoding: .utf8)
        #expect(ImageStore.readInfo(dir)?.recipe == "omaflake.nixos.minimal")
    }

    /// config.json pointing to <checkout>/flavors or <checkout>/recipes moves to <checkout>/nix once that has the flake
    @Test func flakeSettingFollowsTheFolder() throws {
        let paths = try tempPaths()
        let checkout = paths.root.appendingPathComponent("checkout")
        let nix = checkout.appendingPathComponent("nix")
        try FileManager.default.createDirectory(at: nix, withIntermediateDirectories: true)
        for old in ["flavors", "recipes"] {
            var s = Settings()
            s.flake = checkout.appendingPathComponent(old).path
            try s.save(paths)
            try? FileManager.default.removeItem(at: nix.appendingPathComponent("flake.nix"))
            #expect(Settings.migrateRecipesFolder(paths) == nil)   // no flake.nix in nix/ yet: nothing moves
            try Data("{}".utf8).write(to: nix.appendingPathComponent("flake.nix"))
            #expect(Settings.migrateRecipesFolder(paths) == nix.path)
            #expect(Settings.load(paths).flake == nix.path)
            #expect(Settings.migrateRecipesFolder(paths) == nil)   // done once
        }
        // a flake URL is left alone
        var s = Settings()
        s.flake = "github:omaflake/omaflake?dir=flavors"
        try s.save(paths)
        #expect(Settings.migrateRecipesFolder(paths) == nil)
    }
}

@Suite struct RecipeSourceTests {
    @Test func parsesWhatUsersType() throws {
        let a = try RecipeSource.parse("https://github.com/omaflake/recipes")
        #expect(a.kind == .github && a.location == "omaflake/recipes" && a.branch == nil && a.isOfficial)
        let b = try RecipeSource.parse("github.com/Some-One/my.recipes/tree/dev", isPrivate: true)
        #expect(b.kind == .github && b.location == "Some-One/my.recipes" && b.branch == "dev" && b.isPrivateRepository && !b.isOfficial)
        #expect(try RecipeSource.parse("someone/recipes").location == "someone/recipes")
        let c = try RecipeSource.parse("https://example.org/omaflake/")
        #expect(c.kind == .url && c.location == "https://example.org/omaflake/catalog.json")
        #expect(throws: VMError.self) { try RecipeSource.parse("http://example.org/catalog.json") }
        let dir = try tempPaths().root
        #expect(try RecipeSource.parse(dir.path).kind == .folder)
        #expect(throws: VMError.self) { try RecipeSource.parse(dir.appendingPathComponent("missing").path) }
        // forks are not official, whatever their recipes' author says
        #expect(!(try RecipeSource.parse("someone/recipes")).isOfficial)
        #expect(Settings().recipeSources == [.official])
    }

    @Test func references() {
        #expect(RecipeRef.id("omaflake/nixos/minimal") == "omaflake.nixos.minimal")
        #expect(RecipeRef.id("omaflake.macos.lab") == "omaflake.macos.lab")
        for bad in ["omaflake/linux/x", "../nixos/x", "a/nixos/B", "a/nixos", "-a/nixos/x"] { #expect(RecipeRef.id(bad) == nil) }
    }

    /// A local catalog folder as source: installing checks every file, rejects paths outside the recipe, brings
    /// what a recipe requires; staging puts local folders first and gives the same folder for the same content
    @Test func installStageRemove() async throws {
        let paths = try tempPaths()
        let fm = FileManager.default
        let src = paths.root.appendingPathComponent("source")
        func put(_ rel: String, _ text: String) throws {
            let u = src.appendingPathComponent(rel)
            try fm.createDirectory(at: u.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data(text.utf8).write(to: u)
        }
        func entry(_ id: String, requires: [String] = [], files: [String: String]) throws -> [String: Any] {
            let (author, kind, name) = RecipeRef.parts(id)!
            let path = "recipes/\(author)/\(kind.rawValue)/\(name)"
            var list: [[String: Any]] = []
            for (f, text) in files.sorted(by: { $0.key < $1.key }) {
                try put("\(path)/\(f)", text)
                let sha = SHA256Hex.of(Data(text.utf8))
                list.append(["path": f, "size": text.utf8.count, "sha256": sha])
            }
            return ["id": id, "author": author, "kind": kind.rawValue, "name": name, "path": path, "description": "d",
                    "systems": ["aarch64-linux"], "requires": requires, "version": "v1-\(name)", "files": list]
        }
        var recipes = [try entry("omaflake.nixos.minimal", files: ["recipe.toml": "\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ncpus = 1", "default.nix": "{ }"]),
                       try entry("someone.nixos.rust", requires: ["omaflake/nixos/minimal"],
                                 files: ["recipe.toml": "\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ncpus = 2\nrequires = [\"omaflake/nixos/minimal\"]\n", "default.nix": "{ imports = [ ../../../omaflake/nixos/minimal ]; }"])]
        func catalog() throws -> RecipeCatalog {
            let json: [String: Any] = ["$schema": "https://omaflake.dev/schema/catalog/v2.json", "source": ["name": "test"], "recipes": recipes]
            return try RecipeCatalog.decode(JSONSerialization.data(withJSONObject: json))
        }
        let source = RecipeSource(kind: .folder, location: src.path)
        let lib = RecipeLibrary(paths: paths)
        let client = SourceClient(source)
        let installed = try await lib.install("someone/nixos/rust", catalog: catalog(), client: client)
        #expect(installed.map(RecipeID.base) == ["omaflake.nixos.minimal", "someone.nixos.rust"])
        let (minimal, rust) = (installed[0], installed[1])
        // the ID's words are the catalog's content hash, the same as the downloaded folder's
        #expect(rust == RecipeID.make(base: "someone.nixos.rust", contentHash: try catalog().recipes[1].contentHash))
        #expect(fm.fileExists(atPath: paths.recipes.appendingPathComponent("\(rust)/default.nix").path))
        #expect(try await lib.install("someone/nixos/rust", catalog: catalog(), client: client).isEmpty)   // same content
        #expect(throws: VMError.self) { try lib.remove(minimal) }                                         // still required

        // a new version whose served file does not match the catalog: nothing is written
        try put("recipes/someone/nixos/rust/default.nix", "{ evil = true; }")
        recipes[1]["version"] = "v2"
        var v2 = recipes[1]["files"] as! [[String: Any]]
        v2[0]["sha256"] = SHA256Hex.of(Data("{ v2 }".utf8))   // default.nix (sorted first)
        v2[0]["size"] = 6
        recipes[1]["files"] = v2
        await #expect(throws: VMError.self) { try await lib.install("someone/nixos/rust", catalog: catalog(), client: client) }
        #expect(try String(contentsOf: paths.recipes.appendingPathComponent("\(rust)/default.nix"), encoding: .utf8).contains("imports"))
        // a path leaving the recipe's folder is refused
        var files = recipes[1]["files"] as! [[String: Any]]
        files[0]["path"] = "../../../../escape"
        recipes[1]["files"] = files
        await #expect(throws: VMError.self) { try await lib.install("someone/nixos/rust", catalog: catalog(), client: client) }

        // staging: the downloaded recipes, a local folder's first; the same content, the same folder
        var settings = Settings()
        settings.sources = []
        let staged = try lib.stage(settings)
        // rust's tree has the minimal it requires next to it
        #expect(fm.fileExists(atPath: staged.appendingPathComponent("\(rust)/omaflake/nixos/minimal/recipe.toml").path))
        #expect(fm.fileExists(atPath: staged.appendingPathComponent("\(minimal)/omaflake/nixos/minimal/recipe.toml").path))
        #expect(try lib.stage(settings) == staged)
        let local = paths.root.appendingPathComponent("local-recipes/omaflake/nixos/minimal")
        try fm.createDirectory(at: local, withIntermediateDirectories: true)
        try Data("\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ncpus = 7".utf8).write(to: local.appendingPathComponent("recipe.toml"))
        try Data("{ }".utf8).write(to: local.appendingPathComponent("default.nix"))
        settings.sources = [RecipeSource(kind: .folder, location: paths.root.appendingPathComponent("local-recipes").path)]
        let withLocal = try lib.stage(settings)
        #expect(withLocal != staged)
        // another minimal (other content, other source) next to the downloaded one; rust still gets its own source's
        let localMinimal = try #require(lib.available(settings).first { $0.source == settings.sources![0].id }).id
        #expect(RecipeID.base(localMinimal) == "omaflake.nixos.minimal" && localMinimal != minimal)
        #expect(lib.available(settings).count == 3)
        #expect(try String(contentsOf: withLocal.appendingPathComponent("\(localMinimal)/omaflake/nixos/minimal/recipe.toml"), encoding: .utf8) == "\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ncpus = 7")
        #expect(try String(contentsOf: withLocal.appendingPathComponent("\(rust)/omaflake/nixos/minimal/recipe.toml"), encoding: .utf8) == "\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ncpus = 1")
        #expect(lib.distinctions(settings)[localMinimal] != nil && lib.distinctions(settings)[rust] == nil)

        try lib.remove(rust)
        try lib.remove(minimal)
        #expect(lib.installed().isEmpty)
    }
}

/// SHA-256 as hex (tests)
enum SHA256Hex {
    static func of(_ data: Data) -> String { CryptoKitHex.hex(data) }
}

@Suite struct InstanceStoreTests {
    @Test func nameValidation() throws {
        let store = InstanceStore(paths: try tempPaths(), qemuImg: "/usr/bin/false")
        for ok in ["dev", "lab-2", "a1"] { try store.validate(name: ok) }
        for bad in ["", "Dev", "-x", "a b", "ä", String(repeating: "x", count: 33)] {
            #expect(throws: VMError.self) { try store.validate(name: bad) }
        }
    }

    @Test func socketPathLimit() throws {
        let long = Paths(root: URL(fileURLWithPath: "/" + String(repeating: "d", count: 80)))
        let store = InstanceStore(paths: long, qemuImg: "/usr/bin/false")
        #expect(throws: VMError.self) { try store.validate(name: String(repeating: "x", count: 20)) }
    }

    @Test func saveLoadAndPorts() throws {
        let paths = try tempPaths()
        let store = InstanceStore(paths: paths, qemuImg: "/usr/bin/false")
        let c = config("t1")
        try FileManager.default.createDirectory(at: paths.instance("t1").dir, withIntermediateDirectories: true)
        try store.save(c)
        #expect(try store.load("t1") == c)
        // 2222 free → first port; ports in use and instance ports are skipped
        #expect(store.nextSSHPort(isFree: { _ in true }) == 2222)
        #expect(store.nextSSHPort(isFree: { $0 >= 2225 }) == 2225)
        var c2 = config("t2")
        c2.sshPort = 2222
        try FileManager.default.createDirectory(at: paths.instance("t2").dir, withIntermediateDirectories: true)
        try store.save(c2)
        #expect(store.nextSSHPort(isFree: { _ in true }) == 2223)
    }
}

@Suite struct DisposalTests {
    @Test func deletesFilesFoldersAndLinks() throws {
        let p = try tempPaths()
        let fm = FileManager.default
        let dir = p.root.appendingPathComponent("d")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = p.root.appendingPathComponent("f")
        try Data("x".utf8).write(to: file)
        let link = p.root.appendingPathComponent("l")
        try fm.createSymbolicLink(atPath: link.path, withDestinationPath: file.path)
        try Disposal.delete([link, dir, p.root.appendingPathComponent("missing")])
        #expect(!fm.fileExists(atPath: dir.path) && (try? fm.destinationOfSymbolicLink(atPath: link.path)) == nil)
        #expect(fm.fileExists(atPath: file.path))   // a link is deleted, not what it points to
    }
}

@Suite struct MacCloneTests {
    @Test func cloneCopiesFilesWithNewMAC() throws {
        let root = try tempPaths().root
        let src = MacVMBundle(root.appendingPathComponent("base"))
        try FileManager.default.createDirectory(at: src.dir, withIntermediateDirectories: true)
        try src.save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 1, macAddress: "c2:c1:76:60:ce:b1"))
        for f in [src.hardwareModel, src.machineID, src.auxStorage] { try Data("x".utf8).write(to: f) }
        try src.createDisk(gigabytes: 1)
        try Data("sock".utf8).write(to: src.dir.appendingPathComponent("runner.pid"))
        for f in src.files { try FileManager.default.setAttributes([.posixPermissions: 0o444], ofItemAtPath: f.path) }

        let copy = try src.clone(to: root.appendingPathComponent("copy"))
        let mac = try copy.loadConfig().macAddress
        #expect(mac != "c2:c1:76:60:ce:b1")
        #expect(FileManager.default.isWritableFile(atPath: copy.disk.path))
        #expect(!FileManager.default.fileExists(atPath: copy.dir.appendingPathComponent("runner.pid").path))
        #expect(try Data(contentsOf: copy.hardwareModel) == Data("x".utf8))
        #expect(throws: VMError.self) { try src.clone(to: copy.dir) }
    }

    @Test func randomMACIsLocalUnicast() {
        for _ in 0..<50 {
            let first = UInt8(MACAddress.randomLocallyAdministered().prefix(2), radix: 16)!
            #expect(first & 0x02 == 0x02)
            #expect(first & 0x01 == 0)
        }
    }
}

@Suite struct ImageStoreTests {
    @Test func imageNameIsUniquePerStorePath() {
        let a = "/nix/store/55pvcxp567f41g0yxxf7blrqicjc3afm-nixos-disk-image"
        let n = ImageStore.imageName(recipe: "omaflake.nixos.dev@able-acid-also", storePath: a)
        #expect(n == "omaflake.nixos.dev@" + OmaflakeID.words(of: a))
        #expect(OmaflakeID.isWords(String(n.split(separator: "@")[1]), full: true))
        #expect(ImageStore.imageName(recipe: "omaflake.nixos.dev", storePath: a + "x") != n)
        #expect(n.split(separator: "@")[1].split(separator: "-").count == 12)
    }

    @Test func flakeRefUsesPathForFoldersWithoutOwnRepo() throws {
        let dir = try tempPaths().root
        #expect(ImageStore.flakeRef(dir.path) == "path:\(dir.path)")
        try FileManager.default.createDirectory(at: dir.appendingPathComponent(".git"), withIntermediateDirectories: true)
        #expect(ImageStore.flakeRef(dir.path) == dir.path)
        #expect(ImageStore.flakeRef("github:omaflake/omaflake") == "github:omaflake/omaflake")
        #expect(ImageStore.flakeRef("path:/x") == "path:/x")
    }

    @Test func diskUsageFollowsSymlinksAndSumsFolders() throws {
        let dir = try tempPaths().root
        let file = dir.appendingPathComponent("disk.img")
        try Data(count: 64 * 1024).write(to: file)
        let link = dir.appendingPathComponent("link.qcow2")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: file)
        let info = ImageStore.Info(recipe: "omaflake.nixos.minimal", kind: .nixos, system: "aarch64-linux", source: "/x", revision: "abc", storePath: nil)
        let viaLink = ImageStore.diskUsage(ImageStore.Image(name: "a", info: info, path: link))
        #expect((viaLink ?? 0) >= 64 * 1024)
        let folder = ImageStore.diskUsage(ImageStore.Image(name: "b", info: info, path: dir))
        #expect((folder ?? 0) >= viaLink ?? 0)
        #expect(ImageStore.diskUsage(ImageStore.Image(name: "c", info: info, path: dir.appendingPathComponent("gone"))) == nil)
    }
}

@Suite struct QMPParseTests {
    @Test func parsesEventAndReturn() throws {
        // Order as observed in the boot probe (plan B13)
        let ev = try QMPClient.parse(Data(#"{"timestamp": {"seconds": 1, "microseconds": 2}, "event": "POWERDOWN"}"#.utf8))
        #expect(ev["event"] as? String == "POWERDOWN")
        let ret = try QMPClient.parse(Data(#"{"return": {}}"#.utf8))
        #expect(ret["return"] != nil)
    }
}

@Suite struct MacVMTests {
    // Excerpt as in /var/db/dhcpd_leases (2026-10-16): leading zeros per octet are missing
    static let leases = """
    {
    \tname=ManagedlMachine
    \tip_address=192.168.64.14
    \thw_address=1,e6:25:86:f:90:4d
    \tidentifier=1,e6:25:86:f:90:4d
    \tlease=0x6ad236f9
    }
    {
    \tname=old
    \tip_address=192.168.64.9
    \thw_address=1,e6:25:86:f:90:4d
    \tidentifier=1,e6:25:86:f:90:4d
    \tlease=0x6a000000
    }
    {
    \tname=archlinux
    \tip_address=192.168.64.13
    \thw_address=ff,f1:f5:dd:7f:0:2:0:0:ab:11:ef:55:ac:1e:8d:9:12:10
    \tlease=0x6a9da38d
    }
    """

    @Test func normalizesMAC() {
        #expect(DHCPLeases.normalize("E6:25:86:0F:90:4D") == "e6:25:86:f:90:4d")
        #expect(DHCPLeases.normalize("00:0a:00:00:00:01") == "0:a:0:0:0:1")
    }

    @Test func findsNewestLease() {
        #expect(DHCPLeases.ip(for: "e6:25:86:0f:90:4d", in: Self.leases) == "192.168.64.14")
        #expect(DHCPLeases.ip(for: "aa:bb:cc:dd:ee:ff", in: Self.leases) == nil)
    }

    @Test func bundleRoundTripAndSparseDisk() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("macvm-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let b = MacVMBundle(dir)
        let c = MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 2, macAddress: "e6:25:86:0f:90:4d", osBuild: "27A1")
        try b.save(c)
        #expect(try b.loadConfig() == c)
        try b.createDisk(gigabytes: 2)
        let attrs = try FileManager.default.attributesOfItem(atPath: b.disk.path)
        #expect(attrs[.size] as? UInt64 == 2 * 1024 * 1024 * 1024)
        let allocated = (try? b.disk.resourceValues(forKeys: [.totalFileAllocatedSizeKey]))?.totalFileAllocatedSize ?? 0
        #expect(allocated < 1024 * 1024)   // sparse
    }
}

@Suite struct BrandTests {
    @Test func embeddedIconsDecode() throws {
        let icon = try #require(Brand.appIcon())
        #expect(icon.representations.first?.pixelsWide == 512)
        let menu = Brand.menuBarImage()
        #expect(menu.isTemplate)
        #expect(menu.size == NSSize(width: 18, height: 18))
        #expect(menu.representations.map(\.pixelsWide).sorted() == [18, 36, 54])
    }

    @Test func dockIconFileIsWrittenOnce() throws {
        let p = try tempPaths()
        let url = try Brand.dockIconFile(p)
        #expect(url == p.dockIcon)
        let first = try FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date
        _ = try Brand.dockIconFile(p)
        let second = try FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date
        #expect(first == second)
        #expect(try Data(contentsOf: url).prefix(4) == Data([0x89, 0x50, 0x4E, 0x47]))
    }
}

@Suite struct VZBackendTests {
    func macConfig(_ name: String = "m1", network: NetworkMode = .shared) -> InstanceConfig {
        let s = RecipeDefaults(kind: .macos, cpus: 4, memoryGB: 8, diskGB: 80, network: network, share: false,
                               ephemeral: false, user: "test")
        return InstanceConfig(name: name, recipe: "mac-minimal", image: "mac-minimal-abc-12345678",
                              imagePath: "/img", system: "aarch64-darwin", settings: s, sshPort: 22, created: Date(timeIntervalSince1970: 0))
    }

    @Test func imageRecipeFromName() {
        #expect(ImageStore.recipe(ofImage: "mac-minimal-d7ad2da948-dirty-3c4r9p1j") == "mac-minimal")
        #expect(ImageStore.recipe(ofImage: "minimal-d1f95739ba-55pvcxp5") == "minimal")
        #expect(ImageStore.recipe(ofImage: "dev-nogit-abcdef12") == "dev")
    }

    @Test func macImageIsAVMFolder() throws {
        let p = try tempPaths()
        let name = "omaflake.macos.minimal-abc1234567-dirty-12345678"
        let dir = p.imageDir(name)
        #expect(dir.lastPathComponent == "\(name).omaflake-image")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try MacVMBundle(dir).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01"))
        // without image.json it is not an image
        #expect(throws: VMError.self) { try ImageStore(paths: p).image(named: name) }
        try ImageStore.writeInfo(.init(recipe: "omaflake.macos.minimal", kind: .macos, system: "aarch64-darwin", source: "/f", revision: "abc1234567-dirty",
                                       storePath: nil), to: dir)
        let image = try ImageStore(paths: p).image(named: name)
        #expect(image.kind == .macos && image.recipe == "omaflake.macos.minimal" && image.path.path == dir.path)
        #expect(image.info.author == "omaflake" && image.info.name == "minimal")
        #expect(ImageStore(paths: p).list().map(\.name) == [name])
    }

    @Test func isolatedNetworkIsRejected() throws {
        let p = try tempPaths()
        let vz = VZBackend(paths: p, store: InstanceStore(paths: p, qemuImg: "/usr/bin/false"), runner: "/usr/bin/false")
        let image = ImageStore.Image(name: "mac-minimal-abc-12345678",
                                     info: .init(recipe: "omaflake.macos.minimal", kind: .macos, system: "aarch64-darwin", source: "", revision: "abc", storePath: nil),
                                     path: p.images)
        var s = RecipeDefaults(kind: .macos, cpus: 4, memoryGB: 8, diskGB: 80, network: .isolated, share: false, ephemeral: false)
        #expect(throws: VMError.self) { try vz.create(name: "m1", recipe: "mac-minimal", image: image, settings: s) }
        #expect(!FileManager.default.fileExists(atPath: p.instance("m1").dir.path))
        s.network = .shared
        // a NixOS image cannot become a macOS instance
        let qcow = ImageStore.Image(name: "dev-abc-12345678",
                                    info: .init(recipe: "omaflake.nixos.dev", kind: .nixos, system: "aarch64-linux", source: "", revision: "abc", storePath: nil),
                                    path: p.images)
        #expect(throws: VMError.self) { try vz.create(name: "m2", recipe: "dev", image: qcow, settings: s) }
    }

    @Test func stoppedWithoutRunnerProcess() throws {
        let p = try tempPaths()
        let vz = VZBackend(paths: p, store: InstanceStore(paths: p, qemuImg: "/usr/bin/false"), runner: "/usr/bin/false")
        let c = macConfig()
        try FileManager.default.createDirectory(at: p.instance(c.name).dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: p.instance(c.name).run, withIntermediateDirectories: true)
        #expect(vz.status(c) == .stopped)
        // a live PID of another program (this test process) does not count as the runner
        try "\(getpid())\n".write(to: p.instance(c.name).runnerPid, atomically: true, encoding: .utf8)
        #expect(vz.status(c) == .stopped)
    }

    @Test func sshGoesToNATAddressPort22() throws {
        let p = try tempPaths()
        let c = macConfig()
        let args = SSHCommand.arguments(c, paths: p, host: "192.168.64.5", port: 22, command: ["uptime"])
        #expect(args.first == "/usr/bin/ssh")
        #expect(Array(args[1...2]) == ["-p", "22"])
        #expect(args.suffix(2) == ["test@192.168.64.5", "uptime"])
        // NixOS default unchanged: port forwarding on 127.0.0.1
        let n = SSHCommand.arguments(config(), paths: p)
        #expect(Array(n[1...2]) == ["-p", "2230"] && n.last?.hasSuffix("@127.0.0.1") == true)
    }

    @Test func instancePathsForMacGuests() throws {
        let p = try tempPaths()
        let ip = p.instance("m1")
        #expect(ip.vm.lastPathComponent == "vm" && ip.runnerSocket.lastPathComponent == "runner.sock")
        // the longer runner socket name limits instance names
        let store = InstanceStore(paths: Paths(root: URL(fileURLWithPath: "/" + String(repeating: "x", count: 60))), qemuImg: "")
        #expect(throws: VMError.self) { try store.validate(name: String(repeating: "a", count: 28)) }
        // runtime files live in run/<name>/, outside the package (plan B108)
        #expect(ip.runnerSocket.deletingLastPathComponent().path == p.run.appendingPathComponent("m1").path)
        #expect(ip.config.deletingLastPathComponent().lastPathComponent == "m1.omaflake-vm")
    }

    @Test func newestBaseSkipsFreshAndIncompleteFolders() throws {
        let p = try tempPaths()
        let vz = VZBackend(paths: p, store: InstanceStore(paths: p, qemuImg: "/usr/bin/false"), runner: "/usr/bin/false")
        let builder = MacImageBuilder(paths: p, vz: vz)
        #expect(builder.newestBase() == nil)
        for name in ["mac-base-26A434", "mac-fresh-27B100", "mac-base-27A200"] {
            let dir = p.bases.appendingPathComponent(name)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try MacVMBundle(dir).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01"))
            FileManager.default.createFile(atPath: dir.appendingPathComponent(MacBaseInstaller.readyMarker).path, contents: nil)
        }
        // a newer base without the ready marker (first boot unfinished) does not count
        let unfinished = p.bases.appendingPathComponent("mac-base-27Z999")
        try FileManager.default.createDirectory(at: unfinished, withIntermediateDirectories: true)
        try MacVMBundle(unfinished).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:02"))
        // newer name without config.json (e.g. an interrupted clone) does not count
        try FileManager.default.createDirectory(at: p.bases.appendingPathComponent("mac-base-28A1"), withIntermediateDirectories: true)
        #expect(builder.newestBase()?.lastPathComponent == "mac-base-27A200")
    }

    @Test func latestBuildFromRunnerOutput() {
        let out = "macOS 27.0.1 (26A434)\nURL:   https://example.invalid/x.ipsw\nSize:  26.6 GB\n"
        #expect(MacBaseInstaller.build(fromLatest: out) == "26A434")
        #expect(MacBaseInstaller.build(fromLatest: "") == nil)
    }
}

@Suite struct ShareFolderTests {
    @Test func defaultRootIsHiddenFolderInHome() {
        guard (ProcessInfo.processInfo.environment["VM_HOME"] ?? "").isEmpty else { return }
        #expect(Paths.standard().root.lastPathComponent == ".omaflake")
    }

    @Test func defaultShareIsCreatedWithMode() throws {
        let p = try tempPaths()
        let c = config("s1")
        let dir = try p.prepareShare(c, mode: 0o777)
        #expect(dir == p.share("s1"))
        let mode = try FileManager.default.attributesOfItem(atPath: dir.path)[.posixPermissions] as? Int
        #expect(mode == 0o777)
    }

    @Test func chosenShareMustExistAndKeepsItsPermissions() throws {
        let p = try tempPaths()
        var c = config("s2")
        c.sharePath = p.root.appendingPathComponent("mine").path
        #expect(throws: VMError.self) { try p.prepareShare(c) }
        try FileManager.default.createDirectory(atPath: c.sharePath!, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        #expect(try p.prepareShare(c, mode: 0o777).path == c.sharePath)
        let mode = try FileManager.default.attributesOfItem(atPath: c.sharePath!)[.posixPermissions] as? Int
        #expect(mode == 0o700)
        #expect(!FileManager.default.fileExists(atPath: p.share("s2").path))
    }

    @Test func instanceWithoutSharePathDecodes() throws {
        let data = try JSONEncoder().encode(config("s3"))
        #expect(try JSONDecoder().decode(InstanceConfig.self, from: data).sharePath == nil)
    }
}

@Suite struct DataFolderTests {
    @Test func movesOwnEntriesRewritesPathsKeepsOthers() throws {
        let tmp = try tempPaths().root
        let old = Paths(root: tmp.appendingPathComponent("a"))
        let settingsFile = tmp.appendingPathComponent("home/config.json")
        let fm = FileManager.default
        var c = config("d1")
        c.kind = .macos
        c.imagePath = old.images.appendingPathComponent("mac-x-abc-12345678").path
        c.sharePath = old.root.appendingPathComponent("share/custom").path
        try fm.createDirectory(at: old.instance("d1").dir, withIntermediateDirectories: true)
        try InstanceStore(paths: old, qemuImg: "/usr/bin/false").save(c)
        try Settings(flake: "/f", qemu: old.root.appendingPathComponent("qemu/current/bin/qemu-system-aarch64").path).save(old)
        try Data("secret".utf8).write(to: old.root.appendingPathComponent("agent.env"))

        let target = tmp.appendingPathComponent("b")
        let new = try DataFolder.move(from: old, to: target, settingsFile: settingsFile)
        #expect(fm.fileExists(atPath: new.instance("d1").config.path))
        #expect(!fm.fileExists(atPath: old.instances.path))
        // not omaflake's: stays behind
        #expect(fm.fileExists(atPath: old.root.appendingPathComponent("agent.env").path))
        let moved = try InstanceStore(paths: new, qemuImg: "/usr/bin/false").load("d1")
        #expect(moved.imagePath == new.images.appendingPathComponent("mac-x-abc-12345678").path)
        #expect(moved.sharePath == new.root.appendingPathComponent("share/custom").path)
        let s = Settings.load(file: settingsFile)
        #expect(s.home == new.root.path && s.flake == "/f")
        #expect(s.qemu == new.root.appendingPathComponent("qemu/current/bin/qemu-system-aarch64").path)
    }

    @Test func refusesClashesAndNesting() throws {
        let tmp = try tempPaths().root
        let old = Paths(root: tmp.appendingPathComponent("a"))
        let settingsFile = tmp.appendingPathComponent("home/config.json")
        try FileManager.default.createDirectory(at: old.images, withIntermediateDirectories: true)
        let target = tmp.appendingPathComponent("b")
        try FileManager.default.createDirectory(at: target.appendingPathComponent("images"), withIntermediateDirectories: true)
        #expect(throws: VMError.self) { try DataFolder.move(from: old, to: target, settingsFile: settingsFile) }
        #expect(FileManager.default.fileExists(atPath: old.images.path))
        #expect(throws: VMError.self) { try DataFolder.move(from: old, to: old.root.appendingPathComponent("x"), settingsFile: settingsFile) }
        #expect(throws: VMError.self) { try DataFolder.move(from: old, to: old.root, settingsFile: settingsFile) }
    }
}

@Suite struct DisposalReadOnlyTests {
    @Test func deletesReadOnlyFolders() throws {
        let p = try tempPaths()
        let dir = p.bases.appendingPathComponent("mac-base-1/sub")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("x".utf8).write(to: dir.appendingPathComponent("disk.img"))
        for d in [dir, dir.deletingLastPathComponent()] {
            try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: d.path)
        }
        try Disposal.delete([dir.deletingLastPathComponent()])
        #expect(!FileManager.default.fileExists(atPath: p.bases.appendingPathComponent("mac-base-1").path))
    }

    @Test func removingAnImageIsRefusedWhileUsedAndMovesCurrent() throws {
        let p = try tempPaths()
        let store = ImageStore(paths: p)
        func make(_ name: String, created: Date) throws {
            let dir = p.imageDir(name)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try MacVMBundle(dir).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01"))
            try ImageStore.writeInfo(.init(recipe: "omaflake.macos.minimal", kind: .macos, system: "aarch64-darwin", source: "", revision: name,
                                           storePath: nil, created: created), to: dir)
            try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path)
        }
        try make("old", created: Date(timeIntervalSince1970: 1))
        try make("new", created: Date(timeIntervalSince1970: 2))
        try store.setCurrent(imageName: "new")
        let instances = InstanceStore(paths: p, qemuImg: "/usr/bin/false")
        var c = config("m1")
        c.image = "new"
        try FileManager.default.createDirectory(at: p.instance("m1").dir, withIntermediateDirectories: true)
        try instances.save(c)
        #expect(throws: VMError.self) { try store.remove("new", instances: instances) }
        try instances.remove("m1")
        try store.remove("new", instances: instances)
        #expect(store.currentName(recipe: "omaflake.macos.minimal") == "old")
        try store.remove("old", instances: instances)
        #expect(store.currentName(recipe: "omaflake.macos.minimal") == nil && store.list().isEmpty)
    }
}

@Suite struct PrerequisitesTests {
    // `nix config show` excerpt and /etc/nix/machines as written by nix-darwin's linux-builder (2026-10-16)
    static let config = """
    builders = @/etc/nix/machines
    experimental-features = fetch-tree flakes nix-command
    max-jobs = auto
    """
    static let machines = "ssh-ng://builder@linux-builder aarch64-linux /etc/nix/builder_ed25519 6 1 kvm,benchmark,big-parallel - c3No\n"

    @Test func parsesConfigAndFlakes() {
        let c = Prerequisites.parseConfig(Self.config)
        #expect(c["builders"] == "@/etc/nix/machines")
        #expect(Prerequisites.flakesEnabled(c))
        #expect(!Prerequisites.flakesEnabled(["experimental-features": "nix-command"]))
    }

    @Test func findsLinuxBuilderInMachinesFile() {
        let lines = Prerequisites.builderLines("@/etc/nix/machines") { $0 == "/etc/nix/machines" ? Self.machines : nil }
        #expect(lines.count == 1 && Prerequisites.isLinuxBuilder(lines[0]))
        #expect(!Prerequisites.isLinuxBuilder("ssh://mac x86_64-darwin"))
        #expect(Prerequisites.builderLines("").isEmpty)
        // inline list, separated by ";"
        #expect(Prerequisites.builderLines("ssh://a aarch64-linux ; ssh://b x86_64-linux").count == 2)
    }

    @Test func hostIsAppleSilicon() {
        #expect(Prerequisites.machine() == "arm64")
    }

    @Test func missingFlakeAndCustomNixPath() throws {
        let p = try tempPaths()
        let pre = Prerequisites(paths: p)
        #expect(pre.flake(Settings(), nix: nil).status == .missing || ProcessInfo.processInfo.environment["VM_FLAKE"] != nil)
        var s = Settings()
        s.nix = "/does/not/exist/nix"
        #expect(Prerequisites.locateNix(s) == nil)
        #expect(pre.nixItem(nil).install == Prerequisites.installNix)
    }
}

@Suite struct PreflightTests {
    @Test func parsesRunnerLatest() {
        let out = "macOS 27.0.1 (26A434)\nURL:   https://updates.cdn-apple.com/x/UniversalMac_27.0.1_26A434_Restore.ipsw\nSize:  26.6 GB\n"
        let r = MacOSRelease.parse(out)
        #expect(r?.version == "27.0.1" && r?.build == "26A434" && r?.bytes == 26_600_000_000)
        #expect(MacOSRelease.parse("Error: offline") == nil)
    }

    @Test func findsBuilderInSSHConfig() {
        let conf = "Host other\n  Port 2\nHost linux-builder\n  User builder\n  Hostname localhost\n  HostKeyAlias linux-builder\n  Port 31022\n"
        let t = Prerequisites.builderSSHTarget(configs: [conf])
        #expect(t?.host == "localhost" && t?.port == 31022)
        #expect(Prerequisites.builderSSHTarget(configs: ["Host x\n  Port 1\n"]) == nil)
    }

    @Test func freeSpaceFallsBackForTheNixStore() {
        // the read-only store volume reports 0 for "important usage" – the plain value is used then
        #expect((freeSpace(at: URL(fileURLWithPath: "/nix/store")) ?? 0) > 0 || !FileManager.default.fileExists(atPath: "/nix/store"))
    }
}

@Suite struct BaseUserTests {
    @Test func userFromMarkerOrFirstBootLog() throws {
        let p = try tempPaths()
        let dir = p.bases.appendingPathComponent("mac-base-1")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: p.logs, withIntermediateDirectories: true)
        #expect(MacBaseInstaller.user(of: dir, logs: p.logs) == nil)
        // older base: empty marker, user from the log of its first boot (last provisioning line wins)
        FileManager.default.createFile(atPath: dir.appendingPathComponent(MacBaseInstaller.readyMarker).path, contents: nil)
        try "[23:37:39] Provisioning: user alice, auto-login on, SSH on\n[00:12:48] Provisioning: user bob, auto-login on, SSH on\n"
            .write(to: p.logs.appendingPathComponent("first-boot-mac-base-1.log"), atomically: true, encoding: .utf8)
        #expect(MacBaseInstaller.user(of: dir, logs: p.logs) == "bob")
        // current base: the marker carries the user
        try "carol\n".write(to: dir.appendingPathComponent(MacBaseInstaller.readyMarker), atomically: true, encoding: .utf8)
        #expect(MacBaseInstaller.user(of: dir, logs: p.logs) == "carol")
    }
}

@Suite struct GuestSettingsTests {
    @Test func keyboardFromInputSource() {
        #expect(GuestSettings.keyboard(fromInputSource: "com.apple.keylayout.German") == "de")
        #expect(GuestSettings.keyboard(fromInputSource: "com.apple.keylayout.ABC") == "us")
        #expect(GuestSettings.keyboard(fromInputSource: "com.apple.keylayout.Klingon") == nil)
    }

    @Test func overridesAndBaseUserWin() throws {
        let p = try tempPaths()
        var s = Settings()
        s.guest = GuestOverrides(user: "dev", keyboard: "us")
        try s.save(p)
        let base = p.bases.appendingPathComponent("mac-base-1")
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        try MacVMBundle(base).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01"))
        try "carol\n".write(to: base.appendingPathComponent(MacBaseInstaller.readyMarker), atomically: true, encoding: .utf8)
        let g = GuestSettings.current(p)
        #expect(g.user == "dev" && g.keyboard == "us" && g.macUser == "carol")
        // written once, stable content (no rewrite when unchanged)
        let dir = try GuestSettings.write(p)
        let first = try FileManager.default.attributesOfItem(atPath: dir.appendingPathComponent("settings.json").path)[.modificationDate] as? Date
        try GuestSettings.write(p)
        let second = try FileManager.default.attributesOfItem(atPath: dir.appendingPathComponent("settings.json").path)[.modificationDate] as? Date
        #expect(first == second)
    }

    @Test func guestArgumentsOnlyForFlakesWithTheInput() throws {
        let p = try tempPaths()
        let withInput = p.root.appendingPathComponent("a"), without = p.root.appendingPathComponent("b")
        for d in [withInput, without] { try FileManager.default.createDirectory(at: d, withIntermediateDirectories: true) }
        try "{ inputs.omaflake-guest = { url = \"path:./guest\"; flake = false; }; }".write(to: withInput.appendingPathComponent("flake.nix"), atomically: true, encoding: .utf8)
        try "{ }".write(to: without.appendingPathComponent("flake.nix"), atomically: true, encoding: .utf8)
        let store = ImageStore(paths: p)
        #expect(store.guestArguments(flake: withInput.path).contains("--no-write-lock-file"))
        #expect(store.guestArguments(flake: without.path).isEmpty)
    }
}

@Suite struct RecipeIDTests {
    static func d(_ author: String, _ kind: GuestKind, _ name: String) -> RecipeDefaults {
        var f = RecipeDefaults(kind: kind, cpus: 4, memoryGB: 4, diskGB: 40, network: .shared, share: true, ephemeral: false)
        f.name = name
        f.author = author
        return f
    }
    static let defaults = [
        "omaflake.nixos.minimal": d("omaflake", .nixos, "minimal"), "omaflake.macos.minimal": d("omaflake", .macos, "minimal"),
        "omaflake.nixos.dev": d("omaflake", .nixos, "dev"), "alice.nixos.dev": d("alice", .nixos, "dev"),
    ]

    @Test func resolvesAsMuchAsNeeded() throws {
        #expect(try RecipeID.resolve("omaflake.nixos.dev", in: Self.defaults) == "omaflake.nixos.dev")
        #expect(try RecipeID.resolve("macos/minimal", in: Self.defaults) == "omaflake.macos.minimal")
        #expect(try RecipeID.resolve("alice/nixos/dev", in: Self.defaults) == "alice.nixos.dev")
        #expect(throws: VMError.self) { try RecipeID.resolve("minimal", in: Self.defaults) }        // two guest kinds
        #expect(throws: VMError.self) { try RecipeID.resolve("nixos/dev", in: Self.defaults) }     // two authors
        #expect(throws: VMError.self) { try RecipeID.resolve("nope", in: Self.defaults) }
        // the chooser decides between several
        #expect(try RecipeID.resolve("dev", in: Self.defaults, choose: { $0.last }) == "omaflake.nixos.dev")
        #expect(RecipeID.candidates("dev", in: Self.defaults) == ["alice.nixos.dev", "omaflake.nixos.dev"])
    }

    @Test func partsAndNames() {
        #expect(RecipeID.parts("omaflake.macos.minimal")?.kind == .macos)
        #expect(RecipeID.parts("minimal") == nil && RecipeID.parts("x.windows.y") == nil)
        #expect(RecipeID.name(ofID: "omaflake.macos.minimal") == "minimal")
        #expect(RecipeID.qualified("alice.nixos.dev") == "alice/nixos/dev")
    }

    @Test func legacyNamesMapToOmaflakeIDs() {
        #expect(ImageStore.legacyRecipeID("minimal", kind: .nixos) == "omaflake.nixos.minimal")
        #expect(ImageStore.legacyRecipeID("nixos-minimal", kind: .nixos) == "omaflake.nixos.minimal")
        #expect(ImageStore.legacyRecipeID("mac-minimal", kind: .macos) == "omaflake.macos.minimal")
        #expect(ImageStore.legacyRecipeID("macos-minimal", kind: .macos) == "omaflake.macos.minimal")
        #expect(ImageStore.legacyRevision("mac-minimal-46c254685a-dirty-8ymcgbkl", recipe: "mac-minimal") == "46c254685a-dirty")
    }

    @Test func migratesLegacyMacImageCurrentLinkAndInstance() throws {
        let p = try tempPaths()
        let fm = FileManager.default
        let img = p.images.appendingPathComponent("mac-minimal-46c254685a-dirty-8ymcgbkl")
        try fm.createDirectory(at: img, withIntermediateDirectories: true)
        try MacVMBundle(img).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01"))
        try fm.setAttributes([.posixPermissions: 0o555], ofItemAtPath: img.path)
        try fm.createDirectory(at: p.currentImages, withIntermediateDirectories: true)
        try fm.createSymbolicLink(atPath: p.currentImages.appendingPathComponent("mac-minimal").path, withDestinationPath: "../\(img.lastPathComponent)")
        var c = config("m1")
        c.kind = .macos
        c.recipe = "mac-minimal"
        try fm.createDirectory(at: p.instance("m1").dir, withIntermediateDirectories: true)
        try InstanceStore(paths: p, qemuImg: "/usr/bin/false").save(c)

        let store = ImageStore(paths: p)
        #expect(!store.migrateLegacy().isEmpty)
        #expect(store.current(recipe: "omaflake.macos.minimal")?.info.revision == "46c254685a-dirty")
        #expect(!fm.isWritableFile(atPath: img.path))   // stays read-only
        #expect(try InstanceStore(paths: p, qemuImg: "/usr/bin/false").load("m1").recipe == "omaflake.macos.minimal")
        #expect(store.migrateLegacy().isEmpty)          // idempotent
    }
}


@Suite struct CancellationTests {
    @Test func secondBuildOfTheSameImageIsRefused() throws {
        let p = try tempPaths()
        let file = p.images.appendingPathComponent(".build-x.lock")
        do {
            let first = try FileLock(file, busy: "busy")
            #expect(throws: VMError.self) { _ = try FileLock(file, busy: "busy") }
            withExtendedLifetime(first) {}
        }
        // Free again after the first is gone – within a moment: a process another test starts at the same time holds
        // a copy of every open file until its exec closes the close-on-exec ones, so the lock may outlive `first` by
        // a few milliseconds (seen with lsof, plan B106); in use that is harmless
        var again: FileLock?
        let deadline = Date().addingTimeInterval(1)
        while again == nil {
            do { again = try FileLock(file, busy: "busy") } catch where Date() < deadline { usleep(5_000) }
        }
        withExtendedLifetime(again) {}
    }

    /// A refused lock must not touch the holder's lock file (plan B104: its deinit used to unlink it)
    @Test func refusedLockKeepsTheHoldersFile() throws {
        let p = try tempPaths()
        let file = p.images.appendingPathComponent(".build-y.lock")
        let holder = try FileLock(file, busy: "busy", removeWhenDone: true)
        #expect(throws: VMError.self) { _ = try FileLock(file, busy: "busy", removeWhenDone: true) }
        #expect(FileManager.default.fileExists(atPath: file.path))
        withExtendedLifetime(holder) {}
    }

    @Test func cancelStopsTheRunningShellProcess() throws {
        let token = Cancellation()
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.5) { token.cancel() }
        let t0 = Date()
        let r = try Cancellation.$current.withValue(token) { try Shell.run("/bin/sleep", ["10"]) }
        #expect(r.status != 0 && Date().timeIntervalSince(t0) < 5)
        #expect(throws: VMError.self) { try Cancellation.$current.withValue(token) { try Shell.run("/bin/echo", ["x"]) } }
    }
}

@Suite struct BuilderMaintenanceTests {
    @Test func shellQuoting() {
        #expect(shellQuote("a b") == "'a b'")
        #expect(shellQuote("it's") == "'it'\\''s'")
    }

    @Test func activateAndRootScript() throws {
        let paths = try tempPaths()
        let root = paths.root
        var settings = Settings.load(paths)
        settings.flake = "/src/omaflake/nix"
        try settings.save(paths)
        #expect(BuilderMaintenance.activateCommand(system: "/nix/store/abc-darwin-system")
                == "nix-env -p /nix/var/nix/profiles/system --set '/nix/store/abc-darwin-system' && '/nix/store/abc-darwin-system'/activate")
        #expect(!BuilderMaintenance.isSystem("/tmp/x"))
        let script = try BuilderMaintenance(paths: paths).rootScript("true")
        #expect(script.hasPrefix("set -o pipefail; export HOME=/var/root"))
        #expect(FileManager.default.fileExists(atPath: root.appendingPathComponent("logs/linux-builder.log").path))
    }
}

@Suite struct GuestChoicesTests {
    /// Every keyboard the app offers has a macOS layout in the darwin guest module
    @Test func everyKeyboardHasAMacLayout() throws {
        let module = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("nix/modules/darwin-guest.nix")
        let text = try String(contentsOf: module, encoding: .utf8)
        for k in GuestChoices.keyboards { #expect(text.contains("    \(k.code) = { id = "), "\(k.code)") }
    }

    @Test func localesAreTheIntersection() {
        let l = GuestChoices.locales
        #expect(l.contains("de_DE.UTF-8") && l.contains("en_US.UTF-8") && l.contains("nb_NO.UTF-8"))
        #expect(!l.contains("aa_DJ.UTF-8"))   // glibc has it, macOS has no user interface in Afar
        #expect(l.allSatisfy { GuestChoices.glibcLocales.contains($0) })
    }
}

@Suite struct RecipeFieldsTests {
    @Test func displayDescriptionLabFromTOMLJSON() throws {
        let json = #"{"systems":["aarch64-linux"],"cpus":4,"memory_gb":4,"disk_gb":40,"network":"shared","share":true,"ephemeral":false,"#
            + #""display":"gpu","description":"NixOS","description_de":"NixOS (de)","lab":true}"#
        let d = try JSONDecoder().decode(RecipeDefaults.self, from: Data(json.utf8))
        #expect(d.display == .gpu && d.isLab)
        #expect(d.description(in: "de") == "NixOS (de)" && d.description(in: "fr") == "NixOS")
        let c = InstanceConfig(name: "a", recipe: "f", image: "i", imagePath: "/i", system: "aarch64-linux", settings: d, sshPort: 2222)
        #expect(c.display == .gpu)
        // encoding keeps the fields (instance.json, round trip)
        let again = try JSONDecoder().decode(RecipeDefaults.self, from: JSONEncoder().encode(d))
        #expect(again == d)
    }
}

@Suite struct RetentionTests {
    /// current + one before + used ones stay; the rest goes
    @Test func pruneKeepsCurrentPreviousAndUsed() throws {
        let paths = try tempPaths()
        let store = ImageStore(paths: paths)
        let fm = FileManager.default
        for (i, name) in ["a1", "a2", "a3", "a4", "a5"].enumerated() {
            let dir = paths.imageDir(name)
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            var info = ImageStore.Info(recipe: "omaflake.macos.x", kind: .macos, system: "aarch64-darwin", source: "/f", revision: "r", storePath: "/nix/store/x")
            info.created = Date(timeIntervalSince1970: TimeInterval(1000 + i))
            try ImageStore.writeInfo(info, to: dir)
        }
        try store.setCurrent(imageName: "a5")
        let removed = store.prune(recipe: "omaflake.macos.x", used: ["a2"])
        #expect(Set(removed) == ["a1", "a3"])
        #expect(Set(store.images(of: "omaflake.macos.x").map(\.name)) == ["a5", "a4", "a2"])
    }
}

@Suite struct SetupCommandTests {
    @Test func firstActivationMovesUnmanagedEtcFilesAside() {
        let cmd = BuilderMaintenance.firstActivationCommand(system: "/nix/store/x-darwin-system")
        #expect(cmd.contains("if [ -f /etc/nix/nix.conf ] && [ ! -L /etc/nix/nix.conf ]; then mv /etc/nix/nix.conf /etc/nix/nix.conf.before-nix-darwin; fi"))
        #expect(cmd.hasSuffix("nix-env -p /nix/var/nix/profiles/system --set '/nix/store/x-darwin-system' && '/nix/store/x-darwin-system'/activate"))
        #expect(!cmd.contains("rm "))   // rename, never delete
    }
}

@Suite struct BuildProgressTests {
    @Test func nixosStepsAndPackageCounts() {
        let t = BuildProgressTracker(kind: .nixos)
        #expect(t.progress.step == 1 && t.progress.total == 4)
        t.feed("these 3 derivations will be built:")
        t.feed("these 2 paths will be fetched (1.2 MiB download, 5 MiB unpacked):")
        #expect(t.progress.step == 2 && t.progress.of == 5)
        t.feed("building '/nix/store/aaa-x.drv'...")
        t.feed("copying path '/nix/store/bbb-y' from 'https://cache.nixos.org'...")
        #expect(t.progress.done == 2)
        t.feed("nixos-disk-image> copying the closure")
        #expect(t.progress.step == 3 && t.progress.done == 0)
        t.feed("copying path '/nix/store/ccc-nixos-disk-image' from 'ssh-ng://builder@linux-builder'...")
        #expect(t.progress.step == 4)
        #expect(t.progress.fraction > 0.74 && t.progress.fraction <= 1)
    }

    @Test func macosStepsFollowTheBuilderLog() {
        let t = BuildProgressTracker(kind: .macos)
        #expect(t.progress.total == 7)
        t.feed("Cloning mac-base-26A434 (log: /x)")
        #expect(t.progress.step == 2)
        t.feed("Activating")
        #expect(t.progress.step == 6)
        t.feed("Cloning again")   // never goes back
        #expect(t.progress.step == 6)
    }
}

@Suite struct NixInstallerPinTests {
    @Test func installerIsCheckedBeforeItRuns() {
        let cmd = BuilderMaintenance.installNixCommand
        let check = cmd.range(of: "shasum -a 256 -c -")!, run = cmd.range(of: "install --no-confirm")!
        #expect(check.lowerBound < run.lowerBound)
        #expect(cmd.hasPrefix("set -e;"))   // a wrong checksum stops the script
        #expect(cmd.contains("/releases/download/\(BuilderMaintenance.nixInstallerVersion)/nix-installer-aarch64-darwin"))
    }
}

@Suite struct UpdateCheckTests {
    @Test func versionComparison() {
        #expect(UpdateCheck.isNewer("0.10.0", than: "0.9.1"))
        #expect(UpdateCheck.isNewer("0.2", than: "0.1.9"))
        #expect(!UpdateCheck.isNewer("0.2.0", than: "0.2"))
        #expect(!UpdateCheck.isNewer("0.2.0-beta", than: "0.2.0"))
    }

    @Test func feedFromAFile() async throws {
        let paths = try tempPaths()
        let feed = paths.root.appendingPathComponent("feed.json")
        try #"{"version":"9.0.0","url":"https://example.invalid/o.zip","sha256":null,"notes":"n","page":null}"#
            .write(to: feed, atomically: true, encoding: .utf8)
        let release = try await UpdateCheck.newer(than: "0.1", feed: feed)
        #expect(release?.version == "9.0.0")
        #expect(try await UpdateCheck.newer(than: "9.0.0", feed: feed) == nil)
    }
}

@Suite struct BuildLogTests {
    @Test func routineLinesAreNoise() {
        #expect(BuildLog.isNoise("nixos-disk-image> error while reading directory /build/root/nix/store/x/share/locale/et: No such file or directory"))
        #expect(BuildLog.isNoise("nixos-disk-image>   "))
        #expect(BuildLog.isNoise("nixos-disk-image> copying path '/nix/store/abc-foo' to 'local'..."))
        #expect(!BuildLog.isNoise("nixos-disk-image> Disk /dev/vda: 12 GiB"))
        #expect(!BuildLog.isNoise("error: build of '/nix/store/x.drv' failed"))
        #expect(BuildLog.isNoise("copying path '/nix/store/abc' from 'https://cache.nixos.org'..."))
        #expect(BuildLog.isNoise("  /nix/store/abc-foo-1.0"))
        #expect(BuildLog.isNoise("nixos-disk-image>   /nix/store/abc-perl-env"))
        #expect(!BuildLog.isNoise("these 57 paths will be fetched (1.2 GiB download):"))
    }
}

@Suite struct ShareCleanupTests {
    @Test func onlyMetadataGoes() throws {
        let paths = try tempPaths()
        let fm = FileManager.default
        let empty = paths.share("a"), used = paths.share("b")
        try fm.createDirectory(at: empty.appendingPathComponent(".fseventsd"), withIntermediateDirectories: true)
        try Data().write(to: empty.appendingPathComponent(".DS_Store"))
        try fm.createDirectory(at: used, withIntermediateDirectories: true)
        try Data("x".utf8).write(to: used.appendingPathComponent("thesis.tex"))
        #expect(InstanceStore.removeShareIfEmpty(empty))
        #expect(!fm.fileExists(atPath: empty.path))
        #expect(!InstanceStore.removeShareIfEmpty(used))
        #expect(fm.fileExists(atPath: used.appendingPathComponent("thesis.tex").path))
    }
}

enum CryptoKitHex {
    static func hex(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
}

@Suite struct PackageTests {
    @Test func namesAndRelativePaths() {
        #expect(Package.name(of: URL(fileURLWithPath: "/a/research.omaflake-vm"), kind: .vm) == "research")
        #expect(Package.name(of: URL(fileURLWithPath: "/a/research.omaflake-vm"), kind: .image) == nil)
        #expect(Package.kind(of: URL(fileURLWithPath: "/a/x.omaflake-recipe")) == .recipe)
        let rel = Package.relativePath(from: URL(fileURLWithPath: "/d/instances/a.omaflake-vm"),
                                       to: URL(fileURLWithPath: "/d/images/x.omaflake-image/disk.qcow2"))
        #expect(rel == "../../images/x.omaflake-image/disk.qcow2")
        #expect(Package.relativePath(from: URL(fileURLWithPath: "/v/a.omaflake-vm"),
                                     to: URL(fileURLWithPath: "/v/a.omaflake-vm/image.omaflake-image/disk.qcow2"))
                == "image.omaflake-image/disk.qcow2")
    }

    /// Plan B108: folders of the old layout become packages, runtime files go to run/<name>/
    @Test func oldLayoutBecomesPackages() throws {
        let p = try tempPaths()
        let fm = FileManager.default
        // NixOS image: folder with image.json + result → a folder with the qcow2 (stands in for the store path)
        let store = p.root.appendingPathComponent("fake-store")
        try fm.createDirectory(at: store, withIntermediateDirectories: true)
        try Data("qcow".utf8).write(to: store.appendingPathComponent("nixos.qcow2"))
        let nix = p.images.appendingPathComponent("omaflake.nixos.lab-abc-12345678")
        try fm.createDirectory(at: nix, withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: nix.appendingPathComponent("result"), withDestinationURL: store)
        try ImageStore.writeInfo(.init(recipe: "omaflake.nixos.lab", kind: .nixos, system: "aarch64-linux", source: "", revision: "abc", storePath: nil), to: nix)
        try fm.createDirectory(at: p.currentImages, withIntermediateDirectories: true)
        try fm.createSymbolicLink(atPath: p.currentImages.appendingPathComponent("omaflake.nixos.lab").path,
                                  withDestinationPath: "../omaflake.nixos.lab-abc-12345678")
        // VM of the old layout with a runtime file
        let old = p.instances.appendingPathComponent("t1")
        try fm.createDirectory(at: old, withIntermediateDirectories: true)
        try Data("{}".utf8).write(to: old.appendingPathComponent("instance.json"))
        try Data("log".utf8).write(to: old.appendingPathComponent("qemu.log"))
        try Data("disk".utf8).write(to: old.appendingPathComponent("disk.qcow2"))

        let images = ImageStore(paths: p)
        let notes = images.migratePackages(qemuImg: "/usr/bin/false")
        #expect(notes.count == 2)
        let image = try images.image(named: "omaflake.nixos.lab-abc-12345678")
        #expect(image.path == p.imageDir(image.name).appendingPathComponent("disk.qcow2"))
        #expect(try Data(contentsOf: image.path) == Data("qcow".utf8))
        #expect((try? fm.destinationOfSymbolicLink(atPath: p.imageDir(image.name).appendingPathComponent("result").path)) == nil)
        #expect(try fm.destinationOfSymbolicLink(atPath: p.currentImages.appendingPathComponent("omaflake.nixos.lab").path)
                == "../\(image.name).omaflake-image")
        let pkg = p.instances.appendingPathComponent("t1.omaflake-vm")
        #expect(fm.fileExists(atPath: pkg.appendingPathComponent("disk.qcow2").path))
        #expect(!fm.fileExists(atPath: pkg.appendingPathComponent("qemu.log").path))
        #expect(fm.fileExists(atPath: p.run.appendingPathComponent("t1/qemu.log").path))
        // idempotent
        #expect(images.migratePackages(qemuImg: "/usr/bin/false").isEmpty)
    }
}

@Suite struct TransferTests {
    func macVM(_ p: Paths, _ name: String) throws -> InstanceConfig {
        let s = RecipeDefaults(kind: .macos, cpus: 4, memoryGB: 8, diskGB: 80, network: .shared, share: false,
                               ephemeral: false, user: "test")
        let c = InstanceConfig(name: name, recipe: "omaflake.macos.minimal", image: "img-1", imagePath: "/img",
                               system: "aarch64-darwin", settings: s, sshPort: 22, created: Date(timeIntervalSince1970: 0))
        let ip = p.instance(name)
        try FileManager.default.createDirectory(at: ip.vm, withIntermediateDirectories: true)
        try Data("disk".utf8).write(to: ip.vm.appendingPathComponent("disk.img"))
        try InstanceStore(paths: p, qemuImg: "/usr/bin/false").save(c)
        return c
    }

    /// Backup and back: a stopped VM goes out as a package and comes back as the same VM
    @Test func vmGoesOutAndComesBack() throws {
        let p = try tempPaths()
        let out = p.root.appendingPathComponent("stick")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        _ = try macVM(p, "m1")
        let t = Transfer(paths: p, qemuImg: "/usr/bin/false", runner: nil)
        let pkg = try t.exportVM("m1", to: out)
        #expect(pkg.lastPathComponent == "m1.omaflake-vm")
        #expect(Package.Manifest.read(pkg)?.kind == .vm && Package.Manifest.read(pkg)?.containsImage == false)
        #expect(throws: VMError.self) { try t.exportVM("m1", to: out) }   // exists already
        // the same VM cannot come in twice; under another name a macOS VM needs omaf-runner (new identity)
        #expect(try t.conflict(pkg) == .vmName("m1"))
        #expect(throws: VMError.self) { try t.importPackage(pkg) }
        #expect(throws: VMError.self) { try t.importPackage(pkg, name: "m2") }
        #expect(!FileManager.default.fileExists(atPath: p.instances.appendingPathComponent("m2.omaflake-vm").path))
        try Disposal.delete([p.instance("m1").dir])
        #expect(try t.conflict(pkg) == .none)
        let r = try t.importPackage(pkg)
        #expect(r.kind == .vm && r.names == ["m1"])
        let store = InstanceStore(paths: p, qemuImg: "/usr/bin/false")
        #expect(try store.load("m1").name == "m1")
        #expect(!FileManager.default.fileExists(atPath: p.instance("m1").dir.appendingPathComponent("manifest.json").path))
        #expect(try Data(contentsOf: p.instance("m1").vm.appendingPathComponent("disk.img")) == Data("disk".utf8))
        // under another name with omaf-runner standing in (the clone itself is checked with a real runner)
        let fakeRunner = p.root.appendingPathComponent("runner.sh")
        try Data("#!/bin/sh\n[ \"$1\" = clone ] && cp -R \"$2\" \"$3\"\n".utf8).write(to: fakeRunner)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: fakeRunner.path)
        _ = try Transfer(paths: p, qemuImg: "/usr/bin/false", runner: fakeRunner.path).importPackage(pkg, name: "m2")
        #expect(try store.load("m2").name == "m2")
        #expect(store.list().map(\.name) == ["m1", "m2"])
    }

    /// A VM package registered in place shows up while its folder is there and can be forgotten while it is not
    @Test func externalVMIsListedWhileThere() throws {
        let p = try tempPaths()
        let out = p.root.appendingPathComponent("stick")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        _ = try macVM(p, "m1")
        let t = Transfer(paths: p, qemuImg: "/usr/bin/false", runner: nil)
        let pkg = try t.exportVM("m1", to: out)
        try Disposal.delete([p.instance("m1").dir])
        #expect(try t.useExternal(pkg) == "m1")
        #expect(throws: VMError.self) { try t.useExternal(pkg) }   // runs from there already
        let store = InstanceStore(paths: p, qemuImg: "/usr/bin/false")
        #expect(store.list().map(\.name) == ["m1"] && store.isExternal("m1"))
        #expect(p.instance("m1").dir.standardizedFileURL.path == pkg.standardizedFileURL.path)
        // imported from its place: back in the data folder, no longer external; then external again
        #expect(try t.conflict(pkg) == .none)
        _ = try t.importPackage(pkg)
        #expect(!store.isExternal("m1") && Settings.load(file: p.configFile).externalVMs == [])
        #expect(throws: VMError.self) { try t.useExternal(pkg) }   // the name is taken now
        try Disposal.delete([p.instance("m1").dir])
        _ = try t.useExternal(pkg)
        // the stick is pulled: gone from the list, still forgettable
        let away = out.deletingLastPathComponent().appendingPathComponent("away")
        try FileManager.default.moveItem(at: out, to: away)
        #expect(store.list().isEmpty)
        try store.forget("m1")
        #expect(Settings.load(file: p.configFile).externalVMs == [])
    }

    @Test func imageGoesOutAndComesBack() throws {
        let p = try tempPaths()
        let dir = p.imageDir("img-1")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try MacVMBundle(dir).save(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01"))
        try ImageStore.writeInfo(.init(recipe: "omaflake.macos.minimal", kind: .macos, system: "aarch64-darwin", source: "", revision: "r", storePath: nil), to: dir)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path)
        let out = p.root.appendingPathComponent("stick")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let t = Transfer(paths: p, qemuImg: "/usr/bin/false", runner: nil)
        let pkg = try t.exportImage("img-1", to: out)
        #expect(Package.Manifest.read(pkg)?.recipe == "omaflake.macos.minimal")
        try Disposal.delete([dir])
        let r = try t.importPackage(pkg)
        #expect(r.kind == .image && r.names == ["img-1"])
        let images = ImageStore(paths: p)
        #expect(images.list().map(\.name) == ["img-1"] && images.currentName(recipe: "omaflake.macos.minimal") == "img-1")
        // there already: an error unless it is replaced
        #expect(try t.conflict(pkg) == .image("img-1"))
        #expect(throws: VMError.self) { try t.importPackage(pkg) }
        #expect(try t.importPackage(pkg, replace: true).notes == ["image img-1 replaced"])
        #expect(images.list().map(\.name) == ["img-1"])
    }

    @Test func recipesTravelWithWhatTheyRequire() throws {
        let p = try tempPaths()
        let src = p.root.appendingPathComponent("src/recipes/me/nixos")
        for (name, req) in [("base", ""), ("desk", "requires = [\"me/nixos/base\"]")] {
            let d = src.appendingPathComponent(name)
            try FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
            try Data("\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ndescription = \"x\"\n\(req)\n".utf8).write(to: d.appendingPathComponent("recipe.toml"))
            try Data("{ ... }: { }\n".utf8).write(to: d.appendingPathComponent("default.nix"))
        }
        let library = RecipeLibrary(paths: p)
        // one recipe folder without the one it requires is refused
        #expect(throws: VMError.self) { try library.importRecipes(from: src.appendingPathComponent("desk")) }
        let ids = try library.importRecipes(from: p.root.appendingPathComponent("src")).sorted()
        #expect(ids.map(RecipeID.base) == ["me.nixos.base", "me.nixos.desk"])
        #expect(ids.allSatisfy { RecipeID.words($0).map { OmaflakeID.isWords($0, full: true) } == true })
        let desk = ids[1]
        #expect(library.installed()[desk]?.requires == ["me.nixos.base"])
        #expect(library.installed()[desk]?.source == RecipeLibrary.importedSource)
        #expect(FileManager.default.fileExists(atPath: p.recipes.appendingPathComponent("\(desk)/recipe.toml").path))
        let out = p.root.appendingPathComponent("stick")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let pkg = try library.exportRecipe(desk, to: out, settings: Settings.load(p))
        #expect(pkg.lastPathComponent == "me.nixos.desk.omaflake-recipe")
        #expect(RecipeLibrary.recipeFolders(in: pkg).map(\.id) == ["me.nixos.base", "me.nixos.desk"])
        // into another data folder: the same content gives the same IDs
        let q = try tempPaths()
        let tq = Transfer(paths: q, qemuImg: "/usr/bin/false", runner: nil)
        #expect(try tq.conflict(pkg) == .none)
        let r = try tq.importPackage(pkg)
        #expect(r.kind == .recipe && r.names.sorted() == ids)
        // a second time, identical: nothing written
        #expect(try tq.importPackage(pkg).names.isEmpty)
        // changed: a recipe of its own next to the first (plan B110) – kept with the setting (B111), told apart by its words
        var keep = Settings.load(q); keep.keepRecipeVersions = true; try keep.save(q)
        let changed = pkg.appendingPathComponent("recipes/me/nixos/desk/default.nix")
        try Data("{ ... }: { networking.hostName = \"x\"; }\n".utf8).write(to: changed)
        #expect(try tq.conflict(pkg) == .none)
        let second = try tq.importPackage(pkg).names
        #expect(second.count == 1 && RecipeID.base(second[0]) == "me.nixos.desk" && second[0] != desk)
        let lq = RecipeLibrary(paths: q)
        let available = lq.available(Settings.load(q)).map(\.id)
        #expect(available.filter { RecipeID.base($0) == "me.nixos.desk" }.count == 2)
        #expect(lq.distinctions(Settings.load(q)).isEmpty)   // one source: versions are told apart in their sub-rows
        // staged: every recipe in a tree of its own, with the one it requires next to it; recipes.json names them
        let staged = try lq.stage(Settings.load(q))
        let mapping = try JSONDecoder().decode([String: String].self, from: Data(contentsOf: staged.appendingPathComponent("recipes.json")))
        #expect(Set(mapping.keys) == Set(available))
        #expect(mapping[second[0]] == "\(second[0])/me/nixos/desk")
        #expect(FileManager.default.fileExists(atPath: staged.appendingPathComponent("\(second[0])/me/nixos/base/default.nix").path))
    }
}

@Suite struct OmaflakeIDTests {
    @Test func wordsSpellHashes() {
        #expect(OmaflakeID.list.count == 2048 && OmaflakeID.list.first == "abandon" && OmaflakeID.list.last == "zoo")
        let zero = [UInt8](repeating: 0, count: 17), ones = [UInt8](repeating: 0xff, count: 17)
        #expect(OmaflakeID.words(zero) == Array(repeating: "abandon", count: 12).joined(separator: "-"))
        #expect(OmaflakeID.words(ones).hasPrefix("zoo-zoo-zoo"))
        // 11 bits each: 00000000 001… → word 1 ("ability")
        #expect(OmaflakeID.words([0x00, 0x20] + zero).hasPrefix("ability-"))
        #expect(OmaflakeID.words(of: "x").split(separator: "-").count == 12)
        #expect(OmaflakeID.words(of: "x") == OmaflakeID.words(of: "x") && OmaflakeID.words(of: "x") != OmaflakeID.words(of: "y"))
        #expect(OmaflakeID.isWords(OmaflakeID.randomWords(), full: true))
        #expect(OmaflakeID.isWords("abandon-ability") && !OmaflakeID.isWords("abandon-ability", full: true))
        #expect(!OmaflakeID.isWords("abandon-notaword"))
    }

    /// As short as possible, the same length for every object of a kind (plan B111)
    @Test func shownAsShortAsNeeded() {
        let a = "able-acid-also-x", b = "able-acid-zoo-x", c = "zoo-acid-also-x"
        #expect(OmaflakeID.shortest([[a, c]]) == 1)
        #expect(OmaflakeID.shortest([[a, c], [b]]) == 1)
        #expect(OmaflakeID.shortest([[a, b]]) == 3)          // two of one identifier share two words
        #expect(OmaflakeID.shortest([[a], [b]]) == 1)        // different identifiers need no telling apart
        #expect(OmaflakeID.short(a, 2) == "able-acid")
        let l = OmaflakeID.Lengths(recipes: ["o.nixos.m@\(a)", "o.nixos.m@\(b)", "o.nixos.n@\(a)"], images: ["o.nixos.m@\(a)"], vms: [])
        #expect(l.recipe == 3 && l.image == 1 && l.vm == 1)
    }

    @Test func idsOfTheThreeKinds() {
        let r = "omaflake.nixos.minimal@" + OmaflakeID.words([UInt8](repeating: 0, count: 17))
        #expect(OmaflakeID.recipe(r, words: 1) == "recipe:omaflake/nixos/minimal:abandon")
        #expect(OmaflakeID.recipe(r).split(separator: ":")[2].split(separator: "-").count == 12)
        #expect(OmaflakeID.parse(OmaflakeID.recipe(r, words: 2))! == (.recipe, "omaflake/nixos/minimal", "abandon-abandon"))
        var c = config("research")
        c.identity = "quiet-otter-lamp"
        #expect(OmaflakeID.vm(c, words: 2) == "vm:research:quiet-otter")
        #expect(OmaflakeID.parse("vm:research:not-words") == nil)
        // a recipe by its omaflake ID (or the start of its words), by base
        let other = "omaflake.nixos.minimal@" + OmaflakeID.words([UInt8](repeating: 0xff, count: 17))
        #expect(RecipeID.candidates("recipe:omaflake/nixos/minimal:zoo", among: [r, other]) == [other])
        #expect(RecipeID.candidates("minimal", among: [r, other]).count == 2)
        #expect(RecipeID.candidates("omaflake.nixos.minimal@zoo", among: [r, other]) == [other])
    }

    /// The source next to a name only when another source has a recipe of the same base (plan B110, B111)
    @Test func distinctionOnlyWhenNeeded() {
        let a = "o.nixos.m@abandon", b = "o.nixos.m@zoo", c = "o.nixos.x@zoo"
        let name = { (s: String) in s.uppercased() }
        #expect(RecipeID.distinction(c, among: [a: "s1", b: "s2", c: "s1"], sourceName: name) == nil)
        #expect(RecipeID.distinction(a, among: [a: "s1", b: "s2"], sourceName: name) == "S1")
        #expect(RecipeID.distinction(a, among: [a: "s1", b: "s1"], sourceName: name) == nil)   // versions: words in sub-rows
    }
}

@Suite struct RecipeVersionTests {
    func recipe(_ root: URL, _ text: String) throws {
        let d = root.appendingPathComponent("me/nixos/desk")
        try FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        try Data("\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ndescription = \"x\"\n".utf8).write(to: d.appendingPathComponent("recipe.toml"))
        try Data(text.utf8).write(to: d.appendingPathComponent("default.nix"))
    }

    /// Imports make versions of one line; without keeping only the newest stays – and one an image is built from
    @Test func versionsAreKeptWhileUsedOrWhenAsked() throws {
        let p = try tempPaths()
        let lib = RecipeLibrary(paths: p)
        let src = p.root.appendingPathComponent("src")
        var s = Settings(); s.sources = []; try s.save(p)
        try recipe(src, "{ v = 1; }")
        let v1 = try lib.importRecipes(from: src)[0]
        // an image built from v1
        let img = p.imageDir("me.nixos.desk@" + OmaflakeID.words(of: "/nix/store/a"))
        try FileManager.default.createDirectory(at: img, withIntermediateDirectories: true)
        try ImageStore.writeInfo(.init(recipe: v1, recipeSource: "import", kind: .nixos, system: "aarch64-linux", source: "", revision: "r", storePath: "/nix/store/a"), to: img)
        try Data().write(to: img.appendingPathComponent("disk.qcow2"))
        try recipe(src, "{ v = 2; }")
        let v2 = try lib.importRecipes(from: src)[0]
        try recipe(src, "{ v = 3; }")
        let v3 = try lib.importRecipes(from: src)[0]
        // v2 went (not newest, no image), v1 stays (its image), v3 is the newest
        let line = RecipeID.lineKey(base: v1, source: "import")
        #expect(lib.versions(Settings.load(p))[line]?.map(\.id) == [v3, v1])
        #expect(lib.versions(Settings.load(p))[line]?.first?.isNewest == true)
        #expect(!FileManager.default.fileExists(atPath: p.recipes.appendingPathComponent(v2).path))
        #expect(throws: VMError.self) { try lib.remove(v1) }           // kept for its image
        // with keeping, an older version stays
        s.keepRecipeVersions = true; try s.save(p)
        try recipe(src, "{ v = 4; }")
        let v4 = try lib.importRecipes(from: src)[0]
        #expect(lib.versions(Settings.load(p))[line]?.map(\.id) == [v4, v3, v1])
        // all versions: what can go goes, the one with an image stays
        let (removed, kept) = lib.removeLine(line)
        #expect(Set(removed) == [v4, v3] && kept.count == 1)
        #expect(lib.versions(Settings.load(p))[line]?.map(\.id) == [v1])
    }

    /// A build keeps the version of a local folder's recipe it used; the folder changes, the version stays
    @Test func buildKeepsTheVersionOfALocalFolder() throws {
        let p = try tempPaths()
        let lib = RecipeLibrary(paths: p)
        let folder = p.root.appendingPathComponent("mine")
        try recipe(folder, "{ v = 1; }")
        var s = Settings(); s.sources = [RecipeSource(kind: .folder, location: folder.path)]; try s.save(p)
        let v1 = try #require(lib.available(s).first).id
        #expect(lib.versions(s).values.first?.first?.isLive == true)
        lib.keep(v1, settings: s)
        #expect(FileManager.default.fileExists(atPath: p.recipes.appendingPathComponent("\(v1)/default.nix").path))
        let img = p.imageDir("me.nixos.desk@" + OmaflakeID.words(of: "/nix/store/b"))
        try FileManager.default.createDirectory(at: img, withIntermediateDirectories: true)
        try ImageStore.writeInfo(.init(recipe: v1, recipeSource: s.sources![0].id, kind: .nixos, system: "aarch64-linux", source: "", revision: "r", storePath: "/nix/store/b"), to: img)
        try Data().write(to: img.appendingPathComponent("disk.qcow2"))
        try recipe(folder, "{ v = 2; }")
        let list = try #require(lib.versions(s).values.first)
        #expect(list.count == 2 && list[0].isLive && list[0].isNewest && list[1].id == v1 && list[1].images.count == 1)
        #expect(lib.pruneVersions().isEmpty)                            // used by its image
        try Disposal.delete([img])
        #expect(lib.pruneVersions() == [v1])                            // no image any more, not the newest
        #expect(throws: VMError.self) { try lib.remove(list[0].id) }    // the folder's own recipe is changed there
    }
}

/// Versions of omaflake's files (`$schema`, docs/schema/) and the architecture of recipes, images and VMs (plan B114)
@Suite struct FileFormatTests {
    /// docs/schema/ of this checkout
    static let schemaFolder = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().appendingPathComponent("docs/schema")

    static let formats = [ImageStore.Info.fileFormat, InstanceConfig.fileFormat, Settings.fileFormat, RecipeLibrary.fileFormat,
                          MacVMConfig.fileFormat, Package.Manifest.fileFormat]

    /// The published schema of a URL of omaflake's
    static func schema(_ url: String) throws -> [String: Any] {
        let (type, version) = try #require(FileFormat.parse(url))
        let file = schemaFolder.appendingPathComponent("\(type)/v\(version).json")
        return try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    }

    /// The keys of `data` are the schema's, and its required ones are there (a shallow check of what omaflake writes)
    static func matches(_ data: Data) throws {
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let schema = try schema(try #require(object["$schema"] as? String))
        let properties = Set((schema["properties"] as? [String: Any] ?? [:]).keys)
        let required = Set(schema["required"] as? [String] ?? [])
        #expect(Set(object.keys).subtracting(properties).isEmpty, "keys not in the schema: \(Set(object.keys).subtracting(properties))")
        #expect(required.subtracting(object.keys).isEmpty, "required keys missing: \(required.subtracting(object.keys))")
    }

    /// Every version up to the current one is in docs/schema/ – a published version is never removed (the catalog's
    /// schemas start at v2: v1 was `"schema": 1`, before B114)
    @Test func everyVersionHasItsSchema() throws {
        var urls = Self.formats.flatMap { f in (1...f.current).map { FileFormat.schema(type: f.type, version: $0) } }
        urls += (2...RecipeCatalog.version).map { FileFormat.schema(type: "catalog", version: $0) }
        urls += (1...RecipeLibrary.recipeVersion).map { FileFormat.schema(type: "recipe", version: $0) }
        for url in urls { #expect(try Self.schema(url)["$id"] as? String == url) }
    }

    @Test func whatOmaflakeWritesMatchesItsSchema() throws {
        let enc = JSONEncoder()
        enc.dateEncodingStrategy = .iso8601
        let info = ImageStore.Info(recipe: "omaflake.nixos.minimal@a", recipeSource: "s", kind: .nixos, system: "aarch64-linux",
                                   source: "/f", revision: "r", storePath: "/nix/store/x")
        try Self.matches(ImageStore.Info.fileFormat.encode(info, encoder: enc))
        var c = config()
        c.identity = "a-b-c"
        c.sharePath = "/s"
        try Self.matches(InstanceConfig.fileFormat.encode(c, encoder: enc))
        var s = Settings(flake: "/f")
        s.sources = [.official]
        s.guest = GuestOverrides(user: "u")
        try Self.matches(Settings.fileFormat.encode(s, encoder: enc))
        try Self.matches(MacVMConfig.fileFormat.encode(MacVMConfig(cpus: 4, memoryMB: 8192, diskGB: 80, macAddress: "02:00:00:00:00:01",
                                                                    osBuild: "26A434"), encoder: enc))
        try Self.matches(Package.Manifest.fileFormat.encode(Package.Manifest(kind: .vm, name: "a", guestKind: .nixos), encoder: enc))
        let installed = RecipeLibrary.InstalledFile(recipes: ["x": .init(source: "s", version: "v", requires: nil, installed: Date())])
        try Self.matches(RecipeLibrary.fileFormat.encode(installed, encoder: enc))
    }

    @Test func newerOrForeignSchemasAreRefused() throws {
        let newer = Data(#"{"$schema":"https://omaflake.dev/schema/instance/v99.json"}"#.utf8)
        #expect(throws: VMError.self) { try InstanceConfig.fileFormat.upgrade(newer) }
        let image = Data(#"{"$schema":"https://omaflake.dev/schema/image/v1.json"}"#.utf8)
        #expect(throws: VMError.self) { try InstanceConfig.fileFormat.upgrade(image) }      // another type
        let foreign = Data(#"{"$schema":"https://example.org/instance/v1.json"}"#.utf8)
        #expect(throws: VMError.self) { try InstanceConfig.fileFormat.upgrade(foreign) }
        #expect(FileFormat.parse("https://omaflake.dev/schema/macos-vm/v12.json")! == ("macos-vm", 12))
        #expect(FileFormat.parse("https://omaflake.dev/schema/image/1.json") == nil)
        let p = try tempPaths()
        try FileManager.default.createDirectory(at: p.root, withIntermediateDirectories: true)
        try Data(#"{"$schema":"https://omaflake.dev/schema/settings/v99.json"}"#.utf8).write(to: p.configFile)
        #expect(throws: VMError.self) { try Settings().save(p) }     // a newer omaflake's settings are not overwritten
    }

    /// instance.json before B114: no $schema, no system, maybe no kind and the old "flavor" key
    @Test func instanceVersionZeroIsMigrated() throws {
        let old = #"{"name":"a","flavor":"dev","image":"i","imagePath":"/i","cpus":2,"memoryGB":2,"diskGB":10,"network":"shared","share":false,"ephemeral":false,"sshPort":2222,"created":"2026-10-01T00:00:00Z"}"#
        let read = try InstanceConfig.fileFormat.decode(InstanceConfig.self, from: Data(old.utf8), decoder: InstanceStore.decoder)
        #expect(read.migrated)
        #expect(read.value.recipe == "dev" && read.value.kind == .nixos && read.value.system == "aarch64-linux")
        let current = try InstanceConfig.fileFormat.encode(read.value, encoder: InstanceStore.encoder)
        #expect(try !InstanceConfig.fileFormat.decode(InstanceConfig.self, from: current, decoder: InstanceStore.decoder).migrated)
    }

    @Test func imageVersionZeroIsMigratedAndWrittenBack() throws {
        let p = try tempPaths()
        let dir = p.imageDir("omaflake.macos.minimal@a")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let old = #"{"flavor":"omaflake.macos.minimal","author":"omaflake","kind":"macos","name":"minimal","source":"","revision":"r","created":"2026-10-01T00:00:00Z"}"#
        try Data(old.utf8).write(to: dir.appendingPathComponent("image.json"))
        let info = try #require(ImageStore.readInfo(dir))
        #expect(info.recipe == "omaflake.macos.minimal" && info.system == "aarch64-darwin")
        let written = try Data(contentsOf: dir.appendingPathComponent("image.json"))
        let text = String(decoding: written, as: UTF8.self)
        #expect(text.contains("https://omaflake.dev/schema/image/v1.json") && !text.contains("flavor"))
        try Self.matches(written)
    }

    @Test func installedIndexIsWrapped() throws {
        let p = try tempPaths()
        try FileManager.default.createDirectory(at: p.recipes, withIntermediateDirectories: true)
        let old = #"{"omaflake.nixos.minimal@abcd1234":{"source":"s","version":"v","installed":"2026-10-01T00:00:00Z"}}"#
        try Data(old.utf8).write(to: p.recipes.appendingPathComponent(".installed.json"))
        let lib = RecipeLibrary(paths: p)
        #expect(lib.installed()["omaflake.nixos.minimal@abcd1234"]?.version == "v")
        try Self.matches(Data(contentsOf: p.recipes.appendingPathComponent(".installed.json")))
        #expect(lib.installed().count == 1)
    }

    @Test func macVMVersionGivesWayToSchema() throws {
        let p = try tempPaths()
        let dir = p.root.appendingPathComponent("vm")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let old = #"{"version":1,"cpus":4,"memoryMB":8192,"diskGB":80,"display":{"width":1920,"height":1200,"ppi":80},"macAddress":"02:00:00:00:00:01"}"#
        try Data(old.utf8).write(to: dir.appendingPathComponent("config.json"))
        #expect(try MacVMBundle(dir).loadConfig().cpus == 4)
        let written = try Data(contentsOf: dir.appendingPathComponent("config.json"))
        #expect(!String(decoding: written, as: UTF8.self).contains("\"version\""))
        try Self.matches(written)
    }

    @Test func recipeVersionIsReadFromTheToml() throws {
        let p = try tempPaths()
        let folder = p.root.appendingPathComponent("omaflake/nixos/minimal")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let toml = folder.appendingPathComponent("recipe.toml")
        try Data("format = 1\ncpus = 1\n".utf8).write(to: toml)
        #expect(RecipeLibrary.version(of: folder) == 0)
        try Data("# \"$schema\" = \"https://omaflake.dev/schema/recipe/v7.json\"\n\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"   # the version\ncpus = 1\n".utf8).write(to: toml)
        #expect(RecipeLibrary.version(of: folder) == 1)
    }

    @Test func recipesNeedSystems() throws {
        let without = #"{"cpus":1,"memory_gb":1,"disk_gb":1,"network":"none","share":false,"ephemeral":false}"#
        #expect(throws: (any Error).self) { try JSONDecoder().decode(RecipeDefaults.self, from: Data(without.utf8)) }
        let x86 = #"{"systems":["x86_64-linux"],"cpus":1,"memory_gb":1,"disk_gb":1,"network":"none","share":false,"ephemeral":false}"#
        let d = try JSONDecoder().decode(RecipeDefaults.self, from: Data(x86.utf8))
        #expect(d.runsHere == (Platform.hostSystem(for: .nixos) == "x86_64-linux"))
        #expect(Platform.architecture("x86_64-linux") == "x86_64")
    }

    @Test func otherArchitecturesDoNotRunHere() throws {
        let foreign = Platform.hostSystem(for: .nixos) == "x86_64-linux" ? "aarch64-linux" : "x86_64-linux"
        #expect(!Platform.runsHere(foreign))
        var c = config()
        c.system = foreign
        #expect(throws: VMError.self) { try InstanceStore.checkRunsHere(c) }
        let info = ImageStore.Info(recipe: "omaflake.nixos.minimal", kind: .nixos, system: foreign, source: "", revision: "r", storePath: nil)
        #expect(throws: VMError.self) { try InstanceStore.checkRunsHere(ImageStore.Image(name: "x", info: info, path: URL(fileURLWithPath: "/x"))) }
    }
}
