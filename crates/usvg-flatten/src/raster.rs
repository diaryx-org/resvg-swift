//! The escape hatch: one node as resvg paints it.
//!
//! Filters, masks, pattern paint, and repeating gradients have no vector form in
//! the display list (yet — masks and patterns are candidates, see
//! `docs/tasks/`). Rather than drop them or approximate, the flattener hands
//! the whole node to resvg at the consumer's scale and emits the pixels as one
//! [`RasterOp`] in canvas space. The consumer chose the scale, so the picture is
//! as sharp as the display it is drawn on; re-flattening at a new scale is the
//! only cost of resizing.

use resvg::tiny_skia;
use usvg::tiny_skia_path as tsp;

use crate::{RasterOp, Rect};

/// The longest side, in pixels, a fallback raster may have. A filtered group
/// scaled by a huge factor would otherwise ask for gigabytes; past this the
/// scale is reduced instead, which softens that one node rather than failing
/// the whole picture.
const MAX_SIDE: f32 = 8192.0;

/// Render `node` to pixels covering its layer bounding box, or `None` if it
/// has no area (which is also when resvg would draw nothing).
///
/// `outer` maps the canvas of the tree `node` belongs to onto the display
/// list's canvas — identity for the document, the `<image>` placement for a
/// nested SVG.
pub(crate) fn render(node: &usvg::Node, outer: tsp::Transform, scale: f32) -> Option<RasterOp> {
    // The bounding box usvg knows is in the node's own tree; the rectangle the
    // pixels cover is that box carried onto the canvas.
    let bbox = node.abs_layer_bounding_box()?;
    let canvas = bbox.to_rect().transform(outer)?;

    let scale = scale.min(MAX_SIDE / canvas.width().max(canvas.height()));
    let width = (canvas.width() * scale).ceil().max(1.0) as u32;
    let height = (canvas.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;

    // `resvg::render_node` applies `T · translate(-bbox.origin) · abs_transform`
    // and we want `scale · translate(-canvas.origin) · outer · abs_transform`,
    // so T is what turns the one into the other.
    let root = tsp::Transform::from_scale(scale, scale)
        .pre_translate(-canvas.x(), -canvas.y())
        .pre_concat(outer)
        .pre_translate(bbox.x(), bbox.y());
    resvg::render_node(node, root, &mut pixmap.as_mut())?;

    Some(RasterOp {
        rgba: pixmap.take(),
        width,
        height,
        rect: Rect {
            x: canvas.x(),
            y: canvas.y(),
            width: canvas.width(),
            height: canvas.height(),
        },
    })
}
