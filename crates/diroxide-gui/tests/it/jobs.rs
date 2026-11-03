//! Jobs (R27's commit 21): step six of the owner's walk (a copy's
//! question, its conflict, the result, the focus back on the list); the
//! progress indicator, which moves only with the app's paced jobs line,
//! so progress never speaks twice inside the interval; Escape cancels the
//! job in front; the Jobs list with a waiting job's Pause and its Held
//! row; and Q2's Interrupted group first after a quit, with Retry and
//! Forget.

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use diroxide_app::config::Paths;
use diroxide_app::{App, Runtime};
use diroxide_core::format::{BRAILLE_CELLS, first_cells, within_cells};
use diroxide_core::{FixedClock, Side, Timestamp, UtcOffset};
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, app_with, fixture};

/// The app over `left` and `right` with its own state folder under
/// `state`, so the job queue is saved across a quit.
fn app_with_state(left: &Path, right: &Path, state: &Path) -> (App, Runtime) {
    let (app, runtime) = app_with(left, right, |_| {});
    let paths = Paths::new(state.join("config"), state.join("state"));
    (app, runtime.with_paths(paths))
}

#[test]
fn step_six_of_the_walk_a_copy_its_conflict_and_the_focus_back() {
    let dir = fixture(&["notes.txt", "budget.xlsx"], &["Backup"]);
    std::fs::write(dir.path().join("report.pdf"), vec![b'r'; 2100]).unwrap();
    let backup = dir.path().join("Backup");
    std::fs::write(backup.join("report.pdf"), b"old").unwrap();
    let (app, runtime) = app_on(dir.path(), &backup);
    let mut rig = Rig::new(app, runtime);
    for name in ["report.pdf", "notes.txt", "budget.xlsx"] {
        assert!(rig.f.app_mut().focus_name(Side::Left, name));
        rig.refresh();
        rig.press("space");
    }
    rig.press("f5");
    let question = rig.dialog_name().unwrap();
    assert!(
        within_cells(&question, "Copy 3 items to Backup"),
        "{question}"
    );
    assert_eq!(rig.focus_key().1, "No");
    rig.press("y");
    // The conflict: named by its fact, the focus on the safe choice.
    assert!(rig.settle_until(|rig| rig.dialog_name().is_some()));
    let conflict = rig.dialog_name().unwrap();
    assert!(
        within_cells(&conflict, "Name taken: report.pdf"),
        "{conflict}"
    );
    let (role, choice) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(choice.starts_with("Skip"), "{choice}");
    rig.press("enter");
    assert!(
        rig.settle_until(|rig| { rig.spoken().iter().any(|s| s.starts_with("Copy finished")) })
    );
    assert!(rig.dialog_name().is_none());
    assert_eq!(std::fs::read(backup.join("report.pdf")).unwrap(), b"old");
    assert!(backup.join("notes.txt").exists() && backup.join("budget.xlsx").exists());
    // The focus is back on the panel's row.
    assert_eq!(rig.focus_node().0, Role::ListBoxOption);
    let finished = rig
        .spoken()
        .into_iter()
        .rfind(|s| s.starts_with("Copy finished"))
        .unwrap();
    eprintln!("heard: {}", first_cells(&question, BRAILLE_CELLS));
    eprintln!("heard: {}", first_cells(&conflict, BRAILLE_CELLS));
    eprintln!("heard: {}", first_cells(&finished, BRAILLE_CELLS));
    // Each was said once: the question and the conflict by their
    // dialogs, never again by the live region.
    assert!(!rig.spoken().iter().any(|s| s.starts_with("Copy 3 items")));
    assert!(!rig.spoken().iter().any(|s| s.starts_with("Name taken")));
}

#[test]
fn progress_never_speaks_twice_inside_the_interval() {
    // Many small files, so the engine reports progress many times; the
    // app's clock stands still, so no interval ever passes: the jobs line,
    // and the progress indicator with it, change only with the job's state.
    let dir = fixture(&[], &["many", "Backup"]);
    for n in 0..300 {
        std::fs::write(
            dir.path().join("many").join(format!("f{n:03}.txt")),
            b"crow",
        )
        .unwrap();
    }
    let backup = dir.path().join("Backup");
    let clock = Arc::new(FixedClock::new(
        Timestamp::from_unix(1_800_000_000, 0),
        UtcOffset::from_seconds(0),
    ));
    let (app, runtime) = app_with(dir.path(), &backup, move |options| {
        options.clock = clock;
        options.settings.progress_interval_seconds = 10;
    });
    let mut rig = Rig::new(app, runtime);
    assert!(
        rig.f.app().screen_reader(),
        "screen-reader mode, the slower pace"
    );
    assert!(rig.f.app_mut().focus_name(Side::Left, "many"));
    rig.refresh();
    rig.press("f5");
    rig.press("y");
    assert!(
        rig.settle_until(|rig| { rig.spoken().iter().any(|s| s.starts_with("Copy finished")) })
    );
    assert_eq!(std::fs::read_dir(backup.join("many")).unwrap().count(), 300);
    let reports = rig.f.progress_reports().to_vec();
    let shown = rig.f.progress_shown().to_vec();
    // The engine's reports end at 100 percent (the job's end), which the
    // app's line never shows as progress: the indicator skipped it, as it
    // skips every report the paced line does not take.
    assert!(reports.contains(&100), "{reports:?}");
    assert!(!shown.contains(&Some(100)), "{reports:?} {shown:?}");
    // A value for each change of state at most, and none when it ends.
    assert!(
        shown.len() <= 4,
        "reports {reports:?}, the indicator {shown:?}"
    );
    assert!(shown.len() < reports.len() + 1, "{reports:?} {shown:?}");
    assert_eq!(shown.last(), Some(&None), "{shown:?}");
    // Progress is shown, never said: no percent reached the live region.
    assert!(
        !rig.spoken().iter().any(|s| s.contains("percent")),
        "{:?}",
        rig.spoken()
    );
}

#[test]
fn pause_holds_a_waiting_job_and_escape_cancels_the_job_in_front() {
    let dir = fixture(&["notes.txt"], &["Backup"]);
    let backup = dir.path().join("Backup");
    let (app, runtime) = app_on(dir.path(), &backup);
    let mut rig = Rig::new(app, runtime);
    rig.run("jobs.pause-queue");
    assert!(rig.spoken().iter().any(|s| s.starts_with("Queue paused")));
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    rig.press("f5");
    rig.press("y");
    assert!(rig.settle_until(|rig| rig.spoken().iter().any(|s| s.starts_with("Queued"))));
    // The Jobs list: the waiting job's row, then its actions.
    rig.run("jobs.list");
    assert!(rig.dialog_name().unwrap().starts_with("Jobs"));
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("Waiting 1 of 1: copy"), "{row}");
    rig.press("enter");
    let (rows, _) = rig.dialog_rows();
    assert!(rows.len() >= 2, "{rows:?}");
    let actions: Vec<String> = rig
        .dialog_walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect();
    let pause = actions.iter().position(|a| a.starts_with("Pause")).unwrap();
    rig.press("home");
    for _ in 0..pause {
        rig.press("down");
    }
    rig.press("enter");
    assert!(
        rig.settle_until(|rig| rig.spoken().iter().any(|s| s.starts_with("Held: copy"))),
        "{:?}",
        rig.spoken()
    );
    // The row says it is held, its state first.
    while rig.dialog_name().is_some() {
        rig.press("escape");
    }
    rig.run("jobs.list");
    let (_, row) = rig.focus_node();
    assert!(row.starts_with("Held 1 of 1: copy"), "{row}");
    rig.press("escape");
    // Escape on the panel stops the job in front: the held job goes.
    let before = rig.spoken().len();
    rig.press("escape");
    assert!(
        rig.settle_until(|rig| rig.f.app().jobs().is_empty()),
        "{:?}",
        rig.spoken()
    );
    assert!(rig.spoken().len() > before, "Escape said what it stopped");
    assert!(!backup.join("notes.txt").exists());
}

#[test]
fn an_interrupted_job_is_listed_first_with_retry_and_forget() {
    let dir = fixture(&["notes.txt"], &["Backup"]);
    let backup = dir.path().join("Backup");
    let state = tempfile::Builder::new()
        .prefix("gui-it-state-")
        .tempdir()
        .unwrap();
    // The first window: a job waits in a paused queue; quit stops it.
    {
        let (app, runtime) = app_with_state(dir.path(), &backup, state.path());
        let mut rig = Rig::new(app, runtime);
        rig.run("jobs.pause-queue");
        assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
        rig.refresh();
        rig.press("f5");
        rig.press("y");
        assert!(rig.settle_until(|rig| rig.spoken().iter().any(|s| s.starts_with("Queued"))));
        rig.press("ctrl+q");
        let quit = rig.dialog_name().unwrap();
        assert!(quit.starts_with("Quit and stop them?"), "{quit}");
        assert_eq!(rig.focus_key().1, "No");
        rig.press("y");
        assert!(rig.f.wants_quit());
    }
    // The next window: one sentence, then the Jobs list's Interrupted
    // group first.
    let (app, runtime) = app_with_state(dir.path(), &backup, state.path());
    let mut rig = Rig::new(app, runtime);
    assert!(
        rig.settle_until(|rig| rig
            .spoken()
            .iter()
            .any(|s| s.contains("interrupted by the last quit"))),
        "{:?}",
        rig.spoken()
    );
    rig.run("jobs.list");
    let walk = rig.dialog_walk();
    let options: Vec<&String> = walk
        .iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect();
    assert!(options[0].starts_with("Interrupted"), "{options:?}");
    let (_, row) = rig.focus_node();
    assert!(row.starts_with("Interrupted: copy"), "{row}");
    // Its actions: Retry, then Forget.
    rig.press("enter");
    let actions: Vec<String> = rig
        .dialog_walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect();
    assert!(actions[0].starts_with("Retry"), "{actions:?}");
    assert!(
        actions.iter().any(|a| a.starts_with("Forget")),
        "{actions:?}"
    );
    rig.press("home");
    rig.press("enter");
    // Retry asks the usual question, No first; y copies.
    assert!(
        rig.settle_until(|rig| rig.dialog_buttons().len() == 2),
        "{:?}",
        rig.spoken()
    );
    assert_eq!(rig.focus_key().1, "No");
    rig.press("y");
    assert!(
        rig.settle_until(|_| backup.join("notes.txt").exists()),
        "{:?}",
        rig.spoken()
    );
}
