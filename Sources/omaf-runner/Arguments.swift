// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

// omaf-runner's own command line (plan B177: replaces swift-argument-parser – omaf-runner is a tool omaf starts, its
// command line is small and fixed). `--name value` and `--name=value`, options given more than once collect, flags
// take no value, everything else is a positional argument (also after `--`). Unknown options are refused.

/// A command line the runner refuses: printed with the command's usage, exit status 64 (EX_USAGE)
struct ValidationError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

/// Ends the runner with this status without a message (the command printed what it had to say)
struct ExitCode: Error {
    let code: Int32
    init(_ code: Int32) { self.code = code }
}

/// One option or flag of a command, for parsing and for `--help`
struct Option {
    enum Kind { case flag, value }
    let name: String
    let kind: Kind
    let help: String

    static func flag(_ name: String, _ help: String) -> Option { Option(name: name, kind: .flag, help: help) }
    static func value(_ name: String, _ help: String) -> Option { Option(name: name, kind: .value, help: help) }
}

struct Arguments {
    private(set) var positional: [String] = []
    private var values: [String: [String]] = [:]
    private var flags: Set<String> = []
    private var next = 0

    init(_ argv: [String], options: [Option]) throws {
        let kinds = Dictionary(options.map { ($0.name, $0.kind) }, uniquingKeysWith: { a, _ in a })
        var i = 0
        while i < argv.count {
            let a = argv[i]
            i += 1
            if a == "--" {
                positional += argv[i...]
                break
            }
            guard a.hasPrefix("--"), a.count > 2 else {
                positional.append(a)
                continue
            }
            var name = String(a.dropFirst(2))
            var inline: String?
            if let eq = name.firstIndex(of: "=") {
                inline = String(name[name.index(after: eq)...])
                name = String(name[..<eq])
            }
            switch kinds[name] {
            case .flag:
                if inline != nil { throw ValidationError("--\(name) takes no value") }
                flags.insert(name)
            case .value:
                let value: String
                if let inline {
                    value = inline
                } else {
                    guard i < argv.count else { throw ValidationError("--\(name) needs a value") }
                    value = argv[i]
                    i += 1
                }
                values[name, default: []].append(value)
            case nil:
                throw ValidationError("Unknown option --\(name)")
            }
        }
    }

    func flag(_ name: String) -> Bool { flags.contains(name) }

    /// The option's last value
    func string(_ name: String) -> String? { values[name]?.last }

    /// Every value of an option given more than once
    func strings(_ name: String) -> [String] { values[name] ?? [] }

    func required(_ name: String) throws -> String {
        guard let v = string(name) else { throw ValidationError("Missing --\(name)") }
        return v
    }

    func int(_ name: String, default value: Int) throws -> Int {
        guard let text = string(name) else { return value }
        guard let n = Int(text) else { throw ValidationError("--\(name) needs a whole number, not \"\(text)\"") }
        return n
    }

    /// The next positional argument
    mutating func argument(_ name: String) throws -> String {
        guard next < positional.count else { throw ValidationError("Missing <\(name)>") }
        defer { next += 1 }
        return positional[next]
    }

    /// Refuses positional arguments nobody took
    func done() throws {
        if next < positional.count { throw ValidationError("Unexpected argument \"\(positional[next])\"") }
    }
}

/// A subcommand of omaf-runner
protocol RunnerCommand {
    static var name: String { get }
    static var abstract: String { get }
    /// `<folder>` etc. after the command's name, for `--help`
    static var arguments: String { get }
    static var options: [Option] { get }
    init(_ a: inout Arguments) throws
    func run() throws
}

extension RunnerCommand {
    static var arguments: String { "" }
    static var options: [Option] { [] }

    static var usage: String {
        var text = "Usage: omaf-runner \(name)\(arguments.isEmpty ? "" : " " + arguments)\(options.isEmpty ? "" : " [options]")\n\(abstract)\n"
        for o in options {
            let left = o.kind == .flag ? "--\(o.name)" : "--\(o.name) <value>"
            text += "  \(left.padding(toLength: max(left.count, 26), withPad: " ", startingAt: 0))  \(o.help)\n"
        }
        return text
    }
}

enum CommandLineTool {
    /// Parses `argv` (without the program), runs the command and exits: 0, the command's `ExitCode`, 64 for a
    /// command line it refuses, 1 for any other error ("Error: …" on stderr)
    static func main(_ commands: [any RunnerCommand.Type], abstract: String, argv: [String]) -> Never {
        func overview() -> String {
            var text = "Usage: omaf-runner <command> [arguments]\n\(abstract)\n\nCommands:\n"
            for c in commands { text += "  \(c.name.padding(toLength: 16, withPad: " ", startingAt: 0))\(c.abstract)\n" }
            return text + "\nSee 'omaf-runner <command> --help'.\n"
        }
        guard let first = argv.first, first != "--help", first != "-h", first != "help" else {
            print(overview(), terminator: "")
            exit(argv.isEmpty ? 64 : 0)
        }
        guard let command = commands.first(where: { $0.name == first }) else {
            FileHandle.standardError.write(Data("Error: Unknown command \"\(first)\"\n\n\(overview())".utf8))
            exit(64)
        }
        let rest = Array(argv.dropFirst())
        if rest.contains("--help") || rest.contains("-h") {
            print(command.usage, terminator: "")
            exit(0)
        }
        do {
            var a = try Arguments(rest, options: command.options)
            let c = try command.init(&a)
            try a.done()
            try c.run()
            exit(0)
        } catch let e as ExitCode {
            exit(e.code)
        } catch let e as ValidationError {
            FileHandle.standardError.write(Data("Error: \(e.description)\n\(command.usage)".utf8))
            exit(64)
        } catch {
            FileHandle.standardError.write(Data("Error: \(describe(error))\n".utf8))
            exit(1)
        }
    }
}
