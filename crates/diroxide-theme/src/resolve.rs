// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Turning a [`ThemeFile`] into a complete [`Theme`].
//!
//! Without `inherits`, a file needs only `colors.background` and
//! `colors.text`; every other role is worked out from the colors it has,
//! the way star's terminal UI derived its chrome from a ten-color palette:
//! highlights put the page color as text on an accent band (the spoken word
//! on heading 1, find matches on headings 3 and 2), so their legibility is
//! the accent's own contrast. With `inherits`, missing keys come from that
//! built-in theme instead.
//!
//! Colors the file does not give are always nudged until they pass. Colors
//! the file does give are only nudged when asked ([`Repair::Explicit`], used
//! to port star's palettes); a user's own colors are reported by
//! [`crate::check()`], never silently changed.

use toml::Table;

use crate::check::{Requirement, minimum};
use crate::color::{Rgb, adjust_away, adjust_lightness, contrast_ratio};
use crate::error::ThemeError;
use crate::file::{StyleFile, ThemeFile};
use crate::model::{Attrs, ColorRole, Meta, Style, StyleRole, Theme, ThemeKind, UserHighlight};

/// Whether colors the file gives may be changed to meet the contrast floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair {
    /// Only colors worked out by the resolver are adjusted (user themes).
    DerivedOnly,
    /// Colors from the file are adjusted too, and each change is recorded
    /// (built-in themes ported from star, and the "fix contrast" action).
    Explicit,
}

/// One color changed to meet a contrast floor.
#[derive(Clone, Debug, PartialEq)]
pub struct Adjustment {
    /// Dotted key, for example `colors.dim_text` or `styles.selection.background`.
    pub key: String,
    /// The color before.
    pub from: Rgb,
    /// The color after.
    pub to: Rgb,
    /// The color it is measured against.
    pub against: Rgb,
    /// Contrast before.
    pub ratio_before: f64,
    /// Contrast after.
    pub ratio_after: f64,
    /// The floor it had to reach.
    pub minimum: f64,
}

impl Adjustment {
    /// A sentence for logs and screen readers.
    pub fn describe(&self) -> String {
        format!(
            "{} changed from {} to {}: {} against {}, needs {}.",
            self.key,
            self.from,
            self.to,
            crate::color::spoken_ratio(self.ratio_before),
            self.against,
            crate::color::spoken_ratio(self.minimum),
        )
    }
}

/// Highlight colors every theme offers unless its file lists its own:
/// name and the pen color mixed into the page.
pub const DEFAULT_USER_HIGHLIGHTS: [(&str, Rgb); 4] = [
    ("yellow", Rgb::from_u32(0xffd60a)),
    ("green", Rgb::from_u32(0x34c759)),
    ("blue", Rgb::from_u32(0x0a84ff)),
    ("pink", Rgb::from_u32(0xff5fa2)),
];

/// `galaxy-light` → `Galaxy Light`.
pub fn display_name_for(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct Fixer {
    repair: Repair,
    adjustments: Vec<Adjustment>,
}

impl Fixer {
    /// Adjusts `c` (a text color) against `against`, when allowed.
    fn text(&mut self, key: &str, c: Rgb, explicit: bool, against: &[Rgb], min: f64) -> Rgb {
        if explicit && self.repair == Repair::DerivedOnly {
            return c;
        }
        let Some(fixed) = adjust_lightness(c, against, min) else {
            return c;
        };
        if fixed != c && explicit {
            let worst = against
                .iter()
                .copied()
                .min_by(|a, b| contrast_ratio(c, *a).total_cmp(&contrast_ratio(c, *b)))
                .unwrap_or(Rgb::BLACK);
            self.adjustments.push(Adjustment {
                key: key.to_owned(),
                from: c,
                to: fixed,
                against: worst,
                ratio_before: contrast_ratio(c, worst),
                ratio_after: contrast_ratio(fixed, worst),
                minimum: min,
            });
        }
        fixed
    }

    /// Adjusts `band` (a background) away from `text`, when allowed.
    fn band(&mut self, key: &str, band: Rgb, explicit: bool, text: Rgb, min: f64) -> Rgb {
        if explicit && self.repair == Repair::DerivedOnly {
            return band;
        }
        let Some(fixed) = adjust_away(band, text, min) else {
            return band;
        };
        if fixed != band && explicit {
            self.adjustments.push(Adjustment {
                key: key.to_owned(),
                from: band,
                to: fixed,
                against: text,
                ratio_before: contrast_ratio(band, text),
                ratio_after: contrast_ratio(fixed, text),
                minimum: min,
            });
        }
        fixed
    }
}

fn meta(file: &ThemeFile, base: Option<&Theme>, name: String, kind: ThemeKind) -> Meta {
    Meta {
        display_name: file
            .display_name
            .clone()
            .unwrap_or_else(|| display_name_for(&name)),
        name,
        kind,
        description: file.description.clone().unwrap_or_default(),
        author: file.author.clone(),
        origin: file.origin.clone().unwrap_or_else(|| "user".to_owned()),
        counterpart: file.counterpart.clone(),
        inherits: file
            .inherits
            .clone()
            .or_else(|| base.map(|b| b.meta.name.clone())),
        extra: file.meta_extra.clone(),
    }
}

/// Resolves a file into a complete theme. `base` is the theme named by
/// `inherits` (the caller looks it up); `fallback_name` is used when the file
/// has no `theme.name` (the loader passes the file stem).
pub fn resolve(
    file: &ThemeFile,
    base: Option<&Theme>,
    fallback_name: Option<&str>,
    repair: Repair,
) -> Result<(Theme, Vec<Adjustment>), ThemeError> {
    let name = match (&file.name, fallback_name) {
        (Some(n), _) => n.clone(),
        (None, Some(stem)) => {
            crate::file::normalize_name(stem).map_err(|m| ThemeError::invalid("theme.name", m))?
        }
        (None, None) => return Err(ThemeError::Missing("theme.name".into())),
    };
    let given = |r: ColorRole| file.colors[r.index()];
    let bg = match (given(ColorRole::Background), base) {
        (Some(c), _) => c,
        (None, Some(b)) => b.color(ColorRole::Background),
        (None, None) => return Err(ThemeError::Missing("colors.background".into())),
    };
    let text0 = match (given(ColorRole::Text), base) {
        (Some(c), _) => c,
        (None, Some(b)) => b.color(ColorRole::Text),
        (None, None) => return Err(ThemeError::Missing("colors.text".into())),
    };
    let kind = file.kind.or(base.map(|b| b.meta.kind)).unwrap_or(
        if bg.relative_luminance() < text0.relative_luminance() {
            ThemeKind::Dark
        } else {
            ThemeKind::Light
        },
    );
    let min = minimum(Requirement::Text, kind);
    let dark = bg.relative_luminance() < text0.relative_luminance();
    let mut fx = Fixer {
        repair,
        adjustments: Vec::new(),
    };

    // Colors: given, else from the base, else derived.
    let from_base = |r: ColorRole| base.map(|b| b.color(r));
    let mut c = [Rgb::BLACK; ColorRole::COUNT];
    let mut explicit = [false; ColorRole::COUNT];
    for &r in ColorRole::ALL {
        explicit[r.index()] = given(r).is_some();
    }
    let pick = |r: ColorRole, derived: Rgb| given(r).or(from_base(r)).unwrap_or(derived);
    use ColorRole as C;
    c[C::Background.index()] = bg;
    c[C::Text.index()] = text0;
    c[C::CodeBackground.index()] = pick(
        C::CodeBackground,
        given(C::Surface).unwrap_or_else(|| bg.mix(text0, 0.06)),
    );
    c[C::Surface.index()] = pick(C::Surface, c[C::CodeBackground.index()]);
    c[C::DimText.index()] = pick(C::DimText, text0.mix(bg, 0.3));
    c[C::Heading1.index()] = pick(C::Heading1, text0);
    for (prev, r) in [
        (C::Heading1, C::Heading2),
        (C::Heading2, C::Heading3),
        (C::Heading3, C::Heading4),
        (C::Heading4, C::Heading5),
        (C::Heading5, C::Heading6),
    ] {
        c[r.index()] = pick(r, c[prev.index()]);
    }
    let link_seed = if dark { 0x7aa2f7 } else { 0x1a5fb4 };
    c[C::Link.index()] = pick(C::Link, Rgb::from_u32(link_seed));
    c[C::Code.index()] = pick(C::Code, text0);
    c[C::Quote.index()] = pick(C::Quote, c[C::DimText.index()]);
    let error_seed = if dark { 0xff6b6b } else { 0xb3261e };
    c[C::Error.index()] = pick(C::Error, Rgb::from_u32(error_seed));

    // Repair: text first, then everything measured against the page.
    let key = |r: ColorRole| format!("colors.{}", r.key());
    for r in [
        C::Text,
        C::DimText,
        C::Heading1,
        C::Heading2,
        C::Heading3,
        C::Heading4,
        C::Heading5,
        C::Heading6,
        C::Link,
        C::Quote,
        C::Error,
    ] {
        c[r.index()] = fx.text(&key(r), c[r.index()], explicit[r.index()], &[bg], min);
    }
    let text = c[C::Text.index()];
    for r in [C::CodeBackground, C::Surface] {
        c[r.index()] = fx.band(&key(r), c[r.index()], explicit[r.index()], text, min);
    }
    c[C::Code.index()] = fx.text(
        &key(C::Code),
        c[C::Code.index()],
        explicit[C::Code.index()],
        &[bg, c[C::CodeBackground.index()]],
        min,
    );

    // Styles.
    use StyleRole as S;
    let col = |r: ColorRole| c[r.index()];
    let selection_band = file.styles[S::Selection.index()]
        .background
        .flatten()
        .or(base.and_then(|b| b.style(S::Selection).background))
        .unwrap_or_else(|| bg.mix(text, 0.2));
    let derived = |r: StyleRole| -> Style {
        let (fg, band, attrs) = match r {
            S::Selection => (text, Some(selection_band), Attrs::REVERSE),
            S::SpokenSentence => (text, Some(selection_band), Attrs::UNDERLINE),
            S::SpokenWord => (bg, Some(col(C::Heading1)), Attrs::BOLD),
            S::FindHit => (bg, Some(col(C::Heading3)), Attrs::UNDERLINE),
            S::CurrentFindHit => (
                bg,
                Some(col(C::Heading2)),
                Attrs::BOLD.with(Attrs::UNDERLINE),
            ),
            S::Bookmark => (col(C::Heading4), None, Attrs::BOLD.with(Attrs::UNDERLINE)),
            S::Note => (col(C::Code), None, Attrs::ITALIC.with(Attrs::UNDERLINE)),
            S::StatusBar => (bg, Some(col(C::Heading1)), Attrs::BOLD),
            S::Focus => (bg, Some(col(C::Link)), Attrs::BOLD),
        };
        match base {
            Some(b) => b.style(r).clone(),
            None => Style::new(fg, band, attrs),
        }
    };
    let styles: [Style; StyleRole::COUNT] = std::array::from_fn(|i| {
        let r = StyleRole::ALL[i];
        let f = &file.styles[i];
        let d = derived(r);
        let mut s = Style {
            foreground: f.foreground.unwrap_or(d.foreground),
            background: f.background.unwrap_or(d.background),
            attributes: f.attributes.unwrap_or(d.attributes),
            extra: if f.extra.is_empty() {
                d.extra
            } else {
                f.extra.clone()
            },
        };
        fix_style(&mut fx, &format!("styles.{}", r.key()), &mut s, f, bg, min);
        s
    });

    // User highlights.
    let user_highlights = match (&file.user_highlights, base) {
        (Some(list), _) => list
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let mut s = Style {
                    foreground: h.style.foreground.unwrap_or(text),
                    background: h.style.background.unwrap_or(None),
                    attributes: h.style.attributes.unwrap_or(Attrs::BOLD),
                    extra: h.style.extra.clone(),
                };
                let key = format!("user_highlights entry {} ({})", i + 1, h.name);
                fix_style(&mut fx, &key, &mut s, &h.style, bg, min);
                UserHighlight {
                    name: h.name.clone(),
                    style: s,
                }
            })
            .collect(),
        (None, Some(b)) => b.user_highlights.clone(),
        (None, None) => DEFAULT_USER_HIGHLIGHTS
            .iter()
            .map(|&(name, pen)| {
                let band = bg.mix(pen, if dark { 0.30 } else { 0.35 });
                let band = adjust_away(band, text, min).unwrap_or(band);
                UserHighlight {
                    name: name.to_owned(),
                    style: Style::new(text, Some(band), Attrs::BOLD),
                }
            })
            .collect(),
    };

    // The interface roles, derived last from the colors and styles above.
    let mut derived = [false; ColorRole::COUNT];
    let given_or_base =
        |r: ColorRole| given(r).or_else(|| base.filter(|b| !b.is_derived(r)).map(|b| b.color(r)));
    derive_interface(&mut c, &mut derived, &styles, kind, dark, given_or_base);

    let theme = Theme {
        meta: meta(file, base, name, kind),
        colors: c,
        styles,
        derived,
        user_highlights,
        colors_extra: merged(base.map(|b| &b.colors_extra), &file.colors_extra),
        styles_extra: merged(base.map(|b| &b.styles_extra), &file.styles_extra),
        extra: merged(base.map(|b| &b.extra), &file.extra),
    };
    Ok((theme, fx.adjustments))
}

/// Works out the fifteen [`ColorRole::DERIVED`] roles from the theme's own
/// colors, each nudged until it meets its class's floor. A role the file
/// gives (or a base theme gave explicitly) is kept as it is and reported by
/// [`crate::check()`] instead.
fn derive_interface(
    c: &mut [Rgb; ColorRole::COUNT],
    derived: &mut [bool; ColorRole::COUNT],
    styles: &[Style; StyleRole::COUNT],
    kind: ThemeKind,
    dark: bool,
    given: impl Fn(ColorRole) -> Option<Rgb>,
) {
    use ColorRole as C;
    let hc = kind == ThemeKind::HighContrast;
    let text_min = minimum(Requirement::Text, kind);
    let ind_min = minimum(Requirement::NonText, kind);
    let resolve = |s: &Style| -> (Rgb, Rgb) {
        let bg = s.background.unwrap_or(c[C::Background.index()]);
        if s.attributes.reverse {
            (bg, s.foreground)
        } else {
            (s.foreground, bg)
        }
    };
    let page = c[C::Background.index()];
    let surface = c[C::Surface.index()];
    let text = c[C::Text.index()];
    let dim = c[C::DimText.index()];
    let (status_fg, status_bg) = resolve(&styles[StyleRole::StatusBar.index()]);
    let (_, focus) = resolve(&styles[StyleRole::Focus.index()]);

    let mut set = |r: ColorRole, value: &dyn Fn(&[Rgb; ColorRole::COUNT]) -> Rgb| match given(r) {
        Some(v) => c[r.index()] = v,
        None => {
            c[r.index()] = value(c);
            derived[r.index()] = true;
        }
    };
    set(C::Raised, &|_| {
        let raised = if hc { surface } else { surface.mix(text, 0.07) };
        adjust_away(raised, text, text_min).unwrap_or(raised)
    });
    set(C::Border, &|_| {
        if hc {
            text
        } else {
            page.mix(text, if dark { 0.20 } else { 0.28 })
        }
    });
    set(C::ControlBorder, &|c| {
        let b = if hc { text } else { c[C::Border.index()] };
        adjust_lightness(b, &[surface, page], ind_min).unwrap_or(text)
    });
    set(C::PanelDimText, &|_| {
        adjust_lightness(dim, &[surface, page], text_min).unwrap_or(text)
    });
    set(C::Accent, &|_| {
        adjust_lightness(status_bg, &[surface], ind_min).unwrap_or(status_bg)
    });
    set(C::OnAccent, &|c| {
        let accent = c[C::Accent.index()];
        adjust_lightness(status_fg, &[accent], text_min).unwrap_or_else(|| {
            if contrast_ratio(Rgb::BLACK, accent) > contrast_ratio(Rgb::WHITE, accent) {
                Rgb::BLACK
            } else {
                Rgb::WHITE
            }
        })
    });
    set(C::Disabled, &|c| {
        adjust_lightness(dim, &[c[C::Raised.index()]], 3.0).unwrap_or(dim)
    });
    set(C::Caret, &|_| text);
    set(C::FocusInner, &|_| {
        if contrast_ratio(page, focus) >= contrast_ratio(text, focus) {
            page
        } else {
            text
        }
    });
    set(C::Ruler, &|_| {
        let band = page.mix(focus, 0.22);
        adjust_away(band, text, text_min).unwrap_or(band)
    });
    set(C::RulerBand, &|_| {
        let band = page.mix(focus, 0.10);
        adjust_away(band, text, text_min).unwrap_or(band)
    });
    for (r, seed) in [
        (C::DifficultWord, text),
        (C::SyllableMark, text),
        (C::Misspelling, focus),
        (C::Lint, text),
    ] {
        set(r, &|_| {
            adjust_lightness(seed, &[page], ind_min).unwrap_or(text)
        });
    }
}

fn merged(base: Option<&Table>, over: &Table) -> Table {
    let mut t = base.cloned().unwrap_or_default();
    for (k, v) in over {
        t.insert(k.clone(), v.clone());
    }
    t
}

fn fix_style(fx: &mut Fixer, key: &str, s: &mut Style, f: &StyleFile, page: Rgb, min: f64) {
    match s.background {
        Some(band) => {
            // Prefer moving the band away from the text; if the band may not
            // change, move the text instead.
            let band = fx.band(
                &format!("{key}.background"),
                band,
                f.background.is_some(),
                s.foreground,
                min,
            );
            s.background = Some(band);
            if contrast_ratio(s.foreground, band) < min {
                s.foreground = fx.text(
                    &format!("{key}.foreground"),
                    s.foreground,
                    f.foreground.is_some(),
                    &[band],
                    min,
                );
            }
        }
        None => {
            s.foreground = fx.text(
                &format!("{key}.foreground"),
                s.foreground,
                f.foreground.is_some(),
                &[page],
                min,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(src: &str) -> ThemeFile {
        ThemeFile::parse(src).unwrap()
    }

    #[test]
    fn two_colors_make_a_whole_theme() {
        let f = file("[colors]\nbackground = \"#101010\"\ntext = \"#e0e0e0\"\n");
        let (t, adj) = resolve(&f, None, Some("Mine"), Repair::DerivedOnly).unwrap();
        assert!(adj.is_empty());
        assert_eq!(t.name(), "mine");
        assert_eq!(t.meta.display_name, "Mine");
        assert_eq!(t.kind(), ThemeKind::Dark);
        assert_eq!(t.user_highlights.len(), 4);
        let report = crate::check::check(&t);
        assert!(report.passed(), "{}", report.summary());
    }

    #[test]
    fn missing_required_keys() {
        let e = resolve(&file(""), None, Some("x"), Repair::DerivedOnly).unwrap_err();
        assert!(matches!(e, ThemeError::Missing(ref k) if k == "colors.background"));
        let e = resolve(
            &file("[colors]\nbackground=\"#000000\"\n"),
            None,
            None,
            Repair::DerivedOnly,
        )
        .unwrap_err();
        assert!(matches!(e, ThemeError::Missing(ref k) if k == "theme.name"));
    }

    #[test]
    fn explicit_colors_are_kept_unless_repairing() {
        let src =
            "[colors]\nbackground = \"#1e1e1e\"\ntext = \"#dadada\"\ndim_text = \"#7d7d7d\"\n";
        let (t, adj) = resolve(&file(src), None, Some("a"), Repair::DerivedOnly).unwrap();
        assert_eq!(t.color(ColorRole::DimText), Rgb::from_u32(0x7d7d7d));
        assert!(adj.is_empty());
        assert!(!crate::check::check(&t).passed());
        let (t, adj) = resolve(&file(src), None, Some("a"), Repair::Explicit).unwrap();
        assert_ne!(t.color(ColorRole::DimText), Rgb::from_u32(0x7d7d7d));
        assert_eq!(adj.len(), 1);
        assert_eq!(adj[0].key, "colors.dim_text");
        assert!(
            adj[0].describe().contains("needs 4.5 to 1"),
            "{}",
            adj[0].describe()
        );
        let r = crate::check::check(&t);
        assert!(r.passed(), "{}", r.summary());
    }

    #[test]
    fn inherits_copies_then_overrides() {
        let base_src = "[colors]\nbackground = \"#000000\"\ntext = \"#ffffff\"\nlink = \"#88ccff\"\n\
                        [colors_note]\nx = 1\n";
        let (base, _) = resolve(&file(base_src), None, Some("base"), Repair::DerivedOnly).unwrap();
        let f = file("[theme]\nname = \"child\"\n[colors]\nlink = \"#99ddff\"\n");
        let (t, _) = resolve(&f, Some(&base), None, Repair::DerivedOnly).unwrap();
        assert_eq!(t.color(ColorRole::Text), Rgb::WHITE);
        assert_eq!(t.color(ColorRole::Link), Rgb::from_u32(0x99ddff));
        assert_eq!(t.meta.inherits.as_deref(), Some("base"));
        assert_eq!(t.meta.display_name, "Child");
        assert_eq!(
            t.style(StyleRole::SpokenWord),
            base.style(StyleRole::SpokenWord)
        );
        assert!(t.extra.contains_key("colors_note"));
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name_for("galaxy-light"), "Galaxy Light");
        assert_eq!(display_name_for("tokyo_night"), "Tokyo Night");
    }
}
