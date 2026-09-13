//! usvg-flatten — a usvg tree as a flat display list.
//!
//! usvg parses an SVG into a tree in which every reference is resolved: CSS is
//! applied, `<use>` is expanded, units are absolute, and `<text>` is laid out
//! into glyph outlines. What remains is groups, paths, images, and paints —
//! close to what a 2D API draws, but still a tree with inherited transforms,
//! group-level compositing, and clip paths that are themselves subtrees.
//!
//! [`flatten`] walks that tree once and emits a [`DisplayList`]: a `Vec<Op>` in
//! paint order where each op carries its own absolute [`Transform`], layers are
//! bracketed by [`Op::PushLayer`]/[`Op::PopLayer`], and a clip is a list of
//! shapes. A renderer replays it as a loop over a dozen variants and never
//! learns what an SVG element is. That is what lets a thin CoreGraphics (or
//! Skia, or Direct2D) backend stay resolution-independent — a vector path in,
//! a vector path out — where a bitmap from resvg would not.
//!
//! ## What stays vector, and what does not
//!
//! Vector: paths filled or stroked with a solid colour or a pad-spread linear or
//! radial gradient; strokes with caps, joins, miter limit, and dashes; group
//! opacity and blend mode; clip paths (as unions of shapes, intersected in
//! sequence); embedded raster images by their encoded bytes; nested SVG images
//! by recursion.
//!
//! Raster, through resvg at [`Options::raster_scale`]: any group with a filter
//! or a mask, any path painted with a pattern or a `reflect`/`repeat` gradient,
//! and any clip path whose contents are not plain shapes. Each becomes one
//! [`Op::Raster`] — premultiplied RGBA pixels and the canvas rectangle they
//! cover. The consumer picks the scale (device pixels per user unit) and
//! re-flattens when it changes; that is the resolution policy, and it lives with
//! the consumer rather than here. Vector masks and pattern paint are candidates
//! for a later release; see `docs/tasks/`.
//!
//! ## Coordinates
//!
//! The canvas is the SVG's own coordinate system, `width × height` user units
//! from [`DisplayList`], y down. Paths are in their local space with the op's
//! `transform` mapping local to canvas — strokes are meant to be applied after
//! the transform, as SVG does, so a non-uniform scale widens a stroke
//! non-uniformly. Gradients are in the *path's* local space with their own
//! additional `transform`, exactly as usvg resolves them. [`Op::Raster`] alone
//! is in canvas space already.
//!
//! [`replay`] rasterizes a display list with tiny-skia. It exists so the tests
//! can put an SVG through resvg and through `flatten` + `replay` and demand the
//! same pixels; a Rust host that wants a bitmap should call resvg directly.

mod flatten;
mod raster;
pub mod replay;

pub use flatten::flatten;

/// How to flatten.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Device pixels per user unit for anything that has to be rasterized. `1.0`
    /// renders a filtered group at the SVG's own size; a consumer drawing a
    /// 100-unit SVG 300 points wide on a 2× display passes `6.0`.
    pub raster_scale: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self { raster_scale: 1.0 }
    }
}

/// The flattened picture.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayList {
    /// Canvas width in user units — the SVG's `width`, as usvg resolved it.
    pub width: f32,
    /// Canvas height in user units.
    pub height: f32,
    /// The operations, in paint order. Every `PushLayer` has a matching
    /// `PopLayer` later in the list; they nest.
    pub ops: Vec<Op>,
}

/// One drawing operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Begin a compositing layer. Everything up to the matching [`Op::PopLayer`]
    /// is drawn onto a fresh transparent surface, then composited with the
    /// layer's opacity and blend mode, clipped by its clips.
    PushLayer(Layer),
    /// End the innermost layer.
    PopLayer,
    /// Fill a path.
    Fill(FillOp),
    /// Stroke a path.
    Stroke(StrokeOp),
    /// Draw an encoded raster image at its pixel size under a transform.
    Image(ImageOp),
    /// Draw pixels resvg produced for a subtree this crate does not express as
    /// vectors. Already in canvas space.
    Raster(RasterOp),
}

/// A compositing layer — what an SVG group becomes when it has to be isolated.
///
/// Groups that need no isolation (opaque, normal blend, no clip) are not layers
/// at all: their children are emitted inline, which is what resvg does too.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Group opacity, `0.0..=1.0`. `1.0` when the layer exists for another reason.
    pub opacity: f32,
    /// How the layer composites onto what is beneath it.
    pub blend: BlendMode,
    /// Clips to apply, intersected in order. Each is a union of shapes.
    pub clips: Vec<Clip>,
}

/// One `clipPath`: the union of its shapes. A `clipPath` that itself carries a
/// `clip-path` attribute contributes a second [`Clip`] to the layer, and the two
/// intersect.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// The shapes whose union is the visible region.
    pub shapes: Vec<ClipShape>,
}

/// One shape of a clip.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipShape {
    /// The outline, in its local space.
    pub path: Path,
    /// Local to canvas.
    pub transform: Transform,
    /// How the outline's interior is decided.
    pub rule: FillRule,
}

/// Fill a path.
#[derive(Clone, Debug, PartialEq)]
pub struct FillOp {
    /// The outline, in its local space.
    pub path: Path,
    /// Local to canvas.
    pub transform: Transform,
    /// What to fill with.
    pub paint: Paint,
    /// `fill-opacity`, `0.0..=1.0`, multiplied into the paint.
    pub opacity: f32,
    /// How the interior is decided.
    pub rule: FillRule,
    /// Whether edges should be anti-aliased (`shape-rendering`).
    pub antialias: bool,
}

/// Stroke a path.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeOp {
    /// The outline, in its local space.
    pub path: Path,
    /// Local to canvas. Apply *before* stroking, so the stroke width is in local
    /// units and scales with the path.
    pub transform: Transform,
    /// What to stroke with.
    pub paint: Paint,
    /// `stroke-opacity`, `0.0..=1.0`, multiplied into the paint.
    pub opacity: f32,
    /// The pen.
    pub stroke: StrokeStyle,
    /// Whether edges should be anti-aliased (`shape-rendering`).
    pub antialias: bool,
}

/// The pen for a [`StrokeOp`], in the path's local units.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    /// `stroke-width`.
    pub width: f32,
    /// `stroke-linecap`.
    pub cap: LineCap,
    /// `stroke-linejoin`.
    pub join: LineJoin,
    /// `stroke-miterlimit`.
    pub miter_limit: f32,
    /// `stroke-dasharray` / `stroke-dashoffset`, if dashed.
    pub dash: Option<Dash>,
}

/// A dash pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Dash {
    /// On/off lengths, in local units. Even in length and non-empty.
    pub array: Vec<f32>,
    /// Where in the pattern the stroke starts.
    pub offset: f32,
}

/// An encoded raster `<image>`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageOp {
    /// The file bytes, still encoded — decoding is the consumer's, whose
    /// platform decoder is the one it already trusts.
    pub data: Vec<u8>,
    /// Which codec `data` is in.
    pub format: ImageFormat,
    /// Pixel width of the decoded image, as usvg read from the header.
    pub width: f32,
    /// Pixel height of the decoded image.
    pub height: f32,
    /// Local to canvas, where local is the image's pixel rectangle
    /// `(0, 0, width, height)`. usvg folds `preserveAspectRatio` into this.
    pub transform: Transform,
    /// Whether to interpolate when scaling (`image-rendering`), rather than
    /// pick nearest pixels.
    pub smooth: bool,
}

/// The encoding of an [`ImageOp`]'s bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

/// Pixels resvg rendered for a subtree that is not expressed as vectors.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterOp {
    /// Premultiplied RGBA, 8 bits per channel, row-major, `width * height * 4`
    /// bytes. Premultiplied because that is what tiny-skia holds and what a
    /// compositor wants; un-premultiply if handing to a decoder that expects
    /// straight alpha.
    pub rgba: Vec<u8>,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// The canvas rectangle the pixels cover, in user units.
    pub rect: Rect,
}

/// What a fill or stroke is painted with.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// One colour.
    Solid(Color),
    /// A linear gradient, in the path's local space.
    Linear(LinearGradient),
    /// A radial gradient, in the path's local space.
    Radial(RadialGradient),
}

/// An opaque sRGB colour. Opacity travels separately, on the op or the stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// A linear gradient. Coordinates are in the path's local space after
/// `transform`; usvg has already turned `objectBoundingBox` units into this.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    /// `gradientTransform`, applied to the gradient's coordinates within the
    /// path's local space.
    pub transform: Transform,
    /// At least two, offsets ascending in `0.0..=1.0`.
    pub stops: Vec<Stop>,
}

/// A radial gradient: from the focal point `(fx, fy)` at radius 0 to the circle
/// centred `(cx, cy)` at radius `r`.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    pub fx: f32,
    pub fy: f32,
    /// `gradientTransform`, as for [`LinearGradient::transform`].
    pub transform: Transform,
    /// At least two, offsets ascending in `0.0..=1.0`.
    pub stops: Vec<Stop>,
}

/// One gradient stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// Position along the gradient, `0.0..=1.0`.
    pub offset: f32,
    pub color: Color,
    /// `stop-opacity`, already multiplied by the op's fill/stroke opacity.
    pub opacity: f32,
}

/// A path as verbs, in the order given, in its local space.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub verbs: Vec<PathVerb>,
}

/// One path verb. Coordinates are absolute in the path's local space.
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// A 2D affine transform, in tiny-skia's field order:
///
/// ```text
/// x' = sx * x + kx * y + tx
/// y' = ky * x + sy * y + ty
/// ```
///
/// `CGAffineTransform(a: sx, b: ky, c: kx, d: sy, tx, ty)` is the same matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub sx: f32,
    pub kx: f32,
    pub ky: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Transform {
    /// The identity.
    pub const IDENTITY: Transform = Transform {
        sx: 1.0,
        kx: 0.0,
        ky: 0.0,
        sy: 1.0,
        tx: 0.0,
        ty: 0.0,
    };
}

/// An axis-aligned rectangle in canvas units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How a path's interior is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// `stroke-linecap`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// `stroke-linejoin`. `MiterClip` is SVG 2's clipped miter; a backend without
/// it should fall back to `Miter`, as resvg's tiny-skia backend does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    MiterClip,
    Round,
    Bevel,
}

/// `mix-blend-mode`. The sixteen CSS compositing modes, all of which
/// CoreGraphics and Skia name the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
