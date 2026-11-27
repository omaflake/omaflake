// swift-tools-version: 6.0
// SPDX-License-Identifier: AGPL-3.0-or-later
// omaflake – clean NixOS and macOS VMs on Apple silicon, one click away.
import Foundation
import PackageDescription

/// Folder of this file – the linker needs absolute paths
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path

let package = Package(
    name: "omaflake",
    // macOS 27 (decided 2026-10-17): the only version tested, guest provisioning needs it
    platforms: [.macOS("27.0")],
    products: [
        .library(name: "OmaflakeKit", targets: ["OmaflakeKit"]),
        .executable(name: "omaf-runner", targets: ["omaf-runner"]),
        .executable(name: "OmaflakeApp", targets: ["OmaflakeApp"]),
        .executable(name: "omaflake-camera", targets: ["omaflake-camera"]),
        .executable(name: "omaflake-tools", targets: ["omaflake-tools"]),
    ],
    targets: [
        // The Swift side of omaflake: the client of the Rust core (`omaf serve`, docs/api.md) with its types, what
        // omaf-runner needs (agent protocol, macOS VM bundle, sockets) and the brand images. The logic lives in the
        // Rust core (crates/); the CLI `omaf` is crates/omaf.
        .target(name: "OmaflakeKit"),
        // One process per macOS guest (Virtualization.framework); sign with scripts/sign-runner.sh
        .executableTarget(
            name: "omaf-runner",
            // No packages of others: its command line is its own (Arguments.swift, plan B177)
            dependencies: ["OmaflakeKit"],
            linkerSettings: [
                .linkedFramework("Virtualization"), .linkedFramework("AppKit"), .linkedFramework("AVFoundation"),
                .linkedFramework("CoreMedia"), .linkedFramework("CoreVideo"),
                // Reading the guest's screen (plan B184): Vision, and ScreenCaptureKit when the view gives no picture
                .linkedFramework("Vision"), .linkedFramework("ScreenCaptureKit"),
                // Usage descriptions for camera/microphone (a command-line tool has no bundle Info.plist)
                .unsafeFlags(["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist",
                              "-Xlinker", packageRoot + "/App/omaf-runner-Info.plist"]),
            ]
        ),
        // Menu bar app, a client of `omaf serve`; scripts/bundle-app.sh wraps it (and the Rust `omaf`) into omaflake.app
        .executableTarget(name: "OmaflakeApp", dependencies: ["OmaflakeKit"]),
        // For macOS guests (plan B184): omaflake's camera extension (a CoreMediaIO system extension reading the
        // agent's frames) and "omaflake Tools", the guest's app it ships in – for now it only asks macOS to activate
        // the extension; what else needs an app in the guest goes there. scripts/bundle-tools.sh wraps both into
        // "omaflake Tools.app"
        .executableTarget(
            name: "omaflake-camera", dependencies: ["OmaflakeKit"],
            linkerSettings: [.linkedFramework("CoreMediaIO"), .linkedFramework("CoreMedia"), .linkedFramework("CoreVideo")]
        ),
        .executableTarget(
            name: "omaflake-tools",
            linkerSettings: [.linkedFramework("SystemExtensions"), .linkedFramework("AppKit")]
        ),
        // For unchained macOS guests (plan B190): omaflake's FSKit module – each volume a volume of its own under the
        // Finder's Locations. Here only compiled (checked); the extension's binary is made by scripts/bundle-tools.sh
        // with swiftc and the entry point NSExtensionMain, as Apple's modules have it – the same source as a SwiftPM
        // executable trapped in ExtensionFoundation at its start (checked in a macOS 27 guest; why: OPEN)
        .target(name: "omaflake-volumes", linkerSettings: [.linkedFramework("FSKit")]),
        .testTarget(name: "OmaflakeKitTests", dependencies: ["OmaflakeKit"]),
    ]
)
