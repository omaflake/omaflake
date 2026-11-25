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
                // Usage descriptions for camera/microphone (a command-line tool has no bundle Info.plist)
                .unsafeFlags(["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist",
                              "-Xlinker", packageRoot + "/App/omaf-runner-Info.plist"]),
            ]
        ),
        // Menu bar app, a client of `omaf serve`; scripts/bundle-app.sh wraps it (and the Rust `omaf`) into omaflake.app
        .executableTarget(name: "OmaflakeApp", dependencies: ["OmaflakeKit"]),
        .testTarget(name: "OmaflakeKitTests", dependencies: ["OmaflakeKit"]),
    ]
)
