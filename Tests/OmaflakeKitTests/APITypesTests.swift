// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// The DTOs decode what docs/api.md shows (§2–§7) and stay lenient where the API may grow.
struct APITypesTests {
    func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder.api.decode(T.self, from: Data(json.utf8))
    }

    @Test func vmFromTheExample() throws {
        // api.md §4.1 without its comments
        let vm = try decode(Vm.self, """
        {"name": "research", "recipe": "omaflake.nixos.dev@velvet-tiger-orbit",
         "image": "omaflake.nixos.dev@velvet-tiger-orbit-x", "kind": "nixos", "system": "aarch64-linux",
         "status": "running", "busy": false, "cpus": 4, "memoryGB": 8, "diskGB": 64, "network": "shared",
         "share": true, "sharePath": "/tmp/share/research", "ephemeral": false, "sshPort": 2201, "user": "alex",
         "audio": true, "microphone": false, "clipboard": true, "camera": false, "display": "gpu",
         "created": "2026-10-20T08:00:00Z", "identity": "amber-lake-quiet", "omaflakeId": "vm:research:amber-lake-quiet",
         "label": "research", "address": "192.168.64.5",
         "agent": {"hello": {"agent": "omaflake-agent 0.1.0", "protocol": {"min": 1, "max": 1}, "osName": "NixOS"},
                   "helloAt": "2026-10-20T08:00:18Z", "protocolVersion": 1,
                   "status": {"load": [0.5, 0.4, 0.3], "memory": {"total": 8000000000, "available": 6000000000}}},
         "location": "/Volumes/Disk/research.omaflake-vm", "latestImage": "omaflake.nixos.dev@velvet-tiger-orbit-y",
         "upgradable": true}
        """)
        #expect(vm.name == "research" && vm.kind == .nixos && vm.status == .running && vm.isRunning)
        #expect(vm.memoryGB == 8 && vm.diskGB == 64 && vm.sshPort == 2201 && vm.network == .shared)
        #expect(vm.display == .gpu && vm.upgradable && vm.audio && !vm.microphone)
        #expect(vm.created == ISO8601DateFormatter().date(from: "2026-10-20T08:00:00Z"))
        #expect(vm.agent?.hello?.agent == "omaflake-agent 0.1.0" && vm.agent?.isReady == true)
        #expect(vm.agent?.status?.memory?.available == 6_000_000_000)
    }

    @Test func missingFieldsGetTheirDefaultsAndUnknownValuesStayReadable() throws {
        let list = try decode(VmList.self, """
        {"vms": [{"name": "m", "kind": "freebsd", "status": "paused", "network": "bridged", "created": "soon"}]}
        """)
        let vm = try #require(list.vms.first)
        #expect(vm.kind == .unknown && vm.status == .unknown && vm.network == .unknown)
        #expect(vm.created == nil && !vm.upgradable && vm.cpus == 0 && list.unreadable.isEmpty)
    }

    @Test func datesWithAFraction() throws {
        let job = try decode(Job.self, #"{"id": "j1", "kind": "build", "state": "running", "started": "2026-10-23T09:41:00.250Z"}"#)
        #expect(job.started != nil)
    }

    @Test func imageAndUnreadable() throws {
        let list = try decode(ImageList.self, """
        {"images": [{"name": "omaflake.nixos.dev@a-b", "recipe": "omaflake.nixos.dev@a", "kind": "nixos",
                     "system": "aarch64-linux", "revision": "abc123-dirty", "storePath": "/nix/store/x-disk",
                     "created": "2026-10-21T10:00:00Z", "line": "omaflake.nixos.dev@src", "path": "/data/images/x",
                     "sizeBytes": 123456789, "label": "dev:cedar", "instances": ["research"], "current": true}],
         "unreadable": [{"path": "/data/instances/old/config.json", "type": "instance", "version": 9,
                         "reason": "too new"}]}
        """)
        let image = try #require(list.images.first)
        #expect(image.current && image.instances == ["research"] && image.sizeBytes == 123_456_789)
        #expect(image.label == "dev:cedar" && image.outdated == nil)
        #expect(list.unreadable.first?.displayName == "old" && list.unreadable.first?.version == 9)
    }

    @Test func recipeList() throws {
        let list = try decode(RecipeList.self, """
        {"recipes": [{"kind": "macos", "systems": ["aarch64-darwin"], "cpus": 4, "memoryGB": 8, "diskGB": 80,
                      "network": "shared", "share": false, "ephemeral": false, "user": "alex", "name": "dev",
                      "author": "omaflake", "description": "Entwicklung", "lab": true, "tags": ["dev"],
                      "id": "omaflake.macos.dev@a-b-c", "base": "omaflake.macos.dev", "qualified": "omaflake/macos/dev",
                      "displayName": "dev", "label": "dev", "runsHere": true, "source": "github:omaflake/recipes@main",
                      "line": "omaflake.macos.dev@1234abcd", "distinction": "words", "folder": "/data/recipes/x",
                      "installed": {"source": "github:omaflake/recipes@main", "version": "3",
                                    "installed": "2026-10-20T08:00:00.123Z"},
                      "image": "omaflake.macos.dev@a-b-x", "outdated": true,
                      "versions": [{"id": "omaflake.macos.dev@a-b-c", "isNewest": true, "isLive": false,
                                    "images": ["omaflake.macos.dev@a-b-x"], "added": "2026-10-20T08:00:00Z"}]}],
         "lines": {}, "idLengths": {"recipe": 2, "image": 1, "vm": 1}, "qemuGL": true, "unreadable": []}
        """)
        let r = try #require(list.recipes.first)
        #expect(r.kind == .macos && r.lab && r.outdated && r.installed?.version == "3" && r.installed?.installed != nil)
        #expect(r.versions.first?.isNewest == true && r.versions.first?.images.count == 1)
        #expect(list.idLengths.recipe == 2 && list.qemuGL)
        // What a new VM gets: the switches' defaults where the recipe says nothing
        #expect(r.settings.audio && r.settings.clipboard && !r.settings.microphone && r.settings.memoryGB == 8)
    }

    @Test func recipeSettingsEncodeAsTheAPINamesThem() throws {
        let data = try JSONEncoder.api.encode(RecipeSettings(kind: .macos, memoryGB: 8, network: .none))
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(object["kind"] as? String == "macos" && object["memoryGB"] as? Int == 8 && object["network"] as? String == "none")
        #expect(object["display"] == nil)
    }

    @Test func sourcesAndCatalog() throws {
        let sources = try decode([Source].self, """
        [{"id": "github:omaflake/recipes@main", "kind": "github", "location": "omaflake/recipes", "displayName": "omaflake/recipes",
          "enabled": true, "private": false, "official": true},
         {"id": "url:https://x/catalog.json", "kind": "url", "location": "https://x/catalog.json"}]
        """)
        #expect(sources[0].official && sources[0].kind == .github)
        #expect(sources[1].kind == .https && sources[1].enabled && sources[1].displayName == "https://x/catalog.json")
        let available = try decode(AvailableRecipes.self, """
        {"offered": [{"id": "omaflake.nixos.dev", "author": "omaflake", "kind": "nixos", "name": "dev", "path": "recipes/x",
                      "description": "Dev", "descriptions": {"de": "Entwicklung"}, "systems": ["aarch64-linux"],
                      "version": "2", "files": [{"path": "recipe.toml", "size": 120, "sha256": "ab"}],
                      "source": "github:omaflake/recipes@main", "runsHere": true}],
         "updates": [], "suggested": [], "errors": {"url:https://x/catalog.json": "404"}}
        """)
        #expect(available.offered.first?.description(in: "de") == "Entwicklung")
        #expect(available.offered.first?.qualified == "omaflake/nixos/dev" && available.errors.count == 1)
    }

    @Test func doctorBlockersAsObjectsOrNames() throws {
        let report = try decode(DoctorReport.self, """
        {"prerequisites": [{"item": "qemu", "status": "missing", "required": true, "detail": "", "hint": "x",
                            "settablePath": true}],
         "nixosPreflight": [{"item": "builderRunning", "status": "missing", "detail": "", "hint": ""}],
         "macosPreflight": [{"item": "macBase", "status": "limited", "detail": "", "hint": "", "baseUser": "alex",
                             "freeBytes": 5000000000}],
         "latestRelease": {"version": "27.0.1", "build": "27A100", "url": "https://x/y.ipsw", "bytes": 18000000000},
         "buildBlockers": {"nixos": ["builderRunning"], "macos": [{"item": "macBase", "status": "limited",
                                                                   "detail": "", "hint": ""}]},
         "startBlockers": {"nixos": ["qemu"], "macos": []}, "issues": 3}
        """)
        #expect(report.buildBlockers[.nixos].map(\.item) == [.builderRunning])
        #expect(report.buildBlockers[.macos].first?.status == .limited)
        #expect(report.startBlockers[.nixos].first?.settablePath == true && report.issues == 3)
        #expect(report.macosPreflight.first?.baseUser == "alex" && report.latestRelease?.build == "27A100")
        #expect(report.nixosPreflight.first?.required == true && report.nixosPreflight.first?.settablePath == false)
    }

    @Test func jobFromTheExample() throws {
        let job = try decode(Job.self, """
        {"id": "j7", "kind": "build", "title": "Build dev", "recipe": "omaflake.nixos.dev@a", "guestKind": "nixos",
         "state": "running", "progress": {"step": 3, "total": 5, "title": "Copying the image", "done": 0, "of": 0,
         "fraction": 0.62}, "started": "2026-10-23T09:41:00Z", "logLines": 412, "hiddenLines": 37,
         "logFile": "/data/logs/image-build-x.log", "result": {"image": "x", "removed": ["y"]}}
        """)
        #expect(job.kind == .build && job.state == .running && job.isActive && job.isRunning && !job.isDone)
        #expect(job.progress?.fraction == 0.62 && job.progress?.step == 3 && job.logLines == 412)
        #expect(job.result?["image"]?.string == "x" && job.result?["removed"]?.strings == ["y"])
        let other = try decode(Job.self, #"{"id": "j8", "kind": "teleport", "state": "later"}"#)
        #expect(other.kind == .unknown && other.state == .unknown && !other.isActive)
    }

    @Test func sessionAndSettings() throws {
        let result = try decode(InitializeResult.self, """
        {"apiVersion": 1, "omafVersion": "0.3.0", "host": {"system": "aarch64-darwin", "os": "macOS 27.0.1"},
         "dataFolder": "/data", "configFile": "/data/config.json"}
        """)
        #expect(result.apiVersion == 1 && result.host?.system == "aarch64-darwin")
        let settings = try decode(SettingsInfo.self, """
        {"settings": {"developerMode": true, "guest": {"user": "alex"}, "sources": [], "home": "/Volumes/x"},
         "guestDefaults": {"user": "mac", "fullName": "Mac User", "timeZone": "Europe/Berlin", "locale": "de_DE.UTF-8",
                           "keyboard": "de", "authorizedKeys": []},
         "choices": {"timeZones": ["Europe/Berlin"], "locales": ["de_DE.UTF-8"],
                     "keyboards": [{"code": "de", "locale": "de_DE"}, "us", ["ch", "de_CH"]]},
         "isDefaultDataFolder": false}
        """)
        #expect(settings.settings.developerMode == true && settings.settings.guest?.user == "alex")
        #expect(settings.guestDefaults.keyboard == "de" && !settings.isDefaultDataFolder)
        #expect(settings.choices.keyboards.map(\.code) == ["de", "us", "ch"])
        #expect(settings.choices.keyboards.map(\.locale) == ["de_DE", nil, "de_CH"])
    }

    @Test func errorsWithTheirKind() throws {
        let error = try decode(RPCError.self, #"{"code":-32000,"message":"running – stop it first","data":{"kind":"vm-running"}}"#).apiError
        #expect(error.kind == .vmRunning && error.code == APIError.Code.failed && error.message.hasPrefix("running"))
        let unknown = try decode(RPCError.self, #"{"code":-32000,"message":"x","data":{"kind":"quantum"}}"#).apiError
        #expect(unknown.kind == .unknown)
        let plain = try decode(RPCError.self, #"{"code":-32601,"message":"unknown method"}"#).apiError
        #expect(plain.kind == nil && plain.code == APIError.Code.unknownMethod)
    }

    @Test func events() throws {
        func event(_ json: String) -> ServerEvent {
            let line = Data(json.utf8)
            let method = (try? JSONSerialization.jsonObject(with: line) as? [String: Any])?["method"] as? String ?? ""
            return ServerEvent.decode(method: method, line: line)
        }
        guard case .vmEnded("research", true) = event(#"{"jsonrpc":"2.0","method":"vm.ended","params":{"name":"research","requested":true}}"#)
        else { Issue.record("vm.ended"); return }
        guard case .vmReady("research", "agent") = event(#"{"jsonrpc":"2.0","method":"vm.ready","params":{"name":"research","via":"agent"}}"#)
        else { Issue.record("vm.ready"); return }
        guard case .jobLog("j7", let lines) = event(#"{"jsonrpc":"2.0","method":"job.log","params":{"id":"j7","lines":["a","b"]}}"#),
              lines == ["a", "b"] else { Issue.record("job.log"); return }
        guard case .message("hi", .warning) = event(#"{"jsonrpc":"2.0","method":"message","params":{"text":"hi","level":"warning"}}"#)
        else { Issue.record("message"); return }
        guard case .jobChanged(let job) = event(#"{"jsonrpc":"2.0","method":"jobs.changed","params":{"job":{"id":"j1","kind":"export","state":"finished","result":{"path":"/x"}}}}"#),
              job.result?["path"]?.string == "/x" else { Issue.record("jobs.changed"); return }
        guard case .unknown("vm.teleported") = event(#"{"jsonrpc":"2.0","method":"vm.teleported","params":{}}"#)
        else { Issue.record("unknown"); return }
        guard case .undecodable("vms.changed", _) = event(#"{"jsonrpc":"2.0","method":"vms.changed","params":{"vms":[{"kind":"nixos"}]}}"#)
        else { Issue.record("undecodable"); return }
    }

    @Test func jsonValueRoundTrip() throws {
        let value = try decode(JSONValue.self, #"{"a": [1, 2.5, "x", true, null], "b": {"c": "d"}}"#)
        #expect(value["a"]?.array?.count == 5 && value["b"]?["c"]?.string == "d")
        let again = try JSONDecoder.api.decode(JSONValue.self, from: JSONEncoder.api.encode(value))
        #expect(again == value)
        let encoded = String(decoding: try JSONEncoder.api.encode(JSONValue.object(["guest": .null, "n": .number(3)])), as: UTF8.self)
        #expect(encoded == #"{"guest":null,"n":3}"#)
    }
}
