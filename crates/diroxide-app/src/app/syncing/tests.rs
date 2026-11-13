//! Compare folders and synchronize in the app, over the fake provider
//! serving the local places, so nothing a test does reaches the owner's
//! files or the real trash: the criteria, the compare as a job, the
//! result's count line and rows, a Different row's file compare, the
//! modes, the plan rows in the short form, a row's changes, the question
//! that starts on No, the run as one job, and the options check. Every
//! quoted line is checked against the 40 cells.

use std::sync::Arc;

use diroxide_core::format::within_cells;
use diroxide_core::{Command, CommandId, Effect, FixedClock, Place, Timestamp, UtcOffset};
use diroxide_ops::FakeProvider;
use diroxide_vfs::Registry;

use super::*;
use crate::app::{AppEvent, AppOptions};
use crate::runtime::Runtime;

fn local(path: &str) -> Place {
    if cfg!(windows) {
        Place::parse(&format!("D:{}", path.replace('/', "\\"))).unwrap()
    } else {
        Place::parse(path).unwrap()
    }
}

fn time(seconds: i64) -> Timestamp {
    Timestamp::from_unix(1_700_000_000 + seconds, 0)
}

fn file(fake: &FakeProvider, path: &str, data: &[u8], seconds: i64) {
    fake.add_file(&local(path), data);
    fake.set_modified(&local(path), time(seconds));
}

/// Here, photos: plan.md (only here), notes.txt (newer here), same.txt.
/// There, backup: notes.txt, same.txt, old.txt (only there).
fn folders() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/photos"));
    fake.add_folder(&local("/backup"));
    file(&fake, "/photos/plan.md", b"plan", 0);
    file(&fake, "/photos/notes.txt", b"new notes", 100);
    file(&fake, "/backup/notes.txt", b"old notes", 0);
    file(&fake, "/photos/same.txt", b"same", 0);
    file(&fake, "/backup/same.txt", b"same", 0);
    file(&fake, "/backup/old.txt", b"old", 0);
    fake
}

fn started(fake: &FakeProvider) -> (App, Runtime) {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    let mut options = AppOptions::new(local("/photos"), local("/backup"));
    options.clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 11, 12, 16, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    options.settings.dialog_settle_ms = 0;
    let mut app = App::new(options);
    let mut runtime = Runtime::new(Arc::new(registry)).with_trash_bin(Arc::new(fake.trash_list()));
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    (app, runtime)
}

fn said(effects: Vec<Effect>) -> Vec<String> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Announce(message) => Some(message.text),
            _ => None,
        })
        .collect()
}

fn run(app: &mut App, runtime: &mut Runtime, id: CommandId) -> Vec<String> {
    let effects = app.dispatch(&Command::new(id));
    said(runtime.run_until_idle(app, effects))
}

/// Focuses the top list's row with this id and presses Enter.
fn enter(app: &mut App, runtime: &mut Runtime, row: &str) -> Vec<String> {
    match app.dialogs.last_mut() {
        Some((_, Dialog::Choose { list, .. })) => {
            assert!(list.focus_id(row), "no row {row} in {:?}", labels(app));
        }
        _ => panic!("no list on top"),
    }
    run(app, runtime, ids::ACCEPT)
}

fn labels(app: &App) -> Vec<String> {
    match app.dialogs.last() {
        Some((_, Dialog::Choose { list, .. })) => {
            list.rows().iter().map(|row| row.label.clone()).collect()
        }
        _ => Vec::new(),
    }
}

fn top(app: &App) -> Option<&'static str> {
    app.top_sync().map(|wants| wants.purpose())
}

/// The id of the top list's row whose label starts with `start`.
fn row_starting(app: &App, start: &str) -> String {
    match app.dialogs.last() {
        Some((_, Dialog::Choose { list, .. })) => list
            .rows()
            .iter()
            .find(|row| row.label.starts_with(start))
            .map(|row| row.id.clone())
            .unwrap_or_else(|| panic!("no row starting {start} in {:?}", labels(app))),
        _ => panic!("no list on top"),
    }
}

/// The line has its meaning in its first 40 cells and `cells` in all.
fn cells(line: &str, meaning: &str, cells: usize) {
    assert!(within_cells(line, meaning), "{line}");
    assert_eq!(line.chars().count(), cells, "{line}");
}

/// Compares the panels by size and time; the result list is on top.
fn compared(app: &mut App, runtime: &mut Runtime) -> Vec<String> {
    let opened = run(app, runtime, ids::COMPARE_FOLDERS);
    assert_eq!(
        opened,
        ["Compare folders: By size and time, within 2 seconds, chosen, 1 of 6"]
    );
    assert_eq!(top(app), Some("SyncCriteria"));
    enter(app, runtime, BY_TIME)
}

#[test]
fn compare_folders_says_the_count_line_and_lists_the_differences_status_first() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    let said = compared(&mut app, &mut runtime);
    assert_eq!(said[0], "Comparing photos here with backup there.");
    let count = "3 differences in 4 items: 1 different, 1 only here, 1 only there. 1 same.";
    assert!(said.iter().any(|line| line == count), "{said:?}");
    cells(count, "3 differences in 4 items", 73);
    assert_eq!(top(&app), Some("SyncResult"));
    let rows = labels(&app);
    // The differences in the walk's order, then the actions, then how.
    assert!(rows.contains(&"Only here, plan.md".to_owned()), "{rows:?}");
    assert!(rows.contains(&"Only there, old.txt".to_owned()), "{rows:?}");
    let different = "Different, notes.txt, newer here";
    cells(different, "Different, notes.txt", 32);
    assert!(rows.contains(&different.to_owned()), "{rows:?}");
    assert!(!rows.iter().any(|row| row.starts_with("Same,")), "{rows:?}");
    for action in [
        "Synchronize: plan the changes",
        "Show 1 same row",
        "Showing: all differences",
        "Open the differences as a list",
        "Compare again",
        "Compared by size and time; times within 2 seconds count as the same.",
    ] {
        assert!(rows.contains(&action.to_owned()), "{action}: {rows:?}");
    }
    cells("Synchronize: plan the changes", "Synchronize", 29);
    cells("Open the differences as a list", "Open the differences", 30);
    // The compare was a job, and it is over.
    assert!(app.jobs.ids().is_empty());

    // Show the same rows, then one status at a time.
    let said = enter(&mut app, &mut runtime, SAME);
    assert_eq!(said, ["Showing all 4 rows"]);
    assert!(labels(&app).contains(&"Same, same.txt".to_owned()));
    let said = enter(&mut app, &mut runtime, ONLY);
    assert_eq!(said, ["Showing only only here: 1 row"]);
    let said = enter(&mut app, &mut runtime, ONLY);
    assert_eq!(said, ["Showing only only there: 1 row"]);
    let said = enter(&mut app, &mut runtime, ONLY);
    assert_eq!(said, ["Showing only different: 1 row"]);
    let said = enter(&mut app, &mut runtime, ONLY);
    assert_eq!(said, ["Showing all differences: 3 rows"]);

    // Escape closes the result; nothing was written.
    let effects = app.dialog_escape();
    runtime.run_until_idle(&mut app, effects);
    assert!(app.dialogs.is_empty());
    assert!(!fake.exists(&local("/backup/plan.md")));
}

#[test]
fn the_criteria_turn_settings_and_escape_compares_nothing() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    run(&mut app, &mut runtime, ids::COMPARE_FOLDERS);
    // Hidden items start as the panel shows them (lane AS): hidden
    // files are not shown here, so the row starts off.
    let hidden_row = |app: &App| {
        labels(app)
            .into_iter()
            .find(|l| l.starts_with("Hidden items"))
    };
    assert_eq!(hidden_row(&app).as_deref(), Some("Hidden items: off"));
    let said = enter(&mut app, &mut runtime, HIDDEN);
    assert_eq!(said, ["Hidden items: on"]);
    let said = enter(&mut app, &mut runtime, HIDDEN);
    assert_eq!(said, ["Hidden items: off"]);
    let said = enter(&mut app, &mut runtime, HOUR);
    assert_eq!(said, ["Ignore a 1 hour shift: on"]);
    cells("Ignore a 1 hour shift: on", "Ignore a 1 hour shift", 25);
    match app.top_sync() {
        Some(SyncWants::Criteria(setup)) => {
            assert!(!setup.options.hidden && setup.options.ignore_one_hour);
        }
        other => panic!("{other:?}"),
    }
    let effects = app.dialog_escape();
    assert_eq!(said_of(&mut app, &mut runtime, effects), [NOTHING_COMPARED]);
    assert!(app.jobs.ids().is_empty());
    cells(NOTHING_COMPARED, NOTHING_COMPARED, 21);

    // With hidden files shown in the panel, the row starts on.
    run(&mut app, &mut runtime, ids::TOGGLE_HIDDEN);
    run(&mut app, &mut runtime, ids::COMPARE_FOLDERS);
    assert_eq!(hidden_row(&app).as_deref(), Some("Hidden items: on"));
    let effects = app.dialog_escape();
    said_of(&mut app, &mut runtime, effects);
}

fn said_of(app: &mut App, runtime: &mut Runtime, effects: Vec<Effect>) -> Vec<String> {
    said(runtime.run_until_idle(app, effects))
}

#[test]
fn the_same_folder_in_both_panels_is_refused_in_words() {
    let fake = folders();
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    let mut options = AppOptions::new(local("/photos"), local("/photos"));
    options.settings.dialog_settle_ms = 0;
    let mut app = App::new(options);
    let mut runtime = Runtime::new(Arc::new(registry));
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    let said = run(&mut app, &mut runtime, ids::COMPARE_FOLDERS);
    assert_eq!(said, ["Cannot compare: both panels show photos"]);
    assert!(app.dialogs.is_empty());
}

#[test]
fn enter_on_a_different_row_compares_the_two_files() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    compared(&mut app, &mut runtime);
    let row = row_starting(&app, "Different, notes.txt");
    let said = enter(&mut app, &mut runtime, &row);
    assert_eq!(said[0], "Comparing notes.txt here with notes.txt there");
    assert!(
        matches!(
            app.opens.last().map(|o| &o.purpose),
            Some(Purpose::Compare(_))
        ),
        "the file compare is not open: {said:?}"
    );
    // Escape goes back to the result.
    let effects = app.dialog_escape();
    runtime.run_until_idle(&mut app, effects);
    assert_eq!(top(&app), Some("SyncResult"));
    // A row that is not a Different file says itself.
    let row = row_starting(&app, "Only here");
    assert_eq!(enter(&mut app, &mut runtime, &row), ["Only here, plan.md"]);
}

#[test]
fn the_plan_rows_start_with_their_action_and_a_row_can_change() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    compared(&mut app, &mut runtime);
    let said = enter(&mut app, &mut runtime, SYNC);
    assert_eq!(said, ["Synchronize how: Update the other panel, 1 of 6"]);
    assert_eq!(top(&app), Some("SyncModes"));
    let modes = labels(&app);
    assert_eq!(
        modes,
        [
            "Update the other panel",
            "Mirror to the other panel",
            "Both ways, newer wins",
            "Update this panel",
            "Mirror to this panel",
            "Compare only",
        ]
    );
    cells("Mirror to the other panel", "Mirror", 25);

    let said = enter(&mut app, &mut runtime, &mode_id(SyncMode::UpdateThere));
    let summary = "Update the other panel: 2 to copy there, 1 skipped, 1 same; 13 bytes to copy";
    assert_eq!(said, [summary]);
    assert!(within_cells(summary, "Update the other panel: 2 to copy"));
    assert_eq!(top(&app), Some("SyncPlan"));
    // The plan's title: the mode and the count first (lane AS).
    let title = app
        .dialogs
        .last()
        .map(|(_, d)| d.title())
        .unwrap_or_default();
    cells(&title, "Update the other panel: 2 changes", 33);
    let rows = labels(&app);
    let copy = "Copy there: plan.md, only here";
    cells(copy, copy, 30);
    let replace = "Replace there: notes.txt, newer here";
    cells(replace, replace, 36);
    assert!(rows.contains(&copy.to_owned()), "{rows:?}");
    assert!(rows.contains(&replace.to_owned()), "{rows:?}");
    assert!(rows.contains(&"Skip: old.txt, only there, kept".to_owned()));
    assert!(rows.contains(&"Synchronize now: 2 items".to_owned()));
    assert!(rows.contains(&"Mode: Update the other panel".to_owned()));

    // Enter on a row lists what can change; Leave out says the new line.
    let row = row_starting(&app, "Copy there: plan.md");
    let said = enter(&mut app, &mut runtime, &row);
    assert_eq!(said, ["plan.md: Leave out, 1 of 1"]);
    assert_eq!(top(&app), Some("SyncRowActions"));
    let said = enter(&mut app, &mut runtime, TOGGLE);
    assert_eq!(said, ["Left out: plan.md, would copy there"]);
    assert_eq!(top(&app), Some("SyncPlan"));
    assert!(labels(&app).contains(&"Synchronize now: 1 item".to_owned()));
    // Take it back, then put back the suggestion.
    let said = enter(&mut app, &mut runtime, &row);
    assert_eq!(said, ["plan.md: Take back in, 1 of 2"]);
    let said = enter(&mut app, &mut runtime, TOGGLE);
    assert_eq!(said, [copy]);
    // Space does the same from the plan itself, the new line said once
    // (lane AS).
    let space = |app: &mut App, runtime: &mut Runtime| {
        let effects = app.handle(AppEvent::Key(Key::parse("space").unwrap()));
        self::said(runtime.run_until_idle(app, effects))
    };
    let said = space(&mut app, &mut runtime);
    assert_eq!(said, ["Left out: plan.md, would copy there"]);
    assert_eq!(top(&app), Some("SyncPlan"));
    let said = space(&mut app, &mut runtime);
    assert_eq!(said, [copy]);

    // The skipped row there can be copied toward this panel.
    let row = row_starting(&app, "Skip: old.txt");
    enter(&mut app, &mut runtime, &row);
    let said = enter(&mut app, &mut runtime, TOWARD_HERE);
    assert_eq!(said, ["Copy here: old.txt, only there"]);
    // The command does the same on the focused row of the plan.
    let row = row_starting(&app, "Copy here: old.txt");
    match app.dialogs.last_mut() {
        Some((_, Dialog::Choose { list, .. })) => assert!(list.focus_id(&row)),
        _ => panic!("no plan"),
    }
    let said = run(&mut app, &mut runtime, ids::SYNC_TOWARD_THERE);
    assert_eq!(said, ["Cannot copy old.txt there: it is not here"]);

    // Escape on the plan goes back to the result.
    let effects = app.dialog_escape();
    runtime.run_until_idle(&mut app, effects);
    assert_eq!(top(&app), Some("SyncResult"));
}

#[test]
fn a_mirror_lists_the_trash_first_asks_on_no_and_runs_as_one_job() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    compared(&mut app, &mut runtime);
    enter(&mut app, &mut runtime, SYNC);
    enter(&mut app, &mut runtime, &mode_id(SyncMode::MirrorThere));
    let rows = labels(&app);
    let trash = "Trash there: old.txt, only there";
    cells(trash, trash, 32);
    assert_eq!(rows[0], trash, "removals first: {rows:?}");

    let said = enter(&mut app, &mut runtime, RUN);
    let question = "Synchronize 3 items? No is the default. y or n";
    assert_eq!(said, [question]);
    assert!(within_cells(
        question,
        "Synchronize 3 items? No is the default"
    ));
    assert_eq!(top(&app), Some("SyncConfirm"));
    match app.dialogs.last() {
        Some((_, Dialog::Confirm { listing, .. })) => {
            assert_eq!(listing[0], trash, "{listing:?}");
            assert_eq!(listing.len(), 3, "{listing:?}");
        }
        _ => panic!("no question"),
    }
    // No: nothing changes, the plan stays.
    let said = run(&mut app, &mut runtime, ids::NO);
    assert_eq!(said, [NOTHING_DONE]);
    cells(NOTHING_DONE, NOTHING_DONE, 25);
    assert_eq!(top(&app), Some("SyncPlan"));
    assert!(fake.exists(&local("/backup/old.txt")));

    // Yes: one job; the views close.
    enter(&mut app, &mut runtime, RUN);
    let said = run(&mut app, &mut runtime, ids::YES);
    assert_eq!(said[0], "Synchronize started: 3 items");
    assert!(app.dialogs.is_empty(), "{:?}", top(&app));
    assert_eq!(fake.contents(&local("/backup/plan.md")).unwrap(), b"plan");
    assert_eq!(
        fake.contents(&local("/backup/notes.txt")).unwrap(),
        b"new notes"
    );
    assert!(!fake.exists(&local("/backup/old.txt")));
    assert!(fake.in_trash(&local("/backup/old.txt")));
    assert!(app.jobs.ids().is_empty(), "the job is still running");
    let record = app.undo.last().expect("no undo record");
    assert_eq!(record.kind, Some(JobKind::Synchronize), "{said:?}");
    assert_eq!(app.undo.len(), 1, "one undo record for the whole run");
}

#[test]
fn a_changed_option_blocks_the_run_until_compare_again() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    compared(&mut app, &mut runtime);
    enter(&mut app, &mut runtime, SYNC);
    enter(&mut app, &mut runtime, &mode_id(SyncMode::UpdateThere));
    app.settings.compare.folder_tolerance_seconds = 3;
    let said = enter(&mut app, &mut runtime, RUN);
    assert_eq!(said, [OPTIONS_CHANGED]);
    cells(OPTIONS_CHANGED, OPTIONS_CHANGED, 34);
    assert_eq!(top(&app), Some("SyncPlan"));
    // Compare again closes the views and compares with the new options.
    let said = enter(&mut app, &mut runtime, AGAIN);
    assert_eq!(said[0], "Comparing photos here with backup there.");
    assert_eq!(top(&app), Some("SyncResult"));
    let rows = labels(&app);
    assert!(
        rows.contains(&"By size and time, within 3 seconds".to_owned())
            || rows.iter().any(|row| row.contains("within 3 seconds")),
        "{rows:?}"
    );
}

#[test]
fn synchronize_from_the_panel_compares_then_offers_the_modes() {
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    let said = run(&mut app, &mut runtime, ids::SYNCHRONIZE);
    assert_eq!(said[0], "Comparing photos here with backup there.");
    assert_eq!(top(&app), Some("SyncModes"));
    // Compare only goes back to the result.
    enter(&mut app, &mut runtime, NO_MODE);
    assert_eq!(top(&app), Some("SyncResult"));
    // The commands of a plan say where they work.
    let effects = app.dialog_escape();
    runtime.run_until_idle(&mut app, effects);
    let said = run(&mut app, &mut runtime, ids::SYNC_TOWARD_HERE);
    assert_eq!(said, ["Open a synchronize plan first."]);
    let said = run(&mut app, &mut runtime, ids::COMPARE_AGAIN);
    assert_eq!(said, ["Compare folders first."]);
}

#[test]
fn the_same_folders_have_nothing_to_synchronize() {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/photos"));
    fake.add_folder(&local("/backup"));
    file(&fake, "/photos/a.txt", b"a", 0);
    file(&fake, "/backup/a.txt", b"a", 0);
    let (mut app, mut runtime) = started(&fake);
    let said = compared(&mut app, &mut runtime);
    assert!(
        said.iter().any(|line| line == "No differences in 1 item."),
        "{said:?}"
    );
    assert!(
        !labels(&app)
            .iter()
            .any(|row| row.starts_with("Synchronize"))
    );
    let said = run(&mut app, &mut runtime, ids::SYNCHRONIZE);
    assert_eq!(said, [NOTHING_TO_DO]);
    cells(NOTHING_TO_DO, NOTHING_TO_DO, 23);
}

#[test]
fn the_differences_open_as_a_list_in_the_results_lister() {
    use crate::app::searching::ListKind;
    let fake = folders();
    let (mut app, mut runtime) = started(&fake);
    compared(&mut app, &mut runtime);
    let said = enter(&mut app, &mut runtime, AS_LIST);
    let start = "Differences as a list, 3 files";
    cells(start, start, 30);
    assert_eq!(said[0], start);
    assert_eq!(app.list_kind(), Some(&ListKind::Compared));
}
