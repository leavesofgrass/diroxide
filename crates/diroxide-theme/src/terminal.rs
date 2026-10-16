// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Terminal styles for a theme, independent of any TUI library.
//!
//! Four levels, chosen by [`ColorSupport::detect`]:
//!
//! - **Truecolor**: the theme's exact colors.
//! - **256 colors**: each color becomes the nearest xterm-256 cube or gray
//!   index (16–255, whose RGB values are the same in every terminal) that
//!   still meets the pair's contrast floor, so quantizing never turns a
//!   passing pair into a failing one.
//! - **16 colors**: the base palette has no fixed RGB (every terminal and
//!   user theme differs), so the page is painted explicitly (index 0 for
//!   dark themes, 15 for light) and every color is chosen from the indexes
//!   that pass against it in three reference palettes (xterm, VGA, and
//!   Windows Terminal's Campbell), allowing for terminals that brighten bold
//!   text. Bold is swapped for underline where brightening would break
//!   contrast. Faint bands (under 3:1 against the page) are dropped and the
//!   highlight's attributes carry it; strong bands use dark indexes with
//!   bright white text.
//! - **No color** (`NO_COLOR`, `TERM=dumb`): no colors at all; every role
//!   keeps its attributes and highlights use bold, underline, and reverse
//!   video ([`StyleRole::no_color_attributes`]).
//!
//! Colors are reduced by class ([`RoleClass`]): text keeps the text floor,
//! indicators 3 to 1, and the derived roles that sit on a fill (text on the
//! accent, a raised control, the reading ruler) keep their floor on that
//! fill. Decorative lines only keep their color's character.
//!
//! In every level, each highlight carries at least one attribute, so no
//! state depends on color alone. No level emits sequences that change the
//! terminal's own palette or cursor.

use crate::check::{Requirement, fill, minimum, role_minimum};
use crate::color::{Rgb, contrast_ratio};
use crate::model::{Attrs, ColorRole, RoleClass, Style, StyleRole, Theme, USER_HIGHLIGHT_NO_COLOR};

/// How many colors the terminal can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSupport {
    /// No color: attributes only.
    NoColor,
    /// The 16 base colors, whose RGB values the terminal decides.
    Ansi16,
    /// The xterm 256-color palette.
    Ansi256,
    /// 24-bit color.
    TrueColor,
}

impl ColorSupport {
    /// Detects support from the process environment. See [`Self::from_env`].
    pub fn detect() -> Self {
        Self::from_env(|k| std::env::var(k).ok(), cfg!(windows))
    }

    /// Detects support from environment variables, in order:
    /// `DIRO_COLOR` (`none`, `16`, `256`, or `truecolor`) overrides
    /// everything; a non-empty `NO_COLOR` or `TERM=dumb` turns color off;
    /// `COLORTERM=truecolor|24bit`, `WT_SESSION` (Windows Terminal), a
    /// truecolor `TERM_PROGRAM`, or a `-direct` `TERM` means truecolor; a
    /// `256color` `TERM` means 256; any other `TERM` means 16. With no `TERM`,
    /// Windows consoles (which have handled 24-bit color since Windows 10)
    /// get truecolor and anything else 16.
    pub fn from_env(var: impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        if let Some(v) = var("DIRO_COLOR") {
            match v.trim().to_ascii_lowercase().as_str() {
                "none" | "no" | "off" | "never" | "0" => return ColorSupport::NoColor,
                "16" | "ansi" | "ansi16" | "basic" => return ColorSupport::Ansi16,
                "256" | "ansi256" => return ColorSupport::Ansi256,
                "truecolor" | "24bit" | "rgb" | "full" => return ColorSupport::TrueColor,
                _ => {}
            }
        }
        if var("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return ColorSupport::NoColor;
        }
        let term = var("TERM").unwrap_or_default().to_ascii_lowercase();
        if term == "dumb" {
            return ColorSupport::NoColor;
        }
        let colorterm = var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        if colorterm == "truecolor" || colorterm == "24bit" || var("WT_SESSION").is_some() {
            return ColorSupport::TrueColor;
        }
        if let Some(p) = var("TERM_PROGRAM")
            && matches!(
                p.as_str(),
                "iTerm.app" | "WezTerm" | "vscode" | "ghostty" | "Hyper" | "rio" | "Tabby"
            )
        {
            return ColorSupport::TrueColor;
        }
        if term.ends_with("-direct") {
            return ColorSupport::TrueColor;
        }
        if term.contains("256color") {
            return ColorSupport::Ansi256;
        }
        if term.is_empty() && windows {
            return ColorSupport::TrueColor;
        }
        ColorSupport::Ansi16
    }
}

/// A color as sent to the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TermColor {
    /// 24-bit color (`38;2;r;g;b`).
    Rgb(Rgb),
    /// A palette index (`38;5;n`); 0–15 are the base colors.
    Indexed(u8),
}

impl TermColor {
    /// The RGB shown, taking base colors 0–15 from `base` (for example
    /// [`XTERM_16`]).
    pub fn rgb(self, base: &[Rgb; 16]) -> Rgb {
        match self {
            TermColor::Rgb(c) => c,
            TermColor::Indexed(i) if i < 16 => base[usize::from(i)],
            TermColor::Indexed(i) => xterm_rgb(i),
        }
    }
}

/// A terminal style: colors are `None` to leave what is underneath.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TermStyle {
    /// Foreground.
    pub fg: Option<TermColor>,
    /// Background.
    pub bg: Option<TermColor>,
    /// Attributes to add.
    pub attrs: Attrs,
}

impl TermStyle {
    /// `other` drawn over `self`: its colors win where set, attributes add up.
    pub fn patch(self, other: TermStyle) -> TermStyle {
        TermStyle {
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
            attrs: self.attrs.with(other.attrs),
        }
    }
}

/// xterm's default base colors.
pub const XTERM_16: [Rgb; 16] = rgbs([
    0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
    0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
]);
/// The VGA and Linux console base colors.
pub const VGA_16: [Rgb; 16] = rgbs([
    0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa, 0x555555,
    0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
]);
/// Windows Terminal's default ("Campbell") base colors.
pub const CAMPBELL_16: [Rgb; 16] = rgbs([
    0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc, 0x767676,
    0xe74856, 0x16c60c, 0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2,
]);
/// The palettes the 16-color level must pass in.
pub const REFERENCE_PALETTES: [&[Rgb; 16]; 3] = [&XTERM_16, &VGA_16, &CAMPBELL_16];

const fn rgbs(v: [u32; 16]) -> [Rgb; 16] {
    let mut out = [Rgb::BLACK; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Rgb::from_u32(v[i]);
        i += 1;
    }
    out
}

/// The RGB of an xterm-256 index: 0–15 xterm's defaults, 16–231 the
/// 6×6×6 cube on levels 0, 95, 135, 175, 215, 255, and 232–255 the gray
/// ramp at 8 + 10 i.
pub fn xterm_rgb(index: u8) -> Rgb {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => XTERM_16[usize::from(index)],
        16..=231 => {
            let i = index - 16;
            Rgb::new(
                LEVELS[usize::from(i / 36)],
                LEVELS[usize::from(i / 6 % 6)],
                LEVELS[usize::from(i % 6)],
            )
        }
        _ => {
            let v = 8 + 10 * (index - 232);
            Rgb::new(v, v, v)
        }
    }
}

/// A color chosen by the reader (a setting, not a theme role) at the
/// terminal's color level: itself in truecolor, the nearest of indexes 16
/// to 255 in 256 colors, and `None` with 16 colors or none, where the
/// reading aids keep their attributes only.
pub fn term_color(rgb: Rgb, support: ColorSupport) -> Option<TermColor> {
    match support {
        ColorSupport::TrueColor => Some(TermColor::Rgb(rgb)),
        ColorSupport::Ansi256 => Some(TermColor::Indexed(nearest_256(rgb, None, 0.0))),
        ColorSupport::Ansi16 | ColorSupport::NoColor => None,
    }
}

/// The index in 16–255 nearest to `target` whose contrast against
/// `against` is at least `min`; the nearest overall when none is.
fn nearest_256(target: Rgb, against: Option<Rgb>, min: f64) -> u8 {
    // OKLab and luminance of indexes 16–255, computed once per process.
    static TABLE: std::sync::OnceLock<Vec<([f64; 3], f64)>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (16..=255u8)
            .map(|i| {
                let c = xterm_rgb(i);
                (c.to_oklab(), c.relative_luminance())
            })
            .collect()
    });
    let [tl, ta, tb] = target.to_oklab();
    let against_l = against.map(Rgb::relative_luminance);
    let mut best: Option<(f64, u8)> = None;
    let mut nearest: (f64, u8) = (f64::MAX, 16);
    for (i, &([l, a, b], lum)) in (16..=255u8).zip(table.iter()) {
        // Squared distance orders the same as distance.
        let d = (l - tl).powi(2) + (a - ta).powi(2) + (b - tb).powi(2);
        if d < nearest.0 {
            nearest = (d, i);
        }
        let ok = against_l.is_none_or(|al| {
            let (hi, lo) = if lum >= al { (lum, al) } else { (al, lum) };
            (hi + 0.05) / (lo + 0.05) >= min
        });
        if ok && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.unwrap_or(nearest).1
}

/// The worst contrast of base index `fg` on base index `bg` across the
/// reference palettes, counting the brightened variant when `bold`.
pub fn worst_16(fg: u8, bg: u8, bold: bool) -> f64 {
    let mut worst = f64::MAX;
    for pal in REFERENCE_PALETTES {
        let b = pal[usize::from(bg)];
        let mut fgs = vec![fg];
        if bold && fg < 8 {
            fgs.push(fg + 8);
        }
        for f in fgs {
            worst = worst.min(contrast_ratio(pal[usize::from(f)], b));
        }
    }
    worst
}

/// A theme resolved for one level of color support. Build once per theme
/// change; lookups are array reads.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTheme {
    /// The level this was built for.
    pub support: ColorSupport,
    /// The theme's name.
    pub name: String,
    page: TermStyle,
    colors: [TermStyle; ColorRole::COUNT],
    styles: [TermStyle; StyleRole::COUNT],
    user: Vec<(String, TermStyle)>,
}

impl TerminalTheme {
    /// Resolves `theme` for `support`.
    pub fn new(theme: &Theme, support: ColorSupport) -> Self {
        match support {
            ColorSupport::NoColor => Self::no_color(theme),
            ColorSupport::TrueColor => Self::truecolor(theme),
            ColorSupport::Ansi256 => Self::ansi256(theme),
            ColorSupport::Ansi16 => Self::ansi16(theme),
        }
    }

    /// Body text on the page. Draw everything on this, then patch roles over
    /// it.
    pub fn page(&self) -> TermStyle {
        self.page
    }

    /// A color role. Text roles set only the foreground (and their fixed
    /// attributes); `Background` is the page, `Surface` is text on panels,
    /// and `CodeBackground` is code on its background.
    pub fn color(&self, role: ColorRole) -> TermStyle {
        self.colors[role.index()]
    }

    /// A style role. Highlights always carry at least one attribute.
    pub fn style(&self, role: StyleRole) -> TermStyle {
        self.styles[role.index()]
    }

    /// The user highlight at `index` in the theme's list.
    pub fn user_highlight(&self, index: usize) -> Option<TermStyle> {
        self.user.get(index).map(|(_, s)| *s)
    }

    /// The user highlight with this name (case-insensitive).
    pub fn user_highlight_named(&self, name: &str) -> Option<TermStyle> {
        self.user
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, s)| *s)
    }

    /// The names of the user highlights, in order.
    pub fn user_highlight_names(&self) -> impl Iterator<Item = &str> {
        self.user.iter().map(|(n, _)| n.as_str())
    }

    fn build(
        theme: &Theme,
        support: ColorSupport,
        mut color: impl FnMut(ColorRole) -> TermStyle,
        mut style: impl FnMut(&Style, Attrs) -> TermStyle,
    ) -> Self {
        let page = color(ColorRole::Background);
        let colors = std::array::from_fn(|i| color(ColorRole::ALL[i]));
        let styles = std::array::from_fn(|i| {
            let r = StyleRole::ALL[i];
            style(theme.style(r), r.no_color_attributes())
        });
        let user = theme
            .user_highlights
            .iter()
            .map(|h| (h.name.clone(), style(&h.style, USER_HIGHLIGHT_NO_COLOR)))
            .collect();
        TerminalTheme {
            support,
            name: theme.meta.name.clone(),
            page,
            colors,
            styles,
            user,
        }
    }

    fn no_color(theme: &Theme) -> Self {
        Self::build(
            theme,
            ColorSupport::NoColor,
            |r| TermStyle {
                attrs: r.attributes(),
                ..TermStyle::default()
            },
            |_, fallback| TermStyle {
                attrs: fallback,
                ..TermStyle::default()
            },
        )
    }

    fn truecolor(theme: &Theme) -> Self {
        use ColorRole as C;
        Self::build(
            theme,
            ColorSupport::TrueColor,
            |r| {
                let rgb = |r: C| TermColor::Rgb(theme.color(r));
                let (fg, on) = fill(r);
                TermStyle {
                    fg: Some(rgb(fg)),
                    bg: on.map(rgb),
                    attrs: if on.is_some() {
                        Attrs::NONE
                    } else {
                        r.attributes()
                    },
                }
            },
            |s, fallback| TermStyle {
                fg: Some(TermColor::Rgb(s.foreground)),
                bg: s.background.map(TermColor::Rgb),
                attrs: cue(s.attributes, fallback),
            },
        )
    }

    fn ansi256(theme: &Theme) -> Self {
        use ColorRole as C;
        let kind = theme.kind();
        let text_min = minimum(Requirement::Text, kind);
        let page_rgb = theme.color(C::Background);
        let page_i = nearest_256(page_rgb, None, 0.0);
        let page_q = xterm_rgb(page_i);
        // Keep the floor, but never demand more than the original pair had.
        let fg_on = |fg: Rgb, bg: Rgb, bg_q: Rgb| {
            let floor = text_min.min(contrast_ratio(fg, bg));
            nearest_256(fg, Some(bg_q), floor)
        };
        // Bands keep the lesser of 3 to 1 and their own step from the page.
        let fill_index = |bg: ColorRole| {
            if bg == C::Background {
                return page_i;
            }
            let b = theme.color(bg);
            if bg.class() == RoleClass::Band {
                nearest_256(b, Some(page_q), 3.0_f64.min(contrast_ratio(b, page_rgb)))
            } else {
                nearest_256(b, None, 0.0)
            }
        };
        Self::build(
            theme,
            ColorSupport::Ansi256,
            |r| {
                let (fg, on) = fill(r);
                let (bg_rgb, bi) = match on {
                    Some(b) => (theme.color(b), fill_index(b)),
                    None => (page_rgb, page_i),
                };
                let f = theme.color(fg);
                let floor = role_minimum(r, kind).min(contrast_ratio(f, bg_rgb));
                TermStyle {
                    fg: Some(TermColor::Indexed(nearest_256(
                        f,
                        Some(xterm_rgb(bi)),
                        floor,
                    ))),
                    bg: on.map(|_| TermColor::Indexed(bi)),
                    attrs: if on.is_some() {
                        Attrs::NONE
                    } else {
                        r.attributes()
                    },
                }
            },
            |s, fallback| {
                let (band_rgb, band_i) = match s.background {
                    Some(b) => {
                        let floor = 3.0_f64.min(contrast_ratio(b, page_rgb));
                        let i = nearest_256(b, Some(page_q), floor);
                        (b, Some(i))
                    }
                    None => (page_rgb, None),
                };
                let band_q = band_i.map_or(page_q, xterm_rgb);
                TermStyle {
                    fg: Some(TermColor::Indexed(fg_on(s.foreground, band_rgb, band_q))),
                    bg: band_i.map(TermColor::Indexed),
                    attrs: cue(s.attributes, fallback),
                }
            },
        )
    }

    fn ansi16(theme: &Theme) -> Self {
        use ColorRole as C;
        let kind = theme.kind();
        let min = minimum(Requirement::Text, kind);
        let dark = theme.is_dark();
        let page_i: u8 = if dark { 0 } else { 15 };
        let ink_i: u8 = 15;
        let page_rgb = theme.color(C::Background);
        // A text color on the page: the nearest base index that passes in
        // every reference palette, allowing for bold brightening; bold
        // becomes underline where no index survives it.
        let text = |target: Rgb, attrs: Attrs| -> (u8, Attrs) {
            let pick = |bold: bool| {
                (0..16u8)
                    .filter(|&i| i != page_i && worst_16(i, page_i, bold) >= min)
                    .min_by(|&a, &b| hue_distance(a, target).total_cmp(&hue_distance(b, target)))
            };
            if let Some(i) = pick(attrs.bold) {
                return (i, attrs);
            }
            let plain = Attrs {
                bold: false,
                underline: true,
                ..attrs
            };
            let fallback = if dark { 15 } else { 0 };
            (pick(false).unwrap_or(fallback), plain)
        };
        let (page_fg, _) = text(theme.color(C::Text), Attrs::NONE);
        let page = TermStyle {
            fg: Some(TermColor::Indexed(page_fg)),
            bg: Some(TermColor::Indexed(page_i)),
            attrs: Attrs::NONE,
        };
        // Strong bands: dark base colors under bright white text.
        let bands: Vec<u8> = (0..8u8)
            .filter(|&i| i != page_i && worst_16(ink_i, i, true) >= min)
            .collect();
        Self::build(
            theme,
            ColorSupport::Ansi16,
            |r| match r {
                C::Background | C::Surface | C::Raised => page,
                C::Ruler | C::RulerBand | C::OnAccent
                    if contrast_ratio(theme.color(fill_band(r)), page_rgb) >= 3.0
                        && !bands.is_empty() =>
                {
                    // A strong band: a dark base color under bright white.
                    let b = theme.color(fill_band(r));
                    let i = bands
                        .iter()
                        .copied()
                        .min_by(|&x, &y| hue_distance(x, b).total_cmp(&hue_distance(y, b)))
                        .unwrap_or(4);
                    TermStyle {
                        fg: Some(TermColor::Indexed(ink_i)),
                        bg: Some(TermColor::Indexed(i)),
                        attrs: Attrs::NONE,
                    }
                }
                // Faint bands are dropped: the page shows through.
                C::Ruler | C::RulerBand | C::OnAccent => page,
                C::CodeBackground => {
                    let (i, a) = text(theme.color(C::Code), Attrs::NONE);
                    TermStyle {
                        fg: Some(TermColor::Indexed(i)),
                        attrs: a,
                        ..page
                    }
                }
                _ => {
                    let (i, a) = text(theme.color(r), r.attributes());
                    TermStyle {
                        fg: Some(TermColor::Indexed(i)),
                        bg: None,
                        attrs: a,
                    }
                }
            },
            |s, fallback| {
                let attrs = cue(s.attributes, fallback);
                let strong = s
                    .background
                    .filter(|&b| contrast_ratio(b, page_rgb) >= 3.0 && !bands.is_empty());
                match strong {
                    Some(b) => {
                        let i = bands
                            .iter()
                            .copied()
                            .min_by(|&x, &y| hue_distance(x, b).total_cmp(&hue_distance(y, b)))
                            .unwrap_or(4);
                        TermStyle {
                            fg: Some(TermColor::Indexed(ink_i)),
                            bg: Some(TermColor::Indexed(i)),
                            attrs,
                        }
                    }
                    None => {
                        // Faint or no band: the page shows through and the
                        // attributes carry the highlight.
                        let (i, a) = text(s.foreground, attrs);
                        TermStyle {
                            fg: Some(TermColor::Indexed(i)),
                            bg: None,
                            attrs: a,
                        }
                    }
                }
            },
        )
    }
}

/// The fill a band-like role is measured by at 16 colors: the band itself,
/// or the accent for the text drawn on it.
fn fill_band(r: ColorRole) -> ColorRole {
    if r == ColorRole::OnAccent {
        ColorRole::Accent
    } else {
        r
    }
}

/// How far base index `i` (as xterm draws it) is from `target`, weighting
/// hue and chroma over lightness: the contrast filter has already settled
/// lightness, so the choice should keep the color's character (a gray stays
/// gray, a brown heading goes red rather than black).
fn hue_distance(i: u8, target: Rgb) -> f64 {
    let [l1, a1, b1] = XTERM_16[usize::from(i)].to_oklab();
    let [l2, a2, b2] = target.to_oklab();
    ((0.3 * (l1 - l2)).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt()
}

/// The theme's attributes, or the no-color set when it gave none.
fn cue(attrs: Attrs, fallback: Attrs) -> Attrs {
    if attrs.is_empty() { fallback } else { attrs }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn detection() {
        use ColorSupport::*;
        assert_eq!(ColorSupport::from_env(env(&[]), false), Ansi16);
        assert_eq!(ColorSupport::from_env(env(&[]), true), TrueColor);
        assert_eq!(
            ColorSupport::from_env(env(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]), true),
            NoColor
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("NO_COLOR", ""), ("TERM", "xterm-256color")]), false),
            Ansi256
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("TERM", "dumb")]), true),
            NoColor
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("TERM", "xterm"), ("COLORTERM", "24bit")]), false),
            TrueColor
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("WT_SESSION", "x"), ("TERM", "xterm")]), true),
            TrueColor
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("TERM", "linux")]), false),
            Ansi16
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("TERM", "xterm-direct")]), false),
            TrueColor
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("DIRO_COLOR", "256"), ("NO_COLOR", "1")]), false),
            Ansi256
        );
        assert_eq!(
            ColorSupport::from_env(env(&[("DIRO_COLOR", "none")]), true),
            NoColor
        );
    }

    #[test]
    fn xterm_table() {
        assert_eq!(xterm_rgb(16), Rgb::BLACK);
        assert_eq!(xterm_rgb(231), Rgb::WHITE);
        assert_eq!(xterm_rgb(208), Rgb::from_u32(0xff8700));
        assert_eq!(xterm_rgb(232), Rgb::from_u32(0x080808));
        assert_eq!(xterm_rgb(255), Rgb::from_u32(0xeeeeee));
        assert_eq!(xterm_rgb(203), Rgb::from_u32(0xff5f5f));
    }

    #[test]
    fn patch_layers() {
        let base = TermStyle {
            fg: Some(TermColor::Indexed(7)),
            bg: Some(TermColor::Indexed(0)),
            attrs: Attrs::BOLD,
        };
        let over = TermStyle {
            fg: None,
            bg: Some(TermColor::Indexed(4)),
            attrs: Attrs::UNDERLINE,
        };
        let p = base.patch(over);
        assert_eq!(p.fg, Some(TermColor::Indexed(7)));
        assert_eq!(p.bg, Some(TermColor::Indexed(4)));
        assert_eq!(p.attrs, Attrs::BOLD.with(Attrs::UNDERLINE));
    }
}
