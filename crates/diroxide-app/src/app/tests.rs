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
    assert!(said[0].starts_with("diroxide-app, "), "{said:?}");
    assert!(said[0].contains(" items, in "), "{said:?}");
    assert!(app.panel(Side::Left).list().len() >= 3);
    assert!(
        app.panel(Side::Right)
            .list()
            .rows()
            .iter()
            .any(|r| r.id == "app.rs")
    );
    assert!(
        app.title_line()
            .starts_with("diroxide-app, left panel, in "),
        "{}",
        app.title_line()
    );
    let short = app.title_line_within(40);
    assert!(
        short.starts_with("diroxide-app, left panel, in ..."),
        "{short}"
    );
    assert!(short.chars().count() <= 40, "{short}");
    assert!(short.ends_with("crates"), "{short}");
}

#[test]
fn the_title_says_a_filter_and_a_sort_after_the_panel() {
    let (mut app, mut runtime) = app();
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    let effects = app.handle(AppEvent::Key(key("ctrl+f6")));
    settle(&mut app, &mut runtime, effects);
    let title = app.title_line();
    assert!(
        title.starts_with("diroxide-app, left panel, in "),
        "{title}"
    );
    assert!(title.ends_with(", sorted by size"), "{title}");
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
    assert!(said[0].starts_with("diroxide-app, "), "{said:?}");
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
    // The session is saved through the app's own request, which the core
    // effect at the end hands to the runtime.
    assert_eq!(effects, vec![Effect::Quit, Effect::AppRequests]);
    assert!(app.wants_quit());
    assert_eq!(app.quit_folder(), Some(&here()));
    assert!(!app.keeps_shell_folder());
    assert!(
        app.take_requests()
            .iter()
            .any(|r| matches!(r, Request::SaveSession(_)))
    );
}

/// The app's own requests travel as one core effect, which the runtime
/// takes where it stands and never hands to the frontend (W1d-1's
/// contract request 4).
#[test]
fn queued_requests_travel_as_one_core_effect_the_runtime_takes() {
    let (mut app, mut runtime) = app();
    let quiet = app.dispatch(&Command::new(ids::SAY_STATUS));
    assert!(!quiet.contains(&Effect::AppRequests), "{quiet:?}");
    let effects = app.handle(AppEvent::Key(key("ctrl+q")));
    let marks = effects
        .iter()
        .filter(|e| **e == Effect::AppRequests)
        .count();
    assert_eq!(marks, 1, "{effects:?}");
    let screen = runtime.carry_out(&mut app, effects);
    assert_eq!(screen, vec![Effect::Quit]);
    assert!(app.take_requests().is_empty(), "taken at the effect");
    assert!(runtime.execute(&Effect::AppRequests));
}

#[test]
fn quit_and_keep_the_shell_where_it_was() {
    let (mut app, _) = app();
    let command = Command::new(ids::QUIT_KEEP_FOLDER);
    let effects = app.handle(AppEvent::Command(command));
    assert_eq!(effects, vec![Effect::Quit, Effect::AppRequests]);
    assert!(app.wants_quit());
    assert!(app.keeps_shell_folder());
    assert_eq!(app.quit_folder(), None);
}
