//! The walk: usvg tree in, [`DisplayList`] out.
//!
//! This mirrors resvg's own `render.rs` node for node — the same decision about
//! when a group becomes a layer, the same order of fill and stroke, the same
//! skipping of a degenerate fill — because the test suite holds the two to the
//! same pixels. Where resvg paints, this emits; where this cannot emit a vector
//! (see [`raster`](crate::raster)), it asks resvg to paint that one node.

use usvg::tiny_skia_path as tsp;

use crate::{
    BlendMode, Clip, ClipShape, Color, Dash, DisplayList, FillOp, FillRule, ImageFormat, ImageOp,
    Layer, LineCap, LineJoin, LinearGradient, Op, Options, Paint, Path, PathVerb, RadialGradient,
    Stop, StrokeOp, StrokeStyle, Transform,
};

/// Flatten `tree` into a display list.
///
/// Never fails: a subtree the flattener cannot express is rasterized by resvg
/// at [`Options::raster_scale`], and a node resvg cannot render either (a zero-
/// size layer, say) is dropped, which is what resvg would do with it.
pub fn flatten(tree: &usvg::Tree, options: &Options) -> DisplayList {
    let size = tree.size();
    let mut walker = Walker {
        ops: Vec::new(),
        scale: options.raster_scale,
        canvas: tsp::Rect::from_xywh(0.0, 0.0, size.width(), size.height())
            .expect("a usvg tree has a non-zero size"),
    };
    // resvg renders the root's *children* under the caller's transform and
    // never applies the root group's own; same here.
    walker.children(tree.root(), tsp::Transform::identity());
    DisplayList {
        width: tree.size().width(),
        height: tree.size().height(),
        ops: walker.ops,
    }
}

struct Walker {
    ops: Vec<Op>,
    scale: f32,
    /// The visible area, in canvas units — what a raster fallback is clipped to.
    canvas: tsp::Rect,
}

impl Walker {
    /// Emit every child of `group`.
    ///
    /// `ts` maps the children's local space to the canvas — it already includes
    /// `group`'s own transform, and for an SVG nested through `<image>` the
    /// placement of that image too. It is accumulated here from the relative
    /// transforms, as resvg does, and never read from usvg's absolute ones (see
    /// [`raster`](crate::raster) for why).
    fn children(&mut self, group: &usvg::Group, ts: tsp::Transform) {
        for node in group.children() {
            self.node(node, ts);
        }
    }

    fn node(&mut self, node: &usvg::Node, ts: tsp::Transform) {
        match node {
            usvg::Node::Group(group) => self.group(node, group, ts),
            usvg::Node::Path(path) => self.path(node, path, ts),
            usvg::Node::Image(image) => self.image(image, ts),
            // usvg has laid the text out into glyph paths already.
            usvg::Node::Text(text) => self.group(node, text.flattened(), ts),
        }
    }

    fn group(&mut self, node: &usvg::Node, group: &usvg::Group, parent: tsp::Transform) {
        let ts = parent.pre_concat(group.transform());

        // A filter or a mask has no vector form here; the whole group is one
        // picture from resvg.
        if !group.filters().is_empty() || group.mask().is_some() {
            return self.raster(node, parent);
        }

        // resvg composites through an offscreen layer only when something
        // demands it; otherwise the children paint straight through, and a
        // layer here would be a transparency group the backend pays for
        // without effect.
        if !group.should_isolate() {
            return self.children(group, ts);
        }

        let clips = match group.clip_path() {
            Some(clip) => match clip_chain(clip, ts) {
                Some(clips) => clips,
                None => return self.raster(node, parent),
            },
            None => Vec::new(),
        };

        self.ops.push(Op::PushLayer(Layer {
            opacity: group.opacity().get(),
            blend: blend_mode(group.blend_mode()),
            clips,
        }));
        self.children(group, ts);
        self.ops.push(Op::PopLayer);
    }

    fn path(&mut self, node: &usvg::Node, path: &usvg::Path, ts: tsp::Transform) {
        if !path.is_visible() {
            return;
        }

        // Decide expressibility for both paints before emitting either, so a
        // path never ends up half vector, half raster.
        let fill_paint = path
            .fill()
            .map(|fill| paint(fill.paint(), fill.opacity().get()));
        let stroke_paint = path
            .stroke()
            .map(|stroke| paint(stroke.paint(), stroke.opacity().get()));
        if matches!(fill_paint, Some(None)) || matches!(stroke_paint, Some(None)) {
            return self.raster(node, ts);
        }

        let outline = convert_path(path.data());
        let antialias = path.rendering_mode().use_shape_antialiasing();

        // resvg: "Horizontal and vertical lines cannot be filled. Skip."
        let bounds = path.data().bounds();
        let degenerate = bounds.width() == 0.0 || bounds.height() == 0.0;
        let fill = path
            .fill()
            .zip(fill_paint.flatten())
            .filter(|_| !degenerate)
            .map(|(fill, paint)| {
                Op::Fill(FillOp {
                    path: outline.clone(),
                    transform: transform(ts),
                    paint,
                    opacity: fill.opacity().get(),
                    rule: fill_rule(fill.rule()),
                    antialias,
                })
            });
        let stroke = path
            .stroke()
            .zip(stroke_paint.flatten())
            .map(|(stroke, paint)| {
                Op::Stroke(StrokeOp {
                    path: outline.clone(),
                    transform: transform(ts),
                    paint,
                    opacity: stroke.opacity().get(),
                    stroke: stroke_style(stroke),
                    antialias,
                })
            });

        let (first, second) = match path.paint_order() {
            usvg::PaintOrder::FillAndStroke => (fill, stroke),
            usvg::PaintOrder::StrokeAndFill => (stroke, fill),
        };
        self.ops.extend(first);
        self.ops.extend(second);
    }

    fn image(&mut self, image: &usvg::Image, ts: tsp::Transform) {
        if !image.is_visible() {
            return;
        }
        let (data, format) = match image.kind() {
            // A nested document is vectors too, drawn under the image's
            // placement — which is what `ts` already is.
            usvg::ImageKind::SVG(tree) => return self.children(tree.root(), ts),
            usvg::ImageKind::PNG(data) => (data, ImageFormat::Png),
            usvg::ImageKind::JPEG(data) => (data, ImageFormat::Jpeg),
            usvg::ImageKind::GIF(data) => (data, ImageFormat::Gif),
            usvg::ImageKind::WEBP(data) => (data, ImageFormat::Webp),
        };
        let smooth = !matches!(
            image.rendering_mode(),
            usvg::ImageRendering::OptimizeSpeed
                | usvg::ImageRendering::CrispEdges
                | usvg::ImageRendering::Pixelated
        );
        self.ops.push(Op::Image(ImageOp {
            data: data.to_vec(),
            format,
            width: image.size().width(),
            height: image.size().height(),
            transform: transform(ts),
            smooth,
        }));
    }

    /// The escape hatch: `node` as resvg paints it, at this list's scale.
    /// `parent` is the transform in force where `node` sits — for a path its
    /// `ts`, for a group the transform *before* the group's own.
    fn raster(&mut self, node: &usvg::Node, parent: tsp::Transform) {
        if let Some(op) = crate::raster::render(node, parent, self.canvas, self.scale) {
            self.ops.push(Op::Raster(op));
        }
    }
}

/// A `clipPath` and the chain of `clip-path`s it carries, as clips to
/// intersect; `None` if any link holds something other than plain shapes.
///
/// `ts` is the clipped group's local-to-canvas transform, and each link's own
/// transform composes onto it — what resvg's `clip::apply` does with
/// `transform.pre_concat(clip.transform())`.
fn clip_chain(clip: &usvg::ClipPath, ts: tsp::Transform) -> Option<Vec<Clip>> {
    let mut clips = Vec::new();
    let mut link = Some(clip);
    while let Some(clip) = link {
        let mut shapes = Vec::new();
        clip_shapes(clip.root(), ts.pre_concat(clip.transform()), &mut shapes)?;
        clips.push(Clip { shapes });
        link = clip.clip_path();
    }
    Some(clips)
}

/// Collect the shapes under a `clipPath`, as resvg's `clip::draw_children`
/// would paint them: paths by their fill rule, text by its glyphs, groups by
/// transform alone — a group's opacity, mask, and filters do not apply inside a
/// clip. A group that carries its own `clip-path` would need a boolean
/// operation this list has no vocabulary for, so it declines and the caller
/// rasterizes; images are ignored, as resvg ignores them.
fn clip_shapes(group: &usvg::Group, ts: tsp::Transform, out: &mut Vec<ClipShape>) -> Option<()> {
    for node in group.children() {
        match node {
            usvg::Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }
                // resvg fills a clip child through `fill_path`, which does
                // nothing for a path without a fill or with no area.
                let Some(fill) = path.fill() else { continue };
                let bounds = path.data().bounds();
                if bounds.width() == 0.0 || bounds.height() == 0.0 {
                    continue;
                }
                out.push(ClipShape {
                    path: convert_path(path.data()),
                    transform: transform(ts),
                    rule: fill_rule(fill.rule()),
                });
            }
            // resvg draws the flattened text's children under the clip's
            // transform directly, without the flattened group's own.
            usvg::Node::Text(text) => clip_shapes(text.flattened(), ts, out)?,
            usvg::Node::Group(inner) => {
                if inner.clip_path().is_some() {
                    return None;
                }
                clip_shapes(inner, ts.pre_concat(inner.transform()), out)?;
            }
            usvg::Node::Image(_) => {}
        }
    }
    Some(())
}

/// A usvg paint as a [`Paint`], or `None` for one this list cannot carry — a
/// pattern, or a gradient that repeats or reflects past its stops.
fn paint(paint: &usvg::Paint, opacity: f32) -> Option<Paint> {
    Some(match paint {
        usvg::Paint::Color(c) => Paint::Solid(color(*c)),
        usvg::Paint::LinearGradient(g) => {
            if g.spread_method() != usvg::SpreadMethod::Pad {
                return None;
            }
            Paint::Linear(LinearGradient {
                x1: g.x1(),
                y1: g.y1(),
                x2: g.x2(),
                y2: g.y2(),
                transform: transform(g.transform()),
                stops: stops(g.stops(), opacity),
            })
        }
        usvg::Paint::RadialGradient(g) => {
            if g.spread_method() != usvg::SpreadMethod::Pad {
                return None;
            }
            Paint::Radial(RadialGradient {
                cx: g.cx(),
                cy: g.cy(),
                r: g.r().get(),
                fx: g.fx(),
                fy: g.fy(),
                transform: transform(g.transform()),
                stops: stops(g.stops(), opacity),
            })
        }
        usvg::Paint::Pattern(_) => return None,
    })
}

/// Stops with the op's opacity folded in, as resvg's `convert_base_gradient`
/// folds it into each stop's alpha.
fn stops(stops: &[usvg::Stop], opacity: f32) -> Vec<Stop> {
    stops
        .iter()
        .map(|stop| Stop {
            offset: stop.offset().get(),
            color: color(stop.color()),
            opacity: stop.opacity().get() * opacity,
        })
        .collect()
}

fn convert_path(path: &tsp::Path) -> Path {
    let verbs = path
        .segments()
        .map(|segment| match segment {
            tsp::PathSegment::MoveTo(p) => PathVerb::MoveTo { x: p.x, y: p.y },
            tsp::PathSegment::LineTo(p) => PathVerb::LineTo { x: p.x, y: p.y },
            tsp::PathSegment::QuadTo(c, p) => PathVerb::QuadTo {
                x1: c.x,
                y1: c.y,
                x: p.x,
                y: p.y,
            },
            tsp::PathSegment::CubicTo(c1, c2, p) => PathVerb::CubicTo {
                x1: c1.x,
                y1: c1.y,
                x2: c2.x,
                y2: c2.y,
                x: p.x,
                y: p.y,
            },
            tsp::PathSegment::Close => PathVerb::Close,
        })
        .collect();
    Path { verbs }
}

fn stroke_style(stroke: &usvg::Stroke) -> StrokeStyle {
    StrokeStyle {
        width: stroke.width().get(),
        cap: match stroke.linecap() {
            usvg::LineCap::Butt => LineCap::Butt,
            usvg::LineCap::Round => LineCap::Round,
            usvg::LineCap::Square => LineCap::Square,
        },
        join: match stroke.linejoin() {
            usvg::LineJoin::Miter => LineJoin::Miter,
            usvg::LineJoin::MiterClip => LineJoin::MiterClip,
            usvg::LineJoin::Round => LineJoin::Round,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
        },
        miter_limit: stroke.miterlimit().get(),
        dash: stroke.dasharray().map(|array| Dash {
            array: array.to_vec(),
            offset: stroke.dashoffset(),
        }),
    }
}

pub(crate) fn transform(ts: tsp::Transform) -> Transform {
    Transform {
        sx: ts.sx,
        kx: ts.kx,
        ky: ts.ky,
        sy: ts.sy,
        tx: ts.tx,
        ty: ts.ty,
    }
}

fn color(c: usvg::Color) -> Color {
    Color {
        red: c.red,
        green: c.green,
        blue: c.blue,
    }
}

fn fill_rule(rule: usvg::FillRule) -> FillRule {
    match rule {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn blend_mode(mode: usvg::BlendMode) -> BlendMode {
    match mode {
        usvg::BlendMode::Normal => BlendMode::Normal,
        usvg::BlendMode::Multiply => BlendMode::Multiply,
        usvg::BlendMode::Screen => BlendMode::Screen,
        usvg::BlendMode::Overlay => BlendMode::Overlay,
        usvg::BlendMode::Darken => BlendMode::Darken,
        usvg::BlendMode::Lighten => BlendMode::Lighten,
        usvg::BlendMode::ColorDodge => BlendMode::ColorDodge,
        usvg::BlendMode::ColorBurn => BlendMode::ColorBurn,
        usvg::BlendMode::HardLight => BlendMode::HardLight,
        usvg::BlendMode::SoftLight => BlendMode::SoftLight,
        usvg::BlendMode::Difference => BlendMode::Difference,
        usvg::BlendMode::Exclusion => BlendMode::Exclusion,
        usvg::BlendMode::Hue => BlendMode::Hue,
        usvg::BlendMode::Saturation => BlendMode::Saturation,
        usvg::BlendMode::Color => BlendMode::Color,
        usvg::BlendMode::Luminosity => BlendMode::Luminosity,
    }
}
