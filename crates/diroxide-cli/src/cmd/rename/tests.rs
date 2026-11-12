//! `diro rename`'s tests, on made folders under the target folder's
//! temporary folder only; nothing asks a terminal.

use std::fs;
use std::path::Path;

use diroxide_core::format::{BRAILLE_CELLS, within_cells};
use diroxide_core::{FixedClock, UtcOffset};

use super::*;
use crate::cmd::archive::tests::{args, now, session_in};
use crate::cmd::archive::{Asker, NoAsker, Script};

/// A folder of photos and one note: `IMG_0001.jpg` to `IMG_0003.jpg`,
/// `trip-0003.jpg` already there, `notes.txt`.
fn photos() -> (tempfile::TempDir, Place) {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("photos");
    fs::create_dir(&folder).unwrap();
    for name in [
        "IMG_0001.jpg",
        "IMG_0002.jpg",
        "IMG_0003.jpg",
        "trip-0003.jpg",
        "notes.txt",
    ] {
        fs::write(folder.join(name), name.as_bytes()).unwrap();
    }
    let here = Place::parse(dir.path().to_str().unwrap()).unwrap();
    (dir, here)
}

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn run_in(here: &Place, asker: &dyn Asker, list: &[&str], input: &[u8]) -> (i32, String) {
    let clock = FixedClock::new(now(), UtcOffset::UTC);
    let session = session_in(&clock, here, asker);
    let parsed = parse(&args(list)).unwrap();
    let mut out = Vec::new();
    let code = run_with(&parsed, &mut &input[..], &mut out, &session);
    let text = String::from_utf8(out).unwrap();
    match code {
        Ok(code) => (code, text),
        Err(error) => (-1, format!("{text}{error}")),
    }
}

#[test]
fn arguments_are_read_and_wrong_use_is_refused() {
    let parsed = parse(&args(&[
        "--in",
        "photos",
        "--filter",
        "*.jpg",
        "--find",
        "IMG_",
        "--replace",
        "trip-",
        "-n",
    ]))
    .unwrap();
    assert_eq!(parsed.names, Names::InFolder("photos".into()));
    assert_eq!(parsed.filter.as_deref(), Some("*.jpg"));
    assert!(parsed.dry_run);
    // An empty replacement removes the text.
    let parsed = parse(&args(&["a.txt", "--find", "x", "--replace="])).unwrap();
    assert_eq!(parsed.replace.as_deref(), Some(""));
    let parsed = parse(&args(&[
        "--stdin",
        "-0",
        "--template",
        "{counter}",
        "--start",
        "5",
        "--width",
        "3",
    ]))
    .unwrap();
    assert_eq!(parsed.names, Names::Stdin);
    assert_eq!(
        parsed.counter,
        Some(Counter {
            start: 5,
            step: 1,
            width: 3,
            per_folder: false
        })
    );
    assert_eq!(parse(&args(&[])), Err(CliError::Missing("rename".into())));
    assert_eq!(
        parse(&args(&["a.txt", "--find", "x"])),
        Err(CliError::Missing("--replace".into()))
    );
    assert_eq!(
        parse(&args(&["a.txt", "--bogus"])),
        Err(CliError::UnknownOption("--bogus".into()))
    );
    let refused = [
        &["a.txt"][..],
        &["a.txt", "--in", "f", "--case", "lower"],
        &["a.txt", "--filter", "*.txt", "--case", "lower"],
        &["a.txt", "--null", "--case", "lower"],
        &["a.txt", "--find", "a", "--regex", "b", "--replace", "c"],
        &["a.txt", "--start", "2", "--case", "lower"],
        &["a.txt", "--case", "shouty"],
        &["a.txt", "--preset", "x", "--case", "lower"],
        &["--from-pairs", "--case", "lower"],
        &["--in", "f", "--filter", "size over", "--case", "lower"],
        &["a.txt", "--template", "{counter}", "--width", "99"],
    ];
    for list in refused {
        assert!(
            matches!(parse(&args(list)), Err(CliError::Failed(_))),
            "{list:?}"
        );
    }
    // Wrong use exits 2.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::main_with(
        vec!["rename".into(), "a.txt".into()],
        &mut std::io::empty(),
        &mut out,
        &mut err,
    );
    assert_eq!(code, 2);
}

#[test]
fn a_dry_run_in_a_folder_prints_the_preview_problems_first() {
    let (dir, here) = photos();
    let (code, text) = run_in(
        &here,
        &NoAsker,
        &[
            "--in",
            "photos",
            "--filter",
            "*.jpg",
            "--find",
            "IMG_",
            "--replace",
            "trip-",
            "--dry-run",
        ],
        b"",
    );
    let folder = dir.path().join("photos");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(code, 1, "{text}");
    assert_eq!(lines[0], format!("Rename preview in {}.", folder.display()));
    assert_eq!(lines[1], "1 problem. 2 of 4 files will change.");
    assert_eq!(
        lines[2],
        "Problem, name taken: IMG_0003.jpg becomes trip-0003.jpg"
    );
    assert_eq!(lines[3], "IMG_0001.jpg becomes trip-0001.jpg");
    assert_eq!(lines[4], "IMG_0002.jpg becomes trip-0002.jpg");
    assert_eq!(lines[5], "Same name: trip-0003.jpg");
    assert_eq!(lines.last(), Some(&NOTHING_RENAMED));
    assert_eq!(lines.len(), 7);
    // Nothing moved.
    assert!(folder.join("IMG_0001.jpg").exists());
}

#[test]
fn a_clean_dry_run_exits_zero_and_yes_renames_the_batch() {
    let (dir, here) = photos();
    let folder = dir.path().join("photos");
    let rule = [
        "--in",
        "photos",
        "--filter",
        "IMG_000[12]*",
        "--find",
        "IMG_",
        "--replace",
        "trip-",
    ];
    let mut dry: Vec<&str> = rule.to_vec();
    dry.push("--dry-run");
    let (code, text) = run_in(&here, &NoAsker, &dry, b"");
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("2 of 2 files will change. No problems.\n"),
        "{text}"
    );
    // Nobody to ask: the question, then why nothing happened.
    let (code, text) = run_in(&here, &NoAsker, &rule, b"");
    assert_eq!(code, 1, "{text}");
    let question = format!("Rename 2 files in {}? y or n", folder.display());
    assert!(text.contains(&format!("{question}\n")), "{text}");
    assert!(text.ends_with(&format!(
        "Answered no: there is no terminal to ask.\n{ADD_YES}\n"
    )));
    assert!(folder.join("IMG_0001.jpg").exists());
    // A person who says No: the question asked, nothing renamed.
    let no = Script::answering(&[false], &[]);
    let (code, text) = run_in(&here, &no, &rule, b"");
    assert_eq!(code, 1);
    assert_eq!(no.asked(), std::slice::from_ref(&question));
    assert!(text.ends_with("Nothing was renamed.\n"));
    // y renames them.
    let yes = Script::answering(&[true], &[]);
    let (code, text) = run_in(&here, &yes, &rule, b"");
    assert_eq!(code, 0, "{text}");
    assert!(text.ends_with("Renamed 2 files.\n"), "{text}");
    assert_eq!(
        names_in(&folder),
        [
            "IMG_0003.jpg",
            "notes.txt",
            "trip-0001.jpg",
            "trip-0002.jpg",
            "trip-0003.jpg"
        ]
    );
}

#[test]
fn problems_refuse_the_run_whole() {
    let (dir, here) = photos();
    let (code, text) = run_in(
        &here,
        &NoAsker,
        &[
            "--in",
            "photos",
            "--find",
            "IMG_",
            "--replace",
            "trip-",
            "--yes",
        ],
        b"",
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        text.ends_with(
            "Cannot rename: 1 problem. First: name taken, IMG_0003.jpg becomes trip-0003.jpg.\nNothing was renamed.\n"
        ),
        "{text}"
    );
    assert!(dir.path().join("photos").join("IMG_0001.jpg").exists());
}

#[test]
fn paths_on_standard_input_and_a_template_with_a_counter() {
    let (dir, here) = photos();
    let folder = dir.path().join("photos");
    let input = format!(
        "{}\nphotos/IMG_0002.jpg\r\nphotos/gone.jpg\n",
        folder.join("IMG_0001.jpg").display()
    );
    let (code, text) = run_in(
        &here,
        &NoAsker,
        &[
            "--stdin",
            "--template",
            "trip-{counter}",
            "--start",
            "7",
            "--width",
            "2",
            "-n",
        ],
        input.as_bytes(),
    );
    // The path that is not there is said first, and keeps the exit at 1.
    assert_eq!(code, 1, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("Could not rename photos/gone.jpg: "),
        "{text}"
    );
    assert!(lines[0].ends_with("Check the path."), "{text}");
    assert!(
        text.contains("IMG_0001.jpg becomes trip-07.jpg\n"),
        "{text}"
    );
    assert!(
        text.contains("IMG_0002.jpg becomes trip-08.jpg\n"),
        "{text}"
    );
    // Without the missing one, and --yes, the batch runs.
    let input = "photos/IMG_0001.jpg\0photos/IMG_0002.jpg\0";
    let (code, text) = run_in(
        &here,
        &NoAsker,
        &[
            "--stdin",
            "--null",
            "--case",
            "upper",
            "--apply-to",
            "ext",
            "-y",
        ],
        input.as_bytes(),
    );
    assert_eq!(code, 0, "{text}");
    assert!(
        folder.join("IMG_0001.JPG").exists()
            || names_in(&folder).contains(&"IMG_0001.JPG".to_owned())
    );
}

#[test]
fn pairs_swap_two_names_in_one_job() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "was a").unwrap();
    fs::write(dir.path().join("b.txt"), "was b").unwrap();
    let here = Place::parse(dir.path().to_str().unwrap()).unwrap();
    let pairs = b"a.txt\tb.txt\nb.txt\ta.txt\n";
    let (code, text) = run_in(&here, &NoAsker, &["--from-pairs", "--dry-run"], pairs);
    assert_eq!(code, 0, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "2 of 2 files will change. No problems.");
    assert_eq!(lines[2], "Swap with b.txt: a.txt becomes b.txt");
    assert_eq!(lines[3], "Swap with a.txt: b.txt becomes a.txt");
    let (code, text) = run_in(&here, &NoAsker, &["--from-pairs", "--yes"], pairs);
    assert_eq!(code, 0, "{text}");
    assert!(text.ends_with("Renamed 2 files.\n"), "{text}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "was b"
    );
    // A line with no tab is wrong use.
    let (code, text) = run_in(&here, &NoAsker, &["--from-pairs", "-n"], b"a.txt b.txt\n");
    assert_eq!(code, 2);
    assert_eq!(
        text,
        "Line 1 has no tab between the names.\nNothing was renamed.\n"
    );
}

#[test]
fn a_pattern_that_cannot_be_read_exits_two_in_words() {
    let (_dir, here) = photos();
    let (code, text) = run_in(
        &here,
        &NoAsker,
        &["--in", "photos", "--regex", "(", "--replace", "x", "-n"],
        b"",
    );
    assert_eq!(code, 2, "{text}");
    assert!(text.ends_with("\nNothing was renamed.\n"), "{text}");
    assert!(!text.contains("panicked"));
}

#[test]
fn a_recipe_file_renames_in_its_shape() {
    let (dir, here) = photos();
    let recipe = Recipe::of(vec![Rule::case(CaseForm::Upper)]);
    fs::write(dir.path().join("upper.toml"), recipe.to_toml().unwrap()).unwrap();
    let (code, text) = run_in(
        &here,
        &NoAsker,
        &["photos/notes.txt", "--recipe", "upper.toml", "-n"],
        b"",
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("notes.txt becomes NOTES.txt\n"), "{text}");
}

#[test]
fn every_fixed_sentence_says_its_meaning_in_40_cells() {
    for sentence in [NOTHING_RENAMED, ADD_YES, &renamed_line(50)] {
        assert!(sentence.chars().count() <= BRAILLE_CELLS, "{sentence}");
    }
    // 20 and 17 cells, as the owner's check quotes them.
    assert_eq!(NOTHING_RENAMED.chars().count(), 20);
    assert_eq!(renamed_line(50), "Renamed 50 files.");
    assert!(within_cells(
        help_text().lines().next().unwrap(),
        "diro rename: rename many names by a rule"
    ));
    for line in help_text()
        .lines()
        .filter(|l| l.starts_with("diro rename "))
    {
        assert!(within_cells(line, "diro rename "), "{line}");
    }
}
