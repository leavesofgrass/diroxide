//! Compare files in the app (W3a, plan commit 40): which two, the
//! verdict and the change list, N and P and F7, one change in full,
//! Swap, the unified view and the viewer at its line, a file compared
//! with itself, compare properties, and select the differences.

use std::fs;
use std::path::{Path, PathBuf};

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_vfs::Registry;

use crate::support::{Session, place, settings, temp};

/// The diff crate's two near-identical reports.
fn report_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diff/report")
}

struct Fixture {
    _dir: tempfile::TempDir,
    s: Session,
}

/// The new report here (left), the old one under the same name there.
fn open() -> Fixture {
    let dir = temp();
    let here = dir.path().join("here");
    let there = dir.path().join("there");
    fs::create_dir_all(&here).unwrap();
    fs::create_dir_all(&there).unwrap();
    fs::copy(report_fixture().join("report.txt"), here.join("report.txt")).unwrap();
    fs::copy(report_fixture().join("old.txt"), there.join("report.txt")).unwrap();
    fs::write(here.join("only-here.txt"), b"a").unwrap();
    fs::write(there.join("only-there.txt"), b"b").unwrap();
    let s = Session::start(
        place(&here),
        place(&there),
        Registry::with_local(),
        None,
        settings(),
    );
    Fixture { _dir: dir, s }
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn the_verdict_comes_first_then_one_row_a_change() {
    let mut f = open();
    f.s.focus("report.txt");
    let said = f.s.command(ids::COMPARE_FILES);
    assert_eq!(said[0], "Comparing report.txt here with report.txt there");
    let report = &said[1];
    assert!(
        report.contains(" changes between report.txt and report.txt"),
        "{report}"
    );
    assert!(
        report.contains("From the other panel to this panel."),
        "{report}"
    );
    let title = f.s.dialog_title().unwrap();
    assert!(title.chars().count() <= 40, "{title}");
    let list = rows(&f.s);
    assert!(!list.is_empty());
    for row in &list {
        assert!(
            row.starts_with("Changed ") || row.starts_with("Added ") || row.starts_with("Removed "),
            "{row}"
        );
    }
    // F7 and N go on; Shift+F7 and P go back.
    let said = f.s.key("f7");
    assert!(
        said[0].ends_with(&format!("2 of {}", list.len())),
        "{said:?}"
    );
    let said = f.s.key("p");
    assert!(
        said[0].ends_with(&format!("1 of {}", list.len())),
        "{said:?}"
    );
    let said = f.s.key("shift+f7");
    assert_eq!(said, ["No change before this one"]);
    let said = f.s.key("n");
    assert!(
        said[0].ends_with(&format!("2 of {}", list.len())),
        "{said:?}"
    );
    // Enter: the change in full.
    let said = f.s.key("enter");
    assert!(said[0].starts_with("Change 2 of "), "{said:?}");
    f.s.key("escape");
    // Swap: the direction sentence changes.
    let said = f.s.key("s");
    assert_eq!(said, ["Swapped. From this panel to the other panel."]);
    let said = f.s.key("s");
    assert_eq!(said, ["Swapped. From the other panel to this panel."]);
}

#[test]
fn the_unified_view_opens_the_viewer_at_its_line_and_comes_back() {
    let mut f = open();
    f.s.focus("report.txt");
    f.s.command(ids::COMPARE_FILES);
    let said = f.s.key("u");
    assert!(
        said[0].starts_with("Unified view, report.txt, "),
        "{said:?}"
    );
    let list = rows(&f.s);
    let added = list
        .iter()
        .position(|r| r.starts_with("Added "))
        .expect("an added row");
    f.s.key("home");
    for _ in 0..added {
        f.s.key("down");
    }
    let said = f.s.key("enter");
    assert!(f.s.app.viewer_open(), "{said:?}");
    assert!(said.iter().any(|t| t.starts_with("At line ")), "{said:?}");
    let said = f.s.key("escape");
    assert!(
        said.iter().any(|t| t.starts_with("Back in Added ")),
        "{said:?}"
    );
    assert!(f.s.dialog_title().unwrap().starts_with("Unified view, "));
    f.s.key("escape");
    assert!(f.s.dialog().is_some(), "back to the change list");
}

#[test]
fn a_file_with_itself_and_two_marked_files() {
    let dir = temp();
    let here = dir.path().join("here");
    fs::create_dir_all(&here).unwrap();
    fs::copy(report_fixture().join("report.txt"), here.join("report.txt")).unwrap();
    fs::copy(report_fixture().join("old.txt"), here.join("old.txt")).unwrap();
    let mut s = Session::start(
        place(&here),
        place(&here),
        Registry::with_local(),
        None,
        settings(),
    );
    s.focus("report.txt");
    let said = s.command(ids::COMPARE_FILES);
    assert_eq!(said[0], "Comparing report.txt here with report.txt there");
    assert!(said[1].starts_with("Same file: report.txt"), "{said:?}");
    s.key("escape");
    // Two marked in one panel: the first is before.
    s.focus("old.txt");
    s.key("space");
    s.key("space");
    let said = s.command(ids::COMPARE_FILES);
    assert_eq!(said[0], "Comparing old.txt with report.txt");
    assert!(
        said[1].starts_with("5 changes between old.txt and report.txt."),
        "{said:?}"
    );
    assert!(said[1].contains("From old.txt to report.txt."), "{said:?}");
}

#[test]
fn properties_and_the_differences_from_the_listings() {
    let mut f = open();
    f.s.focus("report.txt");
    let said = f.s.command(ids::COMPARE_PROPERTIES);
    assert!(
        said[0].starts_with("Properties of report.txt and report.txt: "),
        "{said:?}"
    );
    assert!(said[0].contains("Size: "), "{said:?}");
    f.s.key("escape");
    let said = f.s.command(ids::SELECT_DIFFERENCES);
    assert_eq!(said, ["Marked 4: 1 only here, 1 only there, 1 different"]);
    assert_eq!(f.s.app.active_panel().list().selected_count(), 2);
}

#[test]
fn a_file_over_the_text_limit_asks_starting_on_cancel() {
    let dir = temp();
    let here = dir.path().join("here");
    let there = dir.path().join("there");
    fs::create_dir_all(&here).unwrap();
    fs::create_dir_all(&there).unwrap();
    let big: Vec<u8> = b"line of text\n".repeat(90_000);
    fs::write(here.join("big.log"), &big).unwrap();
    let mut other = big.clone();
    other.extend_from_slice(b"one more line\n");
    fs::write(there.join("big.log"), &other).unwrap();
    let mut setup = settings();
    setup.compare.text_limit_mib = 1;
    let mut s = Session::start(
        place(&here),
        place(&there),
        Registry::with_local(),
        None,
        setup,
    );
    s.focus("big.log");
    let said = s.command(ids::COMPARE_FILES);
    assert!(said[1].starts_with("Too large to compare"), "{said:?}");
    let focus = s.dialog().unwrap().focus_line();
    assert!(focus.starts_with("Cancel, compare nothing"), "{focus}");
    s.key("up");
    let said = s.key("enter");
    assert_eq!(said[0], "Comparing big.log here with big.log there");
    assert!(said[1].contains("byte by byte"), "{said:?}");
}
