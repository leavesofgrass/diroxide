//! The access rows of the details view, Change time and Change owner
//! (alpha 4, lane AP, R30 commits 74 to 79): every test works in its own
//! temporary folder under the build's folder, changes the times and the
//! owner of its own files only, never elevates, and never opens the
//! system's Security dialog (a recorder answers for it).

use std::fs;
use std::path::Path;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_core::format::within_cells;
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, place, temp};

fn work(root: &Path) -> std::path::PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(folder.join("inner")).unwrap();
    fs::write(folder.join("main.rs"), "fn main() {}\n").unwrap();
    fs::write(folder.join("notes.txt"), "notes\n").unwrap();
    fs::write(folder.join("inner").join("deep.txt"), "deep\n").unwrap();
    folder
}

fn session(work: &Path) -> Session {
    Session::open(
        place(work),
        place(&work.join("inner")),
        Registry::with_local(),
        Setup::default(),
    )
}

pub fn labels(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("no list: {other:?}"),
    }
}

fn help(s: &Session) -> String {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.help().to_owned(),
        other => panic!("no list: {other:?}"),
    }
}

/// Commits 74 to 77: the details view lists who may do what, "You can"
/// first among them, for a file and for a folder; the panel's own owner
/// row gives way to the access rows that say it in full.
#[test]
fn the_details_list_says_who_may_do_what() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("main.rs");
    s.key("alt+enter");
    let rows = labels(&s);
    let you_can = rows
        .iter()
        .find(|r| r.starts_with("You can: "))
        .unwrap_or_else(|| panic!("no You can row: {rows:?}"));
    assert!(within_cells(you_can, "You can"), "{you_can}");
    assert!(
        help(&s).contains("It is advice"),
        "the help says You can is advice: {}",
        help(&s)
    );
    if cfg!(unix) {
        assert!(
            rows.iter()
                .any(|r| r.starts_with("Owner can read and write (")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|r| r.starts_with("Mode: ")), "{rows:?}");
        assert!(
            !rows.iter().any(|r| r.starts_with("Owner: ")),
            "the owner is said once: {rows:?}"
        );
    } else {
        assert_eq!(
            rows.iter().filter(|r| r.starts_with("Owner: ")).count(),
            1,
            "the owner is said once: {rows:?}"
        );
    }
    s.key("escape");
    s.focus("inner");
    s.key("alt+enter");
    let rows = labels(&s);
    assert!(
        rows.iter()
            .any(|r| r.starts_with("You can: list") || r == "You have no access"),
        "a folder's rights in its own verbs: {rows:?}"
    );
    s.key("escape");
}

/// The modified time of a test's own file, to the second.
fn modified_secs(path: &Path) -> i64 {
    let time = fs::metadata(path).unwrap().modified().unwrap();
    diroxide_core::Timestamp::from_system_time(time).unix_seconds()
}

/// Clears the field and types `text`.
fn retype(s: &mut Session, text: &str) {
    let now = match s.dialog() {
        Some(Dialog::Edit { field, .. }) => field.text().chars().count(),
        other => panic!("no field: {other:?}"),
    };
    s.key("end");
    for _ in 0..now {
        s.key("backspace");
    }
    s.type_text(text);
}

/// Commit 78: Change time sets the modified time to a typed date, the
/// weekday computed; the plan asks first, starting on No; Undo puts the
/// old time back. Only the test's own file changes.
#[test]
fn change_time_sets_a_typed_date_after_asking_and_undo_puts_it_back() {
    let dir = temp();
    let work = work(dir.path());
    let file = work.join("notes.txt");
    let before = modified_secs(&file);
    let mut s = session(&work);
    s.focus("notes.txt");
    let said = s.command(ids::CHANGE_TIME);
    assert!(
        said[0].starts_with("Change time, Change the times of notes.txt, 1 of "),
        "{said:?}"
    );
    let rows = labels(&s);
    assert_eq!(rows[1], "Change: modified only");
    assert_eq!(rows[2], "Set to: now");
    assert_eq!(rows[3], "Type a date and time");
    assert!(rows[4].starts_with("Modified: "), "{rows:?}");
    for row in &rows {
        assert!(within_cells(row, row), "{row}");
    }
    // The change time cannot be set: its row says why.
    if cfg!(unix) {
        let at = rows
            .iter()
            .position(|r| r.starts_with("Changed: "))
            .unwrap();
        for _ in 0..at {
            s.key("down");
        }
        let said = s.key("enter");
        assert!(
            said.iter()
                .any(|l| l.starts_with("Could not set the change time")),
            "{said:?}"
        );
        s.key("home");
    }
    // Set to steps to the other panel's file and back.
    s.key("down");
    s.key("down");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Change time, Set to: the times of deep.txt")),
        "{said:?}"
    );
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Change time, Set to: now")),
        "{said:?}"
    );
    s.key("down");
    let said = s.key("enter");
    assert!(
        said[0].starts_with("Set to, as 2026-10-10 22:09:"),
        "{said:?}"
    );
    retype(&mut s, "yesterday");
    let said = s.key("enter");
    assert_eq!(said, ["Not a date: type it as 2026-10-10 22:09"]);
    retype(&mut s, "2026-10-10 22:09");
    let said = s.key("enter");
    // October 10, 2026 was a Saturday: the weekday is computed.
    assert!(
        said.iter()
            .any(|l| l.starts_with("Change time, Set to: Saturday, 2026-10-10 22:09")),
        "{said:?}"
    );
    s.key("home");
    let said = s.key("enter");
    let question = said.last().cloned().unwrap_or_default();
    assert_eq!(
        question,
        "Change the times of 1 item? No is the default. y or n"
    );
    let listing = match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        listing
            .iter()
            .any(|l| l == "Modified time to Saturday, 2026-10-10 22:09"),
        "{listing:?}"
    );
    // No changes nothing.
    s.key("n");
    assert_eq!(modified_secs(&file), before);
    s.command(ids::CHANGE_TIME);
    s.key("down");
    s.key("down");
    s.key("down");
    s.key("enter");
    retype(&mut s, "2026-10-10 22:09");
    s.key("enter");
    s.key("home");
    s.key("enter");
    s.key("y");
    let asked =
        diroxide_core::Timestamp::from_civil(2026, 10, 10, 22, 9, 0, diroxide_core::UtcOffset::UTC)
            .unwrap()
            .unix_seconds();
    assert_eq!(modified_secs(&file), asked);
    let said = s.key("ctrl+z");
    assert!(
        said.iter().any(|l| l.starts_with("Undid time change")),
        "{said:?}"
    );
    assert_eq!(modified_secs(&file), before);
}

/// Commit 79: Change owner. On Windows the focused item goes to the
/// system's Security dialog (a recorder here); on Linux and macOS the
/// app's own dialog plans the change, and a user who is not root hears
/// the refusal whole. Nothing ever elevates.
#[test]
fn change_owner_asks_or_refuses_whole_or_hands_off_on_windows() {
    let dir = temp();
    let work = work(dir.path());
    let recorder = std::sync::Arc::new(crate::support::Recorder::default());
    let mut s = Session::open(
        place(&work),
        place(&work.join("inner")),
        Registry::with_local(),
        Setup {
            recorder: recorder.clone(),
            ..Setup::default()
        },
    );
    s.focus("notes.txt");
    let said = s.command(ids::CHANGE_OWNER);
    if cfg!(windows) {
        assert_eq!(said, ["Opening the Security dialog for notes.txt"]);
        assert_eq!(
            recorder.security.lock().unwrap().as_slice(),
            [work.join("notes.txt")]
        );
        return;
    }
    assert!(
        said[0].starts_with("Change owner, Change owner of notes.txt, 1 of 4"),
        "{said:?}"
    );
    let rows = labels(&s);
    assert!(rows[1].starts_with("Now: owner "), "{rows:?}");
    assert!(rows[2].ends_with(", as now"), "{rows:?}");
    s.key("down");
    s.key("down");
    let said = s.key("enter");
    assert!(said[0].starts_with("New owner of notes.txt:"), "{said:?}");
    retype(&mut s, "root");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Change owner, Owner: root")),
        "{said:?}"
    );
    s.key("home");
    let said = s.key("enter");
    let root = diroxide_vfs::access::Caller::current().administrator;
    if root {
        assert_eq!(
            said.last().map(String::as_str),
            Some("Change owner of 1 item to root? No is the default. y or n")
        );
        s.key("n");
    } else {
        assert!(
            said.iter().any(|l| l
                == "Could not change the owner of notes.txt: only an administrator can. Start diro as administrator, then try again."),
            "{said:?}"
        );
    }
    assert!(recorder.security.lock().unwrap().is_empty());
}

/// Commit 79: the system's folders are refused before anything is
/// asked, in words, with what to use instead.
#[cfg(target_os = "linux")]
#[test]
fn change_owner_refuses_the_systems_folders_first() {
    let usr = std::path::Path::new("/usr");
    let mut s = Session::open(
        place(usr),
        place(usr),
        Registry::with_local(),
        Setup::default(),
    );
    s.focus("bin");
    let said = s.command(ids::CHANGE_OWNER);
    assert_eq!(
        said,
        [
            "Could not change the owner of /usr/bin: system folders are protected. Use the system's own tools."
        ]
    );
    assert!(s.dialog().is_none());
}
