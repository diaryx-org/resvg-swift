# resvg-swift

SVG on Apple platforms as **vectors**, through [usvg](https://github.com/linebender/resvg):
a Rust crate that flattens usvg's tree into a display list, a UniFFI binding
that hands the list to Swift, and a `CGContext` extension that replays it.
Paths stay paths — crisp at any size, and real vector paths in a PDF — and what
CoreGraphics has no primitive for (filters, masks, patterns) arrives as pixels
resvg rendered at the exact scale it will be shown at.

```swift
import ResvgCoreGraphics

let picture = try SVGPicture(contentsOf: url)
picture.draw(in: context, rect: bounds, scale: window.backingScaleFactor)
```

## Why this exists

Apple has no public API that renders an SVG *file* on iOS: `UIImage(data:)`
returns `nil` for SVG bytes, and only asset-catalog SVGs go through CoreSVG.
On macOS `NSImage(data:)` happens to work, undocumented, through the same
private framework — a subset tuned for icons, with at least one known crash
(CSS `opacity` alongside `fill`). The Swift SVG libraries that do exist each
parse SVG themselves, and none reaches usvg's coverage of the static SVG 1.1
subset.

usvg already does the hard part: CSS, `<use>`, units, `<text>` laid out into
glyph outlines with system fonts. What is left after it runs is a tree of paths
and paints. Flattening that tree is a few hundred lines; replaying the result
into CoreGraphics is a few hundred more. The renderer that results is the same
one on every platform that shares the Rust side — the terminal, gpui, and Apple
all draw what resvg would draw.

## Layout

| part | what it is |
|------|------------|
| [`crates/usvg-flatten`](crates/usvg-flatten) | **usvg tree → display list.** Pure Rust, no FFI. `flatten(&tree, &Options { raster_scale })` gives a `Vec<Op>` in paint order: `Fill`, `Stroke`, `PushLayer`/`PopLayer` (opacity, blend, clips), `Image` (encoded bytes), `Raster` (resvg's pixels for a subtree with no vector form). Ships `replay`, a tiny-skia replayer, so the tests can hold `flatten` to resvg's own pixels. |
| [`crates/resvg-uniffi`](crates/resvg-uniffi) | **The UniFFI binding.** `SvgDocument` parses once; `displayList(rasterScale:)` returns the list as value types. Mirrors `usvg-flatten`'s types rather than deriving on them, so the wire format is versioned here. UniFFI rather than Swift-specific: Kotlin and Python bindings are a generator run away. |
| [`packages/resvg-swift`](packages/resvg-swift) | **The Swift package.** `ResvgFFI` is the committed generated binding; `ResvgCoreGraphics` is `CGContext.draw(_:transform:)` — the replayer — and `SVGPicture`, which parses, caches the list per scale, and aspect-fits into a rect. `Package.swift` sits at the repo root because SwiftPM needs it there. |

## What is vector, what is raster

Vector, at every scale: fills and strokes with solid colour or pad-spread
linear/radial gradients; dashes, caps, joins; group opacity and all sixteen
blend modes; clip paths; embedded PNG/JPEG/GIF/WebP by their bytes; nested SVG
`<image>`s by recursion; text, as glyph outlines.

Raster, through resvg at `rasterScale` device pixels per user unit: groups
with a filter or a mask, paths painted with a pattern or a `reflect`/`repeat`
gradient, and clip paths whose contents aren't plain shapes. Each is one
`Raster` op in canvas coordinates. `SVGPicture` re-flattens when the scale
changes and caches otherwise. Vector masks and pattern paint are the next
candidates — see [`docs/tasks/`](docs/tasks/tasks.md).

One CoreGraphics-specific fallback: a clip of *several overlapping* shapes has
no exact path form (CoreGraphics has no path union), so it becomes an alpha
mask at the context's device resolution. One shape — the common case — is an
exact path clip.

## Linking

**Two Rust staticlibs cannot share one executable**; each carries its own
`std`. So the Swift package does not link a library of its own — it references
the `resvg_uniffi` symbols and expects the host to have force-loaded an archive
that contains them:

- A host that already links a UniFFI crate of its own (leaf's `leaf-ffi`,
  say) makes `resvg-uniffi` a Cargo dependency of *that* crate — a
  `pub use resvg_uniffi as _;` keeps the scaffolding linked — and force-loads
  the one `.a` it already builds.
- A host with no Rust of its own builds `crates/resvg-uniffi` as a staticlib
  (`cargo build -p resvg-uniffi --release --target …`) and force-loads it:
  `OTHER_LDFLAGS = -force_load <path>/libresvg_uniffi.a`.

Both Swift products are then `.package(url: "https://github.com/diaryx-org/resvg-swift.git", from: "X.Y.Z")`.

## Developing

```sh
cargo xtask ci              # fmt, clippy, tests, per-crate check, binding drift, resvg suite
cargo xtask ci suite        # just the resvg suite (fetches it into target/ once)
scripts/gen-bindings.sh     # after changing crates/resvg-uniffi's surface
scripts/test-swift.sh       # the CoreGraphics tests, on a Mac
```

The Rust tests are the contract: an SVG rendered by resvg and by `flatten` +
`replay` must be the same picture. `agrees_with_resvg.rs` is one hand-written
case per feature at a fractional scale; `resvg_suite.rs` is **resvg's own test
suite** — every SVG resvg tests itself with, fetched at the pinned version —
through both. At resvg 0.46.0 that is 1,697 cases compared, 1,676 bit-for-bit
identical, the remaining 21 within a few levels on under 2% of their pixels,
and an empty allowlist. `cargo xtask ci suite` runs it; the other jobs don't
need the network.

The Swift tests draw through the real binding into bitmap contexts and read
pixels back — the y-direction of images and the composition of group opacity
are the two things a CoreGraphics backend gets wrong first, and both are
asserted.

Releases are `dx release` from the org's devtools, configured in
`.config/release.toml`; the tag publishes both crates to crates.io.

## License

MIT or Apache-2.0, at your option.
