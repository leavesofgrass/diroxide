// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Loading user themes from a folder: errors per file, warnings for low
//! contrast, unknown keys kept, built-ins replaceable, cycling in order.

use std::fs;

use diroxide_theme::{ColorRole, Registry, Rgb, ThemeError, builtin};

fn write(dir: &std::path::Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).unwrap();
}

#[test]
fn a_folder_of_themes() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let d = dir.path();
    write(
        d,
        "ocean.toml",
        "[theme]\ndisplay_name = \"Deep Ocean\"\nwave = 3\n\
         [colors]\nbackground = \"#001f2f\"\ntext = \"#e8f4f8\"\nheading1 = \"#7fdbff\"\n\
         [editor]\nfont = \"Atkinson Hyperlegible\"\n",
    );
    write(d, "broken.toml", "[colors\nbackground = \"#000000\"\n");
    write(
        d,
        "badcolor.toml",
        "[colors]\nbackground = \"#000000\"\ntext = \"white\"\n",
    );
    write(
        d,
        "faint.toml",
        "[colors]\nbackground = \"#ffffff\"\ntext = \"#999999\"\n",
    );
    write(
        d,
        "warm.toml",
        "[theme]\ninherits = \"sepia\"\n[colors]\nlink = \"#0b4f6c\"\n",
    );
    write(d, "orphan.toml", "[theme]\ninherits = \"no-such-theme\"\n");
    write(
        d,
        "nord.toml",
        "[theme]\nname = \"nord\"\ninherits = \"nord\"\ndescription = \"My Nord.\"\n",
    );
    write(
        d,
        "twin.toml",
        "[theme]\nname = \"ocean\"\n[colors]\nbackground = \"#000000\"\ntext = \"#ffffff\"\n",
    );
    write(d, "notes.txt", "not a theme");
    fs::create_dir(d.join("sub.toml")).unwrap();

    let mut reg = Registry::builtin();
    let report = reg.load_dir(d);
    assert_eq!(report.loaded, ["faint", "nord", "ocean", "warm"]);
    assert_eq!(report.replaced, ["nord"]);
    let bad: Vec<String> = report
        .errors
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        bad,
        ["badcolor.toml", "broken.toml", "orphan.toml", "twin.toml"]
    );
    assert!(matches!(
        report.errors[1].1,
        ThemeError::Syntax { line: Some(1), .. }
    ));
    assert!(matches!(report.errors[2].1, ThemeError::UnknownBase(_)));
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].theme, "faint");
    let summary = report.summary();
    assert!(summary.starts_with("Loaded 4 user themes."), "{summary}");
    assert!(
        summary.contains("badcolor.toml was not loaded: colors.text"),
        "{summary}"
    );
    assert!(summary.contains("Theme Faint: "), "{summary}");
    assert!(summary.contains("needs 4.5 to 1"), "{summary}");
    assert!(!report.is_clean());

    // User themes follow the built-ins; a replaced built-in keeps its place.
    let names = reg.names();
    assert_eq!(names.len(), 24 + 3);
    assert_eq!(names[20], "nord");
    assert_eq!(&names[24..], ["faint", "ocean", "warm"]);
    assert_eq!(reg.get("nord").unwrap().meta.description, "My Nord.");

    // Unknown keys survive a write.
    let ocean = reg.get("ocean").unwrap();
    assert_eq!(ocean.meta.display_name, "Deep Ocean");
    let text = ocean.to_toml_string();
    assert!(text.contains("wave = 3"), "{text}");
    assert!(
        text.contains("[editor]") && text.contains("Atkinson Hyperlegible"),
        "{text}"
    );

    // Inheriting: sepia's colors with one change.
    let warm = reg.get("warm").unwrap();
    let sepia = builtin::get("sepia").unwrap();
    assert_eq!(
        warm.color(ColorRole::Background),
        sepia.color(ColorRole::Background)
    );
    assert_eq!(warm.color(ColorRole::Link), Rgb::from_u32(0x0b4f6c));
    assert_eq!(warm.meta.inherits.as_deref(), Some("sepia"));
}

#[test]
fn missing_folder_loads_nothing() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut reg = Registry::builtin();
    let r = reg.load_dir(&dir.path().join("absent"));
    assert!(r.is_clean() && r.loaded.is_empty());
    assert_eq!(r.summary(), "No user themes loaded.");
}

#[test]
fn oversized_files_are_refused() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let big = format!(
        "[colors]\nbackground = \"#000000\"\ntext = \"#ffffff\"\n# {}\n",
        "x".repeat(300_000)
    );
    write(dir.path(), "big.toml", &big);
    let mut reg = Registry::builtin();
    let r = reg.load_dir(dir.path());
    assert!(matches!(r.errors[0].1, ThemeError::TooLarge(_)));
}

#[test]
fn cycling_and_fallback() {
    let reg = Registry::builtin();
    assert_eq!(reg.next("galaxy").name(), "galaxy-light");
    assert_eq!(reg.next("galaxy-light").name(), "high-contrast");
    assert_eq!(reg.next("contrast").name(), "lamplight");
    assert_eq!(reg.next("amber").name(), "one-dark");
    assert_eq!(reg.next("phosphor").name(), "galaxy");
    assert_eq!(reg.previous("galaxy").name(), "phosphor");
    // Unknown names start the cycle at the beginning (star skipped to the
    // second theme and saved it).
    assert_eq!(reg.next("solarized").name(), "galaxy");
    assert_eq!(reg.next("OBSIDIAN").name(), "galaxy-light");
    let (t, fell_back) = reg.resolve("solarized");
    assert!(fell_back);
    assert_eq!(t.name(), "galaxy");
    let (t, fell_back) = reg.resolve("Tokyo-Night");
    assert!(!fell_back);
    assert_eq!(t.name(), "tokyo-night");
    let f = reg.facts("solarized-dark").unwrap();
    assert_eq!(f.counterpart, Some("solarized-light"));
}
