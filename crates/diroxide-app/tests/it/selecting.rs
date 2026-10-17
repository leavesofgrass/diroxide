//! Selection on the filter language (W3a, plan commit 36): the count
//! before a pattern applies, by kind, by size and by date, the same
//! extension, restoring the selection before, a range, and the next rows.

use diroxide_app::commands::ids;

use crate::support::{Session, fixture, place, settings, temp};

struct Fixture {
    _dir: tempfile::TempDir,
    s: Session,
}

fn open() -> Fixture {
    let dir = temp();
    let work = fixture(dir.path());
    let backup = work.join("Backup");
    let s = Session::start(
        place(&work),
        place(&backup),
        diroxide_vfs::Registry::with_local(),
        None,
        settings(),
    );
    Fixture { _dir: dir, s }
}

fn selected(s: &Session) -> Vec<String> {
    s.app
        .active_panel()
        .list()
        .selected_ids()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_count_comes_before_a_pattern_and_no_changes_nothing() {
    let mut f = open();
    assert_eq!(f.s.key("plus"), ["Select by pattern: *"]);
    f.s.key("backspace");
    f.s.type_text("ext txt");
    assert_eq!(
        f.s.key("enter"),
        ["4 entries will be selected. No is the default. y or n"]
    );
    assert_eq!(f.s.key("n"), ["Nothing changed."]);
    assert!(selected(&f.s).is_empty());
    f.s.key("plus");
    f.s.key("backspace");
    f.s.type_text("ext txt");
    f.s.key("enter");
    assert_eq!(f.s.key("y"), ["4 selected"]);
    // A second pass with nothing new to select says so and asks nothing.
    f.s.key("plus");
    f.s.key("backspace");
    f.s.type_text("ext txt");
    assert_eq!(
        f.s.key("enter"),
        ["No more entries match. Nothing changed."]
    );
    // `selected` narrows the selection: unselect the small ones.
    f.s.key("-");
    f.s.key("backspace");
    f.s.type_text("selected and size under 3 B");
    assert_eq!(
        f.s.key("enter"),
        ["2 entries will be unselected. No is the default. y or n"]
    );
    assert_eq!(f.s.key("y"), ["2 selected"]);
    // Restore selection brings back the four.
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 4 selected."]
    );
}

#[test]
fn a_depth_test_or_a_bad_pattern_is_refused_in_words() {
    let mut f = open();
    f.s.key("plus");
    f.s.key("backspace");
    f.s.type_text("depth at most 1");
    let said = f.s.key("enter");
    assert!(
        said[0].starts_with("Filter problem: depth works in searches"),
        "{said:?}"
    );
    assert_eq!(f.s.dialog_title().as_deref(), Some("Select by pattern"));
}

#[test]
fn by_kind_size_and_date() {
    let mut f = open();
    let said = f.s.command(ids::SELECT_BY_KIND);
    assert_eq!(said, ["Select by kind, Folders, 1 of 11"]);
    assert_eq!(f.s.key("enter"), ["Selected 2 folders. 2 selected."]);
    assert_eq!(selected(&f.s), ["Backup", "Photos"]);
    f.s.command(ids::SELECT_NONE);
    let said = f.s.command(ids::SELECT_BY_SIZE);
    assert_eq!(said, ["Select by pattern: size over "]);
    f.s.type_text("1 KB");
    assert_eq!(
        f.s.key("enter"),
        ["1 entry will be selected. No is the default. y or n"]
    );
    assert_eq!(f.s.key("y"), ["1 selected"]);
    assert_eq!(selected(&f.s), ["report.pdf"]);
    f.s.command(ids::SELECT_NONE);
    let said = f.s.command(ids::SELECT_BY_DATE);
    assert_eq!(said, ["Select by pattern: modified within "]);
    // The fixture was written moments ago; the test clock is October 16,
    // 2026, so "within 1000 days" reaches back past now.
    f.s.type_text("1000 days and kind file");
    let said = f.s.key("enter");
    assert!(said[0].ends_with("No is the default. y or n"), "{said:?}");
}

#[test]
fn the_same_extension_a_range_and_the_next_rows() {
    let mut f = open();
    f.s.focus("notes.txt");
    assert_eq!(
        f.s.command(ids::SELECT_SAME_EXTENSION),
        ["Selected 4 with .txt. 4 selected."]
    );
    f.s.command(ids::SELECT_NONE);
    f.s.focus("Backup");
    let said = f.s.command(ids::SELECT_RANGE);
    assert_eq!(said, ["Range started at Backup. Mark range again ends it."]);
    f.s.key("down");
    f.s.key("down");
    let said = f.s.command(ids::SELECT_RANGE);
    assert!(said[0].starts_with("Marked 3, Backup to "), "{said:?}");
    assert!(said[0].ends_with(". 3 selected."), "{said:?}");
    f.s.command(ids::SELECT_NONE);
    f.s.key("home");
    assert_eq!(
        f.s.command(ids::SELECT_NEXT_ROWS),
        ["Select the next rows, how many, empty"]
    );
    f.s.type_text("five");
    assert_eq!(f.s.key("enter"), ["Type a number of rows, such as 5"]);
    for _ in 0..4 {
        f.s.key("backspace");
    }
    f.s.type_text("5");
    assert_eq!(f.s.key("enter"), ["Marked 5 rows. 5 selected."]);
    assert_eq!(
        f.s.app.active_panel().list().focus_index(),
        Some(5),
        "the focus moves past them"
    );
}
