//! The flat view, collections, recent files and the place list for
//! `diro -`, driven through the runtime on a folder of the test's own:
//! every collection and recent file is kept in that folder's state
//! folder, and no test opens, edits or runs a file.

use std::fs;
use std::path::Path;

use diroxide_config::Paths;
use diroxide_core::{FixedClock, Key, Timestamp, UtcOffset};

use super::*;
use crate::app::{AppEvent, AppOptions};
use crate::runtime::Runtime;

struct Fixture {
    _dir: tempfile::TempDir,
    root: Place,
    state: PathBuf,
}

/// A folder `work` with a.txt, b/one.txt and b/deep/two.txt, and a state
/// folder beside it.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    fs::create_dir_all(work.join("b").join("deep")).unwrap();
    fs::write(work.join("a.txt"), "a").unwrap();
    fs::write(work.join("b").join("one.txt"), "one").unwrap();
    fs::write(work.join("b").join("deep").join("two.txt"), "two").unwrap();
    let state = dir.path().join("state");
    fs::create_dir_all(&state).unwrap();
    let root = Place::parse(&work.to_string_lossy()).unwrap();
    Fixture {
        _dir: dir,
        root,
        state,
    }
}

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

fn started(fixture: &Fixture) -> (App, Runtime) {
    let mut options = AppOptions::new(fixture.root.clone(), fixture.root.clone());
    options.clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 11, 12, 10, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    options.settings.dialog_settle_ms = 0;
    let mut app = App::new(options);
    let paths = Paths::new(fixture.state.join("config"), fixture.state.clone());
    let mut runtime = Runtime::with_local().with_paths(paths);
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    (app, runtime)
}

fn run(app: &mut App, runtime: &mut Runtime, id: CommandId) -> Vec<String> {
    let effects = app.dispatch(&Command::new(id));
    settle(app, runtime, effects)
}

fn press(app: &mut App, runtime: &mut Runtime, text: &str) -> Vec<String> {
    let effects = app.handle(AppEvent::Key(Key::parse(text).unwrap()));
    settle(app, runtime, effects)
}

fn typed(app: &mut App, runtime: &mut Runtime, text: &str) -> Vec<String> {
    if let Some((_, Dialog::Edit { field, .. })) = app.dialogs.last_mut() {
        field.set_text(text);
    }
    let effects = app.accept();
    settle(app, runtime, effects)
}

fn focus(app: &mut App, name: &str) {
    let side = app.active;
    app.panels[side.index()].list_mut().focus_id(name);
}

fn top_rows(app: &App) -> Vec<ListRow> {
    match app.dialogs.last() {
        Some((_, Dialog::Choose { list, .. })) => list.rows().to_vec(),
        _ => Vec::new(),
    }
}

fn focus_row(app: &mut App, id: &str) {
    if let Some((_, Dialog::Choose { list, .. })) = app.dialogs.last_mut() {
        assert!(list.focus_id(id), "no row {id}");
    }
}

fn close(app: &mut App, runtime: &mut Runtime) {
    while !app.dialogs.is_empty() {
        let effects = app.dialog_escape();
        settle(app, runtime, effects);
    }
}

fn cells(line: &str) -> usize {
    line.chars().count()
}

fn exists(fixture: &Fixture, name: &str) -> bool {
    Path::new(&fixture.root.join(name).unwrap().to_string()).exists()
}

#[test]
fn save_as_collection_then_open_it_and_the_trash_key_keeps_the_file() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    focus(&mut app, "a.txt");
    let said = run(&mut app, &mut runtime, ids::SAVE_AS_COLLECTION);
    assert_eq!(said, ["Collection name for 1 item, empty"]);
    let said = typed(&mut app, &mut runtime, "Trip photos");
    assert_eq!(said, ["Added 1 to Trip photos."]);

    let said = run(&mut app, &mut runtime, ids::COLLECTIONS);
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("Collections, 1 list, Trip photos, collection, 1 item"),
        "{said:?}"
    );
    let rows = top_rows(&app);
    let words = "Trip photos, collection, 1 item";
    assert_eq!(rows[0].label, words);
    assert_eq!(cells(words), 31);
    assert_eq!(rows[1].label, "Rename a collection");
    assert_eq!(rows[2].label, "Remove a collection");

    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, ["Trip photos, collection, 1 item"]);
    assert_eq!(
        app.list_kind(),
        Some(&ListKind::Collection("Trip photos".to_owned()))
    );

    // The trash key takes the item out; the file stays.
    let said = press(&mut app, &mut runtime, "delete");
    let words = "Removed 1 from Trip photos. The file was not touched.";
    assert_eq!(said, [words]);
    assert_eq!(cells("Removed 1 from Trip photos."), 27);
    assert!(exists(&fixture, "a.txt"));
    assert!(top_rows(&app).is_empty());
}

#[test]
fn a_missing_item_is_said_and_remove_missing_takes_it_out() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let collections = diroxide_search::Collections::in_state_folder(&fixture.state);
    let gone = fixture.root.join("gone.txt").unwrap();
    let kept = fixture.root.join("a.txt").unwrap();
    collections
        .add("Trip photos", [kept, gone.clone()], app.clock.as_ref())
        .unwrap();

    let said = run(&mut app, &mut runtime, ids::COLLECTIONS);
    assert!(said[0].starts_with("Collections, 1 list"), "{said:?}");
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    let start = "Trip photos, collection, 2 items, 1 missing";
    assert_eq!(said, [start]);
    let rows = top_rows(&app);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].label, "gone.txt");
    assert_eq!(rows[1].details.first().map(String::as_str), Some("missing"));

    let said = run(&mut app, &mut runtime, ids::COLLECTION_REMOVE_MISSING);
    assert_eq!(
        said,
        ["Removed 1 from Trip photos. The file was not touched."]
    );
    assert_eq!(top_rows(&app).len(), 1);
    let said = run(&mut app, &mut runtime, ids::COLLECTION_REMOVE_MISSING);
    assert_eq!(said, ["Nothing missing in Trip photos."]);
}

#[test]
fn remove_missing_outside_a_collection_says_where_to_go() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let said = run(&mut app, &mut runtime, ids::COLLECTION_REMOVE_MISSING);
    let line = "Open a collection first: Collections, in the Go menu.";
    assert_eq!(said, [line]);
    assert!(line.starts_with("Open a collection first"));
}

#[test]
fn collect_adds_to_collected_and_says_what_was_there() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    focus(&mut app, "a.txt");
    let said = run(&mut app, &mut runtime, ids::COLLECT);
    assert_eq!(said, ["Added 1 to Collected."]);
    let said = run(&mut app, &mut runtime, ids::COLLECT);
    assert_eq!(said, ["Added 0 to Collected. 1 was there already."]);
}

#[test]
fn the_collections_list_says_when_there_are_none() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let said = run(&mut app, &mut runtime, ids::COLLECTIONS);
    let line = "No collections yet. Save as collection makes one.";
    assert_eq!(said, [line]);
    assert_eq!(cells("No collections yet."), 19);
}

#[test]
fn a_saved_search_is_listed_as_a_live_collection_and_runs_on_open() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let query = diroxide_search::SearchQuery::everything(fixture.root.clone());
    let saved = diroxide_search::Saved::search("Large images", &query, app.clock.as_ref());
    saved
        .save(&fixture.state.join(diroxide_search::SAVED_FOLDER))
        .unwrap();
    let said = run(&mut app, &mut runtime, ids::COLLECTIONS);
    let words = "Large images, saved search, runs on open";
    assert_eq!(cells(words), 40);
    assert_eq!(top_rows(&app)[0].label, words);
    // No collection, so no rename and remove rows.
    assert_eq!(top_rows(&app).len(), 1);
    assert!(said[0].starts_with("Collections, 1 list"));
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert!(
        said.first()
            .is_some_and(|l| l.starts_with("Running Large images")),
        "{said:?}"
    );
    assert_eq!(app.list_kind(), Some(&ListKind::Search));
}

#[test]
fn rename_and_remove_a_collection_from_the_collections_list() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let collections = diroxide_search::Collections::in_state_folder(&fixture.state);
    let item = fixture.root.join("a.txt").unwrap();
    collections
        .add("Trip photos", [item], app.clock.as_ref())
        .unwrap();

    run(&mut app, &mut runtime, ids::COLLECTIONS);
    focus_row(&mut app, RENAME_ROW);
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert!(said[0].starts_with("Rename which collection"), "{said:?}");
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert!(said[0].starts_with("New name for Trip photos"), "{said:?}");
    let said = typed(&mut app, &mut runtime, "Trip 2026");
    assert_eq!(said, ["Renamed Trip photos to Trip 2026."]);

    run(&mut app, &mut runtime, ids::COLLECTIONS);
    focus_row(&mut app, REMOVE_ROW);
    let effects = app.accept();
    settle(&mut app, &mut runtime, effects);
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert!(
        said[0].starts_with("Remove collection Trip 2026? No is the default."),
        "{said:?}"
    );
    // Escape: nothing changes.
    let effects = app.dialog_escape();
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, ["Nothing removed; Trip 2026 stays."]);
    assert!(collections.find("Trip 2026").is_some());
    assert!(exists(&fixture, "a.txt"));
}

#[test]
fn the_flat_view_lists_every_file_below_and_turns_off() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let said = run(&mut app, &mut runtime, ids::FLAT_VIEW);
    assert_eq!(said, ["Flat view of work, Start the flat view, 1 of 4"]);
    let labels: Vec<String> = top_rows(&app).into_iter().map(|r| r.label).collect();
    assert_eq!(
        labels,
        [
            "Start the flat view",
            "Depth: every level",
            "Files only: yes",
            "Hidden files: left out"
        ]
    );

    // A setting changes and is said as the form opens on it.
    focus_row(&mut app, "files-only");
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(
        said,
        ["Flat view of work, Files only: no, folders too, 3 of 4"]
    );
    let effects = app.accept();
    settle(&mut app, &mut runtime, effects);

    focus_row(&mut app, "start");
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    let done = "Flat view on, 3 files in 3 folders.";
    assert_eq!(said, ["Flat view of work: listing files only", done]);
    assert_eq!(cells("Flat view on, 14 files in 4 folders"), 35);
    assert_eq!(app.list_kind(), Some(&ListKind::Flat));
    let mut labels: Vec<String> = top_rows(&app).into_iter().map(|r| r.label).collect();
    labels.sort();
    assert_eq!(labels, ["a.txt", "one.txt", "two.txt"]);

    let said = run(&mut app, &mut runtime, ids::FLAT_VIEW);
    assert_eq!(said, ["Flat view off. work."]);
    assert_eq!(cells("Flat view off. work."), 20);
    assert!(app.dialogs.is_empty());
}

#[test]
fn the_flat_view_takes_a_depth() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    run(&mut app, &mut runtime, ids::FLAT_VIEW);
    focus_row(&mut app, "depth");
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert!(
        said[0].starts_with("Depth, empty or 0 for every level"),
        "{said:?}"
    );
    let said = typed(&mut app, &mut runtime, "two");
    assert_eq!(
        said,
        ["Depth is a number of levels: 1 for this folder's files."]
    );
    let said = typed(&mut app, &mut runtime, "1");
    assert_eq!(said, ["Flat view of work, Depth: 1 level, 2 of 4"]);
    focus_row(&mut app, "start");
    let effects = app.accept();
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said[1], "Flat view on, 1 file in 1 folder.");
}

#[test]
fn a_stopped_flat_view_says_where_it_stopped() {
    let summary_words = {
        let fixture = fixture();
        let control = diroxide_search::SearchControl::new();
        control.cancel();
        let registry = diroxide_vfs::Registry::with_local();
        let mut sink = diroxide_search::FnSink(|_| {});
        let summary = diroxide_search::flat_view(
            &fixture.root,
            &FlatOptions::new(),
            &registry,
            &control,
            &mut sink,
        )
        .unwrap();
        summary.on_words()
    };
    assert!(summary_words.starts_with("Stopped at "), "{summary_words}");
    assert_eq!(cells("Stopped at 4,200 entries"), 24);
}

#[test]
fn recent_files_are_kept_listed_newest_first_and_cleared() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let said = run(&mut app, &mut runtime, ids::RECENT_FILES);
    assert_eq!(said, ["Recent: no files."]);

    // F4 on a file asks for it to be kept; the editor is not started
    // here, only the ask is made.
    focus(&mut app, "a.txt");
    app.note_recent(&ids::EDIT);
    focus(&mut app, "b");
    app.note_recent(&ids::ENTER);
    let asks: Vec<ListsAsk> = app
        .requests
        .drain(..)
        .filter_map(|request| match request {
            Request::Lists(ask) => Some(*ask),
            _ => None,
        })
        .collect();
    assert_eq!(asks.len(), 1, "a folder is never kept: {asks:?}");
    let effects = Vec::new();
    for ask in asks {
        app.lists_ask(ask);
    }
    settle(&mut app, &mut runtime, effects);
    let now = app.now();
    let one = fixture.root.join("b").unwrap().join("one.txt").unwrap();
    app.lists_ask(ListsAsk::Record {
        place: one,
        action: RecentAction::Opened,
        now,
    });
    settle(&mut app, &mut runtime, Vec::new());

    let said = run(&mut app, &mut runtime, ids::RECENT_FILES);
    let start = "Recent: 2 files, newest one.txt, today";
    assert_eq!(said, [start]);
    assert!(cells(start) <= 40);
    let rows = top_rows(&app);
    assert_eq!(rows[0].label, "one.txt");
    assert_eq!(rows[0].details[0], "opened today, 10:00 AM");
    assert_eq!(rows[1].label, "a.txt");
    assert_eq!(rows[1].details[0], "edited today, 10:00 AM");

    let said = run(&mut app, &mut runtime, ids::RECENT_CLEAR);
    assert_eq!(said, ["Cleared 2 recent files."]);
    assert!(app.dialogs.is_empty());
    close(&mut app, &mut runtime);
    let said = run(&mut app, &mut runtime, ids::RECENT_FILES);
    assert_eq!(said, ["Recent: no files."]);
}

#[test]
fn recent_files_off_keeps_nothing_and_says_so() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    app.settings.recent_files = false;
    focus(&mut app, "a.txt");
    app.note_recent(&ids::EDIT);
    assert!(
        !app.requests.iter().any(|r| matches!(r, Request::Lists(_))),
        "nothing kept while the setting is off"
    );
    let said = run(&mut app, &mut runtime, ids::RECENT_FILES);
    let line = "Recent files are off in Settings.";
    assert_eq!(said, [line]);
    assert_eq!(cells(line), 33);
}

#[test]
fn a_list_of_places_opens_in_the_lister_for_diro_dash() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let places = vec![
        fixture.root.join("a.txt").unwrap(),
        fixture.root.join("b").unwrap(),
        fixture.root.join("gone.txt").unwrap(),
    ];
    let start = "3 paths from standard input";
    assert_eq!(cells(start), 27);
    let effects = app.open_place_list("Standard input", start, places);
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, ["3 paths from standard input, 1 missing"]);
    assert_eq!(top_rows(&app).len(), 3);
    let effects = app.open_place_list("Standard input", start, Vec::new());
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, ["3 paths from standard input: nothing to list."]);
}

#[test]
fn save_as_collection_keeps_the_list_on_the_screen() {
    let fixture = fixture();
    let (mut app, mut runtime) = started(&fixture);
    let places = vec![
        fixture.root.join("a.txt").unwrap(),
        fixture.root.join("b").unwrap(),
    ];
    let effects = app.open_place_list("Standard input", "2 paths from standard input", places);
    settle(&mut app, &mut runtime, effects);
    let said = run(&mut app, &mut runtime, ids::SAVE_AS_COLLECTION);
    assert_eq!(said, ["Collection name for 2 items, empty"]);
    let said = typed(&mut app, &mut runtime, "From a shell");
    assert_eq!(said, ["Added 2 to From a shell."]);
    let said = run(&mut app, &mut runtime, ids::SAVE_AS_COLLECTION);
    close(&mut app, &mut runtime);
    assert_eq!(
        said,
        ["Collection name for 1 item, empty"],
        "the panel's focused entry"
    );
    assert_eq!(cells(NOTHING_TO_SAVE), 36);
}

#[test]
fn without_a_state_folder_the_lists_say_so() {
    let fixture = fixture();
    let mut options = AppOptions::new(fixture.root.clone(), fixture.root.clone());
    options.settings.dialog_settle_ms = 0;
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local();
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    let said = run(&mut app, &mut runtime, ids::COLLECTIONS);
    assert_eq!(said, ["Lists need a state folder, and this run has none."]);
}

#[test]
fn every_purpose_has_a_name() {
    let root = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let wants = [
        ListsWants::Collections {
            side: Side::Left,
            purpose: ListPurpose::Open,
        },
        ListsWants::SaveName { places: Vec::new() },
        ListsWants::RenameName { old: String::new() },
        ListsWants::RemoveCollection {
            side: Side::Left,
            name: String::new(),
            path: PathBuf::new(),
        },
        ListsWants::FlatForm {
            side: Side::Left,
            root: root.clone(),
            options: FlatOptions::new(),
        },
        ListsWants::FlatDepth {
            side: Side::Left,
            root,
            options: FlatOptions::new(),
        },
    ];
    let names: Vec<&str> = wants.iter().map(ListsWants::purpose).collect();
    assert_eq!(
        names,
        [
            "Collections",
            "CollectionName",
            "CollectionRename",
            "CollectionRemove",
            "FlatView",
            "FlatViewDepth"
        ]
    );
}
