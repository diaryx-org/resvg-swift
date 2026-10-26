---
title: Reflect and repeat gradient spreads
description: reflect/repeat gradients rasterize the path because CGGradient only pads; tiling the stops keeps them vector
author: adammharris
status: done
created: 2026-09-12
updated: 2026-10-25
part_of: '[Tasks](tasks.md)'
---
# Reflect and repeat gradient spreads

> **Done** (2026-10-25), by option 1. `flatten` tiles a `reflect` or `repeat`
> gradient's stops over the bounding box of what it paints (the stroke's, for a
> stroke), plus one tile each side for the anti-aliased edge, and emits a pad
> gradient. A tile is closed with its end colours, so stops short of 0 or 1 pad
> inside it. Two cases still rasterize: a radial gradient whose focal point is
> not strictly inside its circle (some points then have no position on it), and
> one that would take more than 256 tiles. `repeating_gradient_stays_vector`
> (the test this task named) and three more cases in `agrees_with_resvg.rs`
> hold it to resvg's pixels, as does resvg's own suite.

`spreadMethod="reflect"` and `"repeat"` rasterize the whole path today, because
`CGGradient` only pads. Two ways forward, either of which keeps the path
vector:

1. Expand the stops. For a bounded fill the gradient only has to cover the
   path's bounding box; the flattener can compute how many repetitions that
   takes and emit a pad-spread gradient with the stops tiled (reversed every
   other tile for `reflect`) and the endpoints stretched to match. Exact for
   linear; for radial, `r` grows and the stops tile inward, which is also
   exact. The list stays as it is — no new variant.
2. Carry the spread mode and let the replayer do the same expansion.

Option 1 keeps the backends dumb, which is the design. Done when
`repeating_gradient_falls_back_to_raster` asserts a `Fill` with
`Paint::Linear`.
