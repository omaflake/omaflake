// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// The app's client against the real `omaf serve` – skipped unless `OMAF_BIN` names a built omaf
/// (`cargo build && OMAF_BIN=$PWD/target/debug/omaf swift test`). The server gets a fresh data folder of its own
/// (`VM_HOME`, never ~/.omaflake) and a one-recipe image flake (`VM_FLAKE`, evaluated offline in a moment); every read
/// method is called and decoded with the app's DTOs, and the events it sends after subscribing must decode too.
/// Nothing is built, no VM is started.
@Suite(.enabled(if: ProcessInfo.processInfo.environment["OMAF_BIN"].map { !$0.isEmpty } ?? false,
                "set OMAF_BIN to a built omaf (target/debug/omaf)"),
       .timeLimit(.minutes(3)))
struct CoreEndToEndTests {
    /// A data folder and an image flake of its own, in the system's temporary folder (left there for the system)
    static func sandbox() throws -> (home: URL, flake: URL) {
        // Without symlinks (/var → /private/var): Nix refuses a flake path through one
        // (Foundation's resolvingSymlinksInPath keeps /var, realpath does not)
        let temp = try #require(realpath(FileManager.default.temporaryDirectory.path, nil))
        defer { free(temp) }
        let root = URL(fileURLWithPath: String(cString: temp)).appendingPathComponent("omaflake-e2e-\(UUID().uuidString)")
        let home = root.appendingPathComponent("data"), flake = root.appendingPathComponent("flake")
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: flake, withIntermediateDirectories: true)
        // No inputs: `nix eval` needs neither the network nor the recipes
        try """
        {
          outputs = { self }: {
            lib.recipeDefaults."test.nixos.tiny" = {
              kind = "nixos"; systems = [ "aarch64-linux" ]; cpus = 2; memory_gb = 2; disk_gb = 10;
              network = "shared"; share = false; ephemeral = true; description = "A tiny test recipe";
            };
          };
        }
        """.write(to: flake.appendingPathComponent("flake.nix"), atomically: true, encoding: .utf8)
        return (home, flake)
    }

    @Test func everyReadMethodAndTheEventsDecode() async throws {
        let environment = ProcessInfo.processInfo.environment
        let omaf = try #require(CoreClient.locateServer(environment: environment))
        let (home, flake) = try Self.sandbox()
        var env = environment
        env["VM_HOME"] = home.path
        env["VM_FLAKE"] = flake.path
        env.removeValue(forKey: "OMAFLAKE_UPDATE_FEED")
        let client = CoreClient(launcher: CoreClient.processLauncher(executable: omaf, environment: env),
                                initialize: InitializeParams(client: "omaflake-e2e-test", clientVersion: "0", language: "de"),
                                restartDelays: [.seconds(60)])
        let events = EventLog(client)
        client.start()
        defer { client.stop() }

        // The session: this data folder, nothing else
        try await events.wait("connected") { if case .connected = $0 { true } else { false } }
        let session = try #require(client.session)
        #expect(session.apiVersion == CoreClient.apiVersion && !session.omafVersion.isEmpty)
        #expect(URL(fileURLWithPath: session.dataFolder).standardizedFileURL.path == home.standardizedFileURL.path)
        #expect(session.configFile.hasPrefix(session.dataFolder) && session.host.system.hasSuffix("-darwin"))

        let settings: SettingsInfo = try await client.call("settings.get")
        #expect(settings.settings.dataFolder == session.dataFolder && !settings.settings.developerMode)
        #expect(settings.settings.flakeInUse == flake.path && !settings.isDefaultDataFolder)
        #expect(!settings.choices.keyboards.isEmpty && !settings.choices.timeZones.isEmpty && settings.guestDefaults.user != nil)

        let vms: VmList = try await client.call("vms.list")
        #expect(vms.vms.isEmpty && vms.unreadable.isEmpty)
        let images: ImageList = try await client.call("images.list")
        #expect(images.images.isEmpty && images.unreadable.isEmpty)

        let recipes: RecipeList = try await client.call("recipes.list")
        let tiny = try #require(recipes.recipes.first { $0.base == "test.nixos.tiny" })
        #expect(tiny.kind == .nixos && tiny.cpus == 2 && tiny.memoryGB == 2 && tiny.ephemeral && tiny.image == nil)
        #expect(tiny.description == "A tiny test recipe" && tiny.qualified == "test/nixos/tiny" && !tiny.lab)
        #expect(!recipes.labHidden || recipes.recipes.allSatisfy { !$0.lab })

        let sources: [Source] = try await client.call("sources.list")
        #expect(sources.contains { $0.official } && sources.allSatisfy { !$0.hasToken || $0.private })

        let doctor: DoctorReport = try await client.call("doctor.check")
        #expect(!doctor.prerequisites.isEmpty && doctor.checked != nil && doctor.prerequisites.allSatisfy { $0.item != .unknown })
        #expect(!doctor.hasMacBase)

        let jobs: [Job] = try await client.call("jobs.list")
        #expect(jobs.isEmpty)
        let limits: Limits = try await client.call("vms.limits")
        #expect(limits.macMax >= 1 && limits.macRunning.isEmpty)
        let update: UpdateStatus = try await client.call("updates.check")
        #expect(update.state == .noFeed && update.release == nil)

        // Errors arrive with their kind
        do {
            let _: Vm = try await client.call("vms.get", ["name": "nope"])
            Issue.record("vms.get of a missing VM answered")
        } catch let error as APIError {
            #expect(error.kind == .notFound && error.code == APIError.Code.failed)
        }

        // The subscription (CoreClient subscribes to every topic) sends the lists; the Doctor's comes after its checks
        try await events.wait("vms.changed") { if case .vmsChanged(let l) = $0 { l.vms.isEmpty } else { false } }
        try await events.wait("images.changed") { if case .imagesChanged = $0 { true } else { false } }
        try await events.wait("recipes.changed") {
            if case .recipesChanged(let l) = $0 { l.recipes.contains { $0.base == "test.nixos.tiny" } } else { false }
        }
        try await events.wait("doctor.changed", seconds: 120) { if case .doctorChanged = $0 { true } else { false } }

        // The presentation model (api.md §10): every page the app shows comes as `view.changed` and decodes
        struct Subscribe: Encodable, Sendable { let pages: [PageRef] }
        let pages = ["home", "activity", "vms", "images", "recipes", "doctor", "settings", "macBase"].map { PageRef($0) }
        let _: Ignored? = try await client.call("views.subscribe", Subscribe(pages: pages))
        for page in pages.map(\.page) {
            try await events.wait("view.changed \(page)") {
                if case .viewChanged(page, _, let model) = $0 {
                    if case .error = model { false } else if case .unknown = model { false } else { true }
                } else { false }
            }
        }
        let recipesPage: RecipesPage = try await client.call("views.get", PageRef("recipes"))
        #expect(recipesPage.rows.contains { $0.recipe.base == "test.nixos.tiny" })
        let homePage: HomePage = try await client.call("views.get", PageRef("home"))
        #expect(homePage.title.text == "omaflake" && homePage.actions.first { $0.id == "doctor" }?.shortLabel != nil)

        // Form values by `valueParam`: the page is asked again with them; the IPSW is in its line only, set by
        // `ui.field` actions on `values.ipsw` of macBase
        let macBase: MacBasePage = try await client.call("views.get", PageRef("macBase", .object(["values": .object([
            "credential": .string("c-none"), "ipsw": .string("/nowhere/test.ipsw"),
        ])])))
        // The guest user comes from a credential (plan B139); this data folder has none: nothing chosen
        #expect(macBase.fields.first { $0.valueParam == "credential" }?.value == .string(""))
        #expect(!macBase.fields.contains { $0.id == "ipsw" } && macBase.actions.primary?.id == "create")
        let useApple = macBase.ipsw.actions.first { $0.method == "ui.field" && $0.params["value"] == .null }
        let choose = macBase.ipsw.actions.first { $0.method == "ui.field" && $0.params["value"] == nil }
        for a in [useApple, choose].compactMap({ $0 }) {
            #expect(a.params["page"] == .string("macBase") && a.params["valueParam"] == .string("ipsw"))
        }
        #expect(choose?.ask.isEmpty == false || macBase.ipsw.download != nil)

        // `packages.inspect` refuses with a text
        let notAPackage = flake.appendingPathComponent("flake.nix")
        let inspection: PackageInspection = try await client.call("packages.inspect", [
            "path": notAPackage.path, "expect": "image",
        ])
        #expect(!inspection.valid && inspection.reason?.text.isEmpty == false)
        let undecodable = events.all.compactMap { if case .undecodable(let m, let e) = $0 { "\(m): \(e)" } else { nil } }
        #expect(undecodable.isEmpty, "events this app cannot read: \(undecodable)")
        #expect(!events.all.contains { if case .disconnected = $0 { true } else { false } })
    }

    /// Runs `omaf <arguments>` with `environment`; its output
    static func omaf(_ omaf: URL, _ arguments: [String], environment: [String: String]) throws -> Data {
        let process = Process()
        process.executableURL = omaf
        process.arguments = arguments
        process.environment = environment
        let output = Pipe()
        process.standardOutput = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return data
    }

    /// The core service (plan B120): two clients on its socket share one server – the same data folder, the same jobs,
    /// events for both – and a client that stops leaves the service running for the other
    @Test func twoClientsShareTheService() async throws {
        let environment = ProcessInfo.processInfo.environment
        let omaf = try #require(CoreClient.locateServer(environment: environment))
        let (home, flake) = try Self.sandbox()
        var env = environment
        env["VM_HOME"] = home.path
        env["VM_FLAKE"] = flake.path
        env.removeValue(forKey: "OMAFLAKE_UPDATE_FEED")
        func client(_ name: String) -> CoreClient {
            CoreClient(launcher: CoreClient.serviceLauncher(executable: omaf, environment: env),
                       initialize: InitializeParams(client: name, clientVersion: "0", language: "de"),
                       topics: ["vms", "jobs"], restartDelays: [.seconds(60)])
        }
        let a = client("e2e-a"), b = client("e2e-b")
        let eventsA = EventLog(a), eventsB = EventLog(b)
        a.start()
        try await eventsA.wait("connected a") { if case .connected = $0 { true } else { false } }
        b.start()
        try await eventsB.wait("connected b") { if case .connected = $0 { true } else { false } }
        defer {
            a.stop()
            b.stop()
            _ = try? Self.omaf(omaf, ["service", "stop"], environment: env)
        }
        let sa = try #require(a.session), sb = try #require(b.session)
        #expect(sa.dataFolder == sb.dataFolder)
        #expect(URL(fileURLWithPath: sa.dataFolder).standardizedFileURL.path == home.standardizedFileURL.path)

        // Both get the lists; both see the same (empty) jobs
        try await eventsA.wait("vms.changed a") { if case .vmsChanged = $0 { true } else { false } }
        try await eventsB.wait("vms.changed b") { if case .vmsChanged = $0 { true } else { false } }
        let jobsA: [Job] = try await a.call("jobs.list")
        let jobsB: [Job] = try await b.call("jobs.list")
        #expect(jobsA == jobsB && jobsA.isEmpty)

        // One client leaves: the service stays for the other, and keeps running without clients
        a.stop()
        try await Task.sleep(for: .milliseconds(300))
        let still: [Job] = try await b.call("jobs.list")
        #expect(still.isEmpty)
        #expect(!eventsB.all.contains { if case .disconnected = $0 { true } else { false } })
        b.stop()
        try await Task.sleep(for: .milliseconds(300))
        struct Status: Decodable { let running: Bool }
        let status = try JSONDecoder().decode(Status.self, from: Self.omaf(omaf, ["service", "status", "--json"], environment: env))
        #expect(status.running)
    }
}
