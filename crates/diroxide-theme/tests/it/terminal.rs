// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Terminal output keeps every built-in theme readable at every level of
//! color support, and never marks a highlight by color alone.

use diroxide_theme::terminal::{REFERENCE_PALETTES, xterm_rgb};
use diroxide_theme::{
    Attrs, ColorRole, ColorSupport, Requirement, StyleRole, TermColor, TermStyle, TerminalTheme,
    Theme, builtin,
    check::{minimum, role_minimum},
    contrast_ratio,
};

type Pair = (String, TermStyle, TermColor, TermColor, f64);

/// Every foreground/background pair the renderer can produce, with the
/// style it came from, a label, and the floor its class holds it to (text
/// at the text floor, indicators at 3 to 1, decorative lines at none).
fn pairs(tt: &TerminalTheme, theme: &Theme) -> Vec<Pair> {
    let page = tt.page();
    let (Some(pfg), Some(pbg)) = (page.fg, page.bg) else {
        return Vec::new();
    };
    let text = floor(theme);
    let mut v = vec![("page".to_owned(), page, pfg, pbg, text)];
    for &r in ColorRole::ALL {
        let s = page.patch(tt.color(r));
        v.push((
            format!("color {r}"),
            s,
            s.fg.unwrap_or(pfg),
            s.bg.unwrap_or(pbg),
            // The inner focus line sits against the focus ring, not the
            // page; the theme check measures it there.
            if r == ColorRole::FocusInner {
                0.0
            } else {
                role_minimum(r, theme.kind())
            },
        ));
    }
    for &r in StyleRole::ALL {
        let s = page.patch(tt.style(r));
        v.push((
            format!("style {r}"),
            s,
            s.fg.unwrap_or(pfg),
            s.bg.unwrap_or(pbg),
            text,
        ));
    }
    for (i, h) in theme.user_highlights.iter().enumerate() {
        let s = page.patch(tt.user_highlight(i).unwrap());
        v.push((
            format!("highlight {}", h.name),
            s,
            s.fg.unwrap_or(pfg),
            s.bg.unwrap_or(pbg),
            text,
        ));
    }
    v
}

fn floor(theme: &Theme) -> f64 {
    minimum(Requirement::Text, theme.kind())
}

#[test]
fn truecolor_and_256_keep_the_floor() {
    for t in builtin::all() {
        for support in [ColorSupport::TrueColor, ColorSupport::Ansi256] {
            let tt = TerminalTheme::new(t, support);
            for (label, _, fg, bg, min) in pairs(&tt, t) {
                if support == ColorSupport::Ansi256 {
                    for c in [fg, bg] {
                        assert!(matches!(c, TermColor::Indexed(i) if i >= 16), "{label}");
                    }
                }
                // Only the themes that must meet WCAG AA are held to the
                // floor; the others keep star's colors (the owner, 2026-09-26).
                if diroxide_theme::star::must_meet_aa(t.name()) {
                    let base = REFERENCE_PALETTES[0];
                    let r = contrast_ratio(fg.rgb(base), bg.rgb(base));
                    assert!(r >= min, "{} {support:?} {label}: {r:.2}", t.name());
                }
            }
        }
    }
}

#[test]
fn sixteen_colors_pass_in_every_reference_palette_even_with_bold_brightening() {
    for t in builtin::all() {
        let tt = TerminalTheme::new(t, ColorSupport::Ansi16);
        for (label, style, fg, bg, min) in pairs(&tt, t) {
            let (TermColor::Indexed(f), TermColor::Indexed(b)) = (fg, bg) else {
                panic!("{} {label}: 16-color output must use indexes", t.name());
            };
            assert!(f < 16 && b < 16, "{label}");
            for pal in REFERENCE_PALETTES {
                let mut fgs = vec![f];
                if style.attrs.bold && f < 8 {
                    fgs.push(f + 8);
                }
                for f in fgs {
                    let r = contrast_ratio(pal[usize::from(f)], pal[usize::from(b)]);
                    assert!(
                        r >= min,
                        "{} 16-color {label}: {f} on {b} is {r:.2}",
                        t.name()
                    );
                }
            }
        }
    }
}

#[test]
fn sepia_headings_stay_visible_in_16_colors() {
    // star's sepia turned its headings bright yellow on white (1.00:1).
    let t = builtin::get("sepia").unwrap();
    let tt = TerminalTheme::new(t, ColorSupport::Ansi16);
    let h = tt.page().patch(tt.color(ColorRole::Heading1));
    assert_ne!(h.fg, Some(TermColor::Indexed(3)));
    assert_ne!(h.fg, Some(TermColor::Indexed(11)));
    assert!(h.attrs.bold || h.attrs.underline);
}

#[test]
fn highlights_carry_attributes_at_every_level() {
    for t in builtin::all() {
        for support in [
            ColorSupport::NoColor,
            ColorSupport::Ansi16,
            ColorSupport::Ansi256,
            ColorSupport::TrueColor,
        ] {
            let tt = TerminalTheme::new(t, support);
            for &r in StyleRole::ALL {
                if r.is_highlight() {
                    assert!(
                        !tt.style(r).attrs.is_empty(),
                        "{} {support:?} {r}",
                        t.name()
                    );
                }
            }
            for i in 0..t.user_highlights.len() {
                assert!(!tt.user_highlight(i).unwrap().attrs.is_empty());
            }
            let a = |r| tt.style(r).attrs;
            assert_ne!(
                a(StyleRole::SpokenWord),
                a(StyleRole::SpokenSentence),
                "{support:?}"
            );
            assert_ne!(
                a(StyleRole::FindHit),
                a(StyleRole::CurrentFindHit),
                "{support:?}"
            );
        }
    }
}

#[test]
fn no_color_uses_no_color() {
    let t = builtin::get("galaxy").unwrap();
    let tt = TerminalTheme::new(t, ColorSupport::NoColor);
    assert_eq!(tt.page(), TermStyle::default());
    for &r in ColorRole::ALL {
        assert!(tt.color(r).fg.is_none() && tt.color(r).bg.is_none());
    }
    for &r in StyleRole::ALL {
        assert!(tt.style(r).fg.is_none() && tt.style(r).bg.is_none());
        assert_eq!(tt.style(r).attrs, r.no_color_attributes());
    }
    assert!(tt.color(ColorRole::Heading1).attrs.bold);
    assert!(tt.color(ColorRole::Link).attrs.underline);
    assert!(tt.color(ColorRole::Quote).attrs.italic);
    assert_eq!(
        tt.style(StyleRole::SpokenWord).attrs,
        Attrs::REVERSE.with(Attrs::BOLD)
    );
}

#[test]
fn truecolor_is_exact() {
    let t = builtin::get("nord").unwrap();
    let tt = TerminalTheme::new(t, ColorSupport::TrueColor);
    assert_eq!(
        tt.page().bg,
        Some(TermColor::Rgb(t.color(ColorRole::Background)))
    );
    assert_eq!(
        tt.color(ColorRole::Heading2).fg,
        Some(TermColor::Rgb(t.color(ColorRole::Heading2)))
    );
    assert_eq!(tt.user_highlight_names().count(), 4);
    assert!(tt.user_highlight_named("Yellow").is_some());
}

#[test]
fn quantizing_stays_close() {
    // 256-color output picks nearby colors, not arbitrary passing ones.
    for t in builtin::all() {
        let tt = TerminalTheme::new(t, ColorSupport::Ansi256);
        let Some(TermColor::Indexed(i)) = tt.page().bg else {
            panic!()
        };
        assert!(
            xterm_rgb(i).distance(t.color(ColorRole::Background)) < 0.06,
            "{}",
            t.name()
        );
        let Some(TermColor::Indexed(h)) = tt.color(ColorRole::Heading1).fg else {
            panic!()
        };
        assert!(
            xterm_rgb(h).distance(t.color(ColorRole::Heading1)) < 0.12,
            "{}",
            t.name()
        );
    }
}

#[test]
fn sixteen_colors_keep_each_color_s_character() {
    // Dark body text on a light page is black, not a dark hue.
    let sepia = TerminalTheme::new(builtin::get("sepia").unwrap(), ColorSupport::Ansi16);
    assert_eq!(sepia.page().fg, Some(TermColor::Indexed(0)));
    assert_eq!(sepia.page().bg, Some(TermColor::Indexed(15)));
    // Grays stay gray.
    let galaxy = TerminalTheme::new(builtin::get("galaxy").unwrap(), ColorSupport::Ansi16);
    assert_eq!(
        galaxy.color(ColorRole::DimText).fg,
        Some(TermColor::Indexed(7))
    );
    // Green phosphor stays green.
    let phosphor = TerminalTheme::new(builtin::get("phosphor").unwrap(), ColorSupport::Ansi16);
    assert_eq!(phosphor.page().fg, Some(TermColor::Indexed(2)));
}
