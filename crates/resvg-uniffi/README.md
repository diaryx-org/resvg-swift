# resvg-uniffi

The UniFFI binding of [resvg-swift](../../README.md). `SvgDocument` parses bytes
with usvg and `displayList(rasterScale:)` returns a
[`usvg-flatten`](../usvg-flatten) display list as UniFFI records, which the
`ResvgCoreGraphics` Swift package replays into a `CGContext`.

The types here mirror `usvg_flatten`'s one for one rather than deriving on them,
so the wire format is this crate's to version and the pure crate stays free of
FFI machinery.

## Linking

Two Rust staticlibs cannot be linked into one executable — each carries its own
`std`. A host that already links a UniFFI crate of its own (leaf-ffi, say) must
take **this crate as a Cargo dependency of that crate**, so the scaffolding
lands in the host's single `.a`; the Swift package then finds its symbols
there. `pub use resvg_uniffi as _;` is enough, since the host force-loads the
archive. A host with no Rust of its own builds this crate's staticlib and
force-loads that.
