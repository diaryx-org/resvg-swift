---
title: Reflect and repeat gradient spreads
description: reflect/repeat gradients rasterize the path because CGGradient only pads; tiling the stops keeps them vector
author: adammharris
status: open
created: 2026-09-12
updated: 2026-09-12
part_of: '[Tasks](tasks.md)'
---
# Reflect and repeat gradient spreads

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
