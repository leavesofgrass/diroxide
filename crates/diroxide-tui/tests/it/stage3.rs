//! Alpha 2's stage 3 screens in the terminal (lane A3): the first-run
//! components list with nothing chosen, Space marking a row and Enter
//! asking once on No, the hardware cursor on the focused row's first cell
//! throughout; and Say Status with the place's free space.

use std::fs;
use std::path::Path;

use diroxide_app::commands::ids;
use diroxide_app::config::Layout;
use diroxide_app::{AppEvent, Runtime};
use diroxide_core::Command;

use crate::support::{
    Clock, app, cursor_at, fixture, line_of, place, press, session, settings, settle, start, temp,
};

/// A folder source with one small pack, "Test pack", as the components
/// tests use; nothing is fetched from the network.
fn source(root: &Path) -> String {
    let folder = root.join("source");
    fs::create_dir_all(folder.join("test-pack-0.1.0")).unwrap();
    fs::write(folder.join("test-pack-0.1.0").join("pack.txt"), b"abc").unwrap();
    fs::write(
        folder.join("components.toml"),
        r#"format = 1

[[component]]
id = "test-pack"
title = "Test pack"
kind = "data"
version = "0.1.0"
license = "MIT"
action = "place"

[[component.file]]
name = "pack.txt"
size = 3
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
target = "any"
"#,
    )
    .unwrap();
    folder.to_string_lossy().into_owned()
}

#[test]
fn the_first_run_list_marks_with_space_and_asks_once_on_no() {
    let dir = temp();
    let work = fixture(dir.path());
    let mut settings = settings(Layout::Stacked);
    settings.components_source = source(dir.path());
    settings.components_first_run_shown = true;
    let (mut a, _) = app(place(&work), place(&work), settings);
    let paths =
        diroxide_app::config::Paths::new(dir.path().join("config"), dir.path().join("state"));
    let mut r = Runtime::with_local()
        .with_components_offline()
        .with_paths(paths);
    let mut s = session(&mut a, &mut r, 100, 30, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    s.event(
        AppEvent::Command(Command::new(ids::COMPONENTS_FIRST_RUN)),
        clock.now(),
    );
    settle(&mut s, &clock);
    let screen = s.screen();
    line_of(&screen, "Optional components to get now");
    let row = line_of(&screen, "Test pack: not installed, 3 bytes");
    // Nothing chosen: no row is marked.
    assert!(
        !screen.lines.iter().any(|l| l.contains(", selected")),
        "{:#?}",
        screen.lines
    );
    // Walk to the pack's row; the cursor sits on its first cell.
    for _ in 0..10 {
        if s.screen().cursor_line == Some(row) {
            break;
        }
        press(&mut s, &clock, "down");
    }
    cursor_at(&s, 0, row);
    press(&mut s, &clock, "space");
    let marked = line_of(&s.screen(), "Test pack: not installed, 3 bytes");
    assert!(
        s.screen().lines[marked].contains(", selected"),
        "{:#?}",
        s.screen().lines
    );
    press(&mut s, &clock, "up");
    cursor_at(&s, 0, marked);
    press(&mut s, &clock, "enter");
    let screen = s.screen();
    let question = line_of(&screen, "Download 1 component? No is the default. y or n");
    cursor_at(&s, 0, question);
    line_of(&screen, "Test pack: not installed, 3 bytes, license MIT");
    // No goes back to the list, the mark kept.
    press(&mut s, &clock, "n");
    let screen = s.screen();
    let again = line_of(&screen, "Test pack: not installed, 3 bytes");
    assert!(screen.lines[again].contains(", selected"));
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
}

#[test]
fn say_status_shows_the_free_space_of_the_place() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut a, mut r) = app(place(&work), place(&work), settings(Layout::Stacked));
    let mut s = session(&mut a, &mut r, 120, 30, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    press(&mut s, &clock, "ctrl+d");
    let screen = s.screen();
    assert!(
        screen.lines.iter().any(|l| l.contains(" free of ")),
        "{:#?}",
        screen.lines
    );
}
