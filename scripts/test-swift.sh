#!/usr/bin/env bash
#
# Run the ResvgCoreGraphics tests (macOS host): parse SVGs through the real
# Rust binding and draw them into bitmap CGContexts.
#
# The Swift package references the FFI symbols, so the test binary must link
# the Rust staticlib. We force-load it, the same way a consuming app does.
#
# Usage: scripts/test-swift.sh [extra `swift test` args…]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "▸ Building resvg-swift (host) staticlib…"
cargo build -p resvg-swift --manifest-path "$ROOT/Cargo.toml" >/dev/null
STATIC="$ROOT/target/debug/libresvg_swift.a"
[ -f "$STATIC" ] || { echo "missing $STATIC"; exit 1; }

echo "▸ swift test (ResvgCoreGraphics)…"
# The package manifest lives at the repo root (see Package.swift).
swift test --package-path "$ROOT" \
  -Xlinker -force_load -Xlinker "$STATIC" "$@"
