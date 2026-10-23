// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// What the server tells without being asked (api.md §6), and what this client tells about the connection.
public enum ServerEvent: Sendable {
    public enum Level: String, OpenEnum { case info, warning, error, unknown }

    /// The server runs and this client is initialized and subscribed – at the start and after every restart. A UI
    /// loads its lists then (a restarted server has lost its jobs, api.md §5).
    case connected(InitializeResult)
    /// The server ended or could not be started; the client starts it again after a pause
    case disconnected(String)

    case vmsChanged(VmList)
    /// A running VM is off; `requested`: it was stopped through this server
    case vmEnded(name: String, requested: Bool)
    /// After a start through this server: the agent reported in ("agent") or SSH answers ("ssh")
    case vmReady(name: String, via: String)
    case imagesChanged(ImageList)
    case recipesChanged(RecipeList)
    case jobChanged(Job)
    case jobLog(id: String, lines: [String])
    case doctorChanged(DoctorReport)
    /// Something the user should read in the activity list
    case message(text: String, level: Level)
    /// An event this client does not know (api.md §2: ignore it)
    case unknown(method: String)
    /// A known event whose params did not decode
    case undecodable(method: String, error: String)

    /// The topics `events.subscribe` takes
    public static let topics = ["vms", "images", "recipes", "jobs", "doctor", "messages"]

    private struct Notification<P: Decodable>: Decodable { let params: P }
    private struct Ended: Decodable { let name: String; let requested: Bool? }
    private struct Ready: Decodable { let name: String; let via: String? }
    private struct JobParams: Decodable { let job: Job }
    private struct LogParams: Decodable { let id: String; let lines: [String] }
    private struct MessageParams: Decodable { let text: String; let level: Level? }

    /// The event of a notification line
    static func decode(method: String, line: Data) -> ServerEvent {
        let decoder = JSONDecoder.api
        func params<P: Decodable>(_ type: P.Type) throws -> P { try decoder.decode(Notification<P>.self, from: line).params }
        do {
            switch method {
            case "vms.changed": return .vmsChanged(try params(VmList.self))
            case "vm.ended":
                let p = try params(Ended.self)
                return .vmEnded(name: p.name, requested: p.requested ?? false)
            case "vm.ready":
                let p = try params(Ready.self)
                return .vmReady(name: p.name, via: p.via ?? "")
            case "images.changed": return .imagesChanged(try params(ImageList.self))
            case "recipes.changed": return .recipesChanged(try params(RecipeList.self))
            case "jobs.changed": return .jobChanged(try params(JobParams.self).job)
            case "job.log":
                let p = try params(LogParams.self)
                return .jobLog(id: p.id, lines: p.lines)
            case "doctor.changed": return .doctorChanged(try params(DoctorReport.self))
            case "message":
                let p = try params(MessageParams.self)
                return .message(text: p.text, level: p.level ?? .info)
            default: return .unknown(method: method)
            }
        } catch {
            return .undecodable(method: method, error: "\(error)")
        }
    }
}
