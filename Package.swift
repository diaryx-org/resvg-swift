// swift-tools-version:5.9
//
// The Swift package an AppKit/UIKit app links to draw SVGs as vectors through
// usvg. The manifest lives at the repository root — not in
// packages/resvg-swift/ — because SwiftPM resolves git dependencies only from a
// root Package.swift, and by-version resolution is how a consumer is meant to
// take this package:
//
//   .package(url: "https://github.com/diaryx-org/resvg-swift.git", from: "X.Y.Z")
//
// It builds the UniFFI binding + the ResvgCoreGraphics replayer **from
// source**; the Rust staticlib itself is linked by the consuming app. Two Rust
// staticlibs cannot share one executable, so an app that already links a Rust
// FFI crate of its own makes `resvg-swift` a Cargo dependency of that crate and
// force-loads the one archive; an app with no Rust of its own builds
// crates/resvg-swift's staticlib and force-loads that. README.md → Linking.
//
// The two `uniffi-generated/` inputs below are committed — a version-resolved
// clone runs no generators, so they must build as-is. `scripts/gen-bindings.sh`
// writes them from crates/resvg-swift and CI holds them to it (`--check`).
import PackageDescription

let package = Package(
    name: "ResvgSwift",
    platforms: [.macOS(.v12), .iOS(.v16)],
    products: [
        // The low-level binding: `SvgDocument` and the display-list value types.
        .library(name: "ResvgFFI", targets: ["ResvgFFI"]),
        // The CoreGraphics replayer over it: `CGContext.draw(_:in:)`.
        .library(name: "ResvgCoreGraphics", targets: ["ResvgCoreGraphics"]),
    ],
    targets: [
        // The C ABI as a clang module (`import resvg_swiftFFI`). No library to
        // link here — the app force-loads the Rust `.a`, so the symbols the
        // generated Swift references stay undefined until the final link.
        .systemLibrary(
            name: "resvg_swiftFFI",
            path: "packages/resvg-swift/uniffi-generated/headers"
        ),
        // The generated Swift, compiled against that C module.
        .target(
            name: "ResvgFFI",
            dependencies: ["resvg_swiftFFI"],
            path: "packages/resvg-swift/uniffi-generated/Sources/ResvgFFI"
        ),
        // The replayer (committed source).
        .target(
            name: "ResvgCoreGraphics",
            dependencies: ["ResvgFFI"],
            path: "packages/resvg-swift/Sources/ResvgCoreGraphics"
        ),
        // Draws real SVGs into bitmap contexts, so it needs the Rust staticlib:
        // `scripts/test-swift.sh` force-loads it (plain `swift test` won't find
        // the `.a`).
        .testTarget(
            name: "ResvgCoreGraphicsTests",
            dependencies: ["ResvgCoreGraphics"],
            path: "packages/resvg-swift/Tests/ResvgCoreGraphicsTests"
        ),
    ]
)
