// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! star's 23 palettes (`star/themes.py`, `BUILT_IN_PALETTES`), their
//! terminal error colors (`star/tui/theming.py`), and the port that turns
//! each into a complete theme.
//!
//! The eleven palette keys map to roles as star's CSS template used them:
//! `bg` → background, `fg` → text, `sel` → the selection and spoken-sentence
//! band, `h1`–`h4` → headings 1–4 (5 and 6 follow 4, as in star's CSS),
//! `code`, `code_bg` → code and code background (and panels), `link`, and
//! `muted` → dim text and quotes. star had no palette key for errors; the
//! terminal UI's `err` color is used where it had one (the fourteen
//! community themes), and otherwise the color star's hand-written terminal
//! tables used (magenta, avoiding red and green, for `dark`, `light` and
//! `contrast`; green for the monochrome `phosphor`) or the scheme's own red.
//!
//! The owner's policy (2026-09-26): not every theme has to meet WCAG AA, as long
//! as some do. The themes in [`MUST_MEET_AA`] (Galaxy, Galaxy Light, and the
//! two high-contrast themes) keep every value that already meets the
//! contrast floor and nudge only the ones that fail, by the smallest
//! lightness change that passes ([`crate::color::adjust_lightness`]). Every
//! other palette keeps star's colors exactly; its file says whether it meets
//! AA. [`port`] returns each change.

use crate::color::Rgb;
use crate::error::ThemeError;
use crate::file::{StyleFile, ThemeFile};
use crate::model::{ColorRole, StyleRole, Theme, ThemeKind};
use crate::resolve::{Adjustment, Repair, resolve};

/// One of star's palettes, verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StarPalette {
    /// Theme name.
    pub name: &'static str,
    /// Page background.
    pub bg: Rgb,
    /// Body text.
    pub fg: Rgb,
    /// Selection.
    pub sel: Rgb,
    /// Heading 1.
    pub h1: Rgb,
    /// Heading 2.
    pub h2: Rgb,
    /// Heading 3.
    pub h3: Rgb,
    /// Heading 4 (and 5, 6).
    pub h4: Rgb,
    /// Code text.
    pub code: Rgb,
    /// Code background.
    pub code_bg: Rgb,
    /// Links.
    pub link: Rgb,
    /// Quotes, rules, borders.
    pub muted: Rgb,
}

const fn p(
    name: &'static str,
    [bg, fg, sel, h1, h2, h3, h4, code, code_bg, link, muted]: [u32; 11],
) -> StarPalette {
    StarPalette {
        name,
        bg: Rgb::from_u32(bg),
        fg: Rgb::from_u32(fg),
        sel: Rgb::from_u32(sel),
        h1: Rgb::from_u32(h1),
        h2: Rgb::from_u32(h2),
        h3: Rgb::from_u32(h3),
        h4: Rgb::from_u32(h4),
        code: Rgb::from_u32(code),
        code_bg: Rgb::from_u32(code_bg),
        link: Rgb::from_u32(link),
        muted: Rgb::from_u32(muted),
    }
}

/// star's palettes in star's cycle order (galaxy first; it is the default).
/// Key order: bg, fg, sel, h1, h2, h3, h4, code, code_bg, link, muted.
#[rustfmt::skip]
pub const PALETTES: [StarPalette; 23] = [
    p("galaxy",           [0x1e1e1e, 0xdadada, 0x483d6b, 0xc9b6ff, 0xa98eff, 0x8aa2ff, 0x7bd0c1, 0xe0a87a, 0x2a2a2a, 0xa882ff, 0x7d7d7d]),
    p("galaxy-light",     [0xffffff, 0x2e3338, 0xd8caff, 0x6c3fd6, 0x8257e5, 0x3a6df0, 0x1a9e8f, 0xb5673a, 0xf2f0f9, 0x7c3aed, 0x8a8f98]),
    p("one-dark",         [0x282c34, 0xabb2bf, 0x3e4451, 0x61afef, 0xc678dd, 0x56b6c2, 0x98c379, 0xe5c07b, 0x21252b, 0x61afef, 0x5c6370]),
    p("one-light",        [0xfafafa, 0x383a42, 0xdbdbdc, 0x4078f2, 0xa626a4, 0x0184bc, 0x50a14f, 0xc18401, 0xeaeaeb, 0x4078f2, 0xa0a1a7]),
    p("dark",             [0x16181d, 0xc6ccd4, 0x2c313a, 0x82aaff, 0x89ddff, 0xc792ea, 0xf78c6c, 0x7fdbab, 0x1e2128, 0x82aaff, 0x5c6370]),
    p("light",            [0xfafafa, 0x24273a, 0xbcd0f0, 0x1e66f5, 0x209fb5, 0x8839ef, 0xe64553, 0x40a02b, 0xeceff4, 0x1e66f5, 0x8c8fa1]),
    p("contrast",         [0x000000, 0xffffff, 0x404040, 0xffff00, 0x00ffff, 0xff80ff, 0x80ff80, 0x00ff80, 0x1a1a1a, 0x00ffff, 0xc0c0c0]),
    p("high-contrast",    [0x000000, 0xffffff, 0x2a2a2a, 0xffe14d, 0x5fe3ff, 0xff9ce0, 0x7dffb0, 0xffbf66, 0x141414, 0x8ac6ff, 0xc0c0c0]),
    p("phosphor",         [0x001200, 0x00cc00, 0x004400, 0x00ff00, 0x00ee00, 0x00cc00, 0x00aa00, 0x009900, 0x002600, 0x00ff00, 0x008800]),
    p("dracula",          [0x282a36, 0xf8f8f2, 0x44475a, 0xbd93f9, 0xff79c6, 0x8be9fd, 0x50fa7b, 0xf1fa8c, 0x21222c, 0x8be9fd, 0x9aa0b9]),
    p("nord",             [0x2e3440, 0xeceff4, 0x434c5e, 0x88c0d0, 0x81a1c1, 0x8fbcbb, 0xa3be8c, 0xebcb8b, 0x3b4252, 0x88c0d0, 0x94a0b8]),
    p("solarized-dark",   [0x002b36, 0x93a1a1, 0x073642, 0x4aa2df, 0x2aa198, 0x859900, 0xb58900, 0xf0784a, 0x073642, 0x4aa2df, 0x82989f]),
    p("solarized-light",  [0xfdf6e3, 0x586e75, 0xeee8d5, 0x1b6194, 0x1b6b65, 0x566500, 0x7f6100, 0xb34212, 0xeee8d5, 0x1b6194, 0x5e7070]),
    p("gruvbox-dark",     [0x282828, 0xebdbb2, 0x504945, 0xfabd2f, 0xfe8019, 0x8ec07c, 0x83a598, 0xb8bb26, 0x3c3836, 0x83a598, 0xa89984]),
    p("tokyo-night",      [0x1a1b26, 0xc0caf5, 0x33467c, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0x9ece6a, 0xe0af68, 0x24283b, 0x7aa2f7, 0x858dbb]),
    p("catppuccin-mocha", [0x1e1e2e, 0xcdd6f4, 0x45475a, 0xcba6f7, 0xf5c2e7, 0x89dceb, 0xa6e3a1, 0xfab387, 0x181825, 0x89b4fa, 0x9399b2]),
    p("monokai",          [0x272822, 0xf8f8f2, 0x49483e, 0x66d9ef, 0xa6e22e, 0xfd971f, 0xae81ff, 0xe6db74, 0x1e1f1c, 0x66d9ef, 0xa59f85]),
    p("sepia",            [0xf4ecd8, 0x3d2f1f, 0xe0d2b4, 0x8a4510, 0x6b4a20, 0x4f5d2f, 0x175e63, 0x7a3b12, 0xeadfc6, 0x14607a, 0x6d5c42]),
    p("amber",            [0x171204, 0xf0b458, 0x3f3010, 0xffcf70, 0xf5a832, 0xe0953a, 0xd4b96a, 0xffd98a, 0x241b08, 0xffc04d, 0xb08448]),
    p("everforest-dark",  [0x2d353b, 0xd3c6aa, 0x475258, 0xa7c080, 0x83c092, 0x7fbbb3, 0xdbbc7f, 0xe69875, 0x232a2e, 0x7fbbb3, 0xa4b0a0]),
    p("rose-pine",        [0x191724, 0xe0def4, 0x403d52, 0xebbcba, 0x9ccfd8, 0xc4a7e7, 0xf6c177, 0xf6c177, 0x26233a, 0x9ccfd8, 0x908caa]),
    p("kanagawa",         [0x1f1f28, 0xdcd7ba, 0x2d4f67, 0x7e9cd8, 0x98bb6c, 0x7aa89f, 0xe6c384, 0xffa066, 0x16161d, 0x7fb4ca, 0x9c9a90]),
    p("gruvbox-light",    [0xfbf1c7, 0x3c3836, 0xd5c4a1, 0x8f5c0a, 0xaf3a03, 0x38684a, 0x076678, 0x635e0a, 0xebdbb2, 0x076678, 0x665c54]),
];

/// Where a theme's error color comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorSource {
    /// star's terminal `err` color, an xterm-256 index.
    StarTerminal(u8),
    /// star's hand-written terminal table (magenta or green).
    StarTable(Rgb),
    /// The scheme's own red; star had none.
    Scheme(Rgb),
}

impl ErrorSource {
    /// The color.
    pub fn color(self) -> Rgb {
        match self {
            ErrorSource::StarTerminal(i) => crate::terminal::xterm_rgb(i),
            ErrorSource::StarTable(c) | ErrorSource::Scheme(c) => c,
        }
    }

    /// Provenance, for the theme file's comment.
    pub fn describe(self) -> String {
        match self {
            ErrorSource::StarTerminal(i) => {
                format!("star's terminal error color (xterm {i}, {})", self.color())
            }
            ErrorSource::StarTable(c) => {
                format!("the color star's terminal table used for errors ({c})")
            }
            ErrorSource::Scheme(c) => format!("the scheme's red ({c}); star had no error color"),
        }
    }
}

/// Metadata for one ported palette.
#[derive(Clone, Copy, Debug)]
pub struct StarMeta {
    /// Name read aloud.
    pub display_name: &'static str,
    /// Light, dark, or high contrast.
    pub kind: ThemeKind,
    /// One sentence.
    pub description: &'static str,
    /// Light or dark partner.
    pub counterpart: Option<&'static str>,
    /// Error color.
    pub error: ErrorSource,
}

/// Metadata for a star palette by name.
#[rustfmt::skip]
pub fn meta(name: &str) -> Option<StarMeta> {
    use ErrorSource::*;
    use ThemeKind::*;
    let m = |display_name, kind, description, counterpart, error| StarMeta {
        display_name,
        kind,
        description,
        counterpart,
        error,
    };
    Some(match name {
        "galaxy" => m("Galaxy", Dark, "Purple-accented dark theme; the default.", Some("galaxy-light"), Scheme(Rgb::from_u32(0xff7b93))),
        "galaxy-light" => m("Galaxy Light", Light, "Galaxy's light half: purple accents on white.", Some("galaxy"), Scheme(Rgb::from_u32(0xc0264b))),
        "one-dark" => m("One Dark", Dark, "The One Dark editor palette.", Some("one-light"), Scheme(Rgb::from_u32(0xe06c75))),
        "one-light" => m("One Light", Light, "The One Light editor palette.", Some("one-dark"), Scheme(Rgb::from_u32(0xe45649))),
        "dark" => m("Dark", Dark, "star's original dark theme.", Some("light"), StarTable(Rgb::from_u32(0xf07cd0))),
        "light" => m("Light", Light, "star's original light theme.", Some("dark"), StarTable(Rgb::from_u32(0xa1239b))),
        "contrast" => m("Contrast", HighContrast, "Pure yellow and cyan on black; star's original contrast theme.", None, StarTable(Rgb::from_u32(0xff80ff))),
        "high-contrast" => m("High Contrast", HighContrast, "Softened high contrast for low vision; every color at 7 to 1 or more.", None, Scheme(Rgb::from_u32(0xff9a8a))),
        "phosphor" => m("Phosphor", Dark, "Green phosphor monochrome.", None, StarTable(Rgb::from_u32(0x00ff00))),
        "dracula" => m("Dracula", Dark, "The Dracula palette.", None, StarTerminal(203)),
        "nord" => m("Nord", Dark, "The Nord arctic palette.", None, StarTerminal(131)),
        "solarized-dark" => m("Solarized Dark", Dark, "Solarized dark, nudged for contrast.", Some("solarized-light"), StarTerminal(160)),
        "solarized-light" => m("Solarized Light", Light, "Solarized light, nudged for contrast.", Some("solarized-dark"), StarTerminal(160)),
        "gruvbox-dark" => m("Gruvbox Dark", Dark, "Retro warm dark palette.", Some("gruvbox-light"), StarTerminal(203)),
        "tokyo-night" => m("Tokyo Night", Dark, "The Tokyo Night palette.", None, StarTerminal(210)),
        "catppuccin-mocha" => m("Catppuccin Mocha", Dark, "The Catppuccin Mocha pastel palette.", None, StarTerminal(211)),
        "monokai" => m("Monokai", Dark, "The Monokai palette.", None, StarTerminal(197)),
        "sepia" => m("Sepia", Light, "Warm paper tones for long reading.", None, StarTerminal(124)),
        "amber" => m("Amber", Dark, "Low-blue amber on near black, phosphor's warm cousin.", None, StarTerminal(203)),
        "everforest-dark" => m("Everforest Dark", Dark, "Soft green forest palette.", None, StarTerminal(167)),
        "rose-pine" => m("Rose Pine", Dark, "Muted rose and pine palette.", None, StarTerminal(211)),
        "kanagawa" => m("Kanagawa", Dark, "Palette after Hokusai's wave.", None, StarTerminal(210)),
        "gruvbox-light" => m("Gruvbox Light", Light, "Gruvbox's light half.", Some("gruvbox-dark"), StarTerminal(124)),
        _ => return None,
    })
}

/// star's pre-0.1.27 theme names and textweaver's earlier spellings, mapped
/// to current names.
pub const ALIASES: [(&str, &str); 6] = [
    ("obsidian", "galaxy"),
    ("obsidian-light", "galaxy-light"),
    ("zed-one-dark", "one-dark"),
    ("zed-one-light", "one-light"),
    ("high_contrast", "high-contrast"),
    ("highcontrast", "high-contrast"),
];

/// The theme file a palette starts from, before resolving: star's values as
/// explicit keys, everything else left to the resolver.
pub fn to_file(p: &StarPalette) -> ThemeFile {
    let m = meta(p.name);
    let mut f = ThemeFile {
        name: Some(p.name.to_owned()),
        display_name: m.map(|m| m.display_name.to_owned()),
        kind: m.map(|m| m.kind),
        description: m.map(|m| m.description.to_owned()),
        author: Some("star palettes (star/themes.py); ported to textweaver".to_owned()),
        origin: Some("star".to_owned()),
        counterpart: m.and_then(|m| m.counterpart).map(str::to_owned),
        ..ThemeFile::default()
    };
    use ColorRole as C;
    for (role, c) in [
        (C::Background, p.bg),
        (C::Text, p.fg),
        (C::DimText, p.muted),
        (C::Heading1, p.h1),
        (C::Heading2, p.h2),
        (C::Heading3, p.h3),
        (C::Heading4, p.h4),
        (C::Link, p.link),
        (C::Code, p.code),
        (C::CodeBackground, p.code_bg),
    ] {
        f.colors[role.index()] = Some(c);
    }
    if let Some(m) = m {
        f.colors[C::Error.index()] = Some(m.error.color());
    }
    f.styles[StyleRole::Selection.index()] = StyleFile {
        background: Some(Some(p.sel)),
        ..StyleFile::default()
    };
    f
}

/// The built-in themes that must meet WCAG AA (the two high-contrast ones
/// at 7:1). The contrast test gates only these; the rest are labelled.
pub const MUST_MEET_AA: [&str; 4] = ["galaxy", "galaxy-light", "contrast", "high-contrast"];

/// True for a theme in [`MUST_MEET_AA`].
pub fn must_meet_aa(name: &str) -> bool {
    MUST_MEET_AA.contains(&name)
}

/// Ports one palette: the complete theme and every color changed to meet
/// the contrast floor. Only the themes in [`MUST_MEET_AA`] have star's own
/// colors changed; the others keep them exactly.
pub fn port(p: &StarPalette) -> Result<(Theme, Vec<Adjustment>), ThemeError> {
    let repair = if must_meet_aa(p.name) {
        Repair::Explicit
    } else {
        Repair::DerivedOnly
    };
    resolve(&to_file(p), None, None, repair)
}

/// The palette with this name.
pub fn palette(name: &str) -> Option<&'static StarPalette> {
    PALETTES.iter().find(|p| p.name == name)
}

/// The built-in theme file for a palette, exactly as shipped in
/// `crates/diroxide-theme/themes/`: a comment header recording the source
/// and every adjustment, then the complete theme.
pub fn generated_file(p: &StarPalette) -> Result<String, ThemeError> {
    let (theme, adjustments) = port(p)?;
    let mut s = format!(
        "# {}: {}\n#\n# Ported from star's `{}` palette (star/themes.py).\n",
        theme.meta.display_name, theme.meta.description, p.name
    );
    if let Some(m) = meta(p.name) {
        s.push_str(&format!("# Error color: {}.\n", m.error.describe()));
    }
    if !must_meet_aa(p.name) {
        let report = crate::check::check(&theme);
        let failing = report.checks.iter().filter(|c| !c.passed).count();
        if failing == 0 {
            s.push_str("# star's colors, unchanged. Meets WCAG AA.\n");
        } else {
            s.push_str(&format!(
                "# star's colors, unchanged. Does not meet WCAG AA: {failing} of {} contrast checks fall short.\n",
                report.checks.len()
            ));
        }
    } else if adjustments.is_empty() {
        s.push_str("# Every star color meets the contrast floor unchanged.\n");
    } else {
        s.push_str(
            "# Changed to meet the contrast floor (smallest lightness change that passes):\n",
        );
        for a in &adjustments {
            s.push_str(&format!(
                "#   {}: {} -> {} ({} -> {} against {}, needs {})\n",
                a.key,
                a.from,
                a.to,
                crate::color::spoken_ratio(a.ratio_before),
                crate::color::spoken_ratio(a.ratio_after),
                a.against,
                crate::color::spoken_ratio(a.minimum),
            ));
        }
    }
    s.push_str(
        "#\n# Generated by `cargo run -p diroxide-theme --example generate_builtin_themes`,\n\
         # from textweaver's theme crate, relicensed by its author (Apache-2.0 OR MIT).\n\
         # To make your own theme, copy this file into your themes folder under a new\n\
         # name and change theme.name; see docs/themes.md.\n\n",
    );
    s.push_str(&theme.to_toml_string());
    Ok(s)
}

/// Every adjustment made while porting star's palettes, as a Markdown table
/// (theme, key, original color, new color, contrast before and after).
pub fn adjustments_table() -> Result<String, ThemeError> {
    let mut s = String::from(
        "| Theme | Key | Original | textweaver | Before | After | Against | Floor |\n\
         |---|---|---|---|---|---|---|---|\n",
    );
    for p in &PALETTES {
        let (_, adj) = port(p)?;
        for a in adj {
            s.push_str(&format!(
                "| {} | `{}` | `{}` | `{}` | {:.2} | {:.2} | `{}` | {} |\n",
                p.name, a.key, a.from, a.to, a.ratio_before, a.ratio_after, a.against, a.minimum
            ));
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_has_metadata() {
        for p in &PALETTES {
            let m = meta(p.name).unwrap_or_else(|| panic!("{}", p.name));
            if let Some(c) = m.counterpart {
                assert_eq!(meta(c).and_then(|m| m.counterpart), Some(p.name));
            }
            let light = p.bg.relative_luminance() > p.fg.relative_luminance();
            assert_eq!(light, m.kind == ThemeKind::Light, "{}", p.name);
        }
    }

    #[test]
    fn star_values_survive_the_port_where_they_pass() {
        let p = palette("dracula").unwrap();
        let (t, _) = port(p).unwrap();
        assert_eq!(t.color(ColorRole::Background), p.bg);
        assert_eq!(t.color(ColorRole::Text), p.fg);
        assert_eq!(t.color(ColorRole::Heading1), p.h1);
        assert_eq!(t.color(ColorRole::Heading5), t.color(ColorRole::Heading4));
        assert_eq!(t.style(StyleRole::Selection).background, Some(p.sel));
    }
}
