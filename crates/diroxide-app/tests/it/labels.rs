//! Labels through the app (plan commits 22 and 23): Set label on the
//! marked items, the favorite keys, Clear labels, one undo step taken in
//! turn with the jobs, and the store through an injected state folder.

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
