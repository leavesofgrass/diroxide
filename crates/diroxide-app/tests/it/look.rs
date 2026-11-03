//! The window's look in the app (lane A6, G3's contract requests 1 and
//! 2): Full screen and Focus mode as commands with their words in both
//! columns. Every line is checked for its meaning inside the first 40
//! cells. Nothing touches the owner's folders; the config and state
//! folders are the test's.

use std::fs;

use diroxide_app::app::look::{
    FOCUS_MODE_OFF, FOCUS_MODE_ON, FOCUS_MODE_ON_IN_TERMINAL, FULL_SCREEN_IN_TERMINAL,
    FULL_SCREEN_OFF, FULL_SCREEN_ON,
};
use diroxide_app::commands::ids;
use diroxide_app::menu::Menus;
use diroxide_app::{Dialog, Keymap};
use diroxide_config::Paths;
use diroxide_core::KeyColumn;
use diroxide_core::format::within_cells;
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, fixture, place, temp};

/// A session on the fixture with its own config and state folders, with
/// the keys of one column.
fn open(column: KeyColumn) -> (tempfile::TempDir, Paths, Session) {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let session = open_on(&dir, &paths, column);
    (dir, paths, session)
}

/// A session on the fixture in `dir` with these folders, so a test can
/// put a theme file in the config folder before the start reads it.
fn open_on(dir: &tempfile::TempDir, paths: &Paths, column: KeyColumn) -> Session {
    let work = fixture(dir.path());
    Session::open_with_keymap(
        place(&work),
        place(&work.join("Backup")),
        Registry::with_local(),
        Setup {
            paths: Some(paths.clone()),
            ..Setup::default()
        },
        Keymap::defaults_for(column),
    )
}

/// The ids of the top dialog's rows.
fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.id.clone()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

/// Opens the Settings list on a setting's row and presses Enter there.
fn enter_setting(s: &mut Session, key: &str) -> Vec<String> {
    s.command(ids::SETTINGS);
    let at = rows(s).iter().position(|r| r == key).unwrap();
    assert!(s.app.focus_dialog_row(at));
    s.key("enter")
}

/// The config file's text as the runtime wrote it.
fn config_text(paths: &Paths) -> String {
    fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap_or_default()
}

#[test]
fn every_line_says_its_meaning_in_the_first_40_cells() {
    for (line, meaning) in [
        (FULL_SCREEN_ON, "Full screen on"),
        (FULL_SCREEN_OFF, "Full screen off"),
        (FULL_SCREEN_IN_TERMINAL, "Full screen is the window's"),
        (FOCUS_MODE_ON, "Focus mode on: the bars fade"),
        (FOCUS_MODE_ON_IN_TERMINAL, "Focus mode on, for the window"),
        (FOCUS_MODE_OFF, "Focus mode off"),
    ] {
        assert!(within_cells(line, meaning), "{line}");
    }
}

#[test]
fn both_commands_are_in_the_view_menu_and_full_screen_has_f11_in_the_gui() {
    let menus = Menus::builtin();
    for id in [ids::FULL_SCREEN, ids::FOCUS_MODE] {
        assert!(menus.reaches(&id), "{id}");
    }
    let gui = Keymap::defaults_for(KeyColumn::Gui);
    let keys: Vec<String> = gui
        .keys_for(&ids::FULL_SCREEN)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(keys, ["F11"]);
    let terminal = Keymap::defaults_for(KeyColumn::Terminal);
    assert!(terminal.keys_for(&ids::FULL_SCREEN).is_empty());
    assert!(terminal.keys_for(&ids::FOCUS_MODE).is_empty());
    assert!(gui.keys_for(&ids::FOCUS_MODE).is_empty());
}

#[test]
fn full_screen_in_the_window_goes_on_and_off_and_is_said() {
    let (_dir, _paths, mut s) = open(KeyColumn::Gui);
    assert!(!s.app.full_screen());
    assert_eq!(s.command(ids::FULL_SCREEN), [FULL_SCREEN_ON]);
    assert!(s.app.full_screen());
    assert_eq!(s.command(ids::FULL_SCREEN), [FULL_SCREEN_OFF]);
    assert!(!s.app.full_screen());
    // F11, the GUI column's key, runs the same command.
    assert_eq!(s.key("f11"), [FULL_SCREEN_ON]);
    assert!(s.app.full_screen());
    // The window left full screen by itself: nothing said, and the next
    // Full screen turns it on again.
    s.app.set_full_screen(false);
    assert_eq!(s.command(ids::FULL_SCREEN), [FULL_SCREEN_ON]);
    // A dialog keeps the window as it is.
    s.command(ids::SETTINGS);
    assert_eq!(s.command(ids::FULL_SCREEN), ["Close this dialog first."]);
    assert!(s.app.full_screen());
}

#[test]
fn full_screen_in_a_terminal_says_why_not_and_changes_nothing() {
    let (_dir, _paths, mut s) = open(KeyColumn::Terminal);
    assert_eq!(s.command(ids::FULL_SCREEN), [FULL_SCREEN_IN_TERMINAL]);
    assert!(!s.app.full_screen());
}

#[test]
fn focus_mode_from_the_palette_is_said_and_the_setting_kept_in_step() {
    let (_dir, paths, mut s) = open(KeyColumn::Gui);
    assert!(!s.app.settings().focus_mode);
    s.key("ctrl+p");
    s.type_text("focus mode");
    let said = s.key("enter");
    assert_eq!(said.last().map(String::as_str), Some(FOCUS_MODE_ON));
    assert!(s.app.settings().focus_mode);
    assert!(config_text(&paths).contains("focus_mode = true"));
    assert_eq!(s.command(ids::FOCUS_MODE), [FOCUS_MODE_OFF]);
    assert!(!s.app.settings().focus_mode);
    assert!(config_text(&paths).contains("focus_mode = false"));
    // The Settings list's switch says the same words.
    let said = enter_setting(&mut s, "focus_mode");
    assert_eq!(said.last().map(String::as_str), Some(FOCUS_MODE_ON));
    assert!(s.app.settings().focus_mode);
}

#[test]
fn focus_mode_from_a_terminal_is_kept_for_the_window() {
    let (_dir, paths, mut s) = open(KeyColumn::Terminal);
    assert_eq!(s.command(ids::FOCUS_MODE), [FOCUS_MODE_ON_IN_TERMINAL]);
    assert!(s.app.settings().focus_mode);
    assert!(config_text(&paths).contains("focus_mode = true"));
    assert_eq!(s.command(ids::FOCUS_MODE), [FOCUS_MODE_OFF]);
}
