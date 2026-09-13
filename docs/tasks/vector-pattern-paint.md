---
title: Vector pattern paint
description: Pattern-painted paths are rasterized whole; CGPattern with a replayed tile is the vector form
author: adammharris
status: open
created: 2026-09-12
updated: 2026-09-12
part_of: '[Tasks](tasks.md)'
---
# Vector pattern paint

A path filled or stroked with `url(#pattern)` is rasterized whole today
(`paint()` in `flatten.rs` returns `None` for `Paint::Pattern`). CoreGraphics
has `CGPattern` with a draw callback, which is exactly a nested display list
tiled over a rect.

Shape: `Paint::Pattern(Pattern { rect, transform, content: Vec<Op> })`. The
CoreGraphics replayer builds a `CGPattern` whose callback replays `content`;
the tiny-skia replayer mirrors resvg's `render_pattern_pixmap`. Care: usvg's
pattern `rect` is the tile in the pattern's own space and `transform` maps it
to the path's local space; resvg renders the tile at the transform's scale.

Done when `pattern_falls_back_to_raster` asserts a `Fill` with pattern paint.
