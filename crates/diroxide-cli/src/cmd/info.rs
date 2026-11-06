//! `diro info PATH...`: a file's details as plain lines, the sentence
//! first, then each row name first, for scripts and the Braille display
//! (alpha.7, the owner's answer 121).
//!
//! A route stub from alpha 4's contract step (P4): it says "Not built
//! yet" until the metadata-in-the-app lane (commit 56) builds it
//! here.

use std::io::Write;

use crate::CliError;

/// `diro info`'s arguments, kept as given for the lane that reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfoArgs {
    /// The arguments after `info`.
    pub args: Vec<String>,
}

/// Reads the arguments after `info`.
pub fn parse(args: &[String]) -> Result<InfoArgs, CliError> {
    Ok(InfoArgs {
        args: args.to_vec(),
    })
}

/// Runs `diro info`: not built yet.
pub fn run(args: &InfoArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let _ = (args, out);
    Err(super::not_built("info"))
}
