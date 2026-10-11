//! `cargo xtask logo`: the lockup and the raster renders of the approved
//! logo.
//!
//! The lockup puts the mark centered above the wordmark in one
//! self-contained SVG, composed from the two approved files so that neither
//! is edited by hand twice. The renders are PNGs at the sizes a README, a
//! web page and an app icon need, drawn with resvg (pure Rust) and the
//! system's fonts for the tagline. Nothing here touches the network.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use resvg::tiny_skia;
use resvg::usvg;

/// The approved mark, concept B's icon (32 by 32 units).
const MARK: &str = "assets/logo/diroxide-mark.svg";
/// The approved wordmark with the tagline (500 by 180 units).
const WORDMARK: &str = "assets/logo/diroxide-wordmark.svg";
/// The lockup this task writes.
const LOCKUP: &str = "assets/logo/diroxide-lockup.svg";
/// Where the PNG renders go.
const PNG_DIR: &str = "assets/logo/png";

/// The widths at which the lockup is rendered, in pixels.
const LOCKUP_WIDTHS: [u32; 4] = [240, 480, 960, 1600];
/// The tagline's color rule for a light page, as the wordmark writes it.
/// The SVG switches to the dark rule through a media query, which the
/// rasterizer does not evaluate, so the dark renders bake the dark rule in.
const TAG_LIGHT_RULE: &str = ".tag { fill: #3f6e9e; }";
/// The tagline's color rule for a dark page: pale steel, never black.
const TAG_DARK_RULE: &str = ".tag { fill: #7ea6cc; }";
/// Where the check renders for the orchestrator's eyes go (build output,
/// never committed): the lockup over a dark page and over a light one.
const CHECK_DIR: &str = "target/logo-check";
/// The sizes at which the mark alone is rendered, in pixels.
const MARK_SIZES: [u32; 6] = [16, 32, 64, 128, 256, 512];

/// The mark's side in lockup units; the wordmark is 500 wide.
const MARK_SIDE: u32 = 160;
/// The lockup's width in units.
const LOCKUP_WIDTH: u32 = 500;
/// The gap between the mark and the wordmark, in units.
const GAP: u32 = 24;
/// The wordmark's height in units.
const WORDMARK_HEIGHT: u32 = 180;
/// The margin above the mark, in units.
const TOP: u32 = 12;

/// Composes the lockup and renders every PNG size. Returns `Ok(true)` when
/// all files were written.
pub fn run(root: &Path) -> Result<bool> {
    let mark = fs::read_to_string(root.join(MARK)).with_context(|| format!("reading {MARK}"))?;
    let wordmark =
        fs::read_to_string(root.join(WORDMARK)).with_context(|| format!("reading {WORDMARK}"))?;
    let lockup = compose(inner(&mark)?, inner(&wordmark)?);
    fs::write(root.join(LOCKUP), &lockup).with_context(|| format!("writing {LOCKUP}"))?;
    println!("logo: wrote {LOCKUP}");

    let png_dir = root.join(PNG_DIR);
    fs::create_dir_all(&png_dir).with_context(|| format!("creating {PNG_DIR}"))?;
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();

    let lockup_dark = lockup.replacen(TAG_LIGHT_RULE, TAG_DARK_RULE, 1);
    if lockup_dark == lockup {
        bail!("the wordmark has no tagline rule {TAG_LIGHT_RULE:?}; the dark renders need it");
    }

    for width in LOCKUP_WIDTHS {
        let light = format!("diroxide-lockup-light-{width}.png");
        render(&lockup, width, &png_dir.join(&light), &options, None)?;
        println!("logo: wrote {PNG_DIR}/{light}");
        let dark = format!("diroxide-lockup-dark-{width}.png");
        render(&lockup_dark, width, &png_dir.join(&dark), &options, None)?;
        println!("logo: wrote {PNG_DIR}/{dark}");
    }
    for size in MARK_SIZES {
        let name = format!("diroxide-mark-{size}.png");
        render(&mark, size, &png_dir.join(&name), &options, None)?;
        println!("logo: wrote {PNG_DIR}/{name}");
    }

    let check_dir = root.join(CHECK_DIR);
    fs::create_dir_all(&check_dir).with_context(|| format!("creating {CHECK_DIR}"))?;
    let ink = tiny_skia::Color::from_rgba8(0x1b, 0x1f, 0x2b, 0xff);
    let paper = tiny_skia::Color::from_rgba8(0xff, 0xff, 0xff, 0xff);
    render(
        &lockup_dark,
        960,
        &check_dir.join("lockup-on-dark-960.png"),
        &options,
        Some(ink),
    )?;
    render(
        &lockup,
        960,
        &check_dir.join("lockup-on-light-960.png"),
        &options,
        Some(paper),
    )?;
    println!("logo: check renders in {CHECK_DIR} (not committed)");
    Ok(true)
}

/// The content of an SVG file between its root element's tags, so it can
/// be nested inside another `svg` element with its own viewBox.
fn inner(svg: &str) -> Result<&str> {
    let open = svg.find("<svg").context("no svg element")?;
    let open_end = svg[open..]
        .find('>')
        .context("the svg element never closes")?
        + open
        + 1;
    let close = svg.rfind("</svg>").context("no closing svg tag")?;
    if close < open_end {
        bail!("the svg element is malformed");
    }
    Ok(&svg[open_end..close])
}

/// The lockup: the mark centered above the wordmark.
fn compose(mark: &str, wordmark: &str) -> String {
    let mark_x = (LOCKUP_WIDTH - MARK_SIDE) / 2;
    let wordmark_y = TOP + MARK_SIDE + GAP;
    let height = wordmark_y + WORDMARK_HEIGHT;
    format!(
        concat!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" ",
            "width=\"{w}\" height=\"{h}\" role=\"img\" aria-labelledby=\"lk-title lk-desc\">\n",
            "  <title id=\"lk-title\">diroxide: Directory Oxide, a modern Rust file manager</title>\n",
            "  <desc id=\"lk-desc\">The diroxide mark, a friendly orange crab waving from the open ",
            "drawer of a rusty steel-blue filing cabinet, centered above the word diroxide and the ",
            "tagline Directory Oxide: a modern Rust file manager.</desc>\n",
            "  <!-- Composed by cargo xtask logo from diroxide-mark.svg and diroxide-wordmark.svg. ",
            "Edit those files, then run the task again. -->\n",
            "  <svg x=\"{mx}\" y=\"{my}\" width=\"{ms}\" height=\"{ms}\" viewBox=\"0 0 32 32\">{mark}</svg>\n",
            "  <svg x=\"0\" y=\"{wy}\" width=\"{w}\" height=\"{wh}\" viewBox=\"0 0 500 180\">{wordmark}</svg>\n",
            "</svg>\n",
        ),
        w = LOCKUP_WIDTH,
        h = height,
        mx = mark_x,
        my = TOP,
        ms = MARK_SIDE,
        mark = mark,
        wy = wordmark_y,
        wh = WORDMARK_HEIGHT,
        wordmark = wordmark,
    )
}

/// Renders an SVG to a PNG of the given width, keeping its aspect ratio,
/// over a transparent background or the given color.
fn render(
    svg: &str,
    width: u32,
    path: &Path,
    options: &usvg::Options<'_>,
    background: Option<tiny_skia::Color>,
) -> Result<()> {
    let tree = usvg::Tree::from_str(svg, options).context("parsing the SVG")?;
    let size = tree.size();
    let scale = width as f32 / size.width();
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).context("allocating the pixmap")?;
    if let Some(color) = background {
        pixmap.fill(color);
    }
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_strips_the_root_element() {
        let svg = "<svg xmlns=\"x\" viewBox=\"0 0 1 1\">\n  <rect/>\n</svg>\n";
        assert_eq!(inner(svg).unwrap(), "\n  <rect/>\n");
    }

    #[test]
    fn the_dark_rule_replaces_the_light_rule_once() {
        let svg = format!("<style>{TAG_LIGHT_RULE}</style>");
        let dark = svg.replacen(TAG_LIGHT_RULE, TAG_DARK_RULE, 1);
        assert!(dark.contains("#7ea6cc") && !dark.contains("#3f6e9e"));
    }

    #[test]
    fn inner_refuses_a_file_without_a_root() {
        assert!(inner("<rect/>").is_err());
    }

    #[test]
    fn the_lockup_nests_both_files_and_centers_the_mark() {
        let lockup = compose("<rect id=\"m\"/>", "<rect id=\"w\"/>");
        assert!(lockup.contains("viewBox=\"0 0 500 376\""));
        assert!(lockup.contains("<svg x=\"170\" y=\"12\" width=\"160\" height=\"160\" viewBox=\"0 0 32 32\"><rect id=\"m\"/></svg>"));
        assert!(lockup.contains(
            "y=\"196\" width=\"500\" height=\"180\" viewBox=\"0 0 500 180\"><rect id=\"w\"/></svg>"
        ));
    }
}
