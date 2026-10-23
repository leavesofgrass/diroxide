//! Integration tests of the app core, driven without a terminal.

// Test helpers: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

mod access;
mod archives;
mod bookmarks;
mod braille;
mod commands;
// Compare files (W3a).
mod comparing;
mod dialogs;
// The panel filter on the filter language (W3a).
mod filtering;
mod keys;
mod labels;
mod palette;
// Plugins and script commands (W4d).
mod plugins;
mod preview;
// Saved searches, the Space list, sizes and newest files (W3a).
mod search_tools;
// Search, its dialog and its results (W3a).
mod searching;
// Selection on the filter language (W3a).
mod selecting;
mod session;
mod split_join;
mod viewer;
mod views;
// Watch this folder (W3t).
mod watching;

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
