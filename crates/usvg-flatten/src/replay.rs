//! A tiny-skia replayer for a [`DisplayList`].
//!
//! This is the reference backend: the smallest program that turns a display
//! list back into pixels, written against the same rasterizer resvg uses so
//! the two can be compared pixel for pixel. The test suite renders an SVG both
//! ways and asserts the images agree, which is what proves [`flatten`] lossless
//! with no platform renderer involved. Every other backend — the CoreGraphics
//! one in `packages/resvg-swift/` first — is a translation of this file into
//! its own API, one match arm at a time.
//!
//! It is deliberately not clever: a layer is a full-canvas pixmap, a clip is a
//! full-canvas mask. resvg crops layers to their bounding box; the pixels come
//! out the same, this is just slower, and speed is not what it is for.
//!
//! [`flatten`]: crate::flatten

use resvg::tiny_skia;
use usvg::tiny_skia_path as tsp;

use crate::{
    BlendMode, Clip, DisplayList, FillOp, FillRule, ImageFormat, ImageOp, Layer, LineCap, LineJoin,
    Op, Paint, Path, PathVerb, RasterOp, Stop, StrokeOp, Transform,
};

/// Draw `list` onto `pixmap` under `transform` (canvas units to pixels).
///
/// Unbalanced `PushLayer`/`PopLayer` — which [`flatten`](crate::flatten) never
/// produces — is tolerated: a missing pop composites at the end, a stray pop
/// is ignored.
pub fn render(list: &DisplayList, transform: Transform, pixmap: &mut tiny_skia::PixmapMut) {
    let root = to_ts(transform);
    let mut stack: Vec<(Layer, tiny_skia::Pixmap)> = Vec::new();

    for op in &list.ops {
        match op {
            Op::PushLayer(layer) => {
                let Some(surface) = tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()) else {
                    continue;
                };
                stack.push((layer.clone(), surface));
            }
            Op::PopLayer => {
                if let Some((layer, surface)) = stack.pop() {
                    composite(layer, surface, root, &mut top(&mut stack, pixmap));
                }
            }
            other => draw(other, root, &mut top(&mut stack, pixmap)),
        }
    }

    // Pops the list forgot.
    while let Some((layer, surface)) = stack.pop() {
        composite(layer, surface, root, &mut top(&mut stack, pixmap));
    }
}

/// The surface to draw on now: the innermost open layer, or the caller's.
fn top<'a>(
    stack: &'a mut [(Layer, tiny_skia::Pixmap)],
    pixmap: &'a mut tiny_skia::PixmapMut,
) -> tiny_skia::PixmapMut<'a> {
    match stack.last_mut() {
        Some((_, surface)) => surface.as_mut(),
        None => {
            let (width, height) = (pixmap.width(), pixmap.height());
            tiny_skia::PixmapMut::from_bytes(pixmap.data_mut(), width, height)
                .expect("a PixmapMut's own bytes describe a valid PixmapMut")
        }
    }
}

fn draw(op: &Op, root: tsp::Transform, pixmap: &mut tiny_skia::PixmapMut) {
    match op {
        Op::Fill(fill) => draw_fill(fill, root, pixmap),
        Op::Stroke(stroke) => draw_stroke(stroke, root, pixmap),
        Op::Image(image) => draw_image(image, root, pixmap),
        Op::Raster(raster) => draw_raster(raster, root, pixmap),
        Op::PushLayer(_) | Op::PopLayer => unreachable!("layers are handled by render"),
    }
}

/// Clip a finished layer by its clips, then blend it onto `target` with its
/// opacity and mode — resvg's `render_group` tail.
fn composite(
    layer: Layer,
    mut surface: tiny_skia::Pixmap,
    root: tsp::Transform,
    target: &mut tiny_skia::PixmapMut,
) {
    for clip in &layer.clips {
        if let Some(mask) = clip_mask(clip, root, surface.width(), surface.height()) {
            surface.apply_mask(&mask);
        }
    }
    let paint = tiny_skia::PixmapPaint {
        opacity: layer.opacity,
        blend_mode: blend_mode(layer.blend),
        quality: tiny_skia::FilterQuality::Nearest,
    };
    target.draw_pixmap(
        0,
        0,
        surface.as_ref(),
        &paint,
        tsp::Transform::identity(),
        None,
    );
}

/// One clip as a coverage mask: the union of its shapes.
fn clip_mask(
    clip: &Clip,
    root: tsp::Transform,
    width: u32,
    height: u32,
) -> Option<tiny_skia::Mask> {
    let mut mask = tiny_skia::Mask::new(width, height)?;
    for shape in &clip.shapes {
        let Some(path) = to_path(&shape.path) else {
            continue;
        };
        mask.fill_path(
            &path,
            fill_rule(shape.rule),
            true,
            root.pre_concat(to_ts(shape.transform)),
        );
    }
    Some(mask)
}

fn draw_fill(op: &FillOp, root: tsp::Transform, pixmap: &mut tiny_skia::PixmapMut) {
    let Some(path) = to_path(&op.path) else {
        return;
    };
    let Some(mut paint) = to_paint(&op.paint, op.opacity) else {
        return;
    };
    paint.anti_alias = op.antialias;
    pixmap.fill_path(
        &path,
        &paint,
        fill_rule(op.rule),
        root.pre_concat(to_ts(op.transform)),
        None,
    );
}

fn draw_stroke(op: &StrokeOp, root: tsp::Transform, pixmap: &mut tiny_skia::PixmapMut) {
    let Some(path) = to_path(&op.path) else {
        return;
    };
    let Some(mut paint) = to_paint(&op.paint, op.opacity) else {
        return;
    };
    paint.anti_alias = op.antialias;
    let stroke = tiny_skia::Stroke {
        width: op.stroke.width,
        miter_limit: op.stroke.miter_limit,
        line_cap: match op.stroke.cap {
            LineCap::Butt => tiny_skia::LineCap::Butt,
            LineCap::Round => tiny_skia::LineCap::Round,
            LineCap::Square => tiny_skia::LineCap::Square,
        },
        line_join: match op.stroke.join {
            LineJoin::Miter => tiny_skia::LineJoin::Miter,
            LineJoin::MiterClip => tiny_skia::LineJoin::MiterClip,
            LineJoin::Round => tiny_skia::LineJoin::Round,
            LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
        },
        dash: op
            .stroke
            .dash
            .as_ref()
            .and_then(|dash| tiny_skia::StrokeDash::new(dash.array.clone(), dash.offset)),
    };
    pixmap.stroke_path(
        &path,
        &paint,
        &stroke,
        root.pre_concat(to_ts(op.transform)),
        None,
    );
}

/// Only PNG is decoded here — tiny-skia ships a PNG decoder and nothing else,
/// and the replayer exists for tests, which can choose their fixtures. A real
/// backend decodes with whatever its platform provides.
fn draw_image(op: &ImageOp, root: tsp::Transform, pixmap: &mut tiny_skia::PixmapMut) {
    if op.format != ImageFormat::Png {
        return;
    }
    let Ok(image) = tiny_skia::Pixmap::decode_png(&op.data) else {
        return;
    };
    let quality = if op.smooth {
        tiny_skia::FilterQuality::Bicubic
    } else {
        tiny_skia::FilterQuality::Nearest
    };
    blit(
        image.as_ref(),
        root.pre_concat(to_ts(op.transform)),
        quality,
        pixmap,
    );
}

fn draw_raster(op: &RasterOp, root: tsp::Transform, pixmap: &mut tiny_skia::PixmapMut) {
    let Some(size) = tiny_skia::IntSize::from_wh(op.width, op.height) else {
        return;
    };
    let Some(image) = tiny_skia::Pixmap::from_vec(op.rgba.clone(), size) else {
        return;
    };
    // Pixel rectangle onto canvas rectangle, then canvas onto device.
    let place = tsp::Transform::from_translate(op.rect.x, op.rect.y).pre_scale(
        op.rect.width / op.width as f32,
        op.rect.height / op.height as f32,
    );
    blit(
        image.as_ref(),
        root.pre_concat(place),
        tiny_skia::FilterQuality::Bicubic,
        pixmap,
    );
}

/// Draw `image`'s pixel rectangle under `ts`, resvg's `render_raster` way: a
/// pattern shader filling the rectangle, so any transform works.
fn blit(
    image: tiny_skia::PixmapRef,
    ts: tsp::Transform,
    quality: tiny_skia::FilterQuality,
    pixmap: &mut tiny_skia::PixmapMut,
) {
    let Some(rect) = tsp::Rect::from_xywh(0.0, 0.0, image.width() as f32, image.height() as f32)
    else {
        return;
    };
    let paint = tiny_skia::Paint {
        shader: tiny_skia::Pattern::new(
            image,
            tiny_skia::SpreadMode::Pad,
            quality,
            1.0,
            tsp::Transform::identity(),
        ),
        ..Default::default()
    };
    pixmap.fill_rect(rect, &paint, ts, None);
}

fn to_paint(paint: &Paint, opacity: f32) -> Option<tiny_skia::Paint<'static>> {
    let mut out = tiny_skia::Paint::default();
    match paint {
        Paint::Solid(c) => out.set_color_rgba8(c.red, c.green, c.blue, alpha_u8(opacity)),
        Paint::Linear(g) => {
            out.shader = tiny_skia::LinearGradient::new(
                (g.x1, g.y1).into(),
                (g.x2, g.y2).into(),
                to_stops(&g.stops),
                tiny_skia::SpreadMode::Pad,
                to_ts(g.transform),
            )?;
        }
        Paint::Radial(g) => {
            out.shader = tiny_skia::RadialGradient::new(
                (g.fx, g.fy).into(),
                (g.cx, g.cy).into(),
                g.r,
                to_stops(&g.stops),
                tiny_skia::SpreadMode::Pad,
                to_ts(g.transform),
            )?;
        }
    }
    Some(out)
}

fn to_stops(stops: &[Stop]) -> Vec<tiny_skia::GradientStop> {
    stops
        .iter()
        .map(|stop| {
            tiny_skia::GradientStop::new(
                stop.offset,
                tiny_skia::Color::from_rgba8(
                    stop.color.red,
                    stop.color.green,
                    stop.color.blue,
                    alpha_u8(stop.opacity),
                ),
            )
        })
        .collect()
}

/// `0.0..=1.0` to a byte, rounding as resvg's `NormalizedF32::to_u8` does.
fn alpha_u8(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn to_path(path: &Path) -> Option<tsp::Path> {
    let mut builder = tsp::PathBuilder::new();
    for verb in &path.verbs {
        match *verb {
            PathVerb::MoveTo { x, y } => builder.move_to(x, y),
            PathVerb::LineTo { x, y } => builder.line_to(x, y),
            PathVerb::QuadTo { x1, y1, x, y } => builder.quad_to(x1, y1, x, y),
            PathVerb::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => builder.cubic_to(x1, y1, x2, y2, x, y),
            PathVerb::Close => builder.close(),
        }
    }
    builder.finish()
}

fn to_ts(t: Transform) -> tsp::Transform {
    tsp::Transform::from_row(t.sx, t.ky, t.kx, t.sy, t.tx, t.ty)
}

fn fill_rule(rule: FillRule) -> tiny_skia::FillRule {
    match rule {
        FillRule::NonZero => tiny_skia::FillRule::Winding,
        FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
    }
}

fn blend_mode(mode: BlendMode) -> tiny_skia::BlendMode {
    match mode {
        BlendMode::Normal => tiny_skia::BlendMode::SourceOver,
        BlendMode::Multiply => tiny_skia::BlendMode::Multiply,
        BlendMode::Screen => tiny_skia::BlendMode::Screen,
        BlendMode::Overlay => tiny_skia::BlendMode::Overlay,
        BlendMode::Darken => tiny_skia::BlendMode::Darken,
        BlendMode::Lighten => tiny_skia::BlendMode::Lighten,
        BlendMode::ColorDodge => tiny_skia::BlendMode::ColorDodge,
        BlendMode::ColorBurn => tiny_skia::BlendMode::ColorBurn,
        BlendMode::HardLight => tiny_skia::BlendMode::HardLight,
        BlendMode::SoftLight => tiny_skia::BlendMode::SoftLight,
        BlendMode::Difference => tiny_skia::BlendMode::Difference,
        BlendMode::Exclusion => tiny_skia::BlendMode::Exclusion,
        BlendMode::Hue => tiny_skia::BlendMode::Hue,
        BlendMode::Saturation => tiny_skia::BlendMode::Saturation,
        BlendMode::Color => tiny_skia::BlendMode::Color,
        BlendMode::Luminosity => tiny_skia::BlendMode::Luminosity,
    }
}
