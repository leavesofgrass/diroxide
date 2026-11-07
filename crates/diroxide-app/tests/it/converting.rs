//! The Convert command and the conversion reports (alpha 4, lane AW, R30
//! commit 62): the recipes list, the plan's question starting on No, the
//! job through CL's engine, the report said and kept, and Undo. The
//! recipes used are carta's, built in and run in process, so no program
//! starts; a recipe whose tool the fake tools runner does not find is
//! listed last and unavailable. Every file is made in the test's own
//! folder; the trash is a test folder.

use std::fs;
use std::path::Path;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_config::Paths;

use crate::support::{Session, Setup, place, registry_with_trash, temp};

/// A folder with a Markdown file and an empty `out`, a session in it
/// with the state folder under the test's folder and a test trash.
fn session(dir: &Path) -> (Session, std::path::PathBuf) {
    let work = dir.join("work");
    fs::create_dir_all(work.join("out")).unwrap();
    fs::write(
        work.join("notes.md"),
        "# Notes\n\nA line of notes.\n\n- one\n- two\n",
    )
    .unwrap();
    let paths = Paths::new(dir.join("config"), dir.join("state"));
    let (registry, _trash) = registry_with_trash(&dir.join("bin"));
    let setup = Setup {
        paths: Some(paths),
        ..Setup::default()
    };
    let s = Session::open(place(&work), place(&work.join("out")), registry, setup);
    (s, work)
}

/// The open list's rows: id, label and whether each can be chosen.
fn rows(s: &Session) -> Vec<(String, String, bool)> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list
            .rows()
            .iter()
            .map(|r| (r.id.clone(), r.label.clone(), r.enabled))
            .collect(),
        other => panic!("{other:?}"),
    }
}

/// Moves the open list's focus to a row by its id.
fn choose(s: &mut Session, id: &str) {
    for _ in 0..60 {
        let focused = match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.id.clone()),
            _ => None,
        };
        if focused.as_deref() == Some(id) {
            return;
        }
        s.key("down");
    }
    panic!("{id} is not in the list");
}

#[test]
fn convert_lists_the_recipes_plans_on_no_runs_and_undo_trashes_the_result() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    s.focus("notes.md");
    s.command(ids::CONVERT);
    assert_eq!(s.dialog_title().as_deref(), Some("Convert notes.md"));
    let listed = rows(&s);
    // The ready recipes first, by name; pandoc's last, unavailable, the
    // name first.
    let first_missing = listed.iter().position(|(_, _, on)| !on).unwrap();
    assert!(listed[..first_missing].iter().all(|(_, _, on)| *on));
    assert!(listed[first_missing..].iter().all(|(_, _, on)| !on));
    assert!(
        listed.iter().any(|(id, label, on)| id == "markdown-to-word"
            && label == "Markdown to Word, needs pandoc"
            && !on),
        "{listed:?}"
    );
    assert!(
        listed
            .iter()
            .any(|(id, label, _)| id == "markdown-to-plain-text"
                && label == "Markdown to plain text"),
        "{listed:?}"
    );
    for (_, label, _) in &listed {
        assert!(label.chars().count() <= 40, "{label}");
    }
    choose(&mut s, "markdown-to-plain-text");
    let said = s.key("enter");
    let question = said.last().cloned().unwrap_or_default();
    assert!(
        question.starts_with("Convert 1 file to ")
            && question.ends_with("No is the default. y or n"),
        "{said:?}"
    );
    let listing = match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        listing.contains(&"notes.md to notes.txt".to_owned()),
        "{listing:?}"
    );
    assert!(
        listing.contains(&"Tool: carta, built in".to_owned()),
        "{listing:?}"
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Convert started: 1 file to ")),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.starts_with("Report: 1 converted")),
        "{said:?}"
    );
    let made = work.join("out").join("notes.txt");
    let text = fs::read_to_string(&made).unwrap();
    assert!(text.contains("A line of notes."), "{text}");
    // Undo trashes the file the job made.
    let said = s.key("ctrl+z");
    assert!(
        said.iter()
            .any(|l| l == "Undid conversion: 1 item is in the trash"),
        "{said:?}"
    );
    assert!(!made.exists());
}

#[test]
fn no_changes_nothing_and_the_reports_list_reads_a_kept_report() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    s.focus("notes.md");
    s.command(ids::CONVERT);
    choose(&mut s, "markdown-to-org");
    s.key("enter");
    assert_eq!(s.key("n"), ["Nothing changed."]);
    assert!(!work.join("out").join("notes.org").exists());
    assert_eq!(
        s.command(ids::CONVERT_REPORTS),
        ["No conversion reports yet"]
    );
    s.command(ids::CONVERT);
    choose(&mut s, "markdown-to-org");
    s.key("enter");
    s.key("y");
    assert!(work.join("out").join("notes.org").exists());
    s.command(ids::CONVERT_REPORTS);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Conversion reports: 1 kept")
    );
    let listed = rows(&s);
    assert_eq!(listed[0].1, "Report: 1 converted");
    s.key("enter");
    match s.dialog() {
        Some(Dialog::Read { title, lines }) => {
            assert_eq!(title, "Report: 1 converted");
            assert!(
                lines.iter().any(|l| l == "Made: notes.org from notes.md"),
                "{lines:?}"
            );
            assert!(
                lines.iter().any(|l| l.starts_with("Finished: ")),
                "{lines:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_no_recipe_takes_says_so() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    fs::write(work.join("blob.qqq"), "x").unwrap();
    s.key("ctrl+r");
    s.focus("blob.qqq");
    assert_eq!(s.command(ids::CONVERT), ["No recipe takes these files"]);
}
