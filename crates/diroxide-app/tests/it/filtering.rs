//! The panel filter on the one filter language (W3a, plan commit 35):
//! silence while typing, the count line after the pause on the app's
//! clock, the problems in two sentences, the title's read-back, Say
//! filter, stash and restore, the kind toggles, the content question,
//! clearing, and Tab completion.

use diroxide_app::commands::ids;
use diroxide_core::Side;

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

/// Lets the filter's pause pass on the app's clock and runs its timer.
fn pause(s: &mut Session) -> Vec<String> {
    assert!(s.app.app_wants_tick().is_some(), "no pause is waiting");
    s.clock.advance(1);
    let effects = s.app.app_tick();
    s.run(effects)
}

#[test]
fn typing_is_silent_and_the_count_comes_after_the_pause() {
    let mut f = open();
    assert_eq!(f.s.key("ctrl+s"), ["Filter, empty"]);
    let said = f.s.type_text("*.txt");
    assert!(said.is_empty(), "nothing is said while typing: {said:?}");
    assert_eq!(f.s.app.active_panel().list().len(), 4);
    let said = pause(&mut f.s);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("4 matches, first "), "{said:?}");
    // The focus is on the first match.
    assert_eq!(f.s.app.active_panel().list().focus_index(), Some(0));
    // Said once: no second line without a new key.
    assert!(f.s.app.app_wants_tick().is_none());
    assert_eq!(f.s.key("enter"), ["Filter on, 4 of 8 shown"]);
    let title = f.s.app.title_line();
    assert!(
        title.starts_with("work, left panel, filter: name matches *.txt"),
        "{title}"
    );
    // Cancel with no job takes the filter off.
    assert_eq!(f.s.command(ids::CANCEL), ["Filter off, 8 shown"]);
    assert!(f.s.app.active_panel().filter().is_none());
}

#[test]
fn a_problem_is_said_after_the_pause_and_then_its_next_step() {
    let mut f = open();
    f.s.key("ctrl+s");
    assert!(f.s.type_text("size over").is_empty());
    // The panel keeps the last text that read ("siz") while this cannot.
    assert_eq!(f.s.app.active_panel().list().len(), 0);
    assert_eq!(pause(&mut f.s), ["Filter problem: size needs a number"]);
    let next = pause(&mut f.s);
    assert_eq!(next.len(), 1);
    assert!(next[0].starts_with("Word 3. "), "{next:?}");
    assert!(f.s.app.app_wants_tick().is_none());
    // Enter on a problem says its first sentence at once and keeps the
    // prompt open.
    assert_eq!(f.s.key("enter"), ["Filter problem: size needs a number"]);
    assert_eq!(f.s.dialog_title().as_deref(), Some("Filter"));
    f.s.type_text(" 1 KB");
    assert_eq!(
        pause(&mut f.s),
        ["1 match, report.pdf. 2 unknown, folder size not counted"]
    );
    // A depth test means nothing in one folder.
    f.s.key("escape");
    f.s.key("ctrl+s");
    f.s.type_text("depth at most 1");
    assert_eq!(pause(&mut f.s), ["Filter problem: depth works in searches"]);
}

#[test]
fn say_filter_spells_the_days_and_escape_puts_the_filter_back() {
    let mut f = open();
    f.s.key("ctrl+s");
    f.s.type_text("modified last week");
    // The test clock is Friday, October 16, 2026; weeks start on Sunday.
    let said = f.s.command(ids::SAY_FILTER);
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("modified last week, October 4 to 10"),
        "{said:?}"
    );
    let said = f.s.command(ids::SAY_STATUS);
    assert!(
        said[0].starts_with("Filter: modified last week. Reads as modified last week"),
        "{said:?}"
    );
    assert_eq!(f.s.key("escape"), ["No filter, 8 shown"]);
    assert_eq!(f.s.app.active_panel().list().len(), 8);
    assert_eq!(f.s.command(ids::SAY_FILTER), ["No filter, 8 shown"]);
}

#[test]
fn stash_restore_and_the_kind_toggles() {
    let mut f = open();
    f.s.key("ctrl+s");
    f.s.type_text("*.txt");
    f.s.key("enter");
    assert_eq!(
        f.s.command(ids::FILTER_STASH),
        ["Filter stashed, 8 shown. 1 stashed"]
    );
    assert_eq!(
        f.s.command(ids::FILTER_RESTORE),
        ["Filter restored, 4 of 8 shown: name matches *.txt"]
    );
    assert_eq!(f.s.command(ids::FILTER_RESTORE), ["No stashed filter"]);
    assert_eq!(
        f.s.command(ids::ONLY_FOLDERS),
        ["Folders only, 2 of 8 shown"]
    );
    assert_eq!(
        f.s.command(ids::ONLY_FOLDERS),
        ["Folders only off, 4 of 8 shown"],
        "the filter before it comes back"
    );
    f.s.command(ids::FILTER_CLEAR);
    assert_eq!(f.s.command(ids::ONLY_FILES), ["Files only, 6 of 8 shown"]);
    assert_eq!(f.s.command(ids::ONLY_FILES), ["Files only off, 8 shown"]);
}

#[test]
fn a_content_filter_asks_first_and_reads_on_yes() {
    let mut f = open();
    f.s.key("ctrl+s");
    f.s.type_text("content:jimothy");
    assert_eq!(pause(&mut f.s), ["Content test: Enter asks to read files"]);
    assert_eq!(
        f.s.key("enter"),
        ["Read the contents of 6 files to filter? No is the default. y or n"]
    );
    assert_eq!(f.s.key("n"), ["Nothing was read. The filter is as it was."]);
    assert_eq!(f.s.app.active_panel().list().len(), 8);
    f.s.key("ctrl+s");
    f.s.type_text("content:jimothy");
    f.s.key("enter");
    let said = f.s.key("y");
    assert_eq!(
        said,
        [
            "Reading the files for the filter",
            "Filter on, 1 of 8 shown"
        ]
    );
    let names: Vec<String> = f.s.names(Side::Left);
    assert_eq!(names, ["raccoon \u{1f99d}.txt"]);
}

#[test]
fn tab_completes_the_filter_language() {
    let mut f = open();
    f.s.key("ctrl+s");
    f.s.type_text("size ov");
    let said = f.s.key("tab");
    assert_eq!(said, ["completed to over"]);
    match f.s.dialog() {
        Some(diroxide_app::Dialog::Edit { field, .. }) => assert_eq!(field.text(), "size over"),
        other => panic!("{other:?}"),
    }
    f.s.type_text(" 1 ");
    let said = f.s.key("tab");
    assert!(said[0].contains("matches"), "{said:?}");
}

#[test]
fn a_typed_filter_lint_is_said_once() {
    let mut f = open();
    f.s.key("ctrl+s");
    f.s.type_text("*.txt *.md");
    let said = pause(&mut f.s);
    assert!(said[0].starts_with("No matches. "), "{said:?}");
    f.s.type_text(" ");
    let said = pause(&mut f.s);
    assert_eq!(said, ["No matches"], "the lint is not said again");
}
