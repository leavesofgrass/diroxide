//! The themes in the window (R27's commit 27): the window draws the
//! chosen theme from its first frame, another theme chosen repaints it
//! and is said once with its contrast verdict in words, the system's
//! high contrast colors win while that mode is on, and following the
//! system's appearance takes the theme's light or dark partner. The
//! owner's theme files in the config's themes folder are drawn as the
//! terminal draws them (R30's commit 16). Drawn with Vello's CPU renderer
//! in the harness, no graphics card.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use diroxide_app::config::Paths;
use diroxide_gui::theme::{NIGHT_SKY, Palette};
use diroxide_theme::os::OsScheme;
use diroxide_theme::{Registry, Rgb, Theme};

use crate::rig::{Rig, app_on, app_with, fixture};

/// The owner's theme file of the test: Galaxy on a darker, bluer page.
const HARBOR: &str = "[theme]\nname = \"harbor\"\ndisplay_name = \"Harbor\"\n\
                      inherits = \"galaxy\"\n\n[colors]\nbackground = \"#0f1a2a\"\n";

/// Harbor's page, as its file gives it.
const HARBOR_PAGE: Rgb = Rgb::new(0x0f, 0x1a, 0x2a);

/// The color of the window's page as drawn: a pixel of the title line's
/// margin, which only the page fills.
fn page_drawn(rig: &mut Rig) -> Rgb {
    let image = rig.h.render();
    let px = image.get_pixel(2, 2).0;
    Rgb::new(px[0], px[1], px[2])
}

fn page_of(theme: &str) -> Rgb {
    diroxide_theme::builtin::get(theme)
        .unwrap()
        .color(diroxide_theme::ColorRole::Background)
}

#[test]
fn next_theme_repaints_the_window_and_says_its_verdict_once() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert_eq!(rig.f.palette().key, "galaxy");
    assert_eq!(page_drawn(&mut rig), page_of("galaxy"));
    let from = rig.spoken().len();
    rig.run("app.next-theme");
    assert_eq!(rig.f.palette().key, "galaxy-light");
    assert_eq!(page_drawn(&mut rig), page_of("galaxy-light"));
    // Said once, the theme first, its verdict after, inside one line.
    let spoken = rig.spoken()[from..].to_vec();
    let verdicts: Vec<&String> = spoken
        .iter()
        .filter(|l| l.contains("Theme Galaxy Light, meets AA"))
        .collect();
    assert_eq!(verdicts.len(), 1, "{spoken:?}");
    assert!(verdicts[0].starts_with("Theme Galaxy Light, meets AA"));
    assert_eq!(verdicts[0].matches("Theme Galaxy Light").count(), 1);
}

#[test]
fn a_theme_chosen_in_settings_is_said_with_its_verdict() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.run("app.settings");
    rig.focus_dialog_row("theme");
    rig.press("enter");
    rig.focus_dialog_row("one-light");
    let from = rig.spoken().len();
    rig.press("enter");
    assert_eq!(rig.f.palette().key, "one-light");
    let spoken = rig.spoken()[from..].join(" ");
    // The setting's plain line gives way to the verdict, and the window
    // says how many of its roles use the text color: One Light is
    // below AA in the theme crate's checks, and drawn at AA here.
    assert!(!spoken.contains("Theme: one-light"), "{spoken}");
    assert!(spoken.contains("Theme One Light, below AA: "), "{spoken}");
    assert!(
        spoken.contains("In the window, ") && spoken.contains(" roles use the text color."),
        "{spoken}"
    );
    assert!(rig.f.palette().report().passed());
}

#[test]
fn the_systems_high_contrast_colors_win_and_are_said() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let from = rig.spoken().len();
    rig.f.set_system_colors(Some(NIGHT_SKY));
    rig.refresh();
    assert_eq!(rig.f.palette().key, "system");
    assert_eq!(page_drawn(&mut rig), NIGHT_SKY.window);
    let spoken = rig.spoken()[from..].join(" ");
    assert!(
        spoken.contains("System high contrast colors in use."),
        "{spoken}"
    );
    // Turned off, the theme comes back.
    rig.f.set_system_colors(None);
    rig.refresh();
    assert_eq!(rig.f.palette().key, "galaxy");
}

#[test]
fn following_the_system_takes_the_light_partner_until_a_theme_is_chosen() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_with(dir.path(), dir.path(), |options| {
        options.settings.follow_system_appearance = true;
    });
    let mut f = diroxide_gui::frontend::Frontend::new(app, runtime, "diroxide");
    f.set_appearance(OsScheme::Light);
    assert_eq!(f.colors_choice().theme, "galaxy-light");
    let palette: Palette = Palette::of(&f.colors_choice(), &Registry::builtin());
    assert_eq!(palette.page, page_of("galaxy-light"));
    // Dark: the saved theme as it is.
    f.set_appearance(OsScheme::Dark);
    assert_eq!(f.colors_choice().theme, "galaxy");
    // Following off: the setting, whatever the system says.
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut f = diroxide_gui::frontend::Frontend::new(app, runtime, "diroxide");
    f.set_appearance(OsScheme::Light);
    if cfg!(windows) {
        assert_eq!(f.colors_choice().theme, "galaxy");
    }
}

/// A test config folder holding the owner's theme file, and the themes
/// that load from its themes folder, read with the theme crate's loader as
/// the app's start reads them (`diroxide_app::startup::load`).
fn harbor_config() -> (tempfile::TempDir, PathBuf, Vec<Theme>) {
    let dir = fixture(&["notes.txt"], &[]);
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let themes_dir = paths.themes_dir();
    std::fs::create_dir_all(&themes_dir).unwrap();
    std::fs::write(themes_dir.join("harbor.toml"), HARBOR).unwrap();
    let mut registry = Registry::builtin();
    let report = registry.load_dir(&themes_dir);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let themes = report
        .loaded
        .iter()
        .filter_map(|name| registry.get(name).cloned())
        .collect();
    (dir, themes_dir, themes)
}

#[test]
fn a_theme_file_in_the_config_folder_is_drawn_by_the_window() {
    let (dir, themes_dir, themes) = harbor_config();
    assert_eq!(themes.len(), 1);
    let (app, runtime) = app_with(dir.path(), dir.path(), |options| {
        options.settings.theme = "harbor".to_owned();
        options.startup.user_themes = themes;
        options.startup.themes_dir = Some(themes_dir);
    });
    let mut rig = Rig::new(app, runtime);
    // Drawn from its first frame, by its own name, with its own page.
    assert_eq!(rig.f.palette().key, "harbor");
    assert_eq!(rig.f.palette().name, "Harbor");
    assert_eq!(page_drawn(&mut rig), HARBOR_PAGE);
    assert!(rig.f.palette().report().passed());
    // A theme file is known: nothing is said about it at the start.
    let spoken = rig.spoken().join(" ");
    assert!(!spoken.contains("Unknown theme"), "{spoken}");
    // Next theme goes on from it to the first built-in theme.
    rig.run("app.next-theme");
    assert_eq!(rig.f.palette().key, "galaxy");
    assert_eq!(page_drawn(&mut rig), page_of("galaxy"));
    // Chosen again in Settings: drawn, and said once by its own name,
    // never as the default it would be among the built-in themes alone.
    rig.run("app.settings");
    rig.focus_dialog_row("theme");
    rig.press("enter");
    rig.focus_dialog_row("harbor");
    let from = rig.spoken().len();
    rig.press("enter");
    assert_eq!(rig.f.palette().key, "harbor");
    let spoken = rig.spoken()[from..].join(" ");
    assert_eq!(
        spoken.matches("Theme Harbor, meets AA").count(),
        1,
        "{spoken}"
    );
    assert!(!spoken.contains("Theme Galaxy"), "{spoken}");
    // The Settings list closed, the page is Harbor's again.
    rig.press("escape");
    assert_eq!(page_drawn(&mut rig), HARBOR_PAGE);
}
