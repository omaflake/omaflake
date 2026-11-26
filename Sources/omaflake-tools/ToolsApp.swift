// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import SystemExtensions

/// "omaflake Tools.app", in a macOS guest's /Applications (plan B184): omaflake's app in the guest – the home of
/// what needs an app there (a system extension must ship in one; permissions such as Accessibility are given to
/// one). For now that is the camera extension (Contents/Library/SystemExtensions): started, the app asks macOS to
/// activate it – or, with `--remove`, to deactivate it. No window: it says what the system answered on stdout and
/// in ~/Library/Logs/omaflake/tools.log, and exits – 0 when done, 1 when refused, 2 when the user did not approve
/// within ten minutes.
let extensionIdentifier = "app.omaflake.guest.camera"

@MainActor
func say(_ text: String) {
    let line = "\(Date().formatted(.iso8601)) \(text)\n"
    FileHandle.standardOutput.write(Data(line.utf8))
    let dir = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/omaflake")
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    let file = dir.appendingPathComponent("tools.log")
    if let handle = try? FileHandle(forWritingTo: file) {
        _ = try? handle.seekToEnd()
        try? handle.write(contentsOf: Data(line.utf8))
        try? handle.close()
    } else {
        try? Data(line.utf8).write(to: file)
    }
}

@MainActor
final class Delegate: NSObject, OSSystemExtensionRequestDelegate {
    nonisolated func request(_ request: OSSystemExtensionRequest,
                             actionForReplacingExtension existing: OSSystemExtensionProperties,
                             withExtension ext: OSSystemExtensionProperties) -> OSSystemExtensionRequest.ReplacementAction {
        let (old, new) = (existing.bundleVersion, ext.bundleVersion)
        Task { @MainActor in say("replacing version \(old) with \(new)") }
        return .replace
    }

    nonisolated func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        Task { @MainActor in
            say("NEEDS APPROVAL: System Settings → General → Login Items & Extensions → Camera Extensions")
        }
    }

    nonisolated func request(_ request: OSSystemExtensionRequest,
                             didFinishWithResult result: OSSystemExtensionRequest.Result) {
        let done = result == .completed
        Task { @MainActor in
            say("FINISHED: \(done ? "completed" : "completes after a restart")")
            exit(0)
        }
    }

    nonisolated func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        let e = error as NSError
        let text = "FAILED: \(e.domain) \(e.code) – \(e.localizedDescription)"
        Task { @MainActor in
            say(text)
            exit(1)
        }
    }
}

@main
@MainActor
enum ToolsApp {
    static let delegate = Delegate()

    static func main() {
        // The build for ordinary guests carries no extension until omaflake is signed (scripts/bundle-tools.sh)
        let extensions = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/SystemExtensions")
        guard FileManager.default.fileExists(atPath: extensions.path) else {
            say("NOTHING TO DO: this build of omaflake Tools carries no camera extension")
            exit(0)
        }
        let remove = CommandLine.arguments.contains("--remove")
        let request = remove
            ? OSSystemExtensionRequest.deactivationRequest(forExtensionWithIdentifier: extensionIdentifier, queue: .main)
            : OSSystemExtensionRequest.activationRequest(forExtensionWithIdentifier: extensionIdentifier, queue: .main)
        request.delegate = delegate
        say("\(remove ? "deactivation" : "activation") request for \(extensionIdentifier) from \(Bundle.main.bundlePath)")
        OSSystemExtensionManager.shared.submitRequest(request)
        // The approval takes as long as the user needs – but not for ever
        DispatchQueue.main.asyncAfter(deadline: .now() + 600) {
            say("TIMEOUT: no answer after 10 minutes")
            exit(2)
        }
        NSApplication.shared.run()
    }
}
