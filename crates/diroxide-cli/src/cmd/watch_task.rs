//! `diro watch --task NAME [--once] [--dry-run]` and `diro watch
//! --list`: a task's hot folder run by the engine, `--once` the catch-up
//! scan and the queue, then the end, so a scheduler gets catch-up for
//! free; `--dry-run` writes nothing, not even the ledger. `--rule` is
//! another name for `--task`. Exit codes, the family's: 0 all done, 1 the
//! task could not run, 2 wrong use, 3 partly done (alpha.7, R28 6.3;
//! the corrected plan 5.17). `diro watch PATH` without a task stays the
//! say-each-file mode of `watch.rs`.
//!
//! A route stub from alpha 4's contract step (P4): it reads its options
//! and says it is not built yet until the watched-folders lane (commit
//! 69) builds it here.

use std::io::Write;

use crate::CliError;

/// `diro watch --task`'s options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchTaskArgs {
    /// The task's name, its file's stem in the tasks folder.
    pub task: Option<String>,
    /// Catch up, run the queue, then end.
    pub once: bool,
    /// Say what would happen and touch nothing.
    pub dry_run: bool,
    /// List the tasks, one a line, the name first.
    pub list: bool,
}

/// Whether `watch`'s arguments ask for a task rather than the
/// say-each-file mode: the route the command line takes.
pub fn wants_task(args: &[String]) -> bool {
    args.iter().any(|a| {
        let name = a.split_once('=').map_or(a.as_str(), |(n, _)| n);
        matches!(name, "--task" | "--rule" | "--list")
    })
}

/// Reads the arguments after `watch` when they name a task.
pub fn parse(args: &[String]) -> Result<WatchTaskArgs, CliError> {
    let mut parsed = WatchTaskArgs::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--task" | "--rule" => {
                let value = inline
                    .or_else(|| rest.next().cloned())
                    .ok_or_else(|| CliError::Missing(name.to_owned()))?;
                if parsed.task.replace(value).is_some() {
                    return Err(CliError::TooMany(name.to_owned()));
                }
            }
            "--once" => parsed.once = true,
            "--dry-run" => parsed.dry_run = true,
            "--list" => parsed.list = true,
            option if option.starts_with('-') => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            other => return Err(CliError::TooMany(format!("watch {other}"))),
        }
    }
    if parsed.task.is_none() && !parsed.list {
        return Err(CliError::Missing("watch --task".to_owned()));
    }
    Ok(parsed)
}

/// Runs `diro watch --task`: not built yet.
pub fn run(args: &WatchTaskArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let _ = (args, out);
    Err(super::not_built("watch --task"))
}
