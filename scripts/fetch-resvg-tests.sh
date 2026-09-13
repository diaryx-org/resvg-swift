#!/usr/bin/env bash
#
# Fetch resvg's own test suite — ~1,700 SVGs with reference renders — at the
# exact resvg version this workspace pins, into target/resvg-tests/. The suite
# lives in the resvg repository, not in the crates.io package, and it is 24 MB,
# so it is fetched rather than vendored; `cargo xtask ci suite` calls this and
# then runs crates/usvg-flatten/tests/resvg_suite.rs over it.
#
# Usage: scripts/fetch-resvg-tests.sh   (prints the suite directory)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The pinned version is a fact in Cargo.lock; read it rather than repeat it.
VERSION="$(awk '/^name = "resvg"$/ { getline; sub(/^version = "/, ""); sub(/"$/, ""); print; exit }' "$ROOT/Cargo.lock")"
[ -n "$VERSION" ] || { echo "fetch-resvg-tests: resvg not in Cargo.lock" >&2; exit 1; }

DEST="$ROOT/target/resvg-tests"
STAMP="$DEST/.version"
if [ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$VERSION" ]; then
  echo "$DEST/crates/resvg/tests"
  exit 0
fi

rm -rf "$DEST"
echo "▸ Fetching resvg v$VERSION test suite…" >&2
git clone -q --depth 1 --branch "v$VERSION" --filter=blob:none --sparse \
  https://github.com/linebender/resvg "$DEST" >/dev/null 2>&1
git -C "$DEST" sparse-checkout set crates/resvg/tests >/dev/null
echo "$VERSION" > "$STAMP"
echo "$DEST/crates/resvg/tests"
