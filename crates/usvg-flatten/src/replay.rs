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
    BlendMode, Clip, DisplayList, FillOp, FillRule, ImageOp, Layer, LineCap, LineJoin, Op, Paint,
    Path, PathVerb, RasterOp, Stop, StrokeOp, Transform,
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

/// Decoded with the same crates resvg uses — the same pixels, so an image
/// test compares the placement and nothing else. A real backend decodes with
/// whatever its platform provides.
fn draw_image(op: &ImageOp, root: tsp::Transform, pixmap: &mut tiny_skia::PixmapMut) {
    let Some(image) = decode::image(op.format, &op.data) else {
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
    let ts = root.pre_concat(place);
    // The flattener snapped the rectangle to the pixel grid at its scale, so
    // replaying at that scale is a whole-pixel copy — do exactly that, rather
    // than resample a picture that already has the right pixels.
    let unit = |v: f32| (v - 1.0).abs() < 1e-3;
    let zero = |v: f32| v.abs() < 1e-3;
    if unit(ts.sx) && unit(ts.sy) && zero(ts.kx) && zero(ts.ky) {
        pixmap.draw_pixmap(
            ts.tx.round() as i32,
            ts.ty.round() as i32,
            image.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tsp::Transform::identity(),
            None,
        );
        return;
    }
    blit(
        image.as_ref(),
        ts,
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

/// The raster decoders, as resvg's `image.rs` has them: PNG through tiny-skia,
/// JPEG through zune, GIF's first frame, WebP's first frame. Output is
/// premultiplied RGBA, which is what a `Pixmap` holds.
mod decode {
    use resvg::tiny_skia;

    use crate::ImageFormat;

    pub fn image(format: ImageFormat, data: &[u8]) -> Option<tiny_skia::Pixmap> {
        match format {
            ImageFormat::Png => tiny_skia::Pixmap::decode_png(data).ok(),
            ImageFormat::Jpeg => jpeg(data),
            ImageFormat::Gif => gif(data),
            ImageFormat::Webp => webp(data),
        }
    }

    fn jpeg(data: &[u8]) -> Option<tiny_skia::Pixmap> {
        use zune_jpeg::zune_core::colorspace::ColorSpace;
        use zune_jpeg::zune_core::options::DecoderOptions;
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut decoder =
            zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(data), options);
        decoder.decode_headers().ok()?;
        if decoder.output_colorspace()? != ColorSpace::RGBA {
            return None;
        }
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let size = tiny_skia::IntSize::from_wh(u32::from(info.width), u32::from(info.height))?;
        tiny_skia::Pixmap::from_vec(pixels, size)
    }

    fn gif(data: &[u8]) -> Option<tiny_skia::Pixmap> {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(data).ok()?;
        let frame = decoder.read_next_frame().ok()??;
        let mut pixmap = tiny_skia::Pixmap::new(u32::from(frame.width), u32::from(frame.height))?;
        premultiply_into(&frame.buffer, pixmap.data_mut());
        Some(pixmap)
    }

    fn webp(data: &[u8]) -> Option<tiny_skia::Pixmap> {
        let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(data)).ok()?;
        let mut pixels = vec![0; decoder.output_buffer_size()?];
        decoder.read_image(&mut pixels).ok()?;
        let (width, height) = decoder.dimensions();
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        if decoder.has_alpha() {
            premultiply_into(&pixels, pixmap.data_mut());
        } else {
            let (src, _) = pixels.as_chunks::<3>();
            let (dst, _) = pixmap.data_mut().as_chunks_mut::<4>();
            for (s, d) in src.iter().zip(dst) {
                d[..3].copy_from_slice(s);
                d[3] = 255;
            }
        }
        Some(pixmap)
    }

    /// Straight RGBA in, premultiplied RGBA out, rounding as resvg does.
    fn premultiply_into(src: &[u8], dst: &mut [u8]) {
        let (src, _) = src.as_chunks::<4>();
        let (dst, _) = dst.as_chunks_mut::<4>();
        for (s, d) in src.iter().zip(dst) {
            let a = f64::from(s[3]) / 255.0;
            d[0] = (f64::from(s[0]) * a + 0.5) as u8;
            d[1] = (f64::from(s[1]) * a + 0.5) as u8;
            d[2] = (f64::from(s[2]) * a + 0.5) as u8;
            d[3] = s[3];
        }
    }
}
