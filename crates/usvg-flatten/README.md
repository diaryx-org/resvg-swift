# usvg-flatten

Flattens a [`usvg`](https://docs.rs/usvg) tree into a **display list**: a flat
sequence of `Fill`, `Stroke`, `Image`, `PushLayer`/`PopLayer` and `Raster`
operations in canvas order, with every coordinate resolved. A renderer replays
it with a dozen calls of its own 2D API and never sees an SVG element.

```rust
let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())?;
let list = usvg_flatten::flatten(&tree, &usvg_flatten::Options { raster_scale: 2.0 });
for op in &list.ops { /* CGContext / Skia / Direct2D … */ }
```

usvg has already done the hard part by the time the walker runs — CSS, `<use>`,
units, `<text>` laid out into glyph paths — so what is left is a tree of paths,
groups, and paints. Everything a vector API has a primitive for stays vector:
paths with solid or gradient paint, strokes with dashes, group opacity, blend
modes, and clip paths. What it does not — filters, masks, pattern paint, and
gradients with `reflect`/`repeat` spread — is rendered by `resvg` at
`raster_scale` device pixels per user unit and emitted as one `Raster` op. A
consumer that redraws at a new size re-flattens with a new scale; nothing else
changes.

The `replay` module rasterizes a display list with tiny-skia. Its purpose is the
test suite: the same SVG through `resvg` and through `flatten` + `replay` must
produce the same pixels, which is what proves the flattening lossless without
any platform renderer in the loop.

This is the Rust half of [resvg-swift](../../README.md); the Swift half replays
the list into a `CGContext`.
