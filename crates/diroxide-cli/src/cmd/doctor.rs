//! `diro doctor`: what this diro has and finds, one fact a line, the
//! name first: the program, the screen-reader mode, the folders, each
//! tool, each component and each plugin (alpha.7, R28 1.3). Exit code 0,
//! or with `--check` 1 when a tool marked as needed is missing.
//!
//! A route stub from alpha 4's contract step (P4): it says "Not built
//! yet" until the tools registry's lane (commit 20) builds it here.

use std::io::Write;

use crate::CliError;

/// `diro doctor`'s arguments, kept as given for the lane that reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctorArgs {
    /// The arguments after `doctor`.
    pub args: Vec<String>,
}

/// Reads the arguments after `doctor`.
pub fn parse(args: &[String]) -> Result<DoctorArgs, CliError> {
    Ok(DoctorArgs {
        args: args.to_vec(),
    })
}

/// Runs `diro doctor`: not built yet.
pub fn run(args: &DoctorArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let _ = (args, out);
    Err(super::not_built("doctor"))
}
