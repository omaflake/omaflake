// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// An error from the core (api.md §3): the JSON-RPC code, an English message for people and, for what the core refuses
/// or fails at (`-32000`), a stable `kind` a UI can react to or translate.
public struct APIError: LocalizedError, CustomStringConvertible, Sendable, Equatable {
    /// `data.kind` of api.md §3; a client treats an unknown one as a plain error
    public enum Kind: String, OpenEnum {
        case notFound = "not-found", exists, vmRunning = "vm-running", vmStopped = "vm-stopped", inUse = "in-use"
        case macLimit = "mac-limit", unsupportedSystem = "unsupported-system", unreadableFile = "unreadable-file"
        case newerFile = "newer-file", cancelled, prerequisite, network, auth, `internal`
        case unknown
    }

    /// JSON-RPC codes of api.md §3
    public enum Code {
        public static let parse = -32700, invalidRequest = -32600, unknownMethod = -32601, invalidParams = -32602
        public static let notInitialized = -32002, failed = -32000
        /// Not from the server: it ended (or could not be started) while the request was open – this client's own
        public static let serverGone = -1
    }

    public let code: Int
    public let message: String
    /// nil: no `data.kind` (protocol errors, or a kind this client does not know: `.unknown`)
    public let kind: Kind?
    /// The whole `data` object (e.g. `{networkVolume: true}` of `vms.useExternal`)
    public let data: JSONValue?

    public init(code: Int = Code.failed, message: String, kind: Kind? = nil, data: JSONValue? = nil) {
        self.code = code
        self.message = message
        self.kind = kind
        self.data = data
    }

    public var description: String { message }
    public var errorDescription: String? { message }

    /// The server ended or could not be started
    public static func serverGone(_ message: String) -> APIError { APIError(code: Code.serverGone, message: message) }
}

/// The `error` member of a response
struct RPCError: Decodable {
    let code: Int
    let message: String
    let data: JSONValue?

    var apiError: APIError {
        APIError(code: code, message: message, kind: data?["kind"]?.string.map { APIError.Kind(rawValue: $0) ?? .unknown }, data: data)
    }
}
