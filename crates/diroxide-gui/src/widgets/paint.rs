//! The window's colors, fonts and text layouts. The colors are the
//! chosen theme's ([`crate::theme::Palette`], R27's commit 27), handed
//! to every widget of this crate as the [`Colors`] property in the
//! window's default properties ([`default_properties`]), so choosing
//! another theme repaints the whole window at once, dialogs included.
//! Masonry's own labels (the title, status and jobs lines, a dialog's
//! lines) take their text color from [`ContentColor`].
//!
//! Color is never the only sign: a marked row starts with an asterisk
//! and says "selected", the focused row has an outline as well as its
//! band (a ring and a line inside it, so it shows on a dark page and a
//! light band alike), the focused panel has a ring, and every role keeps
//! its theme's attribute (bold folders, underlined links).

use std::any::TypeId;
use std::sync::{Arc, OnceLock};

use diroxide_theme::{Attrs, PanelRole, Rgb};
use masonry::core::{
    BrushIndex, DefaultProperties, LayoutCtx, MeasureCtx, PropertiesRef, Property, PropertyCache,
    StyleProperty,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Rect, Stroke};
use masonry::parley::style::{FontFamily, FontStyle, FontWeight, LineHeight};
use masonry::parley::{FontContext, Layout, LayoutContext};
use masonry::peniko::Color;
use masonry::properties::ContentColor;

use crate::theme::Palette;

/// The interface font: Atkinson Hyperlegible Next (compiled in), then
/// the system's.
pub const FONT_STACK: &str = "\"Atkinson Hyperlegible Next\", system-ui, sans-serif";

/// The text size of rows and lines, in logical pixels.
pub const TEXT_SIZE: f32 = 16.0;

/// The window's colors, as a property of every widget of this crate: the
/// palette of the chosen theme. The default is Galaxy's, so a widget
/// draws even before the window's properties are set.
#[derive(Clone, Debug, PartialEq)]
pub struct Colors(pub Arc<Palette>);

impl Default for Colors {
    fn default() -> Self {
        Self::static_default().clone()
    }
}

impl Property for Colors {
    fn static_default() -> &'static Self {
        static DEFAULT: OnceLock<Colors> = OnceLock::new();
        DEFAULT.get_or_init(|| Colors(Arc::new(Palette::default())))
    }
}

/// A theme color as Masonry's.
pub fn color(rgb: Rgb) -> Color {
    Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}

/// The palette a widget draws with: its [`Colors`] property.
pub fn palette_of(props: &PropertiesRef<'_>, cache: &mut PropertyCache) -> Arc<Palette> {
    Arc::clone(&props.get::<Colors>(cache).0)
}

/// Whether a changed property is the window's colors, which a widget
/// answers by drawing again.
pub fn is_colors(property_type: TypeId) -> bool {
    Colors::matches(property_type)
}

/// The focused row's outline round `bar`: the ring outside it, and, when
/// the row is filled with the focus band, a line just inside in the
/// palette's inner color, so the outline shows on a dark page and a
/// light band alike (textweaver's double ring).
pub fn outline(painter: &mut Painter<'_>, bar: Rect, palette: &Palette, filled: bool) {
    painter
        .stroke(bar.inset(-1.0), &Stroke::new(2.0), color(palette.ring))
        .draw();
    if filled {
        painter
            .stroke(
                bar.inset(-0.5),
                &Stroke::new(1.0),
                color(palette.ring_inner),
            )
            .draw();
    }
}

/// The window's default properties for `palette`: Masonry's own set, the
/// [`Colors`] of every widget of this crate, and the labels' text color
/// (a dialog's lines, on the dialog's card; the title, status and jobs
/// lines get theirs from the driver).
pub fn default_properties(palette: &Arc<Palette>) -> DefaultProperties {
    use super::{buttons, choice, field, modal, panel, progress, push, root, viewer};
    let mut props = masonry::theme::default_property_set();
    let colors = Colors(Arc::clone(palette));
    props.insert::<root::Root, _>(colors.clone());
    props.insert::<root::Region, _>(colors.clone());
    props.insert::<panel::PanelBox, _>(colors.clone());
    props.insert::<choice::ChoiceBox, _>(colors.clone());
    props.insert::<field::Field, _>(colors.clone());
    props.insert::<modal::Modal, _>(colors.clone());
    props.insert::<buttons::ButtonRow, _>(colors.clone());
    props.insert::<push::PushButton, _>(colors.clone());
    props.insert::<progress::JobProgressBar, _>(colors.clone());
    props.insert::<viewer::ViewerBox, _>(colors);
    props.insert::<masonry::widgets::Label, _>(ContentColor::new(color(
        palette.look(PanelRole::Dialog).fg,
    )));
    props
}

/// The text layout of `text` in the interface font, unbroken (a row is
/// one line; the window cuts it at the panel's edge).
pub fn line_layout(
    ctx: &mut LayoutCtx<'_>,
    text: &str,
    size: f32,
    bold: bool,
) -> Layout<BrushIndex> {
    let (fcx, lcx) = ctx.text_contexts();
    let attrs = if bold { Attrs::BOLD } else { Attrs::NONE };
    layout_with(fcx, lcx, text, size, attrs)
}

/// The same with a role's attributes: bold, italic, underlined.
pub fn styled_layout(
    ctx: &mut LayoutCtx<'_>,
    text: &str,
    size: f32,
    attrs: Attrs,
) -> Layout<BrushIndex> {
    let (fcx, lcx) = ctx.text_contexts();
    layout_with(fcx, lcx, text, size, attrs)
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
    let attrs = if bold { Attrs::BOLD } else { Attrs::NONE };
    layout_with(fcx, lcx, text, size, attrs)
}

fn layout_with(
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<BrushIndex>,
    text: &str,
    size: f32,
    attrs: Attrs,
) -> Layout<BrushIndex> {
    let mut builder = lcx.ranged_builder(fcx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
        FONT_STACK.into(),
    )));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.3)));
    if attrs.bold {
        builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
    }
    if attrs.italic {
        builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
    }
    if attrs.underline {
        builder.push_default(StyleProperty::Underline(true));
    }
    builder.push_default(StyleProperty::Brush(BrushIndex(0)));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

/// A run of a line drawn its own way: a byte range of the text, the brush
/// (an index into the colors given when drawing), bold, underlined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// The first byte.
    pub start: usize,
    /// The byte after the run.
    pub end: usize,
    /// The brush's index.
    pub brush: usize,
    /// Bold.
    pub bold: bool,
    /// Underlined.
    pub underline: bool,
}

/// The text layout of a line with runs drawn their own way, in the
/// interface font, unbroken; brush 0 is the line's own color.
pub fn runs_layout(
    ctx: &mut LayoutCtx<'_>,
    text: &str,
    size: f32,
    runs: &[Run],
) -> Layout<BrushIndex> {
    let (fcx, lcx) = ctx.text_contexts();
    let mut builder = lcx.ranged_builder(fcx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
        FONT_STACK.into(),
    )));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.3)));
    builder.push_default(StyleProperty::Brush(BrushIndex(0)));
    for run in runs {
        let (start, end) = (run.start.min(text.len()), run.end.min(text.len()));
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        builder.push(StyleProperty::Brush(BrushIndex(run.brush)), start..end);
        if run.bold {
            builder.push(StyleProperty::FontWeight(FontWeight::BOLD), start..end);
        }
        if run.underline {
            builder.push(StyleProperty::Underline(true), start..end);
        }
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

/// How a token class's run is drawn in the viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TokenLook {
    /// The text color.
    pub color: Color,
    /// Bold (keywords and headings by nature).
    pub bold: bool,
    /// Underlined (links by nature; a reversed problem too, since the
    /// window draws no reverse).
    pub underline: bool,
}

/// The look of a token role in the window's palette: the role's color,
/// the owner's `[colors]` in place, where it keeps the theme's floor on
/// the page, else the text color (the palette's rule for every role);
/// and the role's shape, so a class shows by its shape too, never by
/// color alone.
pub fn token_look(palette: &Palette, role: PanelRole) -> TokenLook {
    let look = palette.look(role);
    TokenLook {
        color: color(look.fg),
        bold: look.attrs.bold,
        underline: look.attrs.underline || look.attrs.reverse,
    }
}

/// The contrast ratio of two colors, by WCAG's relative luminance.
pub fn contrast(a: Color, b: Color) -> f64 {
    let rgb = |c: Color| {
        let rgba = c.to_rgba8();
        Rgb::new(rgba.r, rgba.g, rgba.b)
    };
    diroxide_theme::contrast_ratio(rgb(a), rgb(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_look_of_every_theme_meets_the_floor_on_the_page() {
        let none = std::collections::BTreeMap::new();
        for theme in diroxide_theme::builtin::all() {
            let palette = Palette::from_theme(theme, &none);
            let floor = if theme.kind() == diroxide_theme::ThemeKind::HighContrast {
                7.0
            } else {
                4.5
            };
            for role in PanelRole::TOKENS {
                let look = token_look(&palette, role);
                let ratio = contrast(look.color, color(palette.page));
                assert!(ratio >= floor, "{} {role:?}: {ratio:.2} to 1", theme.name());
            }
        }
        // Keywords keep their bold, links and problems their underline.
        let galaxy = Palette::default();
        assert!(token_look(&galaxy, PanelRole::TokenKeyword).bold);
        assert!(token_look(&galaxy, PanelRole::TokenLink).underline);
        assert!(token_look(&galaxy, PanelRole::TokenProblem).underline);
    }

    #[test]
    fn the_window_properties_carry_the_palette() {
        let palette = Arc::new(Palette::default());
        let props = default_properties(&palette);
        let _ = props;
        assert_eq!(Colors::default().0.key, "galaxy");
    }
}
