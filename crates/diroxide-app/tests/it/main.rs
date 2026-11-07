//! Integration tests of the app core, driven without a terminal.

// Test helpers: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

mod access;
mod archives;
mod bookmarks;
mod braille;
mod checksums;
mod columns;
mod commands;
// Optional components (W4d).
mod components;
// Compare files (W3a).
mod comparing;
// The Convert command and the conversion reports (alpha 4, lane AW).
mod converting;
// The details view, Say Details, Copy details and Show all (alpha 4,
// lane VA).
mod details;
mod dialogs;
// The panel filter on the filter language (W3a).
mod filtering;
// What a frontend reaches beside keys and commands (lane A5).
mod frontend;
// A place behind a gate, for held jobs and streamed listings (lane A2).
mod held;
// Hooks through the plugin host (lane A2).
mod hooks;
// A plugin's forms, diffs and place descriptions (alpha 4, lane HP).
mod host_pieces;
// The transfer queue and the Jobs list (lane A2).
mod jobs;
mod keys;
mod labels;
// Full screen, focus mode and the themes (lane A6).
mod look;
// The access rows, Change time and Change owner (alpha 4, lane AP).
mod owner_time;
mod palette;
// Read text in picture (alpha 4, lane VA).
mod picture_text;
// The queue's settings, messages and questions (lane A2).
mod queue;
// The queue saved across restarts and its interrupted jobs (lane Q2).
mod saved_queue;
// A plugin's keys in keymap.toml and the Keys list (lane A2).
mod plugin_keys;
// Plugins and script commands (W4d).
mod plugins;
mod preview;
mod privacy;
// Saved searches, the Space list, sizes and newest files (W3a).
mod search_tools;
// Search, its dialog and its results (W3a).
mod searching;
// Selection on the filter language (W3a).
mod selecting;
mod session;
// Sites in the go-to-place list and the connect flow (lane A2).
mod sites;
// Free space in the plan and in Say Status (lane A3).
mod room;
// The Sites list's commands and dialogs (lane A3).
mod site_commands;
mod split_join;
// The Settings Tools page (alpha 4, lane VA).
mod tools_page;
// The trash browser (lane A3).
mod trash;
// Streamed listings reach the panel (lane A2).
mod streaming;
// File types in the app: the background check, typed actions, groups,
// Open with and the encoding chooser (alpha 4, lane TA).
mod types;
mod viewer;
mod views;
// The Volumes list (lane A3).
mod volumes;
// Watch this folder (W3t).
mod watching;
// The Watched folders list (alpha 4, lane AW).
mod watched_folders;
// Delete, Rename and Make folder inside a zip (alpha 4, lane AW).
mod zip_edits;

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
