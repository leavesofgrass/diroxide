//! Show picture (alpha 4, lane AE; G5b's request): the window's picture
//! view as the app's command. In the GUI column Enter runs it in the
//! viewer, and in the preview Enter on a picture (no link on its lines)
//! does too; the window takes the ask after the event. "Is a picture" is
//! the file's type through the type registry. A terminal says the window
//! has the picture view. Every picture is made in memory and written
//! under the test's own folder; none is fetched.

use std::fs;

use diroxide_app::app::picture_view::{
    NOT_IN_TERMINAL, NOTHING_SHOWN, PictureSource, hint_words, not_a_picture,
};
use diroxide_app::commands::{self, ids};
use diroxide_app::menu::Menus;
use diroxide_app::{Keymap, Lookup};
use diroxide_core::format::BRAILLE_CELLS;
use diroxide_core::{Key, KeyCode, KeyColumn, Layer};
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, place, png_bytes, temp};

/// A session on a folder holding `photo.png` and `notes.txt`, with the
/// keys of one column.
fn open(column: KeyColumn) -> (tempfile::TempDir, Session) {
    let dir = temp();
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("photo.png"), png_bytes(4, 3)).unwrap();
    fs::write(work.join("notes.txt"), "a line\n").unwrap();
    fs::create_dir_all(work.join("albums")).unwrap();
    let session = Session::open_with_keymap(
        place(&work),
        place(dir.path()),
        Registry::with_local(),
        Setup::default(),
        Keymap::defaults_for(column),
    );
    (dir, session)
}

#[test]
fn f3_in_the_window_asks_for_the_picture_at_once_with_no_viewer() {
    let (_dir, mut s) = open(KeyColumn::Gui);
    s.focus("photo.png");
    let said = s.key("f3");
    assert!(s.app.viewer().is_none(), "no hex on the way: {said:?}");
    let ask = s.app.take_picture_ask().expect("F3 asks for the picture");
    assert_eq!(ask.name, "photo.png");
    assert_eq!(ask.from, PictureSource::Panel);
    assert_eq!(ask.page, None);
    // Show picture from the palette or the View menu with nothing open.
    s.command(ids::VIEW_PICTURE);
    let ask = s.app.take_picture_ask().expect("Show picture asks too");
    assert_eq!(ask.from, PictureSource::Panel);
    // A text file still opens in the viewer.
    s.focus("notes.txt");
    s.key("f3");
    assert!(s.app.viewer().is_some());
    assert!(s.app.take_picture_ask().is_none());
}

#[test]
fn hex_mode_reaches_the_bytes_and_enter_in_the_windows_viewer_asks_for_the_picture() {
    let (_dir, mut s) = open(KeyColumn::Gui);
    s.focus("photo.png");
    // Hex mode with no viewer open: the picture's bytes (the road for
    // anyone who wants them).
    let said = s.command(ids::VIEWER_HEX);
    assert!(s.app.viewer().is_some(), "{said:?}");
    assert!(
        said.iter().any(|l| l == "photo.png looks binary; hex mode"),
        "{said:?}"
    );
    let hint = hint_words("Enter");
    assert_eq!(said.iter().filter(|l| **l == hint).count(), 1, "{said:?}");
    assert!(s.app.take_picture_ask().is_none(), "nothing asked yet");
    s.key("enter");
    let ask = s
        .app
        .take_picture_ask()
        .expect("Enter asks for the picture");
    assert_eq!(ask.name, "photo.png");
    assert_eq!(ask.from, PictureSource::Viewer);
    assert_eq!(ask.place.name(), Some("photo.png"));
    // Taken once: the window reads it, and the next event asks nothing.
    assert!(s.app.take_picture_ask().is_none());
    // The palette and the View menu reach it too.
    s.command(ids::VIEW_PICTURE);
    assert!(s.app.take_picture_ask().is_some());
}

#[test]
fn a_text_file_is_not_a_picture_and_says_so() {
    let (_dir, mut s) = open(KeyColumn::Gui);
    s.focus("notes.txt");
    let said = s.key("f3");
    assert!(!said.contains(&hint_words("Enter")), "{said:?}");
    let said = s.key("enter");
    assert_eq!(said.last().cloned(), Some(not_a_picture("notes.txt")));
    assert!(s.app.take_picture_ask().is_none());
}

#[test]
fn with_nothing_shown_or_in_a_terminal_it_says_why_in_words() {
    let (_dir, mut s) = open(KeyColumn::Gui);
    // A folder focused: nothing to show.
    s.focus("albums");
    let said = s.command(ids::VIEW_PICTURE);
    assert_eq!(said.last().map(String::as_str), Some(NOTHING_SHOWN));
    let (_dir, mut t) = open(KeyColumn::Terminal);
    t.focus("photo.png");
    let said = t.key("f3");
    assert!(t.app.viewer().is_some());
    assert!(!said.contains(&hint_words("Enter")), "{said:?}");
    // The terminal says how to see it, never a refusal, never hex.
    assert!(said.iter().any(|l| l == NOT_IN_TERMINAL), "{said:?}");
    assert!(!said.iter().any(|l| l.contains("hex mode")), "{said:?}");
    let said = t.command(ids::VIEW_PICTURE);
    assert_eq!(said.last().map(String::as_str), Some(NOT_IN_TERMINAL));
    assert_eq!(NOT_IN_TERMINAL, "Pictures show in the window: diro --gui");
    assert!(t.app.take_picture_ask().is_none());
    for line in [NOT_IN_TERMINAL, NOTHING_SHOWN] {
        assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
    }
}

#[test]
fn enter_in_the_preview_on_a_picture_asks_for_it() {
    let (_dir, mut s) = open(KeyColumn::Gui);
    s.focus("photo.png");
    s.command(ids::QUICK_VIEW);
    assert!(s.app.quick_view_on());
    // The quick view reads on its own thread: its ticks, in real time.
    for _ in 0..400 {
        if s.app.quick_view_wants_tick().is_none() {
            break;
        }
        let effects = s.app.quick_view_tick();
        s.run(effects);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let shown = s
        .app
        .quick_view()
        .and_then(|q| q.shown())
        .map(|shown| shown.name.clone());
    assert_eq!(shown.as_deref(), Some("photo.png"));
    let said = s.command(ids::SWITCH_PANEL);
    assert!(s.app.in_preview(), "{said:?}");
    assert!(said.contains(&hint_words("Enter")), "{said:?}");
    s.key("enter");
    let ask = s
        .app
        .take_picture_ask()
        .expect("Enter asks for the picture");
    assert_eq!(ask.from, PictureSource::Preview);
    assert_eq!(ask.name, "photo.png");
}

#[test]
fn the_command_is_in_the_catalog_the_view_menu_and_the_guis_viewer_keys() {
    let info = commands::info(&ids::VIEW_PICTURE).unwrap();
    assert_eq!(info.name, "Show picture");
    assert_eq!(info.layer, Layer::Viewer);
    assert!(info.palette_only, "no terminal key");
    let gui = Keymap::defaults_for(KeyColumn::Gui);
    let enter = Key::plain(KeyCode::Enter);
    assert_eq!(
        gui.lookup(&[Layer::Viewer, Layer::List, Layer::Global], &enter),
        Lookup::Command(ids::VIEW_PICTURE)
    );
    let terminal = Keymap::defaults_for(KeyColumn::Terminal);
    assert_eq!(
        terminal.lookup(&[Layer::Viewer, Layer::List, Layer::Global], &enter),
        Lookup::NoMatch
    );
    assert_eq!(Menus::builtin().holder_of(&ids::VIEW_PICTURE), Some("View"));
}
