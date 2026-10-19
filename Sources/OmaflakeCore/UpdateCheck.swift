// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Is there a newer omaflake? Reads a small JSON feed (spec §11.6, plan B68):
///   {"version": "0.2.0", "url": "https://…/omaflake-0.2.0.zip", "sha256": "…", "notes": "…", "page": "https://…"}
/// The feed's address comes from the app's Info.plist (`OMUpdateFeedURL`, set with the first release) or
/// `OMAFLAKE_UPDATE_FEED`. This is the only network access the app makes on its own, and it can be switched off.
/// Installing the update itself comes with signed releases (M12).
public struct UpdateFeed: Codable, Sendable, Equatable {
    public let version: String
    public let url: String
    public let sha256: String?
    public let notes: String?
    public let page: String?
}

public enum UpdateCheck {
    public static func feedURL(bundle: Bundle = .main) -> URL? {
        let raw = ProcessInfo.processInfo.environment["OMAFLAKE_UPDATE_FEED"]
            ?? bundle.object(forInfoDictionaryKey: "OMUpdateFeedURL") as? String
        return raw.flatMap { $0.isEmpty ? nil : URL(string: $0) }
    }

    /// The feed's release when it is newer than `current`, nil when up to date.
    public static func newer(than current: String, feed: URL) async throws -> UpdateFeed? {
        var request = URLRequest(url: feed, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
        request.setValue("omaflake/\(current)", forHTTPHeaderField: "User-Agent")
        let (data, response) = try await URLSession.shared.data(for: request)
        if let http = response as? HTTPURLResponse, http.statusCode != 200 {
            throw VMError("Update feed answered \(http.statusCode)")
        }
        let release = try JSONDecoder().decode(UpdateFeed.self, from: data)
        return isNewer(release.version, than: current) ? release : nil
    }

    /// "0.10.0" > "0.9.1"; a missing part counts as 0; anything after "-" (pre-release) is ignored
    public static func isNewer(_ a: String, than b: String) -> Bool {
        func parts(_ v: String) -> [Int] {
            v.split(separator: "-").first.map { $0.split(separator: ".").map { Int($0) ?? 0 } } ?? []
        }
        let x = parts(a), y = parts(b)
        for i in 0..<max(x.count, y.count) {
            let l = i < x.count ? x[i] : 0, r = i < y.count ? y[i] : 0
            if l != r { return l > r }
        }
        return false
    }
}
