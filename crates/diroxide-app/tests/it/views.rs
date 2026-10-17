//! Per-folder views and the History list through the app (plan commit
//! 25): a saved view comes back on entering its folder and says so, the
//! usual view returns on leaving, a forgotten view stays forgotten, both
//! through an injected state folder; the History list over the path jump
//! store, Enter and Forget this place; and the label and bookmark
//! completion sources in their fields.

use std::path::PathBuf;

use diroxide_app::commands::ids;
use diroxide_app::views::FILE;
use diroxide_config::Paths;
use diroxide_core::{Side, SortKey};
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
        place(&work),
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

/// Moves the open list's focus to the row whose place ends with `leaf`.
fn focus_row(s: &mut Session, leaf: &str) {
    s.key("home");
    for _ in 0..20 {
        let on = match s.dialog() {
            Some(diroxide_app::Dialog::Choose { list, .. }) => {
                list.focused().map(|r| r.id.clone()).unwrap_or_default()
            }
            _ => String::new(),
        };
        if on.ends_with(leaf) {
            return;
        }
        s.key("down");
    }
    panic!("no row for {leaf}");
}

fn sort_key(s: &Session) -> SortKey {
    s.app.active_panel().sort().key.clone()
}

#[test]
fn a_saved_view_comes_back_on_entering_and_says_so() {
    let mut f = open();
    f.s.key("ctrl+f6");
    let said = f.s.command(ids::SAVE_VIEW);
    assert_eq!(
        last(&said),
        "View saved for work: sorted by size, largest first"
    );
    assert!(f.paths.state_file(FILE).exists());
    // Up, where the usual view is kept: sort by name there.
    f.s.key("backspace");
    f.s.key("ctrl+f3");
    assert_eq!(sort_key(&f.s), SortKey::Name);
    f.s.focus("work");
    let said = f.s.key("enter");
    let line = last(&said);
    assert!(
        line.starts_with("work, ")
            && line.ends_with(". Sorted by size, largest first; saved for this folder."),
        "{line}"
    );
    assert_eq!(sort_key(&f.s), SortKey::Size);
    // Leaving puts the usual view back, without a word about it.
    let said = f.s.key("backspace");
    assert!(!said.iter().any(|l| l.contains("saved")), "{said:?}");
    assert_eq!(sort_key(&f.s), SortKey::Name);
    // The session keeps the usual view, not the folder's.
    f.s.focus("work");
    f.s.key("enter");
    assert_eq!(f.s.app.session().panel(Side::Left).sort.key, SortKey::Name);
}

#[test]
fn a_view_round_trips_and_a_forgotten_one_stays_forgotten() {
    let mut f = open();
    f.s.key("ctrl+f5");
    f.s.command(ids::SAVE_VIEW);
    let mut again = Session::start(
        place(&f.root),
        place(&f.root),
        Registry::with_local(),
        Some(&f.paths),
        settings(),
    );
    assert_eq!(again.app.folder_views().all().len(), 1);
    again.focus("work");
    let said = again.key("enter");
    assert!(
        last(&said).ends_with("Sorted by modified time, newest first; saved for this folder."),
        "{said:?}"
    );
    let said = again.command(ids::FORGET_VIEW);
    assert_eq!(
        last(&said),
        "View forgotten for work. The usual view returns on leaving."
    );
    assert_eq!(
        last(&again.command(ids::FORGET_VIEW)),
        "work has no saved view."
    );
    again.key("backspace");
    again.focus("work");
    let said = again.key("enter");
    assert!(!last(&said).contains("saved"), "{said:?}");
}

#[test]
fn labels_and_bookmarks_complete_in_their_fields() {
    let mut f = open();
    // Tab completes a label name in the Set label field.
    f.s.focus("report.pdf");
    f.s.key("alt+l");
    f.s.type_text("ke");
    let said = f.s.key("tab");
    assert_eq!(last(&said), "completed to Keep");
    f.s.key("escape");
    assert_eq!(f.s.dialog(), None);
    // A bookmark completes to its place in the Go to place field.
    f.s.command(ids::BOOKMARK_ADD);
    f.s.key("backspace");
    f.s.key("ctrl+g");
    f.s.type_text("wor");
    let said = f.s.key("tab");
    assert!(last(&said).starts_with("completed to "), "{said:?}");
    assert!(last(&said).ends_with("work"), "{said:?}");
    let candidates = diroxide_app::entry::bookmark_candidates(f.s.app.bookmarks());
    assert_eq!(
        candidates[0].spoken(),
        format!("{}, bookmark work", place(&f.work))
    );
    let labels = diroxide_app::entry::label_candidates(f.s.app.labels());
    assert_eq!(labels[0].spoken(), "Urgent, label, orange");
}

#[test]
fn the_history_list_goes_back_and_forgets() {
    let mut f = open();
    f.s.focus("Backup");
    f.s.key("enter");
    f.s.key("backspace");
    f.s.focus("Photos");
    f.s.key("enter");
    f.s.key("backspace");
    f.s.focus("Backup");
    f.s.key("enter");
    f.s.key("backspace");
    let said = f.s.command(ids::HISTORY_LIST);
    assert!(
        said[0].starts_with("History, 3 places, work, in "),
        "{said:?}"
    );
    focus_row(&mut f.s, "Backup");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(
        line.starts_with("Backup, in work, used 2 times, today"),
        "{line}"
    );
    let said = f.s.key("delete");
    assert!(last(&said).starts_with("Forgot Backup. "), "{said:?}");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(!line.starts_with("Backup"), "{line}");
    // Enter goes to the place.
    focus_row(&mut f.s, "Photos");
    f.s.key("enter");
    assert_eq!(f.s.dialog(), None);
    assert_eq!(
        f.s.app.active_panel().place(),
        &place(&f.work.join("Photos"))
    );
    // Forget this place, from the palette, for the active folder.
    let said = f.s.command(ids::FORGET_PLACE);
    assert!(last(&said).starts_with("Forgot Photos. "), "{said:?}");
    let said = f.s.command(ids::FORGET_PLACE);
    assert_eq!(last(&said), "Photos is not in the history.");
}
