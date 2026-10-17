//! Bookmarks through the app (plan commit 24): the hotlist rule on the
//! first row, adding and removing with the question that starts on No,
//! the settings list, the alias jump, item bookmarks, bookmarks first in
//! the Go to place list with their kind first, the store through an
//! injected state folder, and the `bookmarked` fact.

use std::path::PathBuf;

use diroxide_app::Dialog;
use diroxide_app::bookmarks::FILE;
use diroxide_app::commands::ids;
use diroxide_config::Paths;
use diroxide_core::filter::Lookup;
use diroxide_core::{ArgValue, Command, Entry, EntryKind, Side};
use diroxide_vfs::Registry;

use crate::support::{Session, fixture, place, settings, temp};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    work: PathBuf,
    paths: Paths,
    s: Session,
}

fn open() -> Fixture {
    let dir = temp();
    let work = fixture(dir.path());
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let s = Session::start(
        place(&work),
        place(&work.join("Backup")),
        Registry::with_local(),
        Some(&paths),
        settings(),
    );
    Fixture {
        root: dir.path().to_owned(),
        _dir: dir,
        work,
        paths,
        s,
    }
}

fn last(said: &[String]) -> &str {
    said.last().map(String::as_str).unwrap_or_default()
}

/// The leaf of the folder that holds the fixture's `work`.
fn root_leaf(f: &Fixture) -> String {
    f.root.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn the_first_row_adds_this_folder_then_removes_it() {
    let mut f = open();
    let said = f.s.key("ctrl+b");
    assert!(
        said[0].starts_with("Bookmarks, no bookmarks, Add this folder, work, in"),
        "{said:?}"
    );
    let said = f.s.key("enter");
    assert_eq!(f.s.dialog(), None);
    assert_eq!(
        last(&said),
        format!("Bookmarked work, in {}", root_leaf(&f))
    );
    assert!(f.paths.state_file(FILE).exists());
    // Alt+B opens the same list; now the first row removes this folder.
    f.s.key("alt+b");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(line.starts_with("Remove this folder, work, in"), "{line}");
    f.s.key("down");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(
        line.starts_with(&format!(
            "work, bookmark, in {}, not used yet",
            root_leaf(&f)
        )),
        "{line}"
    );
    f.s.key("home");
    let said = f.s.key("enter");
    assert_eq!(
        last(&said),
        "Remove bookmark work? No is the default. y or n"
    );
    assert_eq!(last(&f.s.key("n")), "Nothing removed.");
    // The question closed; the list is still there, the bookmark kept.
    assert_eq!(f.s.dialog_title().as_deref(), Some("Bookmarks, 1 bookmark"));
    f.s.key("enter");
    let said = f.s.key("y");
    assert_eq!(last(&said), "Removed bookmark work");
    assert_eq!(
        f.s.dialog_title().as_deref(),
        Some("Bookmarks, no bookmarks")
    );
    let line = f.s.dialog().unwrap().focus_line();
    assert!(line.starts_with("Add this folder"), "{line}");
    f.s.key("escape");
    assert!(f.s.app.bookmarks().is_empty());
}

#[test]
fn an_alias_set_in_the_settings_list_jumps() {
    let mut f = open();
    f.s.command(ids::BOOKMARK_ADD);
    assert_eq!(
        last(&f.s.command(ids::BOOKMARK_ADD)),
        format!("Already bookmarked: work, in {}", root_leaf(&f))
    );
    f.s.key("ctrl+b");
    f.s.key("down");
    // The rename key opens the bookmark's settings.
    let said = f.s.key("f2");
    assert!(said[0].starts_with("Bookmark work, Name: work"), "{said:?}");
    f.s.key("down");
    f.s.key("down");
    assert!(
        f.s.dialog()
            .unwrap()
            .focus_line()
            .starts_with("Alias: none")
    );
    f.s.key("enter");
    assert_eq!(
        f.s.dialog_title().as_deref(),
        Some("Alias, one letter or digit")
    );
    f.s.type_text("ww");
    assert_eq!(last(&f.s.key("enter")), "An alias is one letter or digit.");
    f.s.key("backspace");
    let said = f.s.key("enter");
    assert_eq!(last(&said), "Alias: w");
    assert!(f.s.dialog().unwrap().focus_line().starts_with("Alias: w"));
    f.s.key("down");
    assert_eq!(last(&f.s.key("enter")), "Sticky: on, kept at the top");
    f.s.key("escape");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Bookmarks, 1 bookmark"));
    f.s.key("down");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(line.starts_with("work, alias w, in"), "{line}");
    assert!(line.contains("sticky"), "{line}");
    f.s.key("escape");
    // Elsewhere, the alias letter in the list goes there at once.
    f.s.key("backspace");
    assert_ne!(f.s.app.active_panel().place(), &place(&f.work));
    f.s.key("ctrl+b");
    f.s.key("w");
    assert_eq!(f.s.dialog(), None);
    assert_eq!(f.s.app.active_panel().place(), &place(&f.work));
    assert_eq!(f.s.app.bookmarks().all()[0].uses, 1);
    // A command with the alias jumps too; an unknown one says so.
    f.s.key("backspace");
    f.s.run_command(Command::new(ids::BOOKMARKS).with_arg("alias", ArgValue::Text("w".into())));
    assert_eq!(f.s.app.active_panel().place(), &place(&f.work));
    let said =
        f.s.run_command(Command::new(ids::BOOKMARKS).with_arg("alias", ArgValue::Text("q".into())));
    assert_eq!(last(&said), "No bookmark has the alias q.");
}

#[test]
fn an_item_bookmark_opens_its_folder_on_the_item() {
    let mut f = open();
    f.s.focus("report.pdf");
    let said = f.s.command(ids::BOOKMARK_ITEM);
    assert_eq!(last(&said), "Bookmarked report.pdf, in work");
    f.s.key("backspace");
    f.s.key("ctrl+b");
    f.s.key("down");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(line.contains("opens its folder on the item"), "{line}");
    f.s.key("enter");
    assert_eq!(f.s.app.active_panel().place(), &place(&f.work));
    assert_eq!(f.s.focused(), "report.pdf");
}

#[test]
fn the_trash_key_removes_after_a_question_and_the_store_round_trips() {
    let mut f = open();
    f.s.command(ids::BOOKMARK_ADD);
    f.s.focus("Photos");
    f.s.command(ids::BOOKMARK_ITEM);
    let again = Session::start(
        place(&f.work),
        place(&f.work),
        Registry::with_local(),
        Some(&f.paths),
        settings(),
    );
    assert_eq!(again.app.bookmarks(), f.s.app.bookmarks());
    assert_eq!(again.app.bookmarks().len(), 2);
    f.s.key("ctrl+b");
    f.s.key("end");
    let said = f.s.key("delete");
    assert!(
        last(&said).starts_with("Remove bookmark ") && last(&said).ends_with("y or n"),
        "{said:?}"
    );
    f.s.key("y");
    assert_eq!(f.s.app.bookmarks().len(), 1);
}

#[test]
fn bookmarks_come_first_in_the_go_to_place_list_with_their_kind_first() {
    let mut f = open();
    f.s.command(ids::BOOKMARK_ADD);
    f.s.key("ctrl+g");
    let first = match f.s.dialog() {
        Some(Dialog::Edit {
            candidates: Some(list),
            ..
        }) => list.rows()[0].spoken(),
        other => panic!("{other:?}"),
    };
    assert!(
        first.starts_with(&format!("Bookmark, work, in {}", root_leaf(&f))),
        "{first}"
    );
    // Choosing it counts a use.
    f.s.key("down");
    f.s.key("enter");
    assert_eq!(f.s.app.bookmarks().all()[0].uses, 1);
}

#[test]
fn the_bookmarked_fact_reads_the_store() {
    let mut f = open();
    f.s.focus("Photos");
    f.s.command(ids::BOOKMARK_ITEM);
    let lookup = f.s.app.bookmark_lookup(Side::Left);
    let photos = Entry::new("Photos", EntryKind::Folder);
    let notes = Entry::new("notes.txt", EntryKind::File);
    assert_eq!(lookup.bookmarked(&photos), Some(true));
    assert_eq!(lookup.bookmarked(&notes), Some(false));
}

#[test]
fn the_copy_key_in_the_bookmarks_copies_into_the_bookmark() {
    let mut f = open();
    // Bookmark the Backup folder, then copy notes.txt into it from the
    // Bookmarks list (W3a).
    f.s.focus("Backup");
    f.s.command(ids::BOOKMARK_ITEM);
    f.s.focus("notes.txt");
    f.s.key("ctrl+b");
    f.s.key("end");
    let said = f.s.key("f5");
    assert!(
        said.iter().any(|t| t.starts_with("Copy 1 item to Backup")),
        "{said:?}"
    );
    f.s.key("y");
    assert!(f.work.join("Backup").join("notes.txt").exists());
}

#[test]
fn go_to_a_place_names_each_place_first_and_tab_completes_a_bookmark() {
    let mut f = open();
    f.s.focus("Photos");
    f.s.command(ids::BOOKMARK_ITEM);
    f.s.key("ctrl+g");
    let rows: Vec<String> = match f.s.dialog() {
        Some(Dialog::Edit {
            candidates: Some(list),
            ..
        }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("{other:?}"),
    };
    // A visited folder: its own name first, then what it is, then where.
    assert!(
        rows.iter()
            .any(|r| r.starts_with("work, visited ") && r.contains(", in ")),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|r| r.starts_with("Bookmark, Photos")),
        "{rows:?}"
    );
    // Tab finds a bookmark by its name.
    f.s.type_text("photos");
    let said = f.s.key("tab");
    assert!(last(&said).starts_with("completed to "), "{said:?}");
    assert!(last(&said).ends_with("Photos"), "{said:?}");
}
