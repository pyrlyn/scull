// swift-tools-version: 6.2
// The macOS host: a C module over scull.h, the terminal view in ScullKit,
// and the SwiftUI app. `just macos` builds the Rust static library first
// and wraps the executable in Scull.app.

import Foundation
import PackageDescription

// SwiftPM cannot build Cargo crates, so the app links the library that
// `just macos` puts here. Unsafe flags are fine for a root package.
let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let rustStaticLib = packageDir
    .appendingPathComponent("../target/aarch64-apple-darwin/release/libscull_ffi.a")
    .standardizedFileURL.path

// The header gives `tt_status` a fixed underlying type only in C23; in
// older C it is a plain int32_t typedef beside the enum, which Swift
// cannot tell apart from the enum.
let c23: [SwiftSetting] = [.unsafeFlags(["-Xcc", "-std=c23"])]

let package = Package(
    name: "Scull",
    platforms: [.macOS("26.0")],
    targets: [
        .systemLibrary(name: "CScull", path: "Sources/CScull"),
        .target(
            name: "ScullKit",
            dependencies: ["CScull"],
            swiftSettings: c23,
            linkerSettings: [.unsafeFlags([rustStaticLib])]
        ),
        .testTarget(name: "ScullKitTests", dependencies: ["ScullKit"], swiftSettings: c23),
    ]
)
