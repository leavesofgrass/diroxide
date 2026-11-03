//! The window's colors, fonts and text layouts until the themes arrive
//! (R27's commit 27): Masonry's dark default page, with every pair
//! checked here against the contrast rule (4.5 to 1 for text, 3 to 1 for
//! the focus marks), and a blue and an orange that red-green color
//! blindness keeps apart. Color is never the only sign: a marked row
//! starts with an asterisk and says "selected", the focused row has a
//! filled bar in the active panel and an outline in the other, and the
//! focused panel has a ring.

use masonry::core::{BrushIndex, LayoutCtx, MeasureCtx, StyleProperty};
use masonry::parley::style::{FontFamily, FontWeight, LineHeight};
use masonry::parley::{FontContext, Layout, LayoutContext};
use masonry::peniko::Color;

/// The interface font: Atkinson Hyperlegible Next (compiled in), then
/// the system's.
pub const FONT_STACK: &str = "\"Atkinson Hyperlegible Next\", system-ui, sans-serif";

/// The text size of rows and lines, in logical pixels.
pub const TEXT_SIZE: f32 = 16.0;

/// The page.
pub const PAGE: Color = Color::from_rgb8(0x1d, 0x1d, 0x1d);
/// Text on the page.
pub const TEXT: Color = Color::from_rgb8(0xf2, 0xf2, 0xf2);
/// A header's text, a step quieter than a row's.
pub const HEADER: Color = Color::from_rgb8(0xd0, 0xd0, 0xd0);
/// The focused row's bar in the active panel.
pub const FOCUS_BAR: Color = Color::from_rgb8(0x1f, 0x5f, 0xbf);
/// Text on the focus bar.
pub const ON_FOCUS_BAR: Color = Color::from_rgb8(0xff, 0xff, 0xff);
/// A marked row's text (orange, told from the blue bar without red or
/// green); the asterisk and the word say it too.
pub const MARKED: Color = Color::from_rgb8(0xff, 0xb4, 0x54);
/// An unavailable row's text; the word "unavailable" says it too.
pub const DIM: Color = Color::from_rgb8(0xb0, 0xb0, 0xa8);
/// The ring round the panel with the keyboard focus, and the outline of
/// the focused row in the other panel.
pub const RING: Color = Color::from_rgb8(0xff, 0xff, 0xff);

/// The text layout of `text` in the interface font, unbroken (a row is
/// one line; the window cuts it at the panel's edge).
pub fn line_layout(
    ctx: &mut LayoutCtx<'_>,
    text: &str,
    size: f32,
    bold: bool,
) -> Layout<BrushIndex> {
    let (fcx, lcx) = ctx.text_contexts();
    layout_with(fcx, lcx, text, size, bold)
}

/// The same as [`line_layout`] while measuring, so a widget can size
/// itself to its text.
pub fn measured_layout(
    ctx: &mut MeasureCtx<'_>,
    text: &str,
    size: f32,
    bold: bool,
) -> Layout<BrushIndex> {
    let (fcx, lcx) = ctx.text_contexts();
    layout_with(fcx, lcx, text, size, bold)
}

fn layout_with(
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<BrushIndex>,
    text: &str,
    size: f32,
    bold: bool,
) -> Layout<BrushIndex> {
    let mut builder = lcx.ranged_builder(fcx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
        FONT_STACK.into(),
    )));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.3)));
    if bold {
        builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
    }
    builder.push_default(StyleProperty::Brush(BrushIndex(0)));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

/// The contrast ratio of two colors, by WCAG's relative luminance.
pub fn contrast(a: Color, b: Color) -> f64 {
    fn luminance(c: Color) -> f64 {
        let rgba = c.to_rgba8();
        let lin = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.040_45 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(rgba.r) + 0.7152 * lin(rgba.g) + 0.0722 * lin(rgba.b)
    }
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_pair_meets_four_and_a_half_to_one() {
        for (name, fg, bg) in [
            ("text", TEXT, PAGE),
            ("header", HEADER, PAGE),
            ("marked", MARKED, PAGE),
            ("dim", DIM, PAGE),
            ("on the focus bar", ON_FOCUS_BAR, FOCUS_BAR),
        ] {
            let ratio = contrast(fg, bg);
            assert!(ratio >= 4.5, "{name}: {ratio:.2} to 1");
        }
    }

    #[test]
    fn the_focus_marks_meet_three_to_one_against_the_page() {
        // The focused row is outlined in the ring's color in both
        // panels, filled with the bar under white text in the active
        // one; the outline is what meets 3 to 1 on the page and the bar.
        assert!(contrast(RING, PAGE) >= 3.0);
        assert!(contrast(RING, FOCUS_BAR) >= 3.0);
    }
}
