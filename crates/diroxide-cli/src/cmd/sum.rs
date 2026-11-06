//! `diro sum FILE...` and `diro sum --check LIST`: checksums computed and
//! checked, the verdict first; with `--check`, exit code 0 when all
//! match, 1 for any difference, missing or unreadable file, 2 for wrong
//! use, as `sha256sum --check` does (alpha.7, the owner's answer 121).
//!
//! A route stub from alpha 4's contract step (P4): it says "Not built
//! yet" until the metadata-in-the-app lane (commit 56) builds it
//! here.

use std::io::Write;

use crate::CliError;

/// `diro sum`'s arguments, kept as given for the lane that reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SumArgs {
    /// The arguments after `sum`.
    pub args: Vec<String>,
}

/// Reads the arguments after `sum`.
pub fn parse(args: &[String]) -> Result<SumArgs, CliError> {
    Ok(SumArgs {
        args: args.to_vec(),
    })
}

/// Runs `diro sum`: not built yet.
pub fn run(args: &SumArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let _ = (args, out);
    Err(super::not_built("sum"))
}
