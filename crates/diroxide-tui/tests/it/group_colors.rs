//! A theme file's group colors in the terminal (R30 commit 31, TA's
//! request 2, the fold lane): a plain file's row of a group the theme
//! maps is drawn in that color role's color, held to the text floor; the
//! words of every row are the same whatever the colors; without color, at
//! 16 colors and in a high contrast theme the row stays plain.

use std::fs;
use std::path::Path;

use diroxide_app::config::{Layout, Paths};
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_theme::{ColorRole, ColorSupport, PanelRole, Registry, Theme};
use diroxide_tui::{Session, Styles, TuiOptions};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

use crate::support::{Clock, place, settings, start, temp};

/// The owner's theme file mapping the Images group to heading 2's color,
/// over a built-in theme.
fn theme_file(name: &str, inherits: &str) -> String {
    format!(
        "[theme]\nname = \"{name}\"\ndisplay_name = \"{name}\"\ninherits = \"{inherits}\"\n\n[group-colors]\nimages = \"heading2\"\n"
    )
}

/// The text color of the first cell of the line that holds `name`.
fn fg_of(s: &Session<'_, TestBackend>, name: &str) -> (String, Option<Color>) {
    let buffer = s.terminal().backend().buffer();
    let area = buffer.area;
    for y in area.y..area.bottom() {
        let line: String = (area.x..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(at) = line.find(name) {
            let x = area.x + line[..at].chars().count() as u16;
            return (line.trim_end().to_owned(), buffer[(x, y)].style().fg);
        }
    }
    panic!("no line holds {name}");
}

/// The screen of a folder with a picture and a text file, the focus on
/// the folder above them, in `theme` at `support`; the two rows' lines
/// and text colors.
fn drawn(
    dir: &Path,
    theme: &str,
    support: ColorSupport,
) -> ((String, Option<Color>), (String, Option<Color>)) {
    let work = dir.join("work");
    fs::create_dir_all(work.join("Albums")).unwrap();
    fs::write(work.join("crow.png"), b"crow picture").unwrap();
    fs::write(work.join("notes.txt"), b"notes").unwrap();
    let paths = Paths::new(dir.join("config"), dir.join("state"));
    let themes_dir = paths.themes_dir();
    fs::create_dir_all(&themes_dir).unwrap();
    fs::write(themes_dir.join("crows.toml"), theme_file("crows", "galaxy")).unwrap();
    fs::write(
        themes_dir.join("stark.toml"),
        theme_file("stark", "high-contrast"),
    )
    .unwrap();
    let mut registry = Registry::builtin();
    let report = registry.load_dir(&themes_dir);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let themes: Vec<Theme> = report
        .loaded
        .iter()
        .filter_map(|name| registry.get(name).cloned())
        .collect();
    let mut settings = settings(Layout::SideBySide);
    settings.theme = theme.to_owned();
    let mut options = AppOptions::new(place(&work), place(&work));
    options.settings = settings;
    options.startup.user_themes = themes;
    options.startup.themes_dir = Some(themes_dir.clone());
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local();
    let terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let options = TuiOptions {
        screen_reader: true,
        color: Some(support),
        ..TuiOptions::default()
    };
    let mut s =
        Session::new(&mut app, &mut runtime, terminal, options).with_themes_dir(Some(themes_dir));
    let clock = Clock::at_start();
    start(&mut s, &clock);
    (fg_of(&s, "crow.png"), fg_of(&s, "notes.txt"))
}

#[test]
fn a_theme_files_group_color_draws_the_groups_rows_only() {
    let dir = temp();
    let ((crow_line, crow), (notes_line, notes)) =
        drawn(dir.path(), "crows", ColorSupport::TrueColor);
    // The words say the type either way; the color is the extra.
    assert!(crow_line.contains("crow.png"), "{crow_line}");
    assert!(notes_line.contains("notes.txt"), "{notes_line}");
    let styles = Styles::new(
        "crows",
        &Default::default(),
        Some(&dir.path().join("config").join("themes")),
        ColorSupport::TrueColor,
    );
    let heading2 = styles
        .group(ColorRole::Heading2, PanelRole::Text)
        .and_then(|s| s.fg);
    assert!(heading2.is_some());
    assert_eq!(crow, heading2, "{crow_line}");
    assert_eq!(notes, styles.get(PanelRole::Text).fg, "{notes_line}");
    assert_ne!(crow, notes);
}

#[test]
fn without_color_and_in_high_contrast_the_rows_stay_plain() {
    for (theme, support) in [
        ("crows", ColorSupport::NoColor),
        ("crows", ColorSupport::Ansi16),
        ("stark", ColorSupport::TrueColor),
    ] {
        let dir = temp();
        let ((crow_line, crow), (_, notes)) = drawn(dir.path(), theme, support);
        assert_eq!(crow, notes, "{theme} at {support:?}: {crow_line}");
    }
}

#[test]
fn a_group_color_keeps_the_text_floor_in_every_built_in_theme() {
    use diroxide_theme::check::{Requirement, minimum};
    let registry = Registry::builtin();
    for name in registry.names() {
        let theme = registry.get(name).unwrap();
        let styles = Styles::new(name, &Default::default(), None, ColorSupport::TrueColor);
        let floor = minimum(Requirement::Text, theme.kind());
        let page = diroxide_theme::role_style(theme, PanelRole::Text).background;
        for group in ColorRole::ALL {
            let Some(style) = styles.group(*group, PanelRole::Text) else {
                continue;
            };
            let Some(Color::Rgb(r, g, b)) = style.fg else {
                panic!("{name}: {group:?} is not a true color");
            };
            let ratio = diroxide_theme::contrast_ratio(diroxide_theme::Rgb::new(r, g, b), page);
            assert!(
                ratio >= floor,
                "{name}: {group:?} at {ratio:.2} under {floor}"
            );
        }
    }
}
