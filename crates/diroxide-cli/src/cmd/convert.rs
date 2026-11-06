//! `diro convert FILE... --to FORMAT`: conversions with recipes, carta's
//! writers first, one line per result, the name first, the summary last;
//! exit codes 0 all done, 1 nothing could be done, 2 wrong use, 3 partly
//! done (alpha.7, R28 5.3).
//!
//! A route stub from alpha 4's contract step (P4): it says "Not built
//! yet" until the conversions lane (commit 63) builds it here.

use std::io::Write;

use crate::CliError;

/// `diro convert`'s arguments, kept as given for the lane that reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConvertArgs {
    /// The arguments after `convert`.
    pub args: Vec<String>,
}

/// Reads the arguments after `convert`.
pub fn parse(args: &[String]) -> Result<ConvertArgs, CliError> {
    Ok(ConvertArgs {
        args: args.to_vec(),
    })
}

/// Runs `diro convert`: not built yet.
pub fn run(args: &ConvertArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let _ = (args, out);
    Err(super::not_built("convert"))
}
