//! Integration tests of the app core, driven without a terminal.

// Test helpers: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

mod archives;
mod braille;
mod commands;
mod dialogs;
mod session;

use diroxide_app::{App, AppEvent, AppOptions, Runtime};
use diroxide_core::{Effect, Key, Place, Side};

#[test]
fn every_row_says_its_meaning_within_forty_cells() {
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut app = App::new(AppOptions::new(here.clone(), here));
    let mut runtime = Runtime::with_local();
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    for row in app.panel(Side::Left).list().rows() {
        assert!(diroxide_core::format::within_cells(
            &row.spoken(),
            &row.label
        ));
    }
    let effects = app.handle(AppEvent::Key(Key::parse("down").unwrap()));
    let said: Vec<String> = runtime
        .run_until_idle(&mut app, effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Announce(m) => Some(m.text),
            _ => None,
        })
        .collect();
    let rows = app.panel(Side::Left).list().len();
    assert!(said[0].ends_with(&format!("2 of {rows}")), "{said:?}");
}
