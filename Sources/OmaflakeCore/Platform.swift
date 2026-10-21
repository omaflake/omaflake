// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Architectures, named like Nix names systems (plan B114): what a recipe can be built for (`systems` in recipe.toml),
/// what an image was built for and what a VM runs as (`system` in image.json and instance.json).
public enum Platform {
    /// The systems omaflake knows per guest kind
    public static func systems(for kind: GuestKind) -> [String] {
        switch kind {
        case .nixos: ["aarch64-linux", "x86_64-linux"]
        case .macos: ["aarch64-darwin"]
        }
    }

    /// The system this host builds and runs guests of `kind` as; nil = that kind does not run here
    public static func hostSystem(for kind: GuestKind) -> String? {
        #if os(macOS) && arch(arm64)
        switch kind {
        case .nixos: "aarch64-linux"
        case .macos: "aarch64-darwin"
        }
        #elseif os(Linux) && arch(x86_64)
        kind == .nixos ? "x86_64-linux" : nil
        #elseif os(Linux) && arch(arm64)
        kind == .nixos ? "aarch64-linux" : nil
        #else
        nil
        #endif
    }

    /// The system of a file or recipe from before B114, which has none: everything then ran on a Mac with Apple silicon
    public static func legacySystem(for kind: GuestKind) -> String {
        kind == .macos ? "aarch64-darwin" : "aarch64-linux"
    }

    /// A guest of `system` runs on this host
    public static func runsHere(_ system: String) -> Bool {
        GuestKind.allCases.contains { hostSystem(for: $0) == system }
    }

    /// "aarch64-linux" → "aarch64" (the part shown next to a recipe or image)
    public static func architecture(_ system: String) -> String {
        String(system.split(separator: "-").first ?? Substring(system))
    }

    /// The error for a guest that does not run here
    static func notHere(_ what: String, system: String) -> VMError {
        VMError(String(localized: "\(what) is built for \(system) – this computer runs \(hostDescription)"))
    }

    /// What runs here, for messages ("aarch64-linux, aarch64-darwin")
    static var hostDescription: String {
        let systems = GuestKind.allCases.compactMap { hostSystem(for: $0) }
        return systems.isEmpty ? "no guests" : systems.joined(separator: ", ")
    }
}
