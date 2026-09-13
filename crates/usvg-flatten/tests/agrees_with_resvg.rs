//! The contract: an SVG rendered by resvg and the same SVG flattened and
//! replayed through tiny-skia are the same picture.
//!
//! Each case is one SVG feature the flattener handles a particular way, checked
//! at a non-integer scale so that transforms, stroke widths, and gradient
//! geometry all have to be carried correctly rather than happen to line up.
//! The comparison is tolerant of a few anti-aliasing levels on a small fraction
//! of pixels — the two paths build their clip masks differently — and of
//! nothing else.

use resvg::tiny_skia;
use usvg_flatten::{Op, Options, Paint, Transform, flatten, replay};

/// Render at this many device pixels per user unit. Fractional on purpose.
const SCALE: f32 = 2.5;
/// Per-channel difference that counts as a differing pixel.
const CHANNEL_TOLERANCE: u8 = 8;
/// Fraction of pixels allowed to differ (edge anti-aliasing).
const PIXEL_BUDGET: f64 = 0.01;

fn parse(svg: &str) -> usvg::Tree {
    usvg::Tree::from_str(svg, &usvg::Options::default()).expect("test SVG parses")
}

fn pixmap_for(tree: &usvg::Tree) -> tiny_skia::Pixmap {
    let size = tree.size();
    tiny_skia::Pixmap::new(
        (size.width() * SCALE).ceil() as u32,
        (size.height() * SCALE).ceil() as u32,
    )
    .expect("non-empty canvas")
}

fn via_resvg(tree: &usvg::Tree) -> tiny_skia::Pixmap {
    let mut pixmap = pixmap_for(tree);
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    pixmap
}

fn via_flatten(tree: &usvg::Tree) -> (tiny_skia::Pixmap, usvg_flatten::DisplayList) {
    let list = flatten(
        tree,
        &Options {
            raster_scale: SCALE,
        },
    );
    let mut pixmap = pixmap_for(tree);
    let scale = Transform {
        sx: SCALE,
        sy: SCALE,
        ..Transform::IDENTITY
    };
    replay::render(&list, scale, &mut pixmap.as_mut());
    (pixmap, list)
}

/// Assert the two renderings agree, and hand back the display list so the
/// case can also assert on its shape.
fn agree(name: &str, svg: &str) -> usvg_flatten::DisplayList {
    let tree = parse(svg);
    let expected = via_resvg(&tree);
    let (actual, list) = via_flatten(&tree);
    assert_eq!(expected.data().len(), actual.data().len(), "{name}: sizes");

    let (expected_px, _) = expected.data().as_chunks::<4>();
    let (actual_px, _) = actual.data().as_chunks::<4>();
    let differing = expected_px
        .iter()
        .zip(actual_px)
        .filter(|(a, b)| {
            a.iter()
                .zip(*b)
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    let total = expected.width() as usize * expected.height() as usize;
    let fraction = differing as f64 / total as f64;
    if fraction > PIXEL_BUDGET {
        // Leave the evidence where a human can look at it.
        let dir = std::env::temp_dir().join("usvg-flatten-diffs");
        std::fs::create_dir_all(&dir).ok();
        expected
            .save_png(dir.join(format!("{name}-resvg.png")))
            .ok();
        actual
            .save_png(dir.join(format!("{name}-flatten.png")))
            .ok();
        panic!(
            "{name}: {differing} of {total} pixels differ ({:.2}%); see {}",
            fraction * 100.0,
            dir.display()
        );
    }
    list
}

#[test]
fn solid_fill_and_stroke() {
    let list = agree(
        "solid_fill_and_stroke",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30">
              <rect x="5" y="5" width="30" height="20" fill="#3060c0" stroke="#c03060" stroke-width="3"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Fill(_), Op::Stroke(_)]));
}

#[test]
fn paint_order_stroke_first() {
    let list = agree(
        "paint_order",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30">
              <rect x="5" y="5" width="30" height="20" fill="#3060c0" stroke="#c03060" stroke-width="6" paint-order="stroke"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Stroke(_), Op::Fill(_)]));
}

#[test]
fn nested_transforms_and_curves() {
    agree(
        "nested_transforms",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60">
              <g transform="translate(30 30) rotate(30)">
                <g transform="scale(1.5 0.8)">
                  <circle r="12" fill="#20a060"/>
                  <path d="M -10 -10 Q 0 -20 10 -10 C 15 0 5 10 0 5 Z" fill="none" stroke="#000" stroke-width="2"/>
                </g>
              </g>
            </svg>"##,
    );
}

#[test]
fn dashed_stroke_with_caps_and_joins() {
    agree(
        "dashes",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="40">
              <polyline points="5,35 20,5 35,35 55,5" fill="none" stroke="#333"
                        stroke-width="4" stroke-linecap="round" stroke-linejoin="round"
                        stroke-dasharray="6 3" stroke-dashoffset="2"/>
            </svg>"##,
    );
}

#[test]
fn even_odd_fill() {
    agree(
        "even_odd",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <path d="M5 5 H35 V35 H5 Z M12 12 H28 V28 H12 Z" fill="#906030" fill-rule="evenodd"/>
            </svg>"##,
    );
}

#[test]
fn group_opacity_is_a_layer() {
    let list = agree(
        "group_opacity",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <g opacity="0.5">
                <rect x="5" y="5" width="20" height="20" fill="#f00"/>
                <rect x="15" y="15" width="20" height="20" fill="#00f"/>
              </g>
            </svg>"##,
    );
    assert!(matches!(
        &list.ops[..],
        [Op::PushLayer(layer), Op::Fill(_), Op::Fill(_), Op::PopLayer] if layer.opacity == 0.5
    ));
}

#[test]
fn blend_mode_multiply() {
    agree(
        "blend_multiply",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <rect x="5" y="5" width="20" height="20" fill="#ff0"/>
              <rect x="15" y="15" width="20" height="20" fill="#0ff" style="mix-blend-mode:multiply"/>
            </svg>"##,
    );
}

#[test]
fn clip_path_of_two_shapes() {
    let list = agree(
        "clip_path",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <clipPath id="c" transform="rotate(10 20 20)">
                <circle cx="14" cy="20" r="10"/>
                <rect x="20" y="10" width="14" height="20"/>
              </clipPath>
              <rect width="40" height="40" fill="#408040" clip-path="url(#c)"/>
            </svg>"##,
    );
    let Op::PushLayer(layer) = &list.ops[0] else {
        panic!("clip should open a layer");
    };
    assert_eq!(layer.clips.len(), 1);
    assert_eq!(layer.clips[0].shapes.len(), 2);
}

#[test]
fn linear_gradient_stays_vector() {
    let list = agree(
        "linear_gradient",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <linearGradient id="g" x1="0" y1="0" x2="1" y2="1" gradientTransform="rotate(20)">
                <stop offset="0" stop-color="#f00"/>
                <stop offset="0.5" stop-color="#0f0" stop-opacity="0.5"/>
                <stop offset="1" stop-color="#00f"/>
              </linearGradient>
              <rect x="4" y="4" width="32" height="32" fill="url(#g)" fill-opacity="0.8"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Fill(op)] if matches!(op.paint, Paint::Linear(_))));
}

#[test]
fn radial_gradient_with_focal_point() {
    let list = agree(
        "radial_gradient",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <radialGradient id="g" cx="0.5" cy="0.5" r="0.5" fx="0.3" fy="0.3">
                <stop offset="0" stop-color="#fff"/>
                <stop offset="1" stop-color="#204080"/>
              </radialGradient>
              <circle cx="20" cy="20" r="16" fill="url(#g)"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Fill(op)] if matches!(op.paint, Paint::Radial(_))));
}

#[test]
fn repeating_gradient_falls_back_to_raster() {
    let list = agree(
        "repeat_gradient",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <linearGradient id="g" x1="0" y1="0" x2="0.25" y2="0" spreadMethod="repeat">
                <stop offset="0" stop-color="#f00"/>
                <stop offset="1" stop-color="#00f"/>
              </linearGradient>
              <rect width="40" height="40" fill="url(#g)"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Raster(_)]));
}

#[test]
fn filter_falls_back_to_raster() {
    let list = agree(
        "filter",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60">
              <filter id="b"><feGaussianBlur stdDeviation="2"/></filter>
              <rect x="10" y="10" width="20" height="20" fill="#f00"/>
              <rect x="20" y="20" width="20" height="20" fill="#00f" filter="url(#b)"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Fill(_), Op::Raster(_)]));
}

#[test]
fn mask_falls_back_to_raster() {
    let list = agree(
        "mask",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <mask id="m"><circle cx="20" cy="20" r="15" fill="#fff"/></mask>
              <rect width="40" height="40" fill="#800" mask="url(#m)"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Raster(_)]));
}

#[test]
fn pattern_falls_back_to_raster() {
    let list = agree(
        "pattern",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <pattern id="p" width="8" height="8" patternUnits="userSpaceOnUse">
                <rect width="4" height="4" fill="#06c"/>
              </pattern>
              <rect width="40" height="40" fill="url(#p)"/>
            </svg>"##,
    );
    assert!(matches!(&list.ops[..], [Op::Raster(_)]));
}

#[test]
fn nested_svg_image_is_inlined() {
    // A 10×10 green square, as an SVG document, base64 so it can sit in an
    // attribute.
    let inner = r##"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10' fill='#0a0'/></svg>"##;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <image x="10" y="10" width="20" height="20" href="data:image/svg+xml;base64,{}"/>
            </svg>"##,
        base64(inner.as_bytes())
    );
    let list = agree("nested_svg", &svg);
    assert!(!list.ops.is_empty());
    assert!(list.ops.iter().all(|op| matches!(op, Op::Fill(_))));
}

/// Standard base64 with padding — enough for one fixture, without a dependency.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut n = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            n |= (*b as u32) << (16 - 8 * i);
        }
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn text_is_glyph_outlines() {
    // Whatever font fontdb finds — or none, in which case usvg drops the text
    // and both sides agree on an empty canvas. Either way the pixels match and
    // nothing in the list is a raster.
    let mut opt = usvg::Options::default();
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    opt.fontdb = std::sync::Arc::new(db);
    let tree = usvg::Tree::from_str(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30">
              <text x="4" y="22" font-size="18" fill="#123">Ag</text>
            </svg>"##,
        &opt,
    )
    .expect("parses");
    let expected = via_resvg(&tree);
    let (actual, list) = via_flatten(&tree);
    assert_eq!(expected.data(), actual.data(), "text pixels");
    assert!(list.ops.iter().all(|op| !matches!(op, Op::Raster(_))));
}
