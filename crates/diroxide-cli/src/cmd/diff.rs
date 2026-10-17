//! `diro diff BEFORE AFTER`: compare two files and say the result in
//! words, in the reading order of research/diffing.md: the verdict line
//! (40 cells, "5 changes between old.txt and report.txt"), the counts,
//! the direction ("From old.txt to report.txt."), what was ignored, the
//! facts about the files as wholes, then one line a change ("Changed line
//! 39, was: total due, now: total owed").
//!
//! - `--brief` prints the verdict line only.
//! - `--format unified` prints the unified view in words after the report
//!   ("Added 52: text"), with `--context N` same lines around each change.
//! - `--format json` prints one JSON object, the real names kept.
//! - The ignore options are git's: case, trailing space, the amount of
//!   space, all space, blank lines; line endings are ignored unless
//!   `--line-endings` is given, and a difference in them is one fact.
//! - A text file over 16 MiB is compared byte by byte and the report says
//!   so; `--head` compares the first 16 MiB as text instead.
//!
//! Everything `diro diff` says goes to standard output in order, its
//! problems included, as `diro ls` does, so a screen reader hears one
//! story. Exit codes: 0 the same, 1 different, 2 wrong use or trouble (a
//! file missing, unreadable or not a file, or one that changed while it
//! was compared).

use std::io::{BufWriter, Write};

use diroxide_core::names::display_name;
use diroxide_core::{Explain, Message, Place};
use diroxide_diff::grammar::DEFAULT_CONTEXT;
use diroxide_diff::{
    Change, ChangeKind, CompareOptions, Context, FileComparison, FileOptions, FileSide, Ignore,
    LargeFiles, Outcome, Sides, Verdict, compare_files,
};
use diroxide_vfs::{OpContext, Registry, VfsError};
use serde_json::{Value, json};

use crate::CliError;
use crate::open::current_place;

/// How `diro diff` writes its result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// The report and one line a change, in words: the default.
    #[default]
    Words,
    /// The report, then the unified view in words.
    Unified,
    /// One JSON object.
    Json,
}

/// What `diro diff` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffArgs {
    /// The before file, as typed.
    pub before: String,
    /// The after file, as typed.
    pub after: String,
    /// Print the verdict line only.
    pub brief: bool,
    /// How to write the result.
    pub format: Format,
    /// What to leave out.
    pub ignore: Ignore,
    /// Same lines around each change in the unified view.
    pub context: usize,
    /// Compare the first 16 MiB of a larger text file as text.
    pub head: bool,
    /// Print the help for `diro diff` and nothing else.
    pub help: bool,
}

impl Default for DiffArgs {
    fn default() -> Self {
        DiffArgs {
            before: String::new(),
            after: String::new(),
            brief: false,
            format: Format::Words,
            ignore: Ignore::default(),
            context: DEFAULT_CONTEXT,
            head: false,
            help: false,
        }
    }
}

/// The help for `diro diff`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro diff: compare two files, the verdict first, then one line a change.",
        "",
        "Usage:",
        "diro diff BEFORE AFTER     compare the BEFORE file with the AFTER file",
        "",
        "Options:",
        "--brief, -q                print the verdict line only",
        "--format FORMAT            words (the default), unified, or json",
        "--context N                same lines around each change when unified, 0 to 9; 2 by default",
        "--ignore-case, -i          ignore upper and lower case",
        "--ignore-trailing-space    ignore space at the end of lines",
        "--ignore-space-change, -b  ignore changes in the amount of space",
        "--ignore-all-space, -w     ignore all space",
        "--ignore-blank-lines, -B   ignore changes whose lines are all blank",
        "--line-endings             compare line endings too; ignored by default",
        "--head                     over 16 MiB, compare the first 16 MiB as text, not byte by byte",
        "",
        "Exit codes:",
        "0  the same",
        "1  different",
        "2  wrong use or trouble: a file missing, unreadable or not a file, or one that changed",
    ]
    .join("\n")
}

/// Reads the arguments after `diff`.
pub fn parse(args: &[String]) -> Result<DiffArgs, CliError> {
    let mut parsed = DiffArgs::default();
    let mut files: Vec<String> = Vec::new();
    let mut only_files = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_files {
            files.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |flag: &str| -> Result<String, CliError> {
            match inline.clone().or_else(|| rest.next().cloned()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(CliError::Missing(flag.to_owned())),
            }
        };
        match name {
            "--" => only_files = true,
            "--help" | "-h" => parsed.help = true,
            "--brief" | "-q" => parsed.brief = true,
            "--format" => {
                parsed.format = match value("--format")?.as_str() {
                    "words" => Format::Words,
                    "unified" => Format::Unified,
                    "json" => Format::Json,
                    other => {
                        return Err(CliError::Failed(format!(
                            "there is no {other} format; use words, unified or json"
                        )));
                    }
                }
            }
            "--context" => {
                let text = value("--context")?;
                parsed.context = match text.parse::<usize>() {
                    Ok(n) if n <= 9 => n,
                    _ => {
                        return Err(CliError::Failed(format!(
                            "the context {text} is not a number from 0 to 9"
                        )));
                    }
                };
            }
            "--ignore-case" | "-i" => parsed.ignore.case = true,
            "--ignore-trailing-space" => parsed.ignore.trailing_space = true,
            "--ignore-space-change" | "-b" => parsed.ignore.space_amount = true,
            "--ignore-all-space" | "-w" => parsed.ignore.all_space = true,
            "--ignore-blank-lines" | "-B" => parsed.ignore.blank_lines = true,
            "--line-endings" => parsed.ignore.line_endings = false,
            "--head" => parsed.head = true,
            "-" => {
                return Err(CliError::Failed(
                    "diro diff compares two files by name; standard input is not one".to_owned(),
                ));
            }
            option if option.starts_with('-') => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            file => files.push(file.to_owned()),
        }
    }
    if parsed.help {
        return Ok(parsed);
    }
    match <[String; 2]>::try_from(files) {
        Ok([before, after]) => {
            parsed.before = before;
            parsed.after = after;
            Ok(parsed)
        }
        Err(files) if files.len() > 2 => Err(CliError::TooMany("diff".to_owned())),
        Err(_) => Err(CliError::Missing("diff".to_owned())),
    }
}

/// Runs `diro diff`. Returns the exit code: 0 the same, 1 different, 2
/// trouble.
pub fn run(args: &DiffArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_failed)?;
        return Ok(0);
    }
    let here = current_place()?;
    let registry = crate::cmd::ls::providers();
    run_with(args, out, &registry, &here)
}

fn write_failed(error: std::io::Error) -> CliError {
    CliError::Failed(format!("the comparison could not be written: {error}"))
}

/// A provider's error as the command line says it: a place that is not
/// there asks for the path to be checked.
struct AtTheShell<'a>(&'a dyn Explain, bool);

impl std::fmt::Display for AtTheShell<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Explain for AtTheShell<'_> {
    fn next_step(&self) -> String {
        if self.1 {
            "Check the path.".to_owned()
        } else {
            self.0.next_step()
        }
    }
}

/// "Could not compare X: CAUSE. NEXT STEP." with hidden characters in X
/// as question marks.
fn could_not(what: &str, error: &dyn Explain, not_found: bool) -> String {
    Message::from_error(
        "compare",
        &display_name(what),
        &AtTheShell(error, not_found),
    )
    .text
}

/// The name a file goes by in the report: its own name, or the path as
/// typed when the two files have the same name.
fn names(before: &Place, after: &Place, args: &DiffArgs) -> (String, String) {
    let name = |place: &Place, typed: &str| {
        place
            .name()
            .map_or_else(|| typed.to_owned(), |name| name.to_string())
    };
    let (b, a) = (name(before, &args.before), name(after, &args.after));
    if b == a && args.before != args.after {
        (args.before.clone(), args.after.clone())
    } else {
        (b, a)
    }
}

/// [`run`] with the providers and the current folder given.
pub fn run_with(
    args: &DiffArgs,
    out: &mut dyn Write,
    registry: &Registry,
    here: &Place,
) -> Result<i32, CliError> {
    let mut out = BufWriter::new(out);
    let code = compare_and_say(args, &mut out, registry, here)?;
    out.flush().map_err(write_failed)?;
    Ok(code)
}

fn compare_and_say(
    args: &DiffArgs,
    out: &mut dyn Write,
    registry: &Registry,
    here: &Place,
) -> Result<i32, CliError> {
    let mut say = |line: &str| writeln!(out, "{line}").map_err(write_failed);
    let ctx = OpContext::new();
    let mut sides = Vec::new();
    for typed in [&args.before, &args.after] {
        let place = match Place::resolve(typed, here) {
            Ok(place) => place,
            Err(error) => {
                say(&could_not(typed, &error, false))?;
                return Ok(2);
            }
        };
        let provider = match registry.resolve(&place) {
            Ok(provider) => provider,
            Err(error) => {
                say(&could_not(typed, &error, false))?;
                return Ok(2);
            }
        };
        // Look first, so a problem names its own file.
        if let Err(error) = provider.stat(&place, &ctx) {
            say(&could_not(typed, &error, error == VfsError::NotFound))?;
            return Ok(2);
        }
        sides.push((place, provider));
    }
    let [(before, before_provider), (after, after_provider)] = <[_; 2]>::try_from(sides)
        .map_err(|_| CliError::Failed("two files were not found".to_owned()))?;
    let options = FileOptions {
        compare: CompareOptions {
            ignore: args.ignore,
            ..CompareOptions::default()
        },
        large: if args.head {
            LargeFiles::Head
        } else {
            LargeFiles::Bytes
        },
        ..FileOptions::default()
    };
    let result = compare_files(
        FileSide {
            provider: before_provider.as_ref(),
            place: &before,
        },
        FileSide {
            provider: after_provider.as_ref(),
            place: &after,
        },
        &options,
        &ctx,
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let what = match &error {
                diroxide_diff::DiffError::NotAFile(place) if *place == after.to_string() => {
                    &args.after
                }
                _ => &args.before,
            };
            say(&could_not(what, &error, false))?;
            return Ok(2);
        }
    };
    let (before_name, after_name) = names(&before, &after, args);
    let sides = Sides::files(&before_name, &after_name);
    let verdict = result.verdict();
    let code = if verdict.same() {
        0
    } else if verdict.differs() {
        1
    } else {
        2
    };
    if args.format == Format::Json {
        let value = as_json(&result, &before_name, &after_name, &sides, &options);
        say(&value.to_string())?;
        return Ok(code);
    }
    let summary = result.summary(&before_name, &after_name, &sides, &options);
    if args.brief {
        say(&summary[0])?;
        return Ok(code);
    }
    for line in &summary {
        say(line)?;
    }
    match (&result.outcome, args.format) {
        (Outcome::Text { content, .. }, Format::Unified) => {
            for row in content.unified(Context::Lines(args.context)) {
                say(&row.spoken())?;
            }
        }
        _ => {
            for row in result.rows(&options) {
                say(&row)?;
            }
        }
    }
    Ok(code)
}

/// The verdict as one lowercase word for scripts.
fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Same => "same",
        Verdict::SameText => "same_text",
        Verdict::SameFile => "same_file",
        Verdict::Changes(_) => "changes",
        Verdict::BinaryBytes(_) => "binary_bytes",
        Verdict::BinarySizes => "binary_sizes",
        Verdict::TooLarge => "too_large",
        Verdict::NotCompared => "not_compared",
        Verdict::ChangedWhileComparing => "changed_while_comparing",
    }
}

fn lines_json(range: &std::ops::Range<usize>) -> Value {
    json!({ "first": range.start + 1, "count": range.len() })
}

fn change_json(change: &Change, row: String) -> Value {
    let kind = match change.kind {
        ChangeKind::Changed => "changed",
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
    };
    json!({
        "kind": kind,
        "before": lines_json(&change.before),
        "after": lines_json(&change.after),
        "row": row,
    })
}

/// The result as one JSON object: the verdict, the report's sentences,
/// the counts, and the changes or the byte runs, with the names as given.
fn as_json(
    result: &FileComparison,
    before_name: &str,
    after_name: &str,
    sides: &Sides,
    options: &FileOptions,
) -> Value {
    let verdict = result.verdict();
    let summary = result.summary(before_name, after_name, sides, options);
    let mut value = json!({
        "verdict": verdict_word(verdict),
        "same": verdict.same(),
        "before": before_name,
        "after": after_name,
        "summary": summary,
    });
    match &result.outcome {
        Outcome::Text { content, head } => {
            let counts = content.text().counts();
            value["counts"] = json!({
                "changes": counts.changes,
                "changed": counts.changed,
                "added": counts.added,
                "removed": counts.removed,
                "lines_removed": counts.lines_removed,
                "lines_added": counts.lines_added,
            });
            value["changes"] = content
                .text()
                .changes()
                .iter()
                .enumerate()
                .map(|(index, change)| change_json(change, content.row(index, &options.compare)))
                .collect();
            value["head_bytes"] = json!(head);
        }
        Outcome::Binary { binary, .. } => {
            value["sizes"] = json!({ "before": binary.before_size, "after": binary.after_size });
            value["differing_bytes"] = json!(binary.differing_bytes);
            value["run_count"] = json!(binary.run_count);
            value["runs"] = binary
                .runs
                .iter()
                .map(|run| json!({ "start": run.start, "length": run.len, "row": run.row() }))
                .collect();
        }
        _ => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn fixtures() -> Place {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diff");
        let path = std::fs::canonicalize(path).unwrap();
        let text = path
            .to_str()
            .unwrap()
            .trim_start_matches(r"\\?\")
            .to_owned();
        Place::parse(&text).unwrap()
    }

    fn diff(list: &[&str]) -> (i32, String) {
        let parsed = parse(&args(list)).unwrap();
        let mut out = Vec::new();
        let code = run_with(&parsed, &mut out, &Registry::with_local(), &fixtures()).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn arguments_are_read() {
        let parsed = parse(&args(&[
            "a.txt",
            "b.txt",
            "-w",
            "--format=json",
            "--context",
            "3",
        ]))
        .unwrap();
        assert_eq!(parsed.before, "a.txt");
        assert!(parsed.ignore.all_space && parsed.ignore.line_endings);
        assert_eq!((parsed.format, parsed.context), (Format::Json, 3));
        assert!(
            !parse(&args(&["a", "b", "--line-endings"]))
                .unwrap()
                .ignore
                .line_endings
        );
        assert_eq!(parse(&args(&["a"])), Err(CliError::Missing("diff".into())));
        assert_eq!(
            parse(&args(&["a", "b", "c"])),
            Err(CliError::TooMany("diff".into()))
        );
        assert_eq!(
            parse(&args(&["a", "b", "--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert!(matches!(
            parse(&args(&["a", "b", "--context", "12"])),
            Err(CliError::Failed(_))
        ));
        assert!(matches!(
            parse(&args(&["a", "b", "--format", "side-by-side"])),
            Err(CliError::Failed(_))
        ));
        assert!(parse(&args(&["--help"])).unwrap().help);
        assert_eq!(
            parse(&args(&["--", "-a", "b"])).unwrap().before,
            "-a".to_owned()
        );
    }

    #[test]
    fn two_versions_read_in_order_and_exit_with_one() {
        let (code, text) = diff(&["report/old.txt", "report/report.txt"]);
        assert_eq!(code, 1);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "5 changes between old.txt and report.txt",
                "Changed 3, added 1, removed 1.",
                "From old.txt to report.txt.",
                "Removed line 12, Copyright 2025",
                "Changed line 19, was: ...in Portland opened early and the crows..., now: ...in Portland opened late and the crows...",
                "Changed line 29, trailing spaces added",
                "Changed line 39, was: total due, now: total owed",
                "Added lines 51 to 53, 3 lines",
            ]
        );
    }

    #[test]
    fn brief_says_the_verdict_alone() {
        let (code, text) = diff(&["--brief", "report/old.txt", "report/report.txt"]);
        assert_eq!(
            (code, text.as_str()),
            (1, "5 changes between old.txt and report.txt\n")
        );
    }

    #[test]
    fn the_same_exits_with_zero() {
        let (code, text) = diff(&["report/old.txt", "report/old.txt"]);
        assert_eq!((code, text.as_str()), (0, "Same file: old.txt\n"));
        let (code, text) = diff(&[
            "sounds-same/line-endings/before.txt",
            "sounds-same/line-endings/after.txt",
        ]);
        assert_eq!(code, 0);
        assert_eq!(
            text,
            "Same text: before.txt and after.txt\nLine endings differ: LF in before.txt, CRLF in after.txt. Compared ignoring them.\n"
        );
    }

    #[test]
    fn trouble_exits_with_two_and_says_which_file() {
        let (code, text) = diff(&["report/old.txt", "report/missing.txt"]);
        assert_eq!(code, 2);
        assert_eq!(
            text,
            "Could not compare report/missing.txt: it does not exist. Check the path.\n"
        );
        let (code, text) = diff(&["report/old.txt", "report"]);
        assert_eq!(code, 2);
        assert_eq!(
            text,
            "Could not compare report: it is not a file. Choose two files to compare.\n"
        );
    }

    #[test]
    fn unified_and_json_formats() {
        let (code, text) = diff(&[
            "--format",
            "unified",
            "--context",
            "0",
            "report/old.txt",
            "report/report.txt",
        ]);
        assert_eq!(code, 1);
        assert!(text.contains("Change 4 of 5, line 40 before, line 39 after\nRemoved 40: total due\nAdded 39: total owed\n"));
        assert!(!text.contains("Same "));
        let (code, text) = diff(&["--format", "json", "report/old.txt", "report/report.txt"]);
        assert_eq!(code, 1);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["verdict"], "changes");
        assert_eq!(value["counts"]["changes"], 5);
        assert_eq!(value["changes"][3]["kind"], "changed");
        assert_eq!(value["changes"][3]["after"]["first"], 39);
        assert_eq!(
            value["summary"][0],
            "5 changes between old.txt and report.txt"
        );
    }

    #[test]
    fn ignore_options_are_said() {
        let (code, text) = diff(&[
            "--ignore-trailing-space",
            "report/old.txt",
            "report/report.txt",
        ]);
        assert_eq!(code, 1);
        assert!(text.contains("Ignoring changes in trailing spaces.\n"));
        let (code, _) = diff(&[
            "-i",
            "sounds-same/case/before.txt",
            "sounds-same/case/after.txt",
        ]);
        assert_eq!(code, 0);
    }

    #[test]
    fn binary_files_are_compared_by_bytes() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("a.bin"), b"\x00\x01\x02\x03").unwrap();
        std::fs::write(folder.path().join("b.bin"), b"\x00\x01\x09\x03").unwrap();
        let here = Place::parse(folder.path().to_str().unwrap()).unwrap();
        let parsed = parse(&args(&["a.bin", "b.bin"])).unwrap();
        let mut out = Vec::new();
        let code = run_with(&parsed, &mut out, &Registry::with_local(), &here).unwrap();
        assert_eq!(code, 1);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Binary, 1 byte differs: a.bin and b.bin\nBoth 4 bytes; 1 byte differs in 1 run.\nFirst difference at byte 2.\nByte 2 differs\n"
        );
    }

    #[test]
    fn the_help_says_the_command_first() {
        let help = help_text();
        assert!(diroxide_core::format::within_cells(
            help.lines().next().unwrap(),
            "diro diff: compare two files"
        ));
        assert!(help.contains("Exit codes:"));
    }
}
