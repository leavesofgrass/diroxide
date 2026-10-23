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
//!
//! W4b owns this module and its submodules (alpha 2, commits 18 to 28 of
//! R22's order).

pub mod command;
pub mod context;
pub mod directives;
pub mod header;
pub mod interpreter;
mod read;
pub mod recipe;
pub mod runner;
pub mod words;

use diroxide_core::Place;

use crate::error::PluginError;

pub use directives::{Directive, parse_directive};
pub use header::{HeaderReport, ScriptHeader, parse_header};
pub use recipe::{CwdChoice, ListForm, Per, Recipe, RunMode, StdinForm};
pub use runner::{Cancel, RunEnv, RunReport, RunSink, run};

/// The context a script command runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContext {
    /// The active panel's place.
    pub active: Place,
    /// The other panel's place, the destination by the orthodox rule.
    pub other: Place,
    /// The entry under the cursor.
    pub cursor: Option<Place>,
    /// The selected entries; the cursor's entry when nothing is selected.
    pub selection: Vec<Place>,
}

/// Expands placeholders in a one-line command's arguments into an argument
/// list. `{file}` is the cursor's entry, `{files}` every selected entry as
/// separate arguments, `{dir}` the active place, `{other}` and `{dest}` the
/// other panel's place. Text around a placeholder is kept, except with
/// `{files}`, which must stand alone.
pub fn expand_placeholders(
    args: &[String],
    context: &ScriptContext,
) -> Result<Vec<String>, PluginError> {
    let mut out = Vec::new();
    for arg in args {
        if arg == "{files}" {
            if context.selection.is_empty() {
                return Err(PluginError::Placeholder("{files}".to_owned()));
            }
            out.extend(context.selection.iter().map(ToString::to_string));
            continue;
        }
        let mut text = arg.clone();
        if text.contains("{file}") {
            let cursor = context
                .cursor
                .as_ref()
                .ok_or_else(|| PluginError::Placeholder("{file}".to_owned()))?;
            text = text.replace("{file}", &cursor.to_string());
        }
        text = text
            .replace("{dir}", &context.active.to_string())
            .replace("{other}", &context.other.to_string())
            .replace("{dest}", &context.other.to_string());
        out.push(text);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;

    fn p(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    fn context() -> ScriptContext {
        ScriptContext {
            active: p("/home/ada"),
            other: p("/backup"),
            cursor: Some(p("/home/ada/a.png")),
            selection: vec![p("/home/ada/a.png"), p("/home/ada/b.png")],
        }
    }

    #[test]
    fn placeholders_expand_into_arguments() {
        let args: Vec<String> = ["-resize", "50%", "{files}", "{dest}/thumbs"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let out = expand_placeholders(&args, &context()).unwrap();
        assert_eq!(
            out,
            vec![
                "-resize",
                "50%",
                "/home/ada/a.png",
                "/home/ada/b.png",
                "/backup/thumbs"
            ]
        );
        let mut empty = context();
        empty.cursor = None;
        let err = expand_placeholders(&["{file}".to_owned()], &empty).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the placeholder {file} has nothing to fill it"
        );
    }
}
