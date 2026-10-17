//! `diro join PART`: put a split file back together from any of its
//! parts, as the app's Join files does, with the same plan and the same
//! sentences (W2g, plan commit 56).
//!
//! The set is found from the part's name and number; the plan names any
//! gap or odd size before anything is written ("Part 004 is missing.
//! Nothing was joined."), then lists the parts in order with their sizes.
//! The whole goes to the current folder, or `--into FOLDER`, and is
//! checked against the `.crc` sidecar when there is one; the verdict is
//! said in words ("Joined report.pdf: checksum matches."). After a
//! verified join, in a terminal, it offers to move the parts to the
//! trash, starting on No.

use std::io::Write;

use diroxide_core::{ConflictPolicy, JobId, JobKind, JobRequest};
use diroxide_ops::{JobControl, Preflight, Verdict};

use crate::CliError;
use crate::cmd::archive::{
    Relay, Session, Unanswered, failure_lines, line, lines, report_code, with_terminal,
};
use crate::cmd::split::{ask_preflight, name_of, ops_failed};

/// What `diro join` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinArgs {
    /// Join a set from one of its parts.
    Join {
        /// Any part of the set.
        part: String,
        /// Where the whole goes; the current folder by default.
        into: Option<String>,
        /// Replace a file of the same name (the old one to the trash).
        replace: bool,
    },
    /// Print the help.
    Help,
}

/// The help for `diro join`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro join: put a split file back together from any of its parts.",
        "",
        "Usage:",
        "diro join PART              join PART's set into the current folder",
        "",
        "Options:",
        "--into FOLDER      write the whole there instead",
        "--replace          replace a file of the same name; the old goes to the trash",
        "",
        "The parts are found by name and number (report.pdf.001, .002 and on, or",
        "from .000), every gap is named before anything is written, and the whole",
        "is checked against the .crc checksum file when there is one.",
        "",
        "Exit codes:",
        "0  joined, and the checksum matches or there is none",
        "1  nothing joined, or the checksum differs",
        "2  wrong use, such as an unknown option",
        "3  partly joined: something failed; the output says what",
    ]
    .join("\n")
}

/// Reads the arguments after `join`.
pub fn parse(args: &[String]) -> Result<JoinArgs, CliError> {
    let mut part = None;
    let mut into = None;
    let mut replace = false;
    let mut only_positionals = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_positionals || arg == "-" || !arg.starts_with('-') {
            if part.replace(arg.clone()).is_some() {
                return Err(CliError::TooMany("join".to_owned()));
            }
            continue;
        }
        let (name, given) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--" => only_positionals = true,
            "--help" | "-h" => return Ok(JoinArgs::Help),
            "--replace" => replace = true,
            "--into" => match given.or_else(|| rest.next().cloned()) {
                Some(value) if !value.is_empty() => into = Some(value),
                _ => return Err(CliError::Missing("--into".to_owned())),
            },
            other => return Err(CliError::UnknownOption(other.to_owned())),
        }
    }
    let part = part.ok_or_else(|| CliError::Missing("join".to_owned()))?;
    Ok(JoinArgs::Join {
        part,
        into,
        replace,
    })
}

/// Runs `diro join`, in the terminal, over the real places.
pub fn run(args: &JoinArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if *args == JoinArgs::Help {
        line(out, &help_text())?;
        return Ok(0);
    }
    with_terminal(|session| run_with(args, out, session))
}

/// [`run`] in a session the caller gives.
pub fn run_with(
    args: &JoinArgs,
    out: &mut dyn Write,
    session: &Session<'_>,
) -> Result<i32, CliError> {
    let JoinArgs::Join {
        part,
        into,
        replace,
    } = args
    else {
        line(out, &help_text())?;
        return Ok(0);
    };
    let first = session.resolve("join", part)?;
    let name = name_of(&first);
    let destination = match into {
        Some(text) => session.resolve("join into", text)?,
        None => session.here.clone(),
    };
    let mut request = JobRequest::new(JobId::new(1), JobKind::Join, vec![first]).to(destination);
    request.conflicts = if *replace {
        ConflictPolicy::Overwrite
    } else {
        ConflictPolicy::KeepBoth
    };
    let engine = session.engine();
    let plan_of = |request: &JobRequest| {
        engine
            .plan(request)
            .map_err(|e| CliError::Failed(ops_failed("join", &name, &e)))
    };
    let mut plan = plan_of(&request)?;
    // Two sets could be meant: Yes plans the join again from the other.
    let which = plan.facts.questions.iter().find_map(|q| match q {
        Preflight::WhichParts { question, yes } => Some((question.clone(), yes.clone())),
        _ => None,
    });
    if let Some((question, yes)) = which
        && session.confirm(&question, out)?
    {
        request.sources = yes;
        plan = plan_of(&request)?;
    }
    if let Some(facts) = &plan.facts.join {
        lines(out, &facts.rows())?;
    }
    let others: Vec<Preflight> = plan
        .facts
        .questions
        .iter()
        .filter(|q| !matches!(q, Preflight::WhichParts { .. }))
        .cloned()
        .collect();
    if !ask_preflight(&others, out, session)? {
        line(out, "Nothing was joined.")?;
        return Ok(1);
    }
    let control = JobControl::new();
    let relay = Relay::new();
    let mut unanswered = Unanswered::default();
    let report = session.drive(&relay, &mut unanswered, || {
        engine.run(&plan, &control, relay.as_ref())
    });
    unanswered.write(out)?;
    line(out, &report.summary())?;
    lines(out, &failure_lines(&report))?;
    let mut code = report_code(&report);
    if let Some(joined) = &report.joined {
        line(out, &joined.sentence())?;
        if joined.verdict == Verdict::Differs {
            code = 1;
        }
        if let Some(offer) = joined.trash_offer()
            && session.asker.present()
            && session.asker.confirm(&offer.text)
        {
            let trash = JobRequest::new(JobId::new(2), JobKind::Trash, joined.parts.clone());
            match engine.plan(&trash) {
                Ok(trash_plan) => {
                    let mut unanswered = Unanswered::default();
                    let trashed = session.drive(&relay, &mut unanswered, || {
                        engine.run(&trash_plan, &control, relay.as_ref())
                    });
                    line(out, &trashed.summary())?;
                    lines(out, &failure_lines(&trashed))?;
                }
                Err(error) => line(out, &ops_failed("trash", &name, &error))?,
            }
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::archive::NoAsker;
    use crate::cmd::archive::tests::{args, fixture, now, session_in};
    use crate::cmd::split::tests::{data_file, split_in};
    use diroxide_core::{FixedClock, UtcOffset};
    use std::fs;

    fn join_in(fixture: &crate::cmd::archive::tests::Fixture, list: &[&str]) -> (i32, String) {
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let session = session_in(&clock, &fixture.here, &NoAsker);
        let mut out = Vec::new();
        let code = run_with(&parse(&args(list)).unwrap(), &mut out, &session);
        let text = String::from_utf8(out).unwrap();
        match code {
            Ok(code) => (code, text),
            Err(error) => (-1, format!("{text}{error}")),
        }
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(
            parse(&args(&["big.bin.002", "--into=out"])),
            Ok(JoinArgs::Join {
                part: "big.bin.002".into(),
                into: Some("out".into()),
                replace: false,
            })
        );
        assert_eq!(parse(&args(&["--help"])), Ok(JoinArgs::Help));
        assert_eq!(parse(&args(&[])), Err(CliError::Missing("join".into())));
        assert_eq!(
            parse(&args(&["a.001", "b.001"])),
            Err(CliError::TooMany("join".into()))
        );
        assert_eq!(
            parse(&args(&["a.001", "--into"])),
            Err(CliError::Missing("--into".into()))
        );
        assert_eq!(
            parse(&args(&["a.001", "--all"])),
            Err(CliError::UnknownOption("--all".into()))
        );
    }

    #[test]
    fn a_split_then_a_join_gives_the_same_bytes_and_says_the_verdict() {
        let fixture = fixture();
        let bytes = data_file(&fixture, "data.bin", 50_000);
        fs::create_dir(fixture.path("parts")).unwrap();
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            diroxide_config::builtin_split_presets(),
            &["data.bin", "--size", "20KB", "--into", "../parts"],
        );
        assert_eq!(code, 0, "{text}");
        fs::create_dir(fixture.path("whole")).unwrap();
        let (code, text) = join_in(&fixture, &["../parts/data.bin.002", "--into", "../whole"]);
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Join 3 parts into data.bin, 50 KB");
        assert_eq!(lines[1], "Checked against data.bin.crc");
        assert!(lines[2].starts_with("Enough space: "), "{text}");
        assert_eq!(
            &lines[3..],
            [
                "Part 001, 20 KB",
                "Part 002, 20 KB",
                "Part 003, 10 KB",
                "Join finished: 3 done.",
                "Joined data.bin: checksum matches.",
            ]
        );
        assert_eq!(
            fs::read(fixture.path("whole").join("data.bin")).unwrap(),
            bytes
        );
    }

    #[test]
    fn a_gap_is_named_and_nothing_is_joined() {
        let fixture = fixture();
        data_file(&fixture, "data.bin", 50_000);
        fs::create_dir(fixture.path("parts")).unwrap();
        let (code, _) = split_in(
            &fixture,
            &NoAsker,
            diroxide_config::builtin_split_presets(),
            &["data.bin", "--size", "10KB", "--into", "../parts"],
        );
        assert_eq!(code, 0);
        let parts = fixture.path("parts");
        fs::rename(parts.join("data.bin.003"), parts.join("aside.bin")).unwrap();
        fs::create_dir(fixture.path("whole")).unwrap();
        let (code, text) = join_in(&fixture, &["../parts/data.bin.001", "--into", "../whole"]);
        assert_eq!(code, -1);
        assert_eq!(text, "Part 003 is missing. Nothing was joined.");
        assert_eq!(fs::read_dir(fixture.path("whole")).unwrap().count(), 0);
        fs::rename(parts.join("aside.bin"), parts.join("data.bin.003")).unwrap();
        // Without the checksum file the join is not checked, and says so.
        fs::rename(parts.join("data.bin.crc"), parts.join("aside.crc.txt")).unwrap();
        let (code, text) = join_in(&fixture, &["../parts/data.bin.004", "--into", "../whole"]);
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains("No checksum file: the join is not checked.\n"),
            "{text}"
        );
        assert!(
            text.ends_with("Joined data.bin. No checksum file found.\n"),
            "{text}"
        );
    }

    #[test]
    fn the_help_says_its_meaning_first() {
        for line in help_text().lines() {
            assert!(line.chars().count() <= 80, "{line}");
        }
    }
}
