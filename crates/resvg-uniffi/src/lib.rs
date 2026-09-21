//! resvg-uniffi — usvg's parse and usvg-flatten's display list, across UniFFI.
//!
//! Swift gets one object, [`SvgDocument`], and one method that matters:
//! [`SvgDocument::display_list`]. The document is parsed once and kept; the
//! list is produced per draw, because its one scale-dependent part — the
//! rasterized fallback for filters and the like — has to be redone when the
//! picture is drawn at a different size. Everything vector in it is the same
//! every time.
//!
//! The record and enum types below mirror `usvg_flatten`'s exactly, with the
//! same names and the same documentation, so that a reader of the Swift side
//! can look a field up in either place. They are separate types rather than
//! derives on the originals so the pure crate carries no FFI dependency and the
//! wire format is versioned here, where the consumer is.

use std::sync::{Arc, OnceLock};

uniffi::setup_scaffolding!();

/// Why [`SvgDocument::new`] refused the bytes.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SvgError {
    /// usvg could not parse the document; `message` is its reason.
    #[error("{message}")]
    Parse { message: String },
}

/// How to parse. `Default` is what the SVG spec calls for, with system fonts.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ParseOptions {
    /// Load the system's fonts so `<text>` in a face the document doesn't embed
    /// still lays out. Loading is a directory walk done once per process and
    /// shared by every document after; off, text in an unknown family falls
    /// back to nothing and is dropped, which is what a sandboxed host with no
    /// font access wants.
    #[uniffi(default = true)]
    pub load_system_fonts: bool,
    /// The family for text that names none. usvg's default is Times New Roman.
    #[uniffi(default = "Times New Roman")]
    pub font_family: String,
    /// The size for text that sets none, in user units.
    #[uniffi(default = 12.0)]
    pub font_size: f32,
    /// Dots per inch, for `in`/`cm`/`mm` lengths. 96 is CSS's.
    #[uniffi(default = 96.0)]
    pub dpi: f32,
    /// Directory that relative `href`s (in `<image>`) resolve against; `None`
    /// leaves them unresolved.
    #[uniffi(default = None)]
    pub resources_dir: Option<String>,
}

/// A parsed SVG. Parsing is the expensive part — the XML, the CSS, the text
/// layout — so a host keeps this and asks it for a display list per draw.
#[derive(uniffi::Object)]
pub struct SvgDocument {
    tree: usvg::Tree,
}

#[uniffi::export]
impl SvgDocument {
    /// Parse `data` — plain or gzip-compressed SVG.
    #[uniffi::constructor]
    pub fn new(data: Vec<u8>, options: ParseOptions) -> Result<Arc<Self>, SvgError> {
        let mut opt = usvg::Options {
            font_family: options.font_family,
            font_size: options.font_size,
            dpi: options.dpi,
            resources_dir: options.resources_dir.map(Into::into),
            ..usvg::Options::default()
        };
        if options.load_system_fonts {
            opt.fontdb = system_fonts();
        }
        let tree = usvg::Tree::from_data(&data, &opt).map_err(|e| SvgError::Parse {
            message: e.to_string(),
        })?;
        Ok(Arc::new(Self { tree }))
    }

    /// The document's width in user units — its `width` as usvg resolved it.
    pub fn width(&self) -> f32 {
        self.tree.size().width()
    }

    /// The document's height in user units.
    pub fn height(&self) -> f32 {
        self.tree.size().height()
    }

    /// Whether the document has `<text>` — a host may want to know that fonts
    /// mattered to what it is showing.
    pub fn has_text(&self) -> bool {
        self.tree.has_text_nodes()
    }

    /// The picture as a display list.
    ///
    /// `raster_scale` is device pixels per user unit for anything that has to be
    /// rasterized (filters, masks, patterns): a 100-unit document drawn 300
    /// points wide on a 2× display passes `6.0`. Vector ops are the same at
    /// every scale.
    pub fn display_list(&self, raster_scale: f32) -> DisplayList {
        usvg_flatten::flatten(&self.tree, &usvg_flatten::Options { raster_scale }).into()
    }
}

/// The system font database, loaded once per process. usvg shares it by `Arc`,
/// so every document after the first pays nothing.
fn system_fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            // fontdb knows macOS's font directories and not iOS's: under
            // `target_os = "ios"` it takes its Linux branch and scans
            // `/usr/share/fonts`, which is empty, so every `<text>` was
            // dropped. iOS keeps its faces under /System/Library/Fonts, in
            // subdirectories an app may read; the walk is recursive. (The
            // simulator sees the host Mac's directory of the same name.)
            #[cfg(target_os = "ios")]
            db.load_fonts_dir("/System/Library/Fonts");
            Arc::new(db)
        })
        .clone()
}

// ---------------------------------------------------------------------------
// The display list, as UniFFI records. See `usvg_flatten` for the semantics.
// ---------------------------------------------------------------------------

/// The flattened picture.
#[derive(Clone, Debug, uniffi::Record)]
pub struct DisplayList {
    /// Canvas width in user units.
    pub width: f32,
    /// Canvas height in user units.
    pub height: f32,
    /// The operations, in paint order. Every `PushLayer` has a matching
    /// `PopLayer` later in the list; they nest.
    pub ops: Vec<Op>,
}

/// One drawing operation.
#[derive(Clone, Debug, uniffi::Enum)]
pub enum Op {
    /// Begin a compositing layer, ended by the matching `PopLayer`.
    PushLayer { layer: Layer },
    /// End the innermost layer.
    PopLayer,
    /// Fill a path.
    Fill { op: FillOp },
    /// Stroke a path.
    Stroke { op: StrokeOp },
    /// Draw an encoded raster image.
    Image { op: ImageOp },
    /// Draw pixels resvg produced for a subtree with no vector form here.
    Raster { op: RasterOp },
}

/// A compositing layer.
#[derive(Clone, Debug, uniffi::Record)]
pub struct Layer {
    /// Group opacity, `0.0..=1.0`.
    pub opacity: f32,
    /// How the layer composites onto what is beneath it.
    pub blend: BlendMode,
    /// Clips to apply, intersected in order. Each is a union of shapes.
    pub clips: Vec<Clip>,
}

/// One `clipPath`: the union of its shapes.
#[derive(Clone, Debug, uniffi::Record)]
pub struct Clip {
    pub shapes: Vec<ClipShape>,
}

/// One shape of a clip.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClipShape {
    pub path: Path,
    /// Local to canvas.
    pub transform: Transform,
    pub rule: FillRule,
}

/// Fill a path.
#[derive(Clone, Debug, uniffi::Record)]
pub struct FillOp {
    pub path: Path,
    /// Local to canvas.
    pub transform: Transform,
    pub paint: Paint,
    /// `fill-opacity`, `0.0..=1.0`.
    pub opacity: f32,
    pub rule: FillRule,
    /// Whether edges should be anti-aliased.
    pub antialias: bool,
}

/// Stroke a path.
#[derive(Clone, Debug, uniffi::Record)]
pub struct StrokeOp {
    pub path: Path,
    /// Local to canvas. Apply before stroking, so the width is in local units.
    pub transform: Transform,
    pub paint: Paint,
    /// `stroke-opacity`, `0.0..=1.0`.
    pub opacity: f32,
    pub stroke: StrokeStyle,
    /// Whether edges should be anti-aliased.
    pub antialias: bool,
}

/// The pen for a stroke, in the path's local units.
#[derive(Clone, Debug, uniffi::Record)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dash: Option<Dash>,
}

/// A dash pattern.
#[derive(Clone, Debug, uniffi::Record)]
pub struct Dash {
    /// On/off lengths. Even in length and non-empty.
    pub array: Vec<f32>,
    pub offset: f32,
}

/// An encoded raster `<image>`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageOp {
    /// The file bytes, still encoded.
    pub data: Vec<u8>,
    pub format: ImageFormat,
    /// Pixel width of the decoded image.
    pub width: f32,
    /// Pixel height of the decoded image.
    pub height: f32,
    /// Local to canvas, where local is the pixel rectangle `(0, 0, width, height)`.
    pub transform: Transform,
    /// Whether to interpolate when scaling, rather than pick nearest pixels.
    pub smooth: bool,
}

/// The encoding of an image's bytes.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

/// Pixels for a subtree that is not expressed as vectors.
#[derive(Clone, Debug, uniffi::Record)]
pub struct RasterOp {
    /// Premultiplied RGBA, 8 bits per channel, row-major, `width * height * 4`.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The canvas rectangle the pixels cover, in user units.
    pub rect: Rect,
}

/// What a fill or stroke is painted with.
#[derive(Clone, Debug, uniffi::Enum)]
pub enum Paint {
    Solid { color: Color },
    Linear { gradient: LinearGradient },
    Radial { gradient: RadialGradient },
}

/// An opaque sRGB colour. Opacity travels separately.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// A linear gradient in the path's local space, after `transform`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct LinearGradient {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub transform: Transform,
    /// At least two, offsets ascending in `0.0..=1.0`.
    pub stops: Vec<Stop>,
}

/// A radial gradient: from the focal point `(fx, fy)` at radius 0 to the
/// circle centred `(cx, cy)` at radius `r`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct RadialGradient {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    pub fx: f32,
    pub fy: f32,
    pub transform: Transform,
    /// At least two, offsets ascending in `0.0..=1.0`.
    pub stops: Vec<Stop>,
}

/// One gradient stop.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Stop {
    /// Position along the gradient, `0.0..=1.0`.
    pub offset: f32,
    pub color: Color,
    /// `stop-opacity`, already multiplied by the op's opacity.
    pub opacity: f32,
}

/// A path as verbs, in its local space.
#[derive(Clone, Debug, uniffi::Record)]
pub struct Path {
    pub verbs: Vec<PathVerb>,
}

/// One path verb. Coordinates are absolute in the path's local space.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum PathVerb {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    QuadTo {
        x1: f32,
        y1: f32,
        x: f32,
        y: f32,
    },
    CubicTo {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x: f32,
        y: f32,
    },
    Close,
}

/// A 2D affine transform: `x' = sx·x + kx·y + tx`, `y' = ky·x + sy·y + ty`.
/// `CGAffineTransform(a: sx, b: ky, c: kx, d: sy, tx, ty)` is the same matrix.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Transform {
    pub sx: f32,
    pub kx: f32,
    pub ky: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

/// An axis-aligned rectangle in canvas units.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How a path's interior is decided.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// `stroke-linecap`.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// `stroke-linejoin`. A backend without `MiterClip` falls back to `Miter`.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum LineJoin {
    Miter,
    MiterClip,
    Round,
    Bevel,
}

/// `mix-blend-mode`, the sixteen CSS compositing modes.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

// ---------------------------------------------------------------------------
// usvg_flatten → wire
// ---------------------------------------------------------------------------

use usvg_flatten as f;

impl From<f::DisplayList> for DisplayList {
    fn from(l: f::DisplayList) -> Self {
        Self {
            width: l.width,
            height: l.height,
            ops: l.ops.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<f::Op> for Op {
    fn from(op: f::Op) -> Self {
        match op {
            f::Op::PushLayer(layer) => Op::PushLayer {
                layer: layer.into(),
            },
            f::Op::PopLayer => Op::PopLayer,
            f::Op::Fill(op) => Op::Fill { op: op.into() },
            f::Op::Stroke(op) => Op::Stroke { op: op.into() },
            f::Op::Image(op) => Op::Image { op: op.into() },
            f::Op::Raster(op) => Op::Raster { op: op.into() },
        }
    }
}

impl From<f::Layer> for Layer {
    fn from(l: f::Layer) -> Self {
        Self {
            opacity: l.opacity,
            blend: l.blend.into(),
            clips: l.clips.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<f::Clip> for Clip {
    fn from(c: f::Clip) -> Self {
        Self {
            shapes: c.shapes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<f::ClipShape> for ClipShape {
    fn from(s: f::ClipShape) -> Self {
        Self {
            path: s.path.into(),
            transform: s.transform.into(),
            rule: s.rule.into(),
        }
    }
}

impl From<f::FillOp> for FillOp {
    fn from(o: f::FillOp) -> Self {
        Self {
            path: o.path.into(),
            transform: o.transform.into(),
            paint: o.paint.into(),
            opacity: o.opacity,
            rule: o.rule.into(),
            antialias: o.antialias,
        }
    }
}

impl From<f::StrokeOp> for StrokeOp {
    fn from(o: f::StrokeOp) -> Self {
        Self {
            path: o.path.into(),
            transform: o.transform.into(),
            paint: o.paint.into(),
            opacity: o.opacity,
            stroke: o.stroke.into(),
            antialias: o.antialias,
        }
    }
}

impl From<f::StrokeStyle> for StrokeStyle {
    fn from(s: f::StrokeStyle) -> Self {
        Self {
            width: s.width,
            cap: s.cap.into(),
            join: s.join.into(),
            miter_limit: s.miter_limit,
            dash: s.dash.map(|d| Dash {
                array: d.array,
                offset: d.offset,
            }),
        }
    }
}

impl From<f::ImageOp> for ImageOp {
    fn from(o: f::ImageOp) -> Self {
        Self {
            data: o.data,
            format: match o.format {
                f::ImageFormat::Png => ImageFormat::Png,
                f::ImageFormat::Jpeg => ImageFormat::Jpeg,
                f::ImageFormat::Gif => ImageFormat::Gif,
                f::ImageFormat::Webp => ImageFormat::Webp,
            },
            width: o.width,
            height: o.height,
            transform: o.transform.into(),
            smooth: o.smooth,
        }
    }
}

impl From<f::RasterOp> for RasterOp {
    fn from(o: f::RasterOp) -> Self {
        Self {
            rgba: o.rgba,
            width: o.width,
            height: o.height,
            rect: Rect {
                x: o.rect.x,
                y: o.rect.y,
                width: o.rect.width,
                height: o.rect.height,
            },
        }
    }
}

impl From<f::Paint> for Paint {
    fn from(p: f::Paint) -> Self {
        match p {
            f::Paint::Solid(c) => Paint::Solid { color: c.into() },
            f::Paint::Linear(g) => Paint::Linear {
                gradient: LinearGradient {
                    x1: g.x1,
                    y1: g.y1,
                    x2: g.x2,
                    y2: g.y2,
                    transform: g.transform.into(),
                    stops: g.stops.into_iter().map(Into::into).collect(),
                },
            },
            f::Paint::Radial(g) => Paint::Radial {
                gradient: RadialGradient {
                    cx: g.cx,
                    cy: g.cy,
                    r: g.r,
                    fx: g.fx,
                    fy: g.fy,
                    transform: g.transform.into(),
                    stops: g.stops.into_iter().map(Into::into).collect(),
                },
            },
        }
    }
}

impl From<f::Color> for Color {
    fn from(c: f::Color) -> Self {
        Self {
            red: c.red,
            green: c.green,
            blue: c.blue,
        }
    }
}

impl From<f::Stop> for Stop {
    fn from(s: f::Stop) -> Self {
        Self {
            offset: s.offset,
            color: s.color.into(),
            opacity: s.opacity,
        }
    }
}

impl From<f::Path> for Path {
    fn from(p: f::Path) -> Self {
        Self {
            verbs: p
                .verbs
                .into_iter()
                .map(|v| match v {
                    f::PathVerb::MoveTo { x, y } => PathVerb::MoveTo { x, y },
                    f::PathVerb::LineTo { x, y } => PathVerb::LineTo { x, y },
                    f::PathVerb::QuadTo { x1, y1, x, y } => PathVerb::QuadTo { x1, y1, x, y },
                    f::PathVerb::CubicTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    } => PathVerb::CubicTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    },
                    f::PathVerb::Close => PathVerb::Close,
                })
                .collect(),
        }
    }
}

impl From<f::Transform> for Transform {
    fn from(t: f::Transform) -> Self {
        Self {
            sx: t.sx,
            kx: t.kx,
            ky: t.ky,
            sy: t.sy,
            tx: t.tx,
            ty: t.ty,
        }
    }
}

impl From<f::FillRule> for FillRule {
    fn from(r: f::FillRule) -> Self {
        match r {
            f::FillRule::NonZero => FillRule::NonZero,
            f::FillRule::EvenOdd => FillRule::EvenOdd,
        }
    }
}

impl From<f::LineCap> for LineCap {
    fn from(c: f::LineCap) -> Self {
        match c {
            f::LineCap::Butt => LineCap::Butt,
            f::LineCap::Round => LineCap::Round,
            f::LineCap::Square => LineCap::Square,
        }
    }
}

impl From<f::LineJoin> for LineJoin {
    fn from(j: f::LineJoin) -> Self {
        match j {
            f::LineJoin::Miter => LineJoin::Miter,
            f::LineJoin::MiterClip => LineJoin::MiterClip,
            f::LineJoin::Round => LineJoin::Round,
            f::LineJoin::Bevel => LineJoin::Bevel,
        }
    }
}

impl From<f::BlendMode> for BlendMode {
    fn from(m: f::BlendMode) -> Self {
        match m {
            f::BlendMode::Normal => BlendMode::Normal,
            f::BlendMode::Multiply => BlendMode::Multiply,
            f::BlendMode::Screen => BlendMode::Screen,
            f::BlendMode::Overlay => BlendMode::Overlay,
            f::BlendMode::Darken => BlendMode::Darken,
            f::BlendMode::Lighten => BlendMode::Lighten,
            f::BlendMode::ColorDodge => BlendMode::ColorDodge,
            f::BlendMode::ColorBurn => BlendMode::ColorBurn,
            f::BlendMode::HardLight => BlendMode::HardLight,
            f::BlendMode::SoftLight => BlendMode::SoftLight,
            f::BlendMode::Difference => BlendMode::Difference,
            f::BlendMode::Exclusion => BlendMode::Exclusion,
            f::BlendMode::Hue => BlendMode::Hue,
            f::BlendMode::Saturation => BlendMode::Saturation,
            f::BlendMode::Color => BlendMode::Color,
            f::BlendMode::Luminosity => BlendMode::Luminosity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000"/></svg>"##;

    fn options() -> ParseOptions {
        ParseOptions {
            load_system_fonts: false,
            font_family: "Times New Roman".into(),
            font_size: 12.0,
            dpi: 96.0,
            resources_dir: None,
        }
    }

    #[test]
    fn a_rect_is_one_fill() {
        let doc = SvgDocument::new(RECT.to_vec(), options()).expect("parses");
        assert_eq!((doc.width(), doc.height()), (20.0, 10.0));
        let list = doc.display_list(1.0);
        assert_eq!(list.ops.len(), 1);
        let Op::Fill { op } = &list.ops[0] else {
            panic!("expected a fill, got {:?}", list.ops[0]);
        };
        let Paint::Solid { color } = &op.paint else {
            panic!("expected solid paint");
        };
        assert_eq!((color.red, color.green, color.blue), (255, 0, 0));
    }

    #[test]
    fn garbage_is_a_parse_error() {
        let err = SvgDocument::new(b"not svg".to_vec(), options()).err();
        assert!(matches!(err, Some(SvgError::Parse { .. })));
    }
}
