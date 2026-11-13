//! `diro sync HERE THERE`: compare two folders and synchronize them from
//! the plan, from a shell (alpha 5, lane CX; lane CS's engine).
//!
//! - The first place is "here" and the second "there", said in the first
//!   two lines; every row says here and there, never left and right.
//! - `--mode update` (the default) copies what is missing or newer here
//!   to there; `--mode both` copies each way, the newer wins; `--mode
//!   mirror` makes there the same as here, every removal listed and sent
//!   to the trash.
//! - `--by content` reads files of the same size on both sides; `--by
//!   size` looks at sizes only; `--tolerance SECONDS` is how close two
//!   times count as the same (2 by default).
//! - `--dry-run` prints the plan's lines, problems first, and changes
//!   nothing. Without it the question starts on No, or `--yes` runs it.
//!
//! Nothing is removed or replaced for good from a shell: a plan that
//! would change a place with no trash is refused before anything runs.
//! The run is one job with the engine's checks: every place it writes
//! over or removes is read again first, and one that changed since the
//! compare is skipped with its reason. Ctrl+C stops the compare or the
//! run.
//!
//! Exit codes: 0 done, or a dry run; 1 could not run, refused, a No, or
//! nothing was done; 2 wrong use; 3 partly done.

use std::io::Write;
use std::time::Duration;

use diroxide_core::{JobId, Side};
use diroxide_ops::JobControl;
use diroxide_ops::sync::{
    CompareBy, CompareCriteria, CompareProgress, SyncEnv, SyncMode, SyncPlan, compare_dirs,
    plan_sync, run_sync,
};

use crate::CliError;
use crate::cmd::archive::{
    Relay, Session, Unanswered, failure_lines, line, lines, report_code, with_terminal,
};

/// The last line of a run that changed nothing.
pub const NOTHING_SYNCHRONIZED: &str = "Nothing was synchronized.";

/// The last line when nobody was there to say yes (39 cells).
pub const ADD_YES: &str = "Nothing synchronized. Add --yes to run.";

/// Why a plan that cannot go to the trash is refused (39 cells, then the
/// last line).
pub const NO_TRASH_THERE: &str = "Refused: some changes cannot be undone.";

/// `diro sync`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncArgs {
    /// The place here, as typed.
    pub here: String,
    /// The place there, as typed.
    pub there: String,
    /// The mode.
    pub mode: SyncMode,
    /// What decides "the same".
    pub by: CompareBy,
    /// Two times this close count as the same.
    pub tolerance: Duration,
    /// Print the plan and change nothing.
    pub dry_run: bool,
    /// Run without asking.
    pub yes: bool,
    /// Print the help for `diro sync` and nothing else.
    pub help: bool,
}

impl Default for SyncArgs {
    fn default() -> Self {
        Self {
            here: String::new(),
            there: String::new(),
            mode: SyncMode::UpdateThere,
            by: CompareBy::SizeAndTime,
            tolerance: diroxide_ops::sync::TOLERANCE,
            dry_run: false,
            yes: false,
            help: false,
        }
    }
}

/// The help for `diro sync`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro sync: compare two folders, then copy by the plan.",
        "",
        "Usage:",
        "diro sync HERE THERE       update THERE from HERE: copy what is missing or newer",
        "",
        "Options:",
        "--mode MODE                update (the default), both (newer wins) or mirror",
        "                           (also remove there what is not here, to the trash)",
        "--by WHAT                  time (size and time, the default), size or content",
        "--tolerance SECONDS        times this close count as the same; 2 by default",
        "--dry-run, -n              print the plan and change nothing",
        "--yes, -y                  run without asking",
        "",
        "Nothing is removed or replaced for good: a place with no trash is refused.",
        "",
        "Exit codes:",
        "0  done, or a dry run",
        "1  could not run, refused, answered No, or nothing done",
        "2  wrong use",
        "3  partly done: the output says what was not",
    ]
    .join("\n")
}

/// Reads the arguments after `sync`.
pub fn parse(args: &[String]) -> Result<SyncArgs, CliError> {
    let mut parsed = SyncArgs::default();
    let mut places = Vec::new();
    let mut only_places = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_places {
            places.push(arg.clone());
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
            "--" => only_places = true,
            "--help" | "-h" => parsed.help = true,
            "--mode" => {
                let text = value("--mode")?;
                parsed.mode = match text.to_lowercase().as_str() {
                    "update" => SyncMode::UpdateThere,
                    "both" => SyncMode::BothWays,
                    "mirror" => SyncMode::MirrorThere,
                    _ => {
                        return Err(CliError::Failed(format!(
                            "there is no mode {text}; use update, both or mirror"
                        )));
                    }
                };
            }
            "--by" => {
                let text = value("--by")?;
                parsed.by = match text.to_lowercase().as_str() {
                    "time" | "size-and-time" => CompareBy::SizeAndTime,
                    "size" => CompareBy::Size,
                    "content" | "contents" => CompareBy::Content,
                    _ => {
                        return Err(CliError::Failed(format!(
                            "there is no compare by {text}; use time, size or content"
                        )));
                    }
                };
            }
            "--tolerance" => {
                let text = value("--tolerance")?;
                parsed.tolerance = match text.parse::<f64>() {
                    Ok(seconds) if (0.0..=86_400.0).contains(&seconds) => {
                        Duration::from_secs_f64(seconds)
                    }
                    _ => {
                        return Err(CliError::Failed(format!(
                            "the tolerance {text} is not a number of seconds from 0 to 86400"
                        )));
                    }
                };
            }
            "--dry-run" | "-n" => parsed.dry_run = true,
            "--yes" | "-y" => parsed.yes = true,
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            place => places.push(place.to_owned()),
        }
    }
    if parsed.help {
        return Ok(parsed);
    }
    match <[String; 2]>::try_from(places) {
        Ok([here, there]) => {
            parsed.here = here;
            parsed.there = there;
            Ok(parsed)
        }
        Err(places) if places.len() > 2 => Err(CliError::TooMany("sync".to_owned())),
        Err(_) => Err(CliError::Missing("sync".to_owned())),
    }
}

/// Runs `diro sync` on this computer's folders and the terminal; Ctrl+C
/// stops it.
pub fn run(args: &SyncArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    let control = JobControl::new();
    let cancel = control.cancel.clone();
    let _ = ctrlc::set_handler(move || cancel.cancel());
    with_terminal(|session| run_with(args, out, session, &control))
}

/// The two lines that say which place is which.
pub fn sides_lines(plan: &SyncPlan) -> [String; 2] {
    [
        format!("Here: {}", plan.root(Side::Left)),
        format!("There: {}", plan.root(Side::Right)),
    ]
}

/// "Synchronize 2 items? y or n" (27 cells).
pub fn question(acting: u64) -> String {
    let items = if acting == 1 { "item" } else { "items" };
    format!("Synchronize {acting} {items}? y or n")
}

/// Runs `diro sync` in a session, stopped through `control`.
pub fn run_with(
    args: &SyncArgs,
    out: &mut dyn Write,
    session: &Session<'_>,
    control: &JobControl,
) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    let here = session.resolve("synchronize", &args.here)?;
    let there = session.resolve("synchronize", &args.there)?;
    let criteria = CompareCriteria {
        by: args.by,
        tolerance: args.tolerance,
        ..CompareCriteria::default()
    };
    let progress = session.progress;
    let sink = move |state: &CompareProgress| {
        if progress && (state.done || state.paused) {
            let _ = writeln!(std::io::stderr(), "{}", state.describe());
        }
    };
    let comparison = match compare_dirs(&session.registry, &here, &there, &criteria, control, &sink)
    {
        Ok(comparison) => comparison,
        Err(error) => {
            lines(
                out,
                &[
                    sentence(&error.to_string()),
                    NOTHING_SYNCHRONIZED.to_owned(),
                ],
            )?;
            return Ok(1);
        }
    };
    let plan = plan_sync(&comparison, args.mode);
    lines(out, &sides_lines(&plan))?;
    lines(out, &plan.lines(false))?;
    if args.dry_run {
        line(out, NOTHING_SYNCHRONIZED)?;
        return Ok(0);
    }
    if comparison.cancelled {
        line(out, NOTHING_SYNCHRONIZED)?;
        return Ok(1);
    }
    let counts = plan.counts();
    if counts.for_good > 0 {
        lines(
            out,
            &[NO_TRASH_THERE.to_owned(), NOTHING_SYNCHRONIZED.to_owned()],
        )?;
        return Ok(1);
    }
    if counts.acting() == 0 {
        line(out, NOTHING_SYNCHRONIZED)?;
        return Ok(0);
    }
    if !args.yes && !session.confirm(&question(counts.acting()), out)? {
        let last = if session.asker.present() {
            NOTHING_SYNCHRONIZED
        } else {
            ADD_YES
        };
        line(out, last)?;
        return Ok(1);
    }
    let engine = session.engine();
    let relay = Relay::new();
    let mut unanswered = Unanswered::default();
    let env = SyncEnv {
        engine: &engine,
        job: JobId::new(1),
        control,
        sink: relay.as_ref(),
        hooks_asked: false,
    };
    let result = session.drive(&relay, &mut unanswered, || run_sync(&plan, &env));
    unanswered.write(out)?;
    match result {
        Err(error) => {
            lines(
                out,
                &[
                    sentence(&error.to_string()),
                    NOTHING_SYNCHRONIZED.to_owned(),
                ],
            )?;
            Ok(1)
        }
        Ok(report) => {
            line(out, &finished_line(&report))?;
            let skipped: Vec<String> = report
                .skipped
                .iter()
                .map(|(place, why)| format!("Skipped: {}, {why}", place.title()))
                .collect();
            lines(out, &skipped)?;
            lines(out, &failure_lines(&report))?;
            Ok(report_code(&report))
        }
    }
}

/// "Synchronize finished: 2 done" (28 cells), with what was skipped,
/// failed or cancelled after.
pub fn finished_line(report: &diroxide_ops::JobReport) -> String {
    let head = if report.cancelled {
        "Synchronize cancelled"
    } else {
        "Synchronize finished"
    };
    let mut said = format!("{head}: {} done", report.done.len());
    if !report.skipped.is_empty() {
        said.push_str(&format!(", {} skipped", report.skipped.len()));
    }
    if !report.failed.is_empty() {
        said.push_str(&format!(", {} failed", report.failed.len()));
    }
    said
}

/// A cause made a sentence: capital first, a full stop last.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut said = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    if !said.ends_with('.') {
        said.push('.');
    }
    said
}

#[cfg(test)]
mod tests;
