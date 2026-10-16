//! Unit tests of the app core, driven through the runtime without a
//! terminal. The integration tests in `tests/it` drive whole sessions.

use super::*;
use crate::runtime::Runtime;
use diroxide_core::{FixedClock, Timestamp, UtcOffset};

fn key(text: &str) -> Key {
    Key::parse(text).unwrap()
}

fn here() -> Place {
    Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap()
}

/// Runs effects through a runtime until nothing is left, returning the
/// announcements in order.
fn settle(app: &mut App, runtime: &mut Runtime, effects: Vec<Effect>) -> Vec<String> {
    runtime
        .run_until_idle(app, effects)
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Announce(message) => Some(message.text),
            _ => None,
        })
        .collect()
}

fn app() -> (App, Runtime) {
    let mut options = AppOptions::new(here(), here().join("src").unwrap());
    options.clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 10, 10, 15, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    (App::new(options), Runtime::with_local())
}

#[test]
fn starting_lists_both_panels() {
    let (mut app, mut runtime) = app();
    let start = app.start();
    let said = settle(&mut app, &mut runtime, start);
    assert!(said[0].starts_with("diroxide-app, in "), "{said:?}");
    assert!(app.panel(Side::Left).list().len() >= 3);
    assert!(
        app.panel(Side::Right)
            .list()
            .rows()
            .iter()
            .any(|r| r.id == "app.rs")
    );
    assert!(app.title_line().starts_with("diroxide-app, in "));
    assert!(app.title_line().ends_with(", left panel"));
}

#[test]
fn keys_move_switch_and_enter() {
    let (mut app, mut runtime) = app();
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    let effects = app.handle(AppEvent::Key(key("tab")));
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(app.active_side(), Side::Right);
    assert!(said[0].starts_with("Right panel: src, "), "{said:?}");
    let effects = app.handle(AppEvent::Key(key("backspace")));
    let said = settle(&mut app, &mut runtime, effects);
    assert!(said[0].starts_with("diroxide-app, in "), "{said:?}");
    assert_eq!(app.active_panel().list().focused().unwrap().id, "src");
    let effects = app.handle(AppEvent::Key(key("enter")));
    settle(&mut app, &mut runtime, effects);
    assert_eq!(app.active_panel().place().name(), Some("src"));
    assert_eq!(app.active_panel().history().len(), 2);
}

#[test]
fn letters_jump_and_space_selects() {
    let (mut app, mut runtime) = app();
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    let effects = app.handle(AppEvent::Key(key("c")));
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(
        app.active_panel().list().focused().unwrap().id,
        "Cargo.toml"
    );
    assert_eq!(said, ["Cargo.toml, 1 of 1 matches"]);
    let effects = app.handle(AppEvent::Key(key("space")));
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, vec!["Selected Cargo.toml. 1 selected.".to_owned()]);
    assert_eq!(app.active_panel().targets().len(), 1);
}

#[test]
fn a_failed_listing_keeps_the_panel_where_it_was() {
    let (mut app, mut runtime) = app();
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    let missing = here().join("no-such-folder").unwrap();
    let effects = app.open(Side::Left, missing, None);
    let said = settle(&mut app, &mut runtime, effects);
    assert!(said[0].starts_with("Could not open "), "{said:?}");
    assert!(
        said[0].ends_with("it does not exist. Reload the folder."),
        "{said:?}"
    );
    assert_eq!(app.panel(Side::Left).place(), &here());
}

#[test]
fn quit_asks_the_frontend_to_leave() {
    let (mut app, _) = app();
    let effects = app.handle(AppEvent::Key(key("ctrl+q")));
    assert_eq!(effects, vec![Effect::Quit]);
    assert!(app.wants_quit());
    assert_eq!(app.quit_folder(), Some(&here()));
    assert!(
        app.take_requests()
            .iter()
            .any(|r| matches!(r, Request::SaveSession(_)))
    );
}
