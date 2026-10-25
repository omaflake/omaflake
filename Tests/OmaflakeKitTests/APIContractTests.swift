// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// The contract with the Rust core: every golden file the server's tests write (crates/omaflake-core/testdata/api/,
/// `api::contract`) decodes with this app's DTOs – strictly, required fields are never defaulted – and the values that
/// matter arrive. The params files go the other way: what this app encodes, the server's tests parse.
struct APIContractTests {
    static let folder = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .appendingPathComponent("../../crates/omaflake-core/testdata/api").standardizedFileURL

    static func golden(_ name: String) -> URL { folder.appendingPathComponent("\(name).json") }

    func load<T: Decodable>(_ name: String, as type: T.Type = T.self) throws -> T {
        try JSONDecoder.api.decode(T.self, from: Data(contentsOf: Self.golden(name)))
    }

    func object(_ name: String) throws -> [String: Any] {
        try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: Self.golden(name))) as? [String: Any])
    }

    /// Every golden file and the check that reads it – a new file without a check here fails `everyFileIsChecked`
    var checks: [String: () throws -> Void] {
        [
            "initialize.params": initializeParams,
            "initialize.result": {
                let r: InitializeResult = try load("initialize.result")
                #expect(r.apiVersion == CoreClient.apiVersion && r.host.system == "aarch64-darwin" && r.dataFolder == "/data")
                #expect(r.configFile == "/data/config.json" && r.omafVersion == "0.1.0")
            },
            "settings.get": {
                let s: SettingsInfo = try load("settings.get")
                #expect(s.settings.developerMode && !s.settings.notifications && s.settings.checkForUpdates)
                // user, fullName and email are a credential's since settings v3 (plan B137)
                #expect(!s.settings.keepRecipeVersions && s.settings.guest.user == nil && s.settings.guest.locale == "de_CH.UTF-8")
                #expect(s.settings.dataFolder == "/data" && s.settings.flakeInUse == "/src/omaflake/nix" && s.settings.qemu == nil)
                #expect(s.guestDefaults.timeZone == "Europe/Zurich" && s.guestDefaults.email == nil && !s.isDefaultDataFolder)
                #expect(s.choices.keyboards.first == .init(code: "ch", locale: "de_CH") && s.choices.timeZones.count == 2)
            },
            "settings.set": {
                let s: CoreSettings = try load("settings.set")
                #expect(s.developerMode && s.dataFolder == "/data" && s.guest.keyboard == "ch")
            },
            "updates.check": {
                let u: UpdateStatus = try load("updates.check")
                #expect(u.state == .available && u.release?.version == "0.2.0" && u.release?.page != nil)
            },
            "updates.check.upToDate": {
                let u: UpdateStatus = try load("updates.check.upToDate")
                #expect(u.state == .upToDate && u.release == nil)
            },
            "vms.list": { try vmList("vms.list") },
            "event.vms.changed": { try vmList("event.vms.changed") },
            "vms.get": { try fullVM(load("vms.get")) },
            "vms.stop": {
                let v = try load("vms.stop", as: StopAnswer.self)
                #expect(v.result == .powerdown)
            },
            "vms.set": {
                let a: VmSetAnswer = try load("vms.set")
                #expect(a.takesEffectAtNextStart && a.vm.name == "work")
            },
            "vms.ssh": {
                let a: SshAnswer = try load("vms.ssh")
                #expect(a.argv.first == "ssh" && a.shellLine == "ssh -p 2201 alex@127.0.0.1")
            },
            "vms.agent": {
                let a: AgentState = try JSONDecoder.agent.decode(AgentState.self, from: Data(contentsOf: Self.golden("vms.agent")))
                #expect(a.isReady && a.status?.memory?.available == 6_000_000_000)
            },
            "vms.log": {
                let v = try load("vms.log", as: LogLines.self)
                #expect(v.lines.last == "login:")
            },
            "vms.wait": {
                let v = try load("vms.wait", as: WaitAnswer.self)
                #expect(v.via == .agent)
            },
            "vms.share": {
                let v = try load("vms.share", as: PathAnswer.self)
                #expect(v.path == "/data/share/research")
            },
            "vms.nextName": {
                let v = try load("vms.nextName", as: NameAnswer.self)
                #expect(v.name == "dev-2")
            },
            "vms.limits": {
                let l: Limits = try load("vms.limits")
                #expect(l.macMax == 2 && l.macRunning == ["work"])
            },
            "images.list": { try imageList("images.list") },
            "event.images.changed": { try imageList("event.images.changed") },
            "images.get": {
                let i: VmImage = try load("images.get")
                #expect(i.sizeBytes == 12_345_678_901 && i.current && i.outdated == false)
            },
            "recipes.list": { try recipeList("recipes.list") },
            "event.recipes.changed": { try recipeList("event.recipes.changed") },
            "recipes.available": {
                let a: AvailableRecipes = try load("recipes.available")
                let first = try #require(a.offered.first)
                #expect(first.qualified == "omaflake/nixos/minimal" && first.license == "MIT" && first.requires.count == 1)
                #expect(first.files.count == 2 && first.files.reduce(0) { $0 + $1.size } == 2460 && first.runsHere)
                #expect(a.offered.last?.lab == true && a.offered.last?.license == nil)
                #expect(a.updates.first?.id == "omaflake.nixos.dev@velvet-tiger-orbit" && a.updates.first?.recipe.name == "dev")
                #expect(a.suggested.count == 1 && a.errors["url:https://example.org/catalog.json"] == "HTTP 404")
            },
            "recipes.removeLine": {
                let r: LineRemoval = try load("recipes.removeLine")
                #expect(r.removed.count == 1 && r.kept.isEmpty)
            },
            "recipes.pruneVersions": {
                let v = try load("recipes.pruneVersions", as: Pruned.self)
                #expect(v.removed.count == 1)
            },
            "sources.list": {
                let s: [Source] = try load("sources.list")
                #expect(s[0].kind == .github && s[0].official && s[0].branch == "main" && !s[0].hasToken && s[0].enabled)
                #expect(s[1].kind == .url && s[1].private && s[1].hasToken && !s[1].enabled && s[1].branch == nil)
            },
            "sources.add": {
                let a: SourceAdded = try load("sources.add")
                #expect(a.source.kind == .url && a.note == "acme/recipes: 3 recipes")
            },
            "sources.check": {
                let c: SourceChecks = try load("sources.check")
                #expect(c.results["github:omaflake/recipes@main"]?.ok == true && c.results.count == 2)
            },
            "packages.inspect": {
                let i: PackageInspection = try load("packages.inspect")
                #expect(i.kind == .vm && i.valid && i.conflict?.vmName == "research" && i.conflict?.suggestedName == "research-2")
                #expect(!i.networkVolume && i.reason == nil)
            },
            "packages.inspect.invalid": {
                let i: PackageInspection = try load("packages.inspect.invalid")
                #expect(i.kind == .image && !i.valid && i.conflict == nil && i.networkVolume)
                // A text (today the core's English message, no key)
                #expect(i.reason == ViewText(verbatim: "Not an image package (.omaflake-image)"))
            },
            "doctor.check": { try doctor("doctor.check") },
            "event.doctor.changed": { try doctor("event.doctor.changed") },
            "jobs.list": jobs,
            "jobs.log": {
                let l: JobLog = try load("jobs.log")
                #expect(l.lines.count == 1 && l.next == 1)
            },
            "event.vm.ended": { try event("vm.ended") { if case .vmEnded("research", false) = $0 { true } else { false } } },
            "event.vm.ready": { try event("vm.ready") { if case .vmReady("research", .ssh) = $0 { true } else { false } } },
            "event.jobs.changed": {
                try event("jobs.changed") { if case .jobChanged(let job) = $0 { job.id == "j1" && job.isRunning } else { false } }
            },
            "event.job.log": {
                try event("job.log") { if case .jobLog("j1", let lines) = $0 { lines.count == 2 } else { false } }
            },
            "event.message": {
                try event("message") { if case .message(let text, .info) = $0 { text.hasPrefix("Older recipe") } else { false } }
            },
            "error.vm-running": {
                struct Response: Decodable { let error: RPCError }
                let e = try load("error.vm-running", as: Response.self).error.apiError
                #expect(e.kind == .vmRunning && e.code == APIError.Code.failed)
                // A keyed message: the app renders data.text in its language
                let text = try #require(e.text)
                #expect(text.key == "%@ is running – shut it down first" && text.args == [.string("research")])
                let de = ViewTextRenderer(translate: { $0 == "%@ is running – shut it down first" ? "%@ läuft – fahre sie zuerst herunter" : $0 })
                #expect(de.render(text) == "research läuft – fahre sie zuerst herunter")
            },
            "error.network-volume": {
                struct Response: Decodable { let error: RPCError }
                let e = try load("error.network-volume", as: Response.self).error.apiError
                #expect(e.kind == nil && e.data?["networkVolume"]?.bool == true)
            },
            "vms.create.params": createParams,
        ]
    }

    @Test func everyFileIsChecked() throws {
        let files = try FileManager.default.contentsOfDirectory(atPath: Self.folder.path).filter { $0.hasSuffix(".json") }
        #expect(!files.isEmpty, "no golden files in \(Self.folder.path)")
        let names = Set(files.map { String($0.dropLast(5)) })
        #expect(names == Set(checks.keys), "golden files without a check: \(names.subtracting(checks.keys).sorted()); checks without a file: \(Set(checks.keys).subtracting(names).sorted())")
    }

    @Test func everyGoldenFileDecodes() {
        for (name, check) in checks.sorted(by: { $0.key < $1.key }) {
            do { try check() } catch { Issue.record("\(name).json: \(error)") }
        }
    }

    // MARK: Checks used more than once

    func fullVM(_ vm: Vm) {
        #expect(vm.name == "research" && vm.kind == .nixos && vm.status == .running && vm.system == "aarch64-linux")
        #expect(vm.memoryGB == 8 && vm.diskGB == 64 && vm.cpus == 4 && vm.sshPort == 2201 && vm.network == .shared)
        #expect(vm.display == .gpu && vm.upgradable && vm.audio && !vm.microphone && vm.clipboard && !vm.camera)
        #expect(vm.created == ISO8601DateFormatter().date(from: "2026-10-20T08:00:00Z"))
        #expect(vm.sharePath == "/data/share/research" && vm.user == "alex" && vm.identity == "amber" && !vm.busy)
        #expect(vm.omaflakeId.hasPrefix("vm:research:amber-lake") && vm.label == "research")
        #expect(vm.agent?.hello?.agent == "omaflake-agent 0.1.0" && vm.agent?.isReady == true)
        #expect(vm.location != nil && vm.latestImage == "omaflake.nixos.dev@mint-river-cloud" && vm.address == nil)
    }

    func vmList(_ name: String) throws {
        let list: VmList = try load(name)
        try fullVM(#require(list.vms.first))
        let mac = try #require(list.vms.last)
        #expect(mac.isMac && mac.busy && mac.sshPort == nil && mac.sharePath == nil && mac.identity == nil && mac.agent == nil)
        #expect(mac.display == .window && mac.network == .isolated && mac.ephemeral && !mac.upgradable)
        #expect(list.unreadable == [Unreadable(name: "old", reason: "written by a newer omaflake (version 9)")])
    }

    func imageList(_ name: String) throws {
        let list: ImageList = try load(name)
        let image = try #require(list.images.first)
        #expect(image.current && image.instances == ["research"] && image.sizeBytes == nil && image.outdated == false)
        #expect(image.label == "image:omaflake/nixos/dev:cedar" && image.omaflakeId.hasPrefix("image:omaflake/nixos/dev:cedar-"))
        #expect(image.source == "github:omaflake/recipes@main" && image.revision == "abc123-dirty" && image.storePath != nil)
        #expect(image.line == "omaflake.nixos.dev@github:omaflake/recipes@main" && image.kind == .nixos)
        let old = try #require(list.images.last)
        #expect(!old.current && old.outdated == nil && old.source == nil && old.storePath == nil && old.kind == .macos)
        #expect(list.unreadable.first?.name == "broken")
    }

    func recipeList(_ name: String) throws {
        let list: RecipeList = try load(name)
        let r = try #require(list.recipes.last)
        #expect(r.kind == .nixos && !r.lab && r.outdated && r.installed?.version == "3" && r.qualified == "omaflake/nixos/dev")
        #expect(r.displayName == "dev" && r.label == "recipe:omaflake/nixos/dev:velvet" && r.base == "omaflake.nixos.dev")
        #expect(r.description == "Entwicklungsumgebung" && r.display == .gpu && r.tags == ["dev", "gui"] && r.runsHere)
        #expect(r.image == "omaflake.nixos.dev@cedar-mango-lift" && r.distinction == "velvet" && r.folder != nil)
        #expect(r.settings.memoryGB == 8 && r.settings.audio && !r.settings.camera)
        let v = try #require(r.versions.first)
        #expect(v.isNewest && !v.isLive && v.images.count == 1 && v.added != nil && v.label == "recipe:omaflake/nixos/dev:velvet")
        #expect(r.versions.last?.isNewest == false && r.versions.last?.images.isEmpty == true)
        let lab = try #require(list.recipes.first)
        #expect(lab.lab && !lab.runsHere && lab.name == "base" && lab.author == "" && lab.user == nil && lab.versions.isEmpty)
        #expect(lab.audio == nil && lab.display == nil && lab.installed == nil && lab.network == NetworkMode.none)
        // A lab recipe's switches fall back to the defaults a new VM gets
        #expect(lab.settings.audio && lab.settings.clipboard && !lab.settings.microphone)
        #expect(list.lines["omaflake.nixos.dev@github:omaflake/recipes@main"]?.count == 2)
        #expect(list.idLengths.recipe == 1 && list.qemuGL && !list.labHidden && list.unreadable.first?.name == "acme.nixos.x")
    }

    func doctor(_ name: String) throws {
        let d: DoctorReport = try load(name)
        #expect(d.prerequisites.map(\.item) == [.nix, .qemu, .git, .cli] && d.prerequisites[0].install != nil)
        #expect(!d.prerequisites[3].required && d.prerequisites[3].install?.hasSuffix("doctor --install-cli") == true)
        #expect(d.prerequisites[1].title == "QEMU (NixOS guests)" && d.prerequisites[1].settablePath && d.prerequisites[1].required)
        #expect(!d.prerequisites[2].required && d.prerequisites[2].status == .missing)
        #expect(d.buildBlockers[.nixos].map(\.item) == [.builderRunning] && d.buildBlockers[.macos].first?.status == .limited)
        #expect(d.startBlockers[.nixos].map(\.item) == [.qemu] && d.startBlockers[.macos].isEmpty)
        let space = try #require(d.macosPreflight.first { $0.item == .macSpace })
        #expect(space.freeBytes == 20_000_000_000 && space.neededBytes == 40_000_000_000)
        #expect(d.macosPreflight.first { $0.item == .macBase }?.expectedUser == "sam")
        #expect(d.latestRelease?.build == "27A100" && d.latestRelease?.bytes == 18_000_000_000)
        #expect(d.issues == 3 && d.requirementsMissing && d.hasMacBase && d.macBaseUser == "alex" && !d.builderSystemIsOmaflakes)
        #expect(d.checked == ISO8601DateFormatter().date(from: "2026-10-23T09:41:00Z"))
    }

    func jobs() throws {
        let jobs: [Job] = try load("jobs.list")
        let byID = Dictionary(uniqueKeysWithValues: jobs.map { ($0.id, $0) })
        func job(_ id: String) -> Job { byID[id]! }
        #expect(jobs.count == 14)
        let build = job("j1")
        #expect(build.kind == .build && build.isRunning && build.guestKind == .nixos && build.cancellable)
        #expect(build.progress == Job.Progress(step: 3, total: 5, title: "Copying the image", done: 120, of: 400, fraction: 0.62))
        #expect(build.hiddenLines == 37 && build.logFile == "/data/logs/j1.log" && build.lastOutput != nil && build.finished == nil)
        #expect(job("j2").state == .queued && job("j2").started == nil)
        #expect(job("j3").result(as: BuildResult.self)?.removed == ["omaflake.nixos.dev@old-image"])
        #expect(job("j4").progress?.of == 18_000_000_000 && job("j4").kind == .ipswDownload)
        #expect(job("j5").result(as: IpswResult.self)?.version == "27.0.1")
        #expect(job("j6").result(as: BaseResult.self)?.ready == true)
        #expect(job("j7").result(as: ImportResult.self) == ImportResult(kind: .recipe, names: ["acme.nixos.x@amber"],
                                                                              notes: ["also installed acme.nixos.base"]))
        #expect(job("j8").result(as: ExportResult.self)?.path == "/Volumes/Disk/research.omaflake-vm")
        #expect(job("j9").result(as: DataMoveResult.self)?.dataFolder == "/Volumes/Disk/omaflake")
        #expect(job("j10").result(as: InstallResult.self)?.installed.count == 1 && job("j10").kind == .recipeInstall)
        #expect(job("j11").state == .failed && job("j11").error != nil && job("j11").kind.isBuilderTask)
        #expect(job("j12").state == .cancelled && job("j13").state == .checking && job("j13").kind == .builderApply)
        #expect(job("j14").result(as: MessageResult.self)?.message.hasPrefix("Linux builder") == true)
        #expect(Set(jobs.map(\.kind)).count == 11 && !jobs.contains { $0.kind == .unknown || $0.state == .unknown })
    }

    /// An event's params wrapped as the notification line the server writes
    func event(_ method: String, _ matches: (ServerEvent) -> Bool) throws {
        let params = try JSONSerialization.jsonObject(with: Data(contentsOf: Self.golden("event.\(method)")))
        let line = try JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "method": method, "params": params])
        let event = ServerEvent.decode(method: method, line: line)
        #expect(matches(event), "\(method): \(event)")
    }

    // MARK: Params: what this app sends

    func initializeParams() throws {
        let params = InitializeParams(client: "omaflake-macos", clientVersion: "0.1.0", language: "de",
                                      updateFeed: "https://example.org/feed.json")
        let sent = try #require(try JSONSerialization.jsonObject(with: JSONEncoder.api.encode(params)) as? NSDictionary)
        #expect(sent == (try object("initialize.params")) as NSDictionary)
    }

    func createParams() throws {
        let settings = RecipeSettings(kind: .nixos, cpus: 4, memoryGB: 8, diskGB: 64, network: .shared, share: true,
                                      ephemeral: false, user: "alex", audio: true, microphone: false, clipboard: true, camera: false)
        let sent = try #require(try JSONSerialization.jsonObject(with: JSONEncoder.api.encode(settings)) as? NSDictionary)
        let golden = try object("vms.create.params")
        #expect(sent == (golden["settings"] as? NSDictionary))
        #expect(golden["sharePath"] as? String != nil && golden["start"] as? Bool == true)
    }
}
