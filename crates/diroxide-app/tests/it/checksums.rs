//! The Checksum command through the app (alpha 4, lane MA, R30 commit
//! 54): the value's start said, the whole value in lines of eight groups,
//! copied, compared with a pasted value verdict first, saved as
//! `SHA256SUMS` beside the file under a free name, never over one.

use std::fs;
use std::path::Path;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_core::format::within_cells;
use diroxide_core::{Effect, JobKind};
use diroxide_vfs::Registry;

use crate::support::{Session, place, settings, temp};

/// The SHA-256 of "abc", which every implementation agrees on.
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn work(root: &Path) -> std::path::PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(folder.join("Shots")).unwrap();
    fs::write(folder.join("example-tool.zip"), b"abc").unwrap();
    fs::write(folder.join("notes.txt"), b"notes").unwrap();
    folder
}

fn session(work: &Path) -> Session {
    Session::start(
        place(work),
        place(work),
        Registry::with_local(),
        None,
        settings(),
    )
}

fn rows(s: &Session) -> Vec<(String, String)> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list
            .rows()
            .iter()
            .map(|r| (r.id.clone(), r.label.clone()))
            .collect(),
        other => panic!("no list: {other:?}"),
    }
}

fn to_row(s: &mut Session, id: &str) {
    s.key("home");
    for _ in 0..20 {
        let here = match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.id.clone()),
            _ => None,
        };
        if here.as_deref() == Some(id) {
            return;
        }
        s.key("down");
    }
    panic!("no row {id}");
}

fn clipboard(s: &Session) -> Option<String> {
    s.effects.iter().rev().find_map(|e| match e {
        Effect::SetClipboardText(text) => Some(text.clone()),
        _ => None,
    })
}

#[test]
fn checksum_says_the_start_lists_the_value_and_copies_it() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("example-tool.zip");
    let said = s.command(ids::CHECKSUM);
    assert_eq!(said[0], "Computing the SHA-256 of example-tool.zip");
    let title = s.dialog_title().unwrap();
    assert_eq!(title, "SHA-256 of example-tool.zip, ba78 16bf");
    assert_eq!(title.chars().count(), 38);
    assert_eq!(
        rows(&s),
        [
            ("line:0", "ba78 16bf 8f01 cfea 4141 40de 5dae 2223"),
            ("line:1", "b003 61a3 9617 7a9c b410 ff61 f200 15ad"),
            ("copy", "Copy the value"),
            ("compare", "Compare with a pasted value"),
            ("save", "Save as SHA256SUMS"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    for (_, label) in rows(&s) {
        assert!(within_cells(&label, &label), "{label}");
    }
    to_row(&mut s, "copy");
    let said = s.key("enter");
    assert_eq!(
        said.last().unwrap(),
        "Copied the SHA-256 of example-tool.zip"
    );
    assert_eq!(clipboard(&s).as_deref(), Some(ABC));
    // The job is in the history, as the other jobs are.
    let finished = s
        .app
        .jobs()
        .history()
        .iter()
        .find(|f| f.kind == JobKind::Checksum)
        .map(|f| f.summary.clone());
    assert_eq!(finished.as_deref(), Some("Checksum finished: 1 done."));
}

#[test]
fn a_pasted_value_is_judged_verdict_first() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("example-tool.zip");
    s.command(ids::CHECKSUM);
    to_row(&mut s, "compare");
    s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Compare with the SHA-256 of example-tool.zip, paste the value")
    );
    let mut wrong = ABC.to_owned();
    wrong.replace_range(20..21, "f");
    s.type_text(&wrong);
    let said = s.key("enter");
    assert_eq!(
        said.last().unwrap(),
        "Does not match: example-tool.zip, SHA-256. The first difference is in group 6"
    );
    to_row(&mut s, "compare");
    s.key("enter");
    s.type_text(&ABC.to_uppercase());
    let said = s.key("enter");
    assert_eq!(said.last().unwrap(), "Match: example-tool.zip, SHA-256");
    // A value of SHA-512's length computes SHA-512 first, then judges.
    to_row(&mut s, "compare");
    s.key("enter");
    s.type_text(&"0".repeat(128));
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l == "Computing the SHA-512 of example-tool.zip"),
        "{said:?}"
    );
    assert_eq!(
        said.last().unwrap(),
        "Does not match: example-tool.zip, SHA-512. The first difference is in group 1"
    );
}

#[test]
fn saving_writes_sha256sums_and_never_over_a_file() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("example-tool.zip");
    s.key("space");
    s.focus("notes.txt");
    s.key("space");
    s.focus("Shots");
    s.key("space");
    s.command(ids::CHECKSUM);
    assert_eq!(s.dialog_title().as_deref(), Some("SHA-256 of 2 files"));
    let labels: Vec<String> = rows(&s).into_iter().map(|(_, l)| l).collect();
    assert_eq!(labels[0], "example-tool.zip: ba78 16bf, ending 15ad");
    assert_eq!(labels[2], "Copy all as SHA256SUMS lines");
    to_row(&mut s, "save");
    let said = s.key("enter");
    assert_eq!(said.last().unwrap(), "Saved SHA256SUMS in work");
    let text = fs::read_to_string(work.join("SHA256SUMS")).unwrap();
    assert!(
        text.starts_with(&format!("{ABC}  example-tool.zip\n")),
        "{text}"
    );
    assert!(text.contains("  notes.txt\n"), "{text}");
    // The panel lists it, and a second save takes the next free name.
    assert!(
        s.names(diroxide_core::Side::Left)
            .contains(&"SHA256SUMS".to_owned())
    );
    s.focus("example-tool.zip");
    s.command(ids::CHECKSUM);
    to_row(&mut s, "save");
    let said = s.key("enter");
    assert_eq!(said.last().unwrap(), "Saved SHA256SUMS (2) in work");
    assert_eq!(
        fs::read_to_string(work.join("SHA256SUMS")).unwrap(),
        text,
        "the first list is never overwritten"
    );
}

#[test]
fn a_folder_alone_is_not_checksummed() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("Shots");
    let said = s.command(ids::CHECKSUM);
    assert_eq!(said, ["Checksum works on files; no file is chosen"]);
}

#[test]
fn the_details_say_the_value_computed_this_session() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("example-tool.zip");
    s.command(ids::CHECKSUM);
    s.key("escape");
    s.command(ids::DETAILS);
    let labels: Vec<String> = rows(&s).into_iter().map(|(_, l)| l).collect();
    let spoken: Vec<String> = match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        _ => Vec::new(),
    };
    assert!(
        spoken
            .iter()
            .any(|r| r == "Checksum: SHA-256: ba78 16bf, ending 15ad"),
        "{spoken:?} {labels:?}"
    );
}
