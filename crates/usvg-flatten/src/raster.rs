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

/// Render `node` to pixels covering what it paints, or `None` if that has no
/// area or lies wholly outside the canvas (both are when resvg would draw
/// nothing).
///
/// `parent` is the transform resvg would be holding when it reached `node`:
/// the node's parent's local-to-canvas. [`resvg::render_node`] accumulates
/// the node's own and its descendants' transforms from whatever root it is
/// given, and nothing else — so the ancestors have to be in the root, or a
/// filtered rect inside a `<g transform>` lands where the untransformed one
/// would. `canvas` is the visible area — pixels outside it are never seen and
/// resvg never paints them, so neither does this.
///
/// The extent is computed here from the node's own geometry under `parent`,
/// not read from usvg's absolute bounding boxes: those are stale for a
/// subtree cloned into a `<use>` that carries `context-fill` (usvg 0.46 keeps
/// the clone's pre-`use` transforms), and resvg never notices because it
/// renders with relative transforms. `render_node` does subtract the absolute
/// box's origin internally, so that value — right or wrong — is added back in
/// the root transform, where it cancels.
///
/// The rectangle is snapped to the device pixel grid at `scale`, so a
/// consumer replaying at the same scale draws the pixels 1:1 with no
/// resampling — which is what makes the fallback exact rather than merely
/// close.
pub(crate) fn render(
    node: &usvg::Node,
    parent: tsp::Transform,
    canvas: tsp::Rect,
    scale: f32,
) -> Option<RasterOp> {
    // What will be painted, in canvas units: a path's stroke box (its fill
    // box alone would crop the stroke), a group's layer box (stroke and filter
    // region included), each under the transform resvg would draw it with.
    let painted = match node {
        usvg::Node::Path(path) => path.stroke_bounding_box().transform(parent)?,
        usvg::Node::Group(group) => group
            .layer_bounding_box()
            .to_rect()
            .transform(parent.pre_concat(group.transform()))?,
        usvg::Node::Text(text) => {
            let group = text.flattened();
            group
                .layer_bounding_box()
                .to_rect()
                .transform(parent.pre_concat(group.transform()))?
        }
        usvg::Node::Image(image) => image.bounding_box().transform(parent)?,
    };
    let visible = intersect(painted, canvas)?;

    let scale = scale.min(MAX_SIDE / visible.width().max(visible.height()));

    // Snap outward to whole device pixels, with the two-pixel margin resvg
    // gives its own layers so anti-aliased edges aren't clipped — but never
    // past the canvas, which is not painted anyway.
    let left = ((visible.left() * scale).floor() - 2.0).max((canvas.left() * scale).floor());
    let top = ((visible.top() * scale).floor() - 2.0).max((canvas.top() * scale).floor());
    let right = ((visible.right() * scale).ceil() + 2.0).min((canvas.right() * scale).ceil());
    let bottom = ((visible.bottom() * scale).ceil() + 2.0).min((canvas.bottom() * scale).ceil());
    let width = (right - left).max(1.0) as u32;
    let height = (bottom - top).max(1.0) as u32;
    let rect = Rect {
        x: left / scale,
        y: top / scale,
        width: width as f32 / scale,
        height: height as f32 / scale,
    };
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;

    // resvg applies `T · translate(-layer.origin) · <node's own transforms>`,
    // `layer` being its absolute box; we want
    // `scale · translate(-rect.origin) · parent · <the same>`, so T is what
    // turns the one into the other.
    let layer = node.abs_layer_bounding_box()?;
    let root = tsp::Transform::from_scale(scale, scale)
        .pre_translate(-rect.x, -rect.y)
        .pre_concat(parent)
        .pre_translate(layer.x(), layer.y());
    resvg::render_node(node, root, &mut pixmap.as_mut())?;

    Some(RasterOp {
        rgba: pixmap.take(),
        width,
        height,
        rect,
    })
}

fn intersect(a: tsp::Rect, b: tsp::Rect) -> Option<tsp::Rect> {
    tsp::Rect::from_ltrb(
        a.left().max(b.left()),
        a.top().max(b.top()),
        a.right().min(b.right()),
        a.bottom().min(b.bottom()),
    )
}
