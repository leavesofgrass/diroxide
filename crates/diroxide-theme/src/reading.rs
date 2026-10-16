// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The reader's own highlight colors (`[highlight] color` and
//! `sentence_color` in the settings), laid over a theme's spoken-word and
//! spoken-sentence styles.
//!
//! A color replaces the style's band (its background). The text inside the
//! band becomes whichever of the theme's text and page colors reads better
//! on it, and the style keeps its attributes, so the highlight is still not
//! shown by color alone and the word still differs from its sentence.
//! A color whose band leaves the text below the theme's contrast floor
//! (4.5 to 1, or 7 to 1 in high-contrast themes) is applied anyway and
//! reported, so the reader can choose another.

use crate::check::{Requirement, minimum};
use crate::color::{Rgb, contrast_ratio, spoken_ratio};
use crate::model::{Attrs, ColorRole, StyleRole, Theme};

/// Color names accepted besides `#rrggbb` and `#rgb`: star's highlight
/// names and the common CSS ones.
pub const NAMED_COLORS: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xffffff),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("silver", 0xc0c0c0),
    ("red", 0xff0000),
    ("maroon", 0x800000),
    ("orange", 0xffa500),
    ("yellow", 0xffff00),
    ("gold", 0xffd700),
    ("olive", 0x808000),
    ("lime", 0x00ff00),
    ("green", 0x90ee90),
    ("darkgreen", 0x006400),
    ("teal", 0x008080),
    ("cyan", 0x00ffff),
    ("aqua", 0x00ffff),
    ("lightblue", 0xadd8e6),
    ("skyblue", 0x87ceeb),
    ("blue", 0x0000ff),
    ("navy", 0x000080),
    ("purple", 0x800080),
    ("violet", 0xee82ee),
    ("magenta", 0xff00ff),
    ("fuchsia", 0xff00ff),
    ("pink", 0xffc0cb),
    ("brown", 0xa52a2a),
];

/// The setting value meaning "use the theme's own color".
pub const THEME_DEFAULT: &str = "theme";

/// A color setting as a color: a name from [`NAMED_COLORS`] (any case,
/// spaces ignored), `#rrggbb`, or `#rgb`. `Ok(None)` for an empty value or
/// [`THEME_DEFAULT`]; `Err` with a message for anything else.
pub fn parse_setting(value: &str) -> Result<Option<Rgb>, String> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case(THEME_DEFAULT) {
        return Ok(None);
    }
    let key: String = v
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some((_, hex)) = NAMED_COLORS.iter().find(|(n, _)| *n == key) {
        return Ok(Some(Rgb::from_u32(*hex)));
    }
    Rgb::parse(v)
        .map(Some)
        .map_err(|_| format!("{v} is not a color name or a hex color such as #00ffff"))
}

/// What applying the reader's colors changed and found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReadingColors {
    /// Problems to tell the reader, worded to be read aloud: an unknown
    /// color (ignored) or a color with too little contrast (applied).
    pub warnings: Vec<String>,
    /// True when a color was applied.
    pub changed: bool,
}

/// Lays the reader's word and sentence highlight colors over `theme` (see
/// the module notes). `word` and `sentence` are the setting values;
/// [`THEME_DEFAULT`] or empty keeps the theme's own style.
pub fn apply_reading_colors(
    theme: &mut Theme,
    word: &str,
    sentence: Option<&str>,
) -> ReadingColors {
    let mut out = ReadingColors::default();
    for (role, value, what) in [
        (StyleRole::SpokenWord, Some(word), "word"),
        (StyleRole::SpokenSentence, sentence, "sentence"),
    ] {
        let Some(value) = value else { continue };
        match parse_setting(value) {
            Ok(None) => {}
            Ok(Some(band)) => {
                if let Some(w) = set_band(theme, role, band, what) {
                    out.warnings.push(w);
                }
                out.changed = true;
            }
            Err(e) => out.warnings.push(format!(
                "The {what} highlight color was not used: {e}. The theme's color is used."
            )),
        }
    }
    if out.changed {
        keep_word_and_sentence_apart(theme);
    }
    out
}

/// Gives `role` the band `band` with the more readable of the theme's text
/// and page colors on it; returns a contrast warning when even that is
/// below the theme's floor.
fn set_band(theme: &mut Theme, role: StyleRole, band: Rgb, what: &str) -> Option<String> {
    let text = theme.color(ColorRole::Text);
    let page = theme.color(ColorRole::Background);
    let fg = if contrast_ratio(text, band) >= contrast_ratio(page, band) {
        text
    } else {
        page
    };
    let style = theme.style_mut(role);
    let mut attrs = style.attributes;
    // The colors are given directly now: reverse video would swap them.
    attrs.reverse = false;
    if attrs.is_empty() {
        attrs = role.no_color_attributes();
        attrs.reverse = false;
        if attrs.is_empty() {
            attrs = Attrs::UNDERLINE;
        }
    }
    style.foreground = fg;
    style.background = Some(band);
    style.attributes = attrs;
    let ratio = contrast_ratio(fg, band);
    let floor = minimum(Requirement::Text, theme.kind());
    (ratio < floor).then(|| {
        format!(
            "The {what} highlight color {} leaves its text at {} contrast; {} is needed. Choose a lighter or darker color.",
            band.hex(),
            spoken_ratio(ratio),
            spoken_ratio(floor)
        )
    })
}

/// The spoken word must differ from its sentence by attribute, not only by
/// color: when both ended up with the same attributes, the word is made
/// bold (or underlined, when already bold).
fn keep_word_and_sentence_apart(theme: &mut Theme) {
    let sentence = theme.style(StyleRole::SpokenSentence).attributes;
    let word = &mut theme.style_mut(StyleRole::SpokenWord).attributes;
    if *word == sentence {
        if word.bold {
            word.underline = !word.underline;
        } else {
            word.bold = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Registry;

    fn galaxy() -> Theme {
        Registry::builtin().resolve("galaxy").0.clone()
    }

    #[test]
    fn settings_values_parse() {
        assert_eq!(parse_setting("theme"), Ok(None));
        assert_eq!(parse_setting(""), Ok(None));
        assert_eq!(parse_setting("Cyan"), Ok(Some(Rgb::from_u32(0x00ffff))));
        assert_eq!(
            parse_setting("light blue"),
            Ok(Some(Rgb::from_u32(0xadd8e6)))
        );
        assert_eq!(parse_setting("#f80"), Ok(Some(Rgb::from_u32(0xff8800))));
        assert!(parse_setting("sparkly").is_err());
    }

    #[test]
    fn the_theme_default_changes_nothing() {
        let mut t = galaxy();
        let before = t.clone();
        let r = apply_reading_colors(&mut t, THEME_DEFAULT, None);
        assert_eq!(t, before);
        assert!(!r.changed && r.warnings.is_empty());
    }

    #[test]
    fn colors_become_the_bands_with_readable_text_and_cues() {
        for theme in Registry::builtin().themes() {
            let mut t = theme.clone();
            let r = apply_reading_colors(&mut t, "yellow", Some("#004466"));
            assert!(r.changed);
            let word = t.style(StyleRole::SpokenWord);
            assert_eq!(word.background, Some(Rgb::from_u32(0xffff00)));
            assert!(!word.attributes.is_empty(), "{}", t.meta.name);
            assert!(!word.attributes.reverse);
            let sentence = t.style(StyleRole::SpokenSentence);
            assert_eq!(sentence.background, Some(Rgb::from_u32(0x004466)));
            assert_ne!(word.attributes, sentence.attributes, "{}", t.meta.name);
            // The text color picked is the better of text and page.
            let text = t.color(ColorRole::Text);
            let page = t.color(ColorRole::Background);
            let best = contrast_ratio(text, Rgb::from_u32(0xffff00))
                .max(contrast_ratio(page, Rgb::from_u32(0xffff00)));
            assert_eq!(
                contrast_ratio(word.foreground, Rgb::from_u32(0xffff00)),
                best
            );
        }
    }

    #[test]
    fn low_contrast_and_unknown_colors_are_reported() {
        let mut t = galaxy();
        // Mid grey: neither light nor dark text reads well on it.
        let r = apply_reading_colors(&mut t, "#777777", Some("sparkly"));
        assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
        assert!(
            r.warnings[0].starts_with("The word highlight color #777777 leaves its text at"),
            "{}",
            r.warnings[0]
        );
        assert!(r.warnings[0].contains("4.5 to 1 is needed"));
        assert!(r.warnings[1].contains("sentence highlight color was not used"));
        // The unknown sentence color left the theme's sentence style.
        assert_eq!(
            t.style(StyleRole::SpokenSentence),
            galaxy().style(StyleRole::SpokenSentence)
        );
    }
}
