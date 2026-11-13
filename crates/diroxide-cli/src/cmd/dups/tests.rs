//! `diro dups`'s tests, on made folders under the target folder's
//! temporary folder only.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use diroxide_core::format::BRAILLE_CELLS;
use diroxide_core::{FixedClock, UtcOffset};

use super::*;
use crate::cmd::archive::NoAsker;
use crate::cmd::archive::tests::{args, now, session_in};

fn write_at(path: &Path, bytes: &[u8], days_ago: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(days_ago * 86_400))
        .unwrap();
}

/// Three crows of 16 bytes (the oldest in `a`), a song twice, a unique
/// note and two empty files.
fn fixture() -> (tempfile::TempDir, Place) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("dups");
    let crow = [7u8; 16];
    write_at(&root.join("photos/a/crow.png"), &crow, 30);
    write_at(&root.join("photos/b/crow.png"), &crow, 20);
    write_at(&root.join("photos/c/crow-copy.png"), &crow, 10);
    write_at(&root.join("music/song.txt"), b"la la la", 5);
    write_at(&root.join("music/again/song.txt"), b"la la la", 4);
    write_at(&root.join("note.txt"), b"only one", 3);
    write_at(&root.join("empty-1.txt"), b"", 2);
    write_at(&root.join("empty-2.txt"), b"", 2);
    let here = Place::parse(dir.path().to_str().unwrap()).unwrap();
    (dir, here)
}

fn run_in(here: &Place, list: &[&str]) -> (i32, String) {
    let clock = FixedClock::new(now(), UtcOffset::UTC);
    let session = session_in(&clock, here, &NoAsker);
    let parsed = parse(&args(list)).unwrap();
    let mut out = Vec::new();
    let code = run_with(&parsed, &mut out, &session, &JobControl::new());
    let text = String::from_utf8_lossy(&out).into_owned();
    match code {
        Ok(code) => (code, text),
        Err(error) => (-1, format!("{text}{error}")),
    }
}

#[test]
fn arguments_are_read_and_wrong_use_is_refused() {
    let parsed = parse(&args(&["a", "b", "--keep", "newest", "--format=tsv"])).unwrap();
    assert_eq!(parsed.paths, ["a", "b"]);
    assert_eq!(parsed.keep, Some(Keep::Rule(KeepRule::Newest)));
    assert_eq!(parsed.format, Format::Tsv);
    assert_eq!(parse(&args(&[])), Err(CliError::Missing("dups".into())));
    assert!(matches!(
        parse(&args(&["a", "--keep", "biggest"])),
        Err(CliError::Failed(_))
    ));
    assert!(matches!(
        parse(&args(&["a", "--keep", "oldest", "--keep-in", "b"])),
        Err(CliError::Failed(_))
    ));
    assert!(matches!(
        parse(&args(&["a", "-0", "--format", "json"])),
        Err(CliError::Failed(_))
    ));
    assert_eq!(
        parse(&args(&["a", "--bogus"])),
        Err(CliError::UnknownOption("--bogus".into()))
    );
}

#[test]
fn the_groups_are_said_count_first_in_the_listers_words() {
    let (dir, here) = fixture();
    let (code, text) = run_in(&here, &["dups"]);
    assert_eq!(code, 0, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "2 groups, 5 files, 40 bytes to free");
    assert_eq!(lines[1], "2 empty files left out");
    assert_eq!(
        lines[2],
        "Group 1 of 2, 3 copies, 16 bytes each, 32 bytes to free"
    );
    let a = dir.path().join("dups").join("photos").join("a");
    assert!(
        lines[3..6]
            .contains(&format!("crow.png, copy 1 of 3, group 1, in {}", a.display()).as_str()),
        "{text}"
    );
    assert_eq!(
        lines[6],
        "Group 2 of 2, 2 copies, 8 bytes each, 8 bytes to free"
    );
    assert_eq!(lines.len(), 9, "{text}");
    for line in &lines[..3] {
        assert!(line.chars().count() <= BRAILLE_CELLS + 20, "{line}");
    }
    assert!(lines[0].chars().count() <= BRAILLE_CELLS);
}

#[test]
fn keep_says_what_would_be_marked_and_removes_nothing() {
    let (dir, here) = fixture();
    let (code, text) = run_in(&here, &["dups", "--keep", "oldest"]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("Marked 3 files, 40 bytes. Kept the oldest in 2 groups.\n"),
        "{text}"
    );
    let marked: Vec<&str> = text.lines().filter(|l| l.starts_with("Marked: ")).collect();
    assert_eq!(marked.len(), 3, "{text}");
    assert!(!marked.iter().any(|l| l.contains(&format!(
        "{}",
        dir.path().join("dups").join("photos").join("a").display()
    )) && l.starts_with("Marked: crow.png")));
    assert!(text.ends_with("Nothing was removed.\n"));
    assert!(dir.path().join("dups/photos/b/crow.png").exists());
    // -0 with --keep: only the marked paths, NUL-separated.
    let (code, text) = run_in(&here, &["dups", "--keep", "oldest", "-0"]);
    assert_eq!(code, 0);
    let paths: Vec<&str> = text.split('\0').filter(|p| !p.is_empty()).collect();
    assert_eq!(paths.len(), 3, "{text}");
    assert!(!text.contains('\n'));
}

#[test]
fn tsv_and_json_keep_the_real_paths() {
    let (_dir, here) = fixture();
    let (code, text) = run_in(&here, &["dups", "--format", "tsv", "--empty"]);
    assert_eq!(code, 0, "{text}");
    // Three groups with the empty files; one line a file.
    assert_eq!(text.lines().count(), 7, "{text}");
    assert!(text.lines().all(|l| l.split('\t').count() == 3), "{text}");
    let (code, text) = run_in(&here, &["dups", "--format", "json"]);
    assert_eq!(code, 0);
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["group"], 1);
    assert_eq!(first["size"], 16);
    assert_eq!(first["hash"], "BLAKE3");
    assert_eq!(first["files"].as_array().unwrap().len(), 3);
}

#[test]
fn a_folder_that_is_not_there_is_said_and_exits_one() {
    let (_dir, here) = fixture();
    let (code, text) = run_in(&here, &["gone"]);
    assert_eq!(code, 1, "{text}");
    assert_eq!(
        text,
        "Could not find duplicates in gone: it does not exist. Check the path.\n"
    );
}

#[test]
fn a_large_read_nobody_confirms_stops_in_words() {
    // The estimate's question, through the sink, with nobody to ask.
    let (_dir, here) = fixture();
    let clock = FixedClock::new(now(), UtcOffset::UTC);
    let session = session_in(&clock, &here, &NoAsker);
    let sink = ShellSink {
        session: &session,
        asked: Mutex::new(None),
    };
    let estimate = DupEstimate {
        files: 2,
        bytes: 600,
    };
    assert!(!sink.confirm(&estimate));
    assert_eq!(
        sink.asked.lock().unwrap().as_deref(),
        Some("Read 600 bytes to find duplicates? 2 files. No is the default. y or n")
    );
}

#[test]
fn the_help_says_its_meaning_first() {
    for line in help_text().lines().filter(|l| l.starts_with("diro dups")) {
        assert!(
            diroxide_core::format::within_cells(line, "diro dups"),
            "{line}"
        );
    }
    assert!(help_text().contains("Nothing is removed"));
}
