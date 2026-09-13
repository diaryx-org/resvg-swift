//! resvg's whole test suite through both renderers.
//!
//! `agrees_with_resvg.rs` is one hand-written SVG per feature. This is the same
//! contract over the ~1,700 SVGs resvg tests itself with — filters, masking,
//! paint servers, painting, shapes, structure, text — rendered by resvg and by
//! `flatten` + `replay`, and compared pixel for pixel. It is the number behind
//! "mirrors resvg": how many of resvg's own cases the flattening loses nothing
//! on.
//!
//! The suite is fetched by `scripts/fetch-resvg-tests.sh` into
//! `target/resvg-tests/` at the pinned resvg version; `cargo xtask ci suite`
//! does both. Without it this test passes trivially with a note, so a plain
//! `cargo test` needs no network; set `RESVG_SUITE_REQUIRED=1` to make its
//! absence a failure, which CI does.
//!
//! Options mirror resvg's own harness (`tests/integration/main.rs`): its font
//! directory with its family defaults, `resources_dir` at the SVG, and a
//! render scaled to 300 pixels wide — a fractional scale for most cases.
//!
//! Cases listed in `resvg-suite-allowlist.txt` are permitted to differ; each
//! line says why. The list is meant to shrink — at resvg 0.46.0 it is empty:
//! 1,697 cases compared, 1,676 bit-for-bit identical, the other 21 within a
//! few levels on under 2% of their pixels (f32 composition order in transform
//! chains, pattern-tile phase). The run prints those counts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use resvg::tiny_skia;
use usvg_flatten::{Options, Transform, flatten, replay};

/// Width every case is rendered at, as in resvg's harness.
const IMAGE_WIDTH: u32 = 300;
/// Per-channel difference that counts as a differing pixel.
const CHANNEL_TOLERANCE: u8 = 8;
/// Fraction of a case's pixels allowed to differ (edge anti-aliasing).
const PIXEL_BUDGET: f64 = 0.005;

fn suite_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RESVG_SUITE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("target/resvg-tests/crates/resvg/tests");
    dir.is_dir().then_some(dir)
}

fn fontdb(suite: &Path) -> Arc<usvg::fontdb::Database> {
    let mut db = usvg::fontdb::Database::new();
    db.load_fonts_dir(suite.join("fonts"));
    db.set_serif_family("Noto Serif");
    db.set_sans_serif_family("Noto Sans");
    db.set_cursive_family("Yellowtail");
    db.set_fantasy_family("Sedgwick Ave Display");
    db.set_monospace_family("Noto Mono");
    Arc::new(db)
}

fn svgs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            svgs(&path, out);
        } else if path.extension().is_some_and(|e| e == "svg") {
            out.push(path);
        }
    }
}

fn allowlist(suite_tests: &Path) -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/resvg-suite-allowlist.txt");
    let _ = suite_tests;
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (name, why) = l.split_once(char::is_whitespace)?;
            Some((name.to_owned(), why.trim().to_owned()))
        })
        .collect()
}

struct Outcome {
    name: String,
    /// Pixels differing by more than `CHANNEL_TOLERANCE` on some channel.
    differing: usize,
    total: usize,
    /// Byte-for-byte the same image.
    identical: bool,
}

impl Outcome {
    fn fraction(&self) -> f64 {
        self.differing as f64 / self.total.max(1) as f64
    }
}

/// Render one case both ways; `None` if usvg won't parse it or it has no
/// size (resvg's harness would fail such a case outright; we have nothing to
/// compare).
fn compare(svg: &Path, tests: &Path, fonts: &Arc<usvg::fontdb::Database>) -> Option<Outcome> {
    let name = svg
        .strip_prefix(tests)
        .ok()?
        .with_extension("")
        .to_string_lossy()
        .into_owned();
    let opt = usvg::Options {
        fontdb: fonts.clone(),
        resources_dir: Some(svg.parent()?.to_owned()),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(&std::fs::read(svg).ok()?, &opt).ok()?;

    let size = tree.size().to_int_size().scale_to_width(IMAGE_WIDTH)?;
    let sx = size.width() as f32 / tree.size().width();
    let sy = size.height() as f32 / tree.size().height();

    let mut expected = tiny_skia::Pixmap::new(size.width(), size.height())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut expected.as_mut(),
    );

    let list = flatten(
        &tree,
        &Options {
            raster_scale: sx.max(sy),
        },
    );
    let mut actual = tiny_skia::Pixmap::new(size.width(), size.height())?;
    replay::render(
        &list,
        Transform {
            sx,
            sy,
            ..Transform::IDENTITY
        },
        &mut actual.as_mut(),
    );

    let (e, _) = expected.data().as_chunks::<4>();
    let (a, _) = actual.data().as_chunks::<4>();
    let differing = e
        .iter()
        .zip(a)
        .filter(|(x, y)| {
            x.iter()
                .zip(*y)
                .any(|(p, q)| p.abs_diff(*q) > CHANNEL_TOLERANCE)
        })
        .count();
    let outcome = Outcome {
        name,
        differing,
        total: e.len(),
        identical: expected.data() == actual.data(),
    };
    if outcome.fraction() > PIXEL_BUDGET {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/resvg-suite-diffs");
        let flat = outcome.name.replace('/', "__");
        std::fs::create_dir_all(&dir).ok();
        expected
            .save_png(dir.join(format!("{flat}.resvg.png")))
            .ok();
        actual
            .save_png(dir.join(format!("{flat}.flatten.png")))
            .ok();
    }
    Some(outcome)
}

#[test]
fn the_whole_resvg_suite_agrees() {
    let Some(suite) = suite_dir() else {
        if std::env::var_os("RESVG_SUITE_REQUIRED").is_some() {
            panic!("resvg test suite not found; run scripts/fetch-resvg-tests.sh");
        }
        eprintln!("resvg test suite not fetched; skipping (scripts/fetch-resvg-tests.sh)");
        return;
    };
    let tests = suite.join("tests");
    let fonts = fontdb(&suite);
    let allowed = allowlist(&tests);

    let mut files = Vec::new();
    svgs(&tests, &mut files);
    files.sort();
    assert!(
        files.len() > 1000,
        "suite looks incomplete: {} SVGs",
        files.len()
    );

    let mut compared = 0usize;
    let mut exact = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<Outcome> = Vec::new();
    let mut allowed_but_passing: Vec<String> = Vec::new();
    for svg in &files {
        let Some(outcome) = compare(svg, &tests, &fonts) else {
            skipped += 1;
            continue;
        };
        compared += 1;
        exact += usize::from(outcome.identical);
        let failed = outcome.fraction() > PIXEL_BUDGET;
        let listed = allowed.contains_key(&outcome.name);
        match (failed, listed) {
            (true, false) => failures.push(outcome),
            (false, true) => allowed_but_passing.push(outcome.name),
            _ => {}
        }
    }

    failures.sort_by(|a, b| b.fraction().total_cmp(&a.fraction()));
    eprintln!(
        "resvg suite: {compared} compared, {exact} bit-exact, {skipped} unparsed, {} allowed, {} unexpected differences",
        allowed.len(),
        failures.len()
    );
    for f in &failures {
        eprintln!("  {:>6.2}%  {}", f.fraction() * 100.0, f.name);
    }
    if !allowed_but_passing.is_empty() {
        eprintln!("allowlisted cases that now pass (remove them):");
        for name in &allowed_but_passing {
            eprintln!("  {name}");
        }
    }

    assert!(
        failures.is_empty(),
        "{} cases differ from resvg beyond {:.1}% of pixels; diffs in target/resvg-suite-diffs/",
        failures.len(),
        PIXEL_BUDGET * 100.0
    );
    assert!(
        allowed_but_passing.is_empty(),
        "allowlist is stale: {} cases pass",
        allowed_but_passing.len()
    );
}
