//! The Watched folders list (alpha 4, lane AW, R30 commit 70): one row a
//! task, a task's actions, the approval and catch-up questions starting
//! on No, a task's notes in words, Say Status's tally, and a task's jobs
//! kept out of Ctrl+Z. Every task's home is in the test's own folder; its
//! recipe is carta's, in process, so no program starts.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::watched::WatchedNote;
use diroxide_app::{AppEvent, Dialog};
use diroxide_config::Paths;
use diroxide_ops::hot::{FileNote, FileOutcome, HotNote};

use crate::support::{Session, Setup, place, temp};

/// A home for the task `notes` with two Markdown files waiting in `in`,
/// and its task file, which converts each to Org and moves the original
/// to `processed`. Returns the home and the folders.
fn task_home(dir: &Path) -> (PathBuf, Paths) {
    let home = dir.join("notes");
    for folder in ["in", "out", "processed", "failed"] {
        fs::create_dir_all(home.join(folder)).unwrap();
    }
    fs::write(home.join("in").join("a.md"), "# A\n\nOne.\n").unwrap();
    fs::write(home.join("in").join("b.md"), "# B\n\nTwo.\n").unwrap();
    let paths = Paths::new(dir.join("config"), dir.join("state"));
    let tasks = paths.config_dir.join(Paths::TASKS);
    fs::create_dir_all(&tasks).unwrap();
    let task = format!(
        "name = \"notes\"\nkind = \"convert\"\nrecipe = \"markdown-to-org\"\n\n[trigger]\nkind = \"folder\"\nhome = '{}'\nmatch = [\"*.md\"]\nsettle = \"2s\"\n",
        home.display()
    );
    fs::write(tasks.join("notes.toml"), task).unwrap();
    (home, paths)
}

fn session(dir: &Path, paths: &Paths) -> Session {
    let work = dir.join("work");
    fs::create_dir_all(&work).unwrap();
    Session::open(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        Setup {
            paths: Some(paths.clone()),
            ..Setup::default()
        },
    )
}

/// The open list's labels.
fn labels(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("{other:?}"),
    }
}

/// Moves the open list's focus to a row by its label.
fn choose(s: &mut Session, label: &str) {
    for _ in 0..30 {
        let focused = match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.label.clone()),
            _ => None,
        };
        if focused.as_deref() == Some(label) {
            return;
        }
        s.key("down");
    }
    panic!("{label} is not in the list");
}

/// Opens the list, then the task's actions, then does one.
fn action(s: &mut Session, label: &str) -> Vec<String> {
    s.command(ids::WATCHED_FOLDERS);
    s.key("enter");
    choose(s, label);
    s.key("enter")
}

#[test]
fn no_tasks_says_where_they_go() {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let mut s = session(dir.path(), &paths);
    let said = s.command(ids::WATCHED_FOLDERS);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("No watched folders. Each task is a file in "),
        "{said:?}"
    );
}

#[test]
fn the_list_says_each_task_and_its_actions_preview_and_say_its_rule() {
    let dir = temp();
    let (home, paths) = task_home(dir.path());
    let mut s = session(dir.path(), &paths);
    s.command(ids::WATCHED_FOLDERS);
    assert_eq!(s.dialog_title().as_deref(), Some("Watched folders: 1 task"));
    assert_eq!(labels(&s), ["Notes, off"]);
    s.key("enter");
    assert_eq!(s.dialog_title().as_deref(), Some("Notes actions"));
    assert_eq!(
        labels(&s),
        [
            "Turn on",
            "Preview",
            "Show log",
            "Put failed back",
            "Say rule",
            "Edit"
        ]
    );
    choose(&mut s, "Say rule");
    let said = s.key("enter");
    assert!(
        said.iter().any(|l| l.starts_with("Notes. Watches ")),
        "{said:?}"
    );
    action(&mut s, "Preview");
    match s.dialog() {
        Some(Dialog::Read { title, lines }) => {
            assert_eq!(title, "Notes: Dry run: 2 would convert, 0 skipped");
            assert_eq!(lines, &["Would convert a.md", "Would convert b.md"]);
        }
        other => panic!("{other:?}"),
    }
    s.key("escape");
    assert!(
        home.join("in").join("a.md").exists(),
        "a dry run touches nothing"
    );
    let said = action(&mut s, "Show log");
    assert_eq!(said.last().map(String::as_str), Some("Notes log is empty"));
    let said = action(&mut s, "Put failed back");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Nothing failed in Notes")
    );
}

#[test]
fn turning_on_asks_approval_then_catch_up_both_on_no_and_off_stops_it() {
    let dir = temp();
    let (home, paths) = task_home(dir.path());
    let mut s = session(dir.path(), &paths);
    let said = action(&mut s, "Turn on");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Turn on Notes? No is the default. y or n")
    );
    match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => {
            assert!(listing[0].starts_with("Notes. Watches "), "{listing:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.key("n"), ["Nothing changed."]);
    action(&mut s, "Turn on");
    let said = s.key("y");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Process the 2 files in Notes? No is the default. y or n")
    );
    // No: the waiting files stay, and the task watches for new ones.
    let said = s.key("n");
    assert!(
        said.iter().any(|l| l == "Hot folder Notes is on"),
        "{said:?}"
    );
    s.command(ids::WATCHED_FOLDERS);
    assert_eq!(labels(&s), ["Notes, on"]);
    s.key("escape");
    let said = action(&mut s, "Turn off");
    assert!(
        said.iter().any(|l| l == "Hot folder Notes is off"),
        "{said:?}"
    );
    assert!(home.join("in").join("a.md").exists());
}

#[test]
fn a_task_s_notes_are_said_and_say_status_has_its_tally() {
    let dir = temp();
    let (_home, paths) = task_home(dir.path());
    let mut s = session(dir.path(), &paths);
    let event = |note: HotNote| {
        AppEvent::HotFolder(Box::new(WatchedNote::Hot {
            name: "notes".to_owned(),
            note,
        }))
    };
    let effects = s.app.handle(event(HotNote::Started {
        waiting: 0,
        polled: None,
    }));
    assert_eq!(s.run(effects), ["Hot folder Notes is on"]);
    for outcome in [FileOutcome::Done, FileOutcome::Done, FileOutcome::Failed] {
        let effects = s.app.handle(event(HotNote::File(FileNote {
            name: "a.md".to_owned(),
            outcome,
            line: String::new(),
            output: None,
        })));
        assert!(s.run(effects).is_empty(), "a file's own line is not said");
    }
    let effects = s
        .app
        .handle(event(HotNote::Say("Notes: 2 done, 1 failed".to_owned())));
    assert_eq!(s.run(effects), ["Notes: 2 done, 1 failed"]);
    let effects = s.app.handle(event(HotNote::Paused(
        "Notes paused: 10 failures in a row".to_owned(),
    )));
    assert_eq!(s.run(effects), ["Notes paused: 10 failures in a row"]);
    let said = s.command(ids::SAY_STATUS);
    assert!(
        said.iter().any(|l| l.contains(". Notes: 2 done, 1 failed")),
        "{said:?}"
    );
}

/// The real thread: approved and caught up, the task converts both files
/// with carta in process, moves the originals and says its summary; its
/// jobs never reach Ctrl+Z. Timing: it waits for the task's thread, at
/// most 30 seconds, moving the test clock past the settle time.
#[test]
fn a_running_task_converts_what_waits_and_ctrl_z_leaves_it() {
    let dir = temp();
    let (home, paths) = task_home(dir.path());
    let mut s = session(dir.path(), &paths);
    action(&mut s, "Turn on");
    s.key("y");
    s.key("y");
    let made = home.join("out").join("b.org");
    let started = Instant::now();
    while !(made.exists() && home.join("processed").join("b.md").exists())
        && started.elapsed() < Duration::from_secs(30)
    {
        s.clock.advance(3);
        std::thread::sleep(Duration::from_millis(100));
        s.run(Vec::new());
    }
    assert!(made.exists(), "the task did not convert b.md in time");
    assert!(home.join("out").join("a.org").exists());
    let said = s.key("ctrl+z");
    assert!(!said.iter().any(|l| l.starts_with("Undid")), "{said:?}");
    assert!(made.exists());
    action(&mut s, "Turn off");
}
