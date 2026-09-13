---
title: Vector masks
description: A masked group is rasterized whole; a mask is a nested display list applied as coverage, so the content can stay vector
author: adammharris
status: open
created: 2026-09-12
updated: 2026-09-12
part_of: '[Tasks](tasks.md)'
---
# Vector masks

A group with a `mask` is rasterized whole today (`flatten.rs`, the filter/mask
branch of `Walker::group`). A mask is a subtree painted into a luminance or
alpha coverage and applied to the group's layer — CoreGraphics can do that at
draw time with `clip(to:mask:)` on an image rendered at device resolution, or
stay vector for the common single-shape luminance mask by clipping.

Shape: `Layer.mask: Option<Mask>` where `Mask { rect, kind: Luminance | Alpha,
content: Vec<Op>, mask: Option<Box<Mask>> }` — a nested display list. The
replayer renders `content` to a grayscale bitmap at the current device scale
and clips with it; the tiny-skia replayer mirrors resvg's `mask::apply`. The
group's *content* then stays vector; only the coverage is pixels, and at device
resolution, so it is indistinguishable on screen. In a PDF the mask is a
bitmap — still better than today, where the whole group is.

Done when `mask_falls_back_to_raster` in `tests/agrees_with_resvg.rs` is
renamed and asserts a `PushLayer` with a mask rather than a `Raster`.
