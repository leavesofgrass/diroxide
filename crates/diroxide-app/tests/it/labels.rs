//! Labels through the app (plan commits 22 and 23): Set label on the
//! marked items, the favorite keys, Clear labels, one undo step taken in
//! turn with the jobs, the store through an injected state folder, and
//! the Labels list with its missing counts and its items.

use std::fs;
use std::path::PathBuf;

use diroxide_app::commands::ids;
use diroxide_app::labels::FILE;
use diroxide_config::Paths;
use diroxide_core::{ArgValue, Command, Side};
use diroxide_vfs::Registry;

use crate::support::{Session, fixture, place, settings, temp};

struct Fixture {
    _dir: tempfile::TempDir,
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
        _dir: dir,
        work,
        paths,
        s,
    }
}

fn last(said: &[String]) -> &str {
    said.last().map(String::as_str).unwrap_or_default()
}

/// The spoken row of a name in the active panel.
fn row(s: &Session, name: &str) -> String {
    s.app
        .active_panel()
        .list()
        .rows()
        .iter()
        .find(|r| r.id == name)
        .map(|r| r.spoken())
        .unwrap_or_default()
}

#[test]
fn set_label_on_three_items_then_one_undo_step() {
    let mut f = open();
    f.s.focus("budget.xlsx");
    f.s.key("space");
    f.s.key("space");
    f.s.key("space");
    let said = f.s.key("alt+l");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Set label on 3 items"));
    assert!(
        said[0].starts_with("Set label on 3 items, empty"),
        "{said:?}"
    );
    f.s.type_text("urg");
    let said = f.s.key("enter");
    assert_eq!(last(&said), "Label urgent added to 3 items");
    assert_eq!(f.s.dialog(), None);
    assert!(
        row(&f.s, "budget.xlsx").starts_with("budget.xlsx, selected, label: urgent"),
        "{}",
        row(&f.s, "budget.xlsx")
    );
    assert!(row(&f.s, "notes.txt").contains("label: urgent"));
    // The other panel shows the same folder: its rows say it too.
    let other = f.s.app.panel(Side::Right).list().rows();
    assert!(other.iter().any(|r| r.spoken().contains("label: urgent")));
    let said = f.s.key("ctrl+z");
    assert_eq!(last(&said), "Undid label urgent on 3 items");
    assert!(!row(&f.s, "budget.xlsx").contains("label"));
    // Nothing more to undo: the job log is empty.
    assert_eq!(last(&f.s.key("ctrl+z")), "Nothing to undo");
}

#[test]
fn a_new_name_makes_a_new_label_and_the_list_offers_it() {
    let mut f = open();
    f.s.focus("report.pdf");
    f.s.key("alt+l");
    f.s.type_text("Taxes");
    let said = f.s.key("enter");
    assert_eq!(
        last(&said),
        "Label taxes added to report.pdf; a new label, blue"
    );
    assert!(f.s.app.label_names().contains(&"Taxes".to_owned()));
    // The label is the seventh: its favorite key toggles it.
    let said = f.s.key("ctrl+7");
    assert_eq!(last(&said), "Label taxes removed from report.pdf");
    f.s.key("alt+l");
    f.s.key("escape");
    assert_eq!(f.s.dialog(), None);
}

#[test]
fn the_favorite_keys_toggle_the_first_labels() {
    let mut f = open();
    f.s.focus("report.pdf");
    assert_eq!(last(&f.s.key("ctrl+1")), "Label urgent added to report.pdf");
    assert_eq!(
        last(&f.s.key("ctrl+1")),
        "Label urgent removed from report.pdf"
    );
    assert_eq!(
        last(&f.s.key("ctrl+7")),
        "No label 7 yet. The Labels list has 6 labels."
    );
    f.s.key("ctrl+2");
    f.s.key("ctrl+5");
    assert!(row(&f.s, "report.pdf").starts_with("report.pdf, labels: waiting, keep"));
    let said = f.s.command(ids::LABEL_CLEAR);
    assert_eq!(last(&said), "Labels cleared from report.pdf");
    assert_eq!(
        last(&f.s.command(ids::UNDO)),
        "Undid clear labels on report.pdf"
    );
    // A command with a label applies it at once (banks, scripts).
    let said = f.s.run_command(
        Command::new(ids::LABEL_TOGGLE).with_arg("label", ArgValue::Text("Done".into())),
    );
    assert_eq!(last(&said), "Label done added to report.pdf");
    let said = f.s.command(ids::SORT_LABEL);
    assert_eq!(last(&said), "Sorted by label, A to Z");
}

#[test]
fn undo_takes_a_job_and_a_label_change_newest_first() {
    let mut f = open();
    f.s.focus("report.pdf");
    f.s.key("ctrl+1");
    let said = f.s.run_command(
        Command::new(ids::MAKE_FOLDER).with_arg("name", ArgValue::Text("Drafts".into())),
    );
    assert!(said.iter().any(|l| l.contains("Drafts")), "{said:?}");
    assert!(f.work.join("Drafts").is_dir());
    let said = f.s.key("ctrl+z");
    assert!(!said.iter().any(|l| l.contains("label")), "{said:?}");
    assert!(!f.work.join("Drafts").exists(), "{said:?}");
    let said = f.s.key("ctrl+z");
    assert_eq!(last(&said), "Undid label urgent on report.pdf");
}

#[test]
fn the_store_round_trips_through_the_state_folder() {
    let mut f = open();
    f.s.focus("report.pdf");
    f.s.key("ctrl+5");
    assert!(f.paths.state_file(FILE).exists());
    let again = Session::start(
        place(&f.work),
        place(&f.work),
        Registry::with_local(),
        Some(&f.paths),
        settings(),
    );
    assert!(
        row(&again, "report.pdf").starts_with("report.pdf, label: keep"),
        "{}",
        row(&again, "report.pdf")
    );
    assert_eq!(
        again
            .app
            .labels()
            .labels_of(&place(&f.work.join("report.pdf"))),
        ["Keep"]
    );
}

#[test]
fn the_labels_list_counts_missing_items_and_lists_a_labels_items() {
    let mut f = open();
    for name in ["budget.xlsx", "notes.txt", "report.pdf"] {
        f.s.focus(name);
        f.s.key("ctrl+1");
    }
    // Two of the three move away: their labels stay, now missing.
    fs::rename(
        f.work.join("budget.xlsx"),
        f.work.join("Backup").join("budget.xlsx"),
    )
    .unwrap();
    fs::rename(
        f.work.join("notes.txt"),
        f.work.join("Photos").join("notes.txt"),
    )
    .unwrap();
    let said = f.s.command(ids::LABELS);
    assert_eq!(f.s.dialog_title().as_deref(), Some("Labels, 6 labels"));
    assert!(
        said[0].starts_with("Labels, 6 labels, Urgent, label, 3 items, checking"),
        "{said:?}"
    );
    let focus = f.s.dialog().unwrap().focus_line();
    assert_eq!(
        focus,
        "Urgent, label, 3 items, 2 missing, orange, Ctrl+1, 1 of 6"
    );
    assert!(diroxide_core::format::within_cells(
        &focus,
        "Urgent, label, 3 items, 2 missing"
    ));
    assert!(
        f.s.said
            .iter()
            .any(|m| m.text == "Checked 3 labeled items: 2 missing"),
        "{:?}",
        f.s.said
    );
    let rows = f.s.app.dialog().map(|(_, d)| d.focus_line());
    assert!(rows.is_some());
    // Enter lists the label's items, the missing ones said so.
    f.s.key("enter");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Urgent, 3 items"));
    let line = f.s.dialog().unwrap().focus_line();
    assert!(line.starts_with("budget.xlsx, missing, in work"), "{line}");
    let said = f.s.key("enter");
    assert!(last(&said).starts_with("Missing: budget.xlsx."), "{said:?}");
    f.s.key("end");
    let line = f.s.dialog().unwrap().focus_line();
    assert!(line.starts_with("report.pdf, in work"), "{line}");
    f.s.key("enter");
    assert_eq!(f.s.dialog(), None);
    assert_eq!(f.s.focused(), "report.pdf");
    // The trash key in a label's items takes the label off that item.
    f.s.command(ids::LABELS);
    f.s.key("enter");
    let said = f.s.key("delete");
    assert_eq!(last(&said), "Label urgent removed from budget.xlsx");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Urgent, 2 items"));
    f.s.key("escape");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Labels, 6 labels"));
    f.s.key("escape");
    assert_eq!(f.s.dialog(), None);
}
