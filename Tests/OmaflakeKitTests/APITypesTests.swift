// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// How the DTOs decode beyond the shapes the golden files pin (APIContractTests): required fields are required, enums
/// stay open (api.md §2), errors and events.
struct APITypesTests {
    func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder.api.decode(T.self, from: Data(json.utf8))
    }

    /// The golden VM with some of its values replaced
    func vm(_ changes: [String: Any] = [:], removing: [String] = []) throws -> Data {
        let data = try Data(contentsOf: APIContractTests.golden("vms.get"))
        var object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        for (k, v) in changes { object[k] = v }
        for k in removing { object.removeValue(forKey: k) }
        return try JSONSerialization.data(withJSONObject: object)
    }

    @Test func unknownEnumValuesStayReadable() throws {
        let vm = try JSONDecoder.api.decode(Vm.self, from: vm(["kind": "freebsd", "status": "paused", "network": "bridged",
                                                                "display": "hologram"]))
        #expect(vm.kind == .unknown && vm.status == .unknown && vm.network == .unknown && vm.display == .unknown)
        let job = try decode(Job.self, """
        {"id": "j8", "kind": "teleport", "title": "x", "state": "later", "logLines": 0, "hiddenLines": 0, "cancellable": false}
        """)
        #expect(job.kind == .unknown && job.state == .unknown && !job.isActive)
    }

    @Test func aMissingRequiredFieldIsAnError() throws {
        for key in ["recipe", "status", "busy", "memoryGB", "created", "omaflakeId", "upgradable"] {
            #expect(throws: DecodingError.self, "\(key)") { try JSONDecoder.api.decode(Vm.self, from: vm(removing: [key])) }
        }
        // Optional ones may be left out
        let vm = try JSONDecoder.api.decode(Vm.self, from: vm(removing: ["sshPort", "agent", "location", "latestImage"]))
        #expect(vm.sshPort == nil && vm.agent == nil)
        #expect(throws: DecodingError.self) { try decode(Unreadable.self, #"{"reason": "too new"}"#) }
    }

    @Test func datesWholeOrWithAFraction() throws {
        let vm = try JSONDecoder.api.decode(Vm.self, from: vm(["created": "2026-10-23T09:41:00.250Z"]))
        #expect(vm.created.timeIntervalSince1970.truncatingRemainder(dividingBy: 1) > 0.2)
        #expect(throws: DecodingError.self) { try JSONDecoder.api.decode(Vm.self, from: self.vm(["created": "soon"])) }
    }

    @Test func anAgentReportThisAppCannotReadDoesNotLoseTheVM() throws {
        let vm = try JSONDecoder.api.decode(Vm.self, from: vm(["agent": ["helloAt": 42]]))
        #expect(vm.agent == nil && vm.name == "research")
    }

    @Test func recipeSettingsEncodeAsTheAPINamesThem() throws {
        let data = try JSONEncoder.api.encode(RecipeSettings(kind: .macos, memoryGB: 8, network: .none))
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(object["kind"] as? String == "macos" && object["memoryGB"] as? Int == 8 && object["network"] as? String == "none")
        #expect(object["display"] == nil)
    }

    @Test func jobResultsByKind() throws {
        let job = try decode(Job.self, """
        {"id": "j3", "kind": "build", "title": "Build dev", "state": "finished", "logLines": 1, "hiddenLines": 0,
         "cancellable": false, "result": {"image": "x", "removed": ["y"]}}
        """)
        #expect(job.result(as: BuildResult.self) == BuildResult(image: "x", removed: ["y"]))
        #expect(job.result(as: ImportResult.self) == nil)
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
        guard case .vmReady("research", .agent) = event(#"{"jsonrpc":"2.0","method":"vm.ready","params":{"name":"research","via":"agent"}}"#)
        else { Issue.record("vm.ready"); return }
        guard case .jobLog("j7", let lines) = event(#"{"jsonrpc":"2.0","method":"job.log","params":{"id":"j7","lines":["a","b"]}}"#),
              lines == ["a", "b"] else { Issue.record("job.log"); return }
        guard case .message("hi", .warning) = event(#"{"jsonrpc":"2.0","method":"message","params":{"text":"hi","level":"warning"}}"#)
        else { Issue.record("message"); return }
        guard case .unknown("vm.teleported") = event(#"{"jsonrpc":"2.0","method":"vm.teleported","params":{}}"#)
        else { Issue.record("unknown"); return }
        guard case .undecodable("vms.changed", _) = event(#"{"jsonrpc":"2.0","method":"vms.changed","params":{"vms":[{"kind":"nixos"}]}}"#)
        else { Issue.record("undecodable"); return }
        guard case .undecodable("vm.ended", _) = event(#"{"jsonrpc":"2.0","method":"vm.ended","params":{"name":"research"}}"#)
        else { Issue.record("vm.ended without requested"); return }
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
