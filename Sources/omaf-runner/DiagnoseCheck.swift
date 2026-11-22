// SPDX-License-Identifier: AGPL-3.0-or-later
import ArgumentParser
import Darwin
import Foundation
import OmaflakeKit

/// `omaf-runner diagnose-check`: the diagnosis relay on this Mac, without a VM (plan B162) – the guest agent runs here
/// on a pseudo-terminal instead of the virtio port (as in `midi-check`), an AgentLink on the other end, and a control
/// socket in front of it as `run` and `bridge --control` have one. Each check goes through the socket
/// (`{"command": "diagnose", "check", "args"?}`) and prints the agent's answer.
struct DiagnoseCheck: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "diagnose-check",
        abstract: "Check the diagnosis relay on this Mac: guest agent on a pseudo-terminal, checks over a control socket (development).")
    @Option(help: "The omaflake-agent binary (nix/agent, cargo build)") var agent: String
    @Option(help: "The checks to ask, comma-separated") var checks = "info,network,nonexistent-check"
    @Option(help: "Args for every check, as a JSON object") var args: String?

    func run() throws {
        var argsObject: [String: Any]?
        if let args {
            guard let obj = (try? JSONSerialization.jsonObject(with: Data(args.utf8))) as? [String: Any] else {
                throw ValidationError("--args must be a JSON object")
            }
            argsObject = obj
        }
        // The port: a pseudo-terminal – the agent opens its slave as its port, the link reads and writes the master
        let master = posix_openpt(O_RDWR | O_NOCTTY)
        guard master >= 0, grantpt(master) == 0, unlockpt(master) == 0, let name = ptsname(master) else {
            throw VMError("no pseudo-terminal")
        }
        let slave = String(cString: name)
        let tmp = FileManager.default.temporaryDirectory
        let state = tmp.appendingPathComponent("omaflake-diagnose-check-\(getpid()).json")
        let socket = tmp.appendingPathComponent("omaflake-diagnose-\(getpid()).sock")
        let link = AgentLink(fd: master, stateFile: state, host: "omaflake check", allowed: ["status"],
                             log: { print("  runner: \($0)") })
        Thread.detachNewThread { _ = link.run() }
        // Handled on a global queue: this command's main thread waits below and serves no main queue
        let server = try ControlServer(path: socket, queue: .global()) { request, reply in
            switch request.object["command"] as? String {
            case "diagnose": ControlServer.diagnose(link, request, reply: reply)
            case let other: reply(ControlServer.unknown(other ?? "", available: ["diagnose"]))
            }
        }

        let agentProcess = Process()
        agentProcess.executableURL = URL(fileURLWithPath: url(agent).path)
        agentProcess.environment = ProcessInfo.processInfo.environment.merging(["OMAFLAKE_AGENT_PORT": slave]) { $1 }
        let log = tmp.appendingPathComponent("omaflake-diagnose-check-agent.log")
        FileManager.default.createFile(atPath: log.path, contents: nil)
        agentProcess.standardError = try FileHandle(forWritingTo: log)
        try agentProcess.run()
        defer {
            agentProcess.terminate()
            agentProcess.waitUntilExit()
            server.close()
            try? FileManager.default.removeItem(at: state)
        }
        print("agent \(url(agent).path) on \(slave) (log: \(log.path)), control socket \(socket.path)")

        // The welcome: the agent says hello once its port is there
        let deadline = Date().addingTimeInterval(10)
        while link.features == nil, Date() < deadline { usleep(100_000) }
        guard let features = link.features else {
            print("FAILED: no hello from the agent within 10 s – see its log")
            throw ExitCode(1)
        }
        print("welcome: \(features.sorted().joined(separator: ", "))")

        var ok = true
        for check in checks.split(separator: ",").map({ String($0).trimmingCharacters(in: .whitespaces) }) {
            var request: [String: Any] = ["command": "diagnose", "check": check]
            if let argsObject { request["args"] = argsObject }
            let start = Date()
            let answer = try RunnerClient.request(request, socket: socket, timeout: 20)
            let ms = Int(Date().timeIntervalSince(start) * 1000)
            let json = (try? JSONSerialization.data(withJSONObject: answer, options: [.sortedKeys, .withoutEscapingSlashes]))
                .map { String(decoding: $0, as: UTF8.self) } ?? "\(answer)"
            print("\(check) (\(ms) ms): \(json)")
            if answer["error"] != nil || answer["check"] as? String != check { ok = false }
        }
        print(ok ? "OK" : "FAILED")
        if !ok { throw ExitCode(1) }
    }
}
