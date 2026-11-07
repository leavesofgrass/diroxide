//! The access rows of the details view, Change time and Change owner
//! (alpha 4, lane AP, R30 commits 74 to 79): every test works in its own
//! temporary folder under the build's folder, changes the times and the
//! owner of its own files only, never elevates, and never opens the
//! system's Security dialog (a recorder answers for it).

use std::fs;
use std::path::Path;

use diroxide_app::Dialog;
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
