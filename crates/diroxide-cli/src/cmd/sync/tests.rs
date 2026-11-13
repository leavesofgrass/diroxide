//! `diro sync`'s tests, on made folders under the target folder's
//! temporary folder only. No test runs a plan that sends anything to the
//! trash: a mirror and a replace are tried as dry runs only, so the
//! computer's own trash is never touched.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use diroxide_core::format::BRAILLE_CELLS;
use diroxide_core::{FixedClock, Place, UtcOffset};

use super::*;
use crate::cmd::archive::tests::{args, now, session_in};
use crate::cmd::archive::{Asker, NoAsker, Script};

fn write_at(path: &Path, bytes: &[u8], at: SystemTime) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(at).unwrap();
}

/// `here` and `there`: `same.txt` on both, `new.txt` only here,
/// `old.txt` only there, `notes.txt` newer here.
fn fixture() -> (tempfile::TempDir, Place) {
    made(true)
}

/// The fixture, `notes.txt` on both sides or on neither.
fn made(notes: bool) -> (tempfile::TempDir, Place) {
    let dir = tempfile::tempdir().unwrap();
    let then = SystemTime::now() - Duration::from_secs(86_400);
    let later = then + Duration::from_secs(3_600);
    let here = dir.path().join("here");
    let there = dir.path().join("there");
    write_at(&here.join("same.txt"), b"same", then);
    write_at(&there.join("same.txt"), b"same", then);
    write_at(&here.join("new.txt"), b"new here", then);
    write_at(&there.join("old.txt"), b"old there", then);
    if notes {
        write_at(&here.join("notes.txt"), b"notes, newer", later);
        write_at(&there.join("notes.txt"), b"notes", then);
    }
    let place = Place::parse(dir.path().to_str().unwrap()).unwrap();
    (dir, place)
}

fn run_in(here: &Place, asker: &dyn Asker, list: &[&str]) -> (i32, String) {
    let clock = FixedClock::new(now(), UtcOffset::UTC);
    let session = session_in(&clock, here, asker);
    let parsed = parse(&args(list)).unwrap();
    let mut out = Vec::new();
    let code = run_with(&parsed, &mut out, &session, &JobControl::new());
    let text = String::from_utf8(out).unwrap();
    match code {
        Ok(code) => (code, text),
        Err(error) => (-1, format!("{text}{error}")),
    }
}

#[test]
fn arguments_are_read_and_wrong_use_is_refused() {
    let parsed = parse(&args(&[
        "a",
        "b",
        "--mode",
        "mirror",
        "--by=content",
        "--tolerance",
        "1.5",
        "-n",
    ]))
    .unwrap();
    assert_eq!(parsed.mode, SyncMode::MirrorThere);
    assert_eq!(parsed.by, CompareBy::Content);
    assert_eq!(parsed.tolerance, Duration::from_millis(1_500));
    assert!(parsed.dry_run);
    assert_eq!(parse(&args(&["a"])), Err(CliError::Missing("sync".into())));
    assert_eq!(
        parse(&args(&["a", "b", "c"])),
        Err(CliError::TooMany("sync".into()))
    );
    for list in [
        &["a", "b", "--mode", "sideways"][..],
        &["a", "b", "--by", "smell"],
        &["a", "b", "--tolerance", "-1"],
    ] {
        assert!(
            matches!(parse(&args(list)), Err(CliError::Failed(_))),
            "{list:?}"
        );
    }
}

#[test]
fn a_dry_run_says_here_and_there_then_the_plan() {
    let (dir, place) = fixture();
    let (code, text) = run_in(&place, &NoAsker, &["here", "there", "--dry-run"]);
    assert_eq!(code, 0, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        format!("Here: {}", dir.path().join("here").display())
    );
    assert_eq!(
        lines[1],
        format!("There: {}", dir.path().join("there").display())
    );
    assert!(lines[2].starts_with("Update the other panel: "), "{text}");
    assert!(text.contains("Copy there: new.txt, only here\n"), "{text}");
    assert!(text.contains("Skip: old.txt, only there, kept\n"), "{text}");
    assert!(text.contains("Copy there: notes.txt"), "{text}");
    assert!(text.ends_with("Nothing was synchronized.\n"));
    assert!(!dir.path().join("there").join("new.txt").exists());
    // A mirror lists its removal and still changes nothing.
    let (code, text) = run_in(
        &place,
        &NoAsker,
        &["here", "there", "--mode", "mirror", "-n"],
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("old.txt"), "{text}");
    assert!(dir.path().join("there").join("old.txt").exists());
}

#[test]
fn nobody_to_ask_runs_nothing_and_yes_copies() {
    // Only the copy of a file that is not there: nothing is replaced.
    let (dir, place) = made(false);
    let (code, text) = run_in(&place, &NoAsker, &["here", "there"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("Synchronize 1 item? y or n\n"), "{text}");
    assert!(text.ends_with(&format!(
        "Answered no: there is no terminal to ask.\n{ADD_YES}\n"
    )));
    let no = Script::answering(&[false], &[]);
    let (code, _) = run_in(&place, &no, &["here", "there"]);
    assert_eq!(code, 1);
    assert_eq!(no.asked(), ["Synchronize 1 item? y or n"]);
    assert!(!dir.path().join("there").join("new.txt").exists());
    let (code, text) = run_in(&place, &NoAsker, &["here", "there", "--yes"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Synchronize finished: 1 done\n"), "{text}");
    assert_eq!(
        fs::read_to_string(dir.path().join("there").join("new.txt")).unwrap(),
        "new here"
    );
    // The same again: nothing to do.
    let (code, text) = run_in(&place, &NoAsker, &["here", "there", "--yes"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.ends_with("Nothing was synchronized.\n"), "{text}");
}

#[test]
fn one_folder_inside_the_other_is_refused_in_words() {
    let (_dir, place) = fixture();
    fs::create_dir_all(place.to_string() + "/here/inner").unwrap();
    let inner = Path::new(&place.to_string()).join("here").join("inner");
    let (code, text) = run_in(
        &place,
        &NoAsker,
        &["here", inner.to_str().unwrap(), "--dry-run"],
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        text.starts_with("Cannot compare: one folder is inside the other"),
        "{text}"
    );
}

#[test]
fn every_fixed_sentence_says_its_meaning_in_40_cells() {
    for sentence in [
        NOTHING_SYNCHRONIZED,
        NO_TRASH_THERE,
        ADD_YES,
        &question(2),
        "Synchronize finished: 2 done",
    ] {
        assert!(sentence.chars().count() <= BRAILLE_CELLS, "{sentence}");
    }
    assert_eq!(question(2), "Synchronize 2 items? y or n");
    assert_eq!(question(2).chars().count(), 27);
    for line in help_text().lines().filter(|l| l.starts_with("diro sync")) {
        assert!(
            diroxide_core::format::within_cells(line, "diro sync"),
            "{line}"
        );
    }
}
