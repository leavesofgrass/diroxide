//! Script commands: no protocol at all. A one-file script in
//! `plugins/scripts/` with a header comment becomes a command:
//!
//! ```text
//! #!/usr/bin/env bash
//! # diro: name = "Total size of selection"
//! # diro: selection_file = "nul"
//! ```
//!
//! A manifest-only `[[command]]` with an `argv` and no `[run]` program is
//! the smallest script command, and runs the same way.
//!
//! - [`header`] reads the `diro:` lines (R3 section 5.1, completed by R22
//!   section 2.6); [`recipe`] holds the run keys a header and a
//!   manifest-only command share, and defines `unattended` once.
//! - [`interpreter`] chooses the program that runs a script on each
//!   system: Git's bash and never WSL's on Windows, `py -3` before any
//!   Python on PATH and never the Store stub, PowerShell with the
//!   per-process policy override the consent text names.
//! - [`command`] makes one runnable shape of a script file and a
//!   manifest-only command; [`context`] checks the selection and writes
//!   the run folder (both selection files, the context JSON, the
//!   directives file, the cancel file, `DIRO_TEMP`) and the `DIRO_*`
//!   environment that names them.
//! - [`directives`] reads what a script asks while it runs; [`runner`]
//!   runs it through the process helper in its mode, follows its
//!   directives, cancels it with the cancel file then the job kill, and
//!   ends with the words of [`words`].
//! - [`placeholders`] fills a manifest-only command's `argv` per
//!   argument (never a shell string), splits a long `{files}` into
//!   batches, and says the argument list in words for `--explain`.
//!
//! W4b owns this module and its submodules (alpha 2, commits 18 to 28 of
//! R22's order).

pub mod command;
pub mod context;
pub mod directives;
pub mod header;
pub mod interpreter;
pub mod placeholders;
mod read;
pub mod recipe;
pub mod runner;
pub mod words;

pub use directives::{Directive, parse_directive};
pub use header::{HeaderReport, ScriptHeader, parse_header};
pub use recipe::{CwdChoice, ListForm, Per, Recipe, RunMode, StdinForm};
pub use runner::{Cancel, RunEnv, RunReport, RunSink, run};
