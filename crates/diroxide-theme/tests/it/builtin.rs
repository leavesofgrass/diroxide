// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The built-in themes: star's 23 palettes, in step with their generator,
//! round-tripping through TOML, and passing every contrast check.

use diroxide_theme::{
    ColorRole, Repair, StyleRole, Theme, ThemeFile, ThemeKind, builtin, check, resolve::resolve,
    star,
};

#[test]
fn all_23_star_palettes_and_lamplight_are_built_in_in_f5_order() {
    let names: Vec<&str> = builtin::all().iter().map(|t| t.name()).collect();
    let star: Vec<&str> = star::PALETTES.iter().map(|p| p.name).collect();
    assert_eq!(names.len(), 24);
    assert_eq!(names, builtin::CYCLE.to_vec());
    assert_eq!(builtin::NAMES.to_vec(), star);
    for n in star.iter().chain(builtin::OWN.iter()) {
        assert!(builtin::CYCLE.contains(n), "{n} is missing from the cycle");
    }
    assert_eq!(builtin::default_theme().name(), "galaxy");
}

/// Next theme steps through the themes that meet AA first, then the ones below;
/// light and dark partners sit side by side.
#[test]
fn the_cycle_puts_aa_first_and_partners_together() {
    let passes: Vec<bool> = builtin::all().iter().map(|t| check(t).passed()).collect();
    let first_below = passes.iter().position(|p| !p).unwrap();
    assert_eq!(first_below, 14);
    assert!(passes[first_below..].iter().all(|p| !p), "{passes:?}");
    let names = builtin::CYCLE;
    for t in builtin::all() {
        if let Some(c) = &t.meta.counterpart {
            let i = names.iter().position(|n| *n == t.name()).unwrap();
            let j = names.iter().position(|n| n == c).unwrap();
            assert_eq!(i.abs_diff(j), 1, "{} and {c} are apart", t.name());
        }
    }
}

#[test]
fn lamplight_is_a_soft_dark_theme_with_aaa_body_text() {
    let t = builtin::get("lamplight").unwrap();
    assert_eq!(t.kind(), ThemeKind::Dark);
    assert_eq!(t.meta.tags(), ["soft"]);
    assert_eq!(t.meta.display_name, "Lamplight");
    let r = check(t);
    assert!(r.passed(), "{}", r.summary());
    let ratio =
        diroxide_theme::contrast_ratio(t.color(ColorRole::Text), t.color(ColorRole::Background));
    assert!(ratio >= 7.0, "{ratio}");
    assert!(
        builtin::source("Lamplight")
            .unwrap()
            .contains(r#"tags = ["soft"]"#)
    );
}

/// Every role is measured by at least one check, in every built-in theme
/// and in a theme made from two colors.
#[test]
fn every_role_is_covered_by_a_check() {
    let two = ThemeFile::parse("[colors]\nbackground = \"#101418\"\ntext = \"#d0d4d8\"\n").unwrap();
    let (two, _) = resolve(&two, None, Some("two"), Repair::DerivedOnly).unwrap();
    for t in builtin::all().iter().chain(std::iter::once(&two)) {
        let r = check(t);
        for &role in ColorRole::ALL {
            let key = format!("colors.{}", role.key());
            let on = format!(" on {}", role.label());
            let covered = r
                .checks
                .iter()
                .any(|c| c.key == key || c.subject.ends_with(&on));
            assert!(covered, "{}: {role} has no check", t.name());
        }
    }
    let r = check(&two);
    assert!(r.passed(), "{}", r.summary());
}

/// The fifteen derived roles pass their class's check in every built-in
/// theme, including the ones whose star colors fall short elsewhere.
#[test]
fn derived_roles_pass_in_every_theme() {
    for t in builtin::all() {
        let r = check(t);
        for &role in ColorRole::DERIVED {
            assert!(t.is_derived(role), "{} {role}", t.name());
            let key = format!("colors.{}", role.key());
            for c in r.checks.iter().filter(|c| c.key == key) {
                assert!(c.passed, "{}: {}", t.name(), c.describe());
            }
        }
    }
}

/// The derived roles leave the file's own colors and styles alone: Galaxy
/// resolves to exactly the 15 colors and 9 styles its file gives.
#[test]
fn galaxy_resolves_the_same_colors_and_styles_as_its_file() {
    let file = ThemeFile::parse(builtin::source("galaxy").unwrap()).unwrap();
    let g = builtin::get("galaxy").unwrap();
    let mut given = 0;
    for &role in ColorRole::ALL {
        match file.colors[role.index()] {
            Some(c) => {
                given += 1;
                assert_eq!(g.color(role), c, "{role}");
                assert!(!g.is_derived(role));
            }
            None => assert!(role.is_derived(), "{role}"),
        }
    }
    assert_eq!(given, 15);
    for &role in StyleRole::ALL {
        let f = &file.styles[role.index()];
        let s = g.style(role);
        assert_eq!(Some(s.foreground), f.foreground, "{role}");
        assert_eq!(Some(s.background), f.background, "{role}");
        assert_eq!(Some(s.attributes), f.attributes, "{role}");
    }
}

/// A child theme that changes the surface derives its raised fill from its
/// own surface, not the base's; an explicit base value is inherited; a
/// same-named key overrides the derived value.
#[test]
fn a_child_rederives_from_its_own_colors() {
    let base = builtin::get("galaxy").unwrap();
    let f = ThemeFile::parse("[theme]\nname = \"kid\"\n[colors]\nsurface = \"#3a3040\"\n").unwrap();
    let (kid, _) = resolve(&f, Some(base), None, Repair::DerivedOnly).unwrap();
    assert_ne!(kid.color(ColorRole::Raised), base.color(ColorRole::Raised));
    assert!(kid.is_derived(ColorRole::Raised));

    let mut parent = base.clone();
    parent.set_color(ColorRole::Accent, diroxide_theme::Rgb::from_u32(0xffcc00));
    let f = ThemeFile::parse("[theme]\nname = \"kid\"\n").unwrap();
    let (kid, _) = resolve(&f, Some(&parent), None, Repair::DerivedOnly).unwrap();
    assert_eq!(kid.color(ColorRole::Accent).hex(), "#ffcc00");

    let f = ThemeFile::parse(
        "[colors]\nbackground = \"#000000\"\ntext = \"#ffffff\"\ncaret = \"#ff8800\"\n",
    )
    .unwrap();
    let (t, _) = resolve(&f, None, Some("c"), Repair::DerivedOnly).unwrap();
    assert_eq!(t.color(ColorRole::Caret).hex(), "#ff8800");
    assert!(!t.is_derived(ColorRole::Caret));
    let toml = t.to_toml_string();
    assert!(toml.contains("caret = \"#ff8800\""));
    assert!(!toml.contains("raised ="));
}

#[test]
fn shipped_files_match_the_generator() {
    for p in &star::PALETTES {
        let expected = star::generated_file(p).unwrap();
        let shipped = builtin::source(p.name).unwrap().replace("\r\n", "\n");
        assert!(
            shipped == expected,
            "themes/{}.toml is stale; run `cargo run -p diroxide-theme --example generate_builtin_themes`",
            p.name
        );
    }
}

#[test]
fn the_required_themes_pass_every_check() {
    // The owner's policy: only Galaxy, Galaxy Light, and the high-contrast themes
    // must meet WCAG AA; the others keep star's colors and are labelled.
    let mut failures = Vec::new();
    for t in builtin::all()
        .iter()
        .filter(|t| star::must_meet_aa(t.name()))
    {
        let r = check(t);
        assert!(r.checks.len() >= 40, "{}", t.name());
        if !r.passed() {
            failures.push(r.summary());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The two-tone highlight: in every built-in theme the spoken word's band
/// stands 3 to 1 above the page and above the sentence's band. The
/// sentence's band itself is a soft tint with no check of its own (the
/// underline marks the sentence), so no theme fails for it.
#[test]
fn every_theme_steps_the_word_band_up_from_page_and_sentence() {
    for t in builtin::all() {
        let r = check(t);
        let word: Vec<_> = r
            .checks
            .iter()
            .filter(|c| c.subject.starts_with("spoken word band against"))
            .collect();
        assert_eq!(word.len(), 2, "{}", t.name());
        for c in word {
            assert!(c.passed, "{}: {}", t.name(), c.describe());
            assert!(c.ratio.unwrap() >= 3.0, "{}: {}", t.name(), c.describe());
        }
        assert!(
            !r.checks
                .iter()
                .any(|c| c.subject.starts_with("spoken sentence band against")),
            "{}: no sentence band check",
            t.name()
        );
    }
}

#[test]
fn high_contrast_themes_reach_seven_to_one() {
    for name in ["contrast", "high-contrast"] {
        let t = builtin::get(name).unwrap();
        assert_eq!(t.kind(), ThemeKind::HighContrast);
        for c in check(t).checks {
            if c.requirement == diroxide_theme::Requirement::Text {
                assert!(c.ratio.unwrap() >= 7.0, "{name}: {}", c.describe());
            }
        }
    }
}

#[test]
fn star_colors_are_kept_where_they_passed() {
    // Only the adjusted keys differ from star.
    for p in &star::PALETTES {
        let (_, adj) = star::port(p).unwrap();
        let t = builtin::get(p.name).unwrap();
        let changed = |key: &str| adj.iter().any(|a| a.key == key);
        for (role, star_color) in [
            (ColorRole::Background, p.bg),
            (ColorRole::Text, p.fg),
            (ColorRole::Heading1, p.h1),
            (ColorRole::Heading2, p.h2),
            (ColorRole::Heading3, p.h3),
            (ColorRole::Heading4, p.h4),
            (ColorRole::Link, p.link),
            (ColorRole::Code, p.code),
            (ColorRole::CodeBackground, p.code_bg),
            (ColorRole::DimText, p.muted),
        ] {
            let key = format!("colors.{}", role.key());
            if !changed(&key) {
                assert_eq!(t.color(role), star_color, "{} {key}", p.name);
            }
        }
        if !changed("styles.selection.background") {
            assert_eq!(
                t.style(StyleRole::Selection).background,
                Some(p.sel),
                "{}",
                p.name
            );
        }
        for a in &adj {
            assert!(
                a.ratio_before < a.minimum && a.ratio_after >= a.minimum,
                "{a:?}"
            );
            // Minimal: the change stays near the floor.
            assert!(a.ratio_after < a.minimum + 0.15, "{a:?}");
        }
    }
}

#[test]
fn every_theme_round_trips_through_toml() {
    for t in builtin::all() {
        let text = t.to_toml_string();
        let file = ThemeFile::parse(&text).unwrap();
        assert_eq!(ThemeFile::parse(&file.to_toml_string()).unwrap(), file);
        let (again, adj) = resolve(&file, None, None, Repair::DerivedOnly).unwrap();
        assert!(adj.is_empty());
        assert_eq!(&again, t, "{}", t.name());
    }
}

#[test]
fn counterparts_pair_up() {
    for t in builtin::all() {
        if let Some(c) = &t.meta.counterpart {
            let other: &Theme = builtin::get(c).unwrap();
            assert_eq!(other.meta.counterpart.as_deref(), Some(t.name()));
            assert_ne!(other.is_dark(), t.is_dark());
        }
    }
}

#[test]
fn names_and_aliases() {
    assert_eq!(builtin::get("obsidian").unwrap().name(), "galaxy");
    assert_eq!(builtin::get("zed-one-light").unwrap().name(), "one-light");
    assert_eq!(
        builtin::get(" HIGH_CONTRAST ").unwrap().name(),
        "high-contrast"
    );
    assert_eq!(builtin::get("Sepia").unwrap().name(), "sepia");
    assert!(builtin::get("solarized").is_none());
    assert!(builtin::source("nord").unwrap().contains("[colors]"));
}

#[test]
fn highlights_that_appear_together_differ_without_color() {
    for t in builtin::all() {
        let a = |r: StyleRole| t.style(r).attributes;
        assert_ne!(
            a(StyleRole::SpokenWord),
            a(StyleRole::SpokenSentence),
            "{}",
            t.name()
        );
        assert_ne!(
            a(StyleRole::FindHit),
            a(StyleRole::CurrentFindHit),
            "{}",
            t.name()
        );
        for &r in StyleRole::ALL {
            if r.is_highlight() {
                assert!(!a(r).is_empty(), "{} {r}", t.name());
            }
        }
    }
}

#[test]
fn gui_rgb_table_covers_every_role() {
    let t = builtin::get("sepia").unwrap();
    let table = t.rgb_table();
    assert_eq!(
        table.len(),
        ColorRole::COUNT + 2 * StyleRole::COUNT + 2 * t.user_highlights.len()
    );
    let get = |k: &str| table.iter().find(|(n, _)| n == k).map(|(_, c)| *c);
    assert_eq!(get("background"), Some(t.color(ColorRole::Background)));
    // Selection is reverse video: the table has the colors as shown.
    assert_eq!(get("selection.background"), Some(t.color(ColorRole::Text)));
    assert_eq!(
        get("bookmark.background"),
        Some(t.color(ColorRole::Background))
    );
    assert!(get("highlight.yellow.background").is_some());
}

#[test]
fn adjustments_table_lists_every_change() {
    let table = star::adjustments_table().unwrap();
    let rows = table.lines().count() - 2;
    let total: usize = star::PALETTES
        .iter()
        .map(|p| star::port(p).unwrap().1.len())
        .sum();
    assert_eq!(rows, total);
    assert!(table.contains("| galaxy | `colors.dim_text` | `#7d7d7d` |"));
}

#[test]
fn galaxy_is_the_default_and_faithful_to_star() {
    // The owner's theme: star's default, modeled on Obsidian's dark palette.
    let g = builtin::default_theme();
    assert_eq!(g.name(), "galaxy");
    assert_eq!(diroxide_theme::DEFAULT_THEME, "galaxy");
    assert_eq!(builtin::NAMES[0], "galaxy");
    assert_eq!(g.kind(), ThemeKind::Dark);
    assert_eq!(g.meta.counterpart.as_deref(), Some("galaxy-light"));
    // Every star value is kept except muted, which moved from 4.05 to 4.52
    // to 1, toward Obsidian's own muted gray.
    let (_, adj) = star::port(star::palette("galaxy").unwrap()).unwrap();
    assert_eq!(adj.len(), 1);
    assert_eq!(adj[0].key, "colors.dim_text");
    let hex = |r: ColorRole| g.color(r).hex();
    assert_eq!(hex(ColorRole::Background), "#1e1e1e");
    assert_eq!(hex(ColorRole::Text), "#dadada");
    assert_eq!(hex(ColorRole::Heading1), "#c9b6ff");
    assert_eq!(hex(ColorRole::Link), "#a882ff");
    assert_eq!(hex(ColorRole::DimText), "#858585");
    // The light pair passes too and points back.
    let gl = builtin::get("galaxy-light").unwrap();
    assert!(check(gl).passed());
    assert_eq!(gl.meta.counterpart.as_deref(), Some("galaxy"));
}

#[test]
fn themes_outside_the_required_set_keep_star_colors_exactly() {
    for p in star::PALETTES
        .iter()
        .filter(|p| !star::must_meet_aa(p.name))
    {
        let (_, adj) = star::port(p).unwrap();
        let explicit: Vec<_> = adj
            .iter()
            .filter(|a| {
                [
                    "colors.background",
                    "colors.text",
                    "colors.heading1",
                    "colors.heading2",
                    "colors.heading3",
                    "colors.heading4",
                    "colors.link",
                    "colors.code",
                    "colors.code_background",
                    "colors.dim_text",
                    "colors.error",
                ]
                .contains(&a.key.as_str())
            })
            .collect();
        assert!(explicit.is_empty(), "{}: {explicit:?}", p.name);
        let t = builtin::get(p.name).unwrap();
        assert_eq!(t.color(ColorRole::DimText), p.muted, "{}", p.name);
        assert_eq!(t.color(ColorRole::Text), p.fg, "{}", p.name);
    }
}
