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
        .library(name: "OmaflakeCore", targets: ["OmaflakeCore"]),
        .executable(name: "omaf", targets: ["omaf"]),
        .executable(name: "omaf-runner", targets: ["omaf-runner"]),
        .executable(name: "OmaflakeApp", targets: ["OmaflakeApp"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser.git", from: "1.8.0"),
    ],
    targets: [
        // Shared logic for the CLI, the runner and the menu bar app
        .target(name: "OmaflakeCore"),
        .executableTarget(
            name: "omaf",
            dependencies: [
                "OmaflakeCore",
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
            ]
        ),
        // One process per macOS guest (Virtualization.framework); sign with scripts/sign-runner.sh
        .executableTarget(
            name: "omaf-runner",
            dependencies: [
                "OmaflakeCore",
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
            ],
            linkerSettings: [
                .linkedFramework("Virtualization"), .linkedFramework("AppKit"), .linkedFramework("AVFoundation"),
                .linkedFramework("CoreMedia"), .linkedFramework("CoreVideo"),
                // Usage descriptions for camera/microphone (a command-line tool has no bundle Info.plist)
                .unsafeFlags(["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist",
                              "-Xlinker", packageRoot + "/App/omaf-runner-Info.plist"]),
            ]
        ),
        // Menu bar app; scripts/bundle-app.sh wraps it into omaflake.app
        .executableTarget(name: "OmaflakeApp", dependencies: ["OmaflakeCore"]),
        .testTarget(name: "OmaflakeCoreTests", dependencies: ["OmaflakeCore"]),
    ]
)
