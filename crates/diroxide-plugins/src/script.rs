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
//!
//! W4b owns this module and its submodules (alpha 2, commits 18 to 28 of
//! R22's order).

pub mod header;
pub mod interpreter;
mod read;
pub mod recipe;

use std::path::Path;

use diroxide_core::Place;

use crate::error::PluginError;

pub use header::{HeaderReport, ScriptHeader, parse_header};
pub use recipe::{CwdChoice, ListForm, Per, Recipe, RunMode, StdinForm};

/// A directive a script prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Directive {
    /// `diro:announce TEXT`: say something.
    Announce(String),
    /// `diro:refresh`: list the panels again.
    Refresh,
    /// `diro:cd PATH`: open a place in the active panel.
    Cd(String),
    /// `diro:select PATH`: select an entry.
    Select(String),
    /// `diro:progress N of M`: report progress.
    Progress {
        /// Done.
        done: u64,
        /// In all.
        total: u64,
    },
}

/// Reads one output line as a directive; `None` for ordinary output and
/// for `diro:` lines that are not understood.
pub fn parse_directive(line: &str) -> Option<Directive> {
    let rest = line.trim_end_matches(['\r', '\n']).strip_prefix("diro:")?;
    let (word, argument) = match rest.split_once(' ') {
        Some((word, argument)) => (word, argument.trim()),
        None => (rest.trim(), ""),
    };
    match word {
        "announce" if !argument.is_empty() => Some(Directive::Announce(argument.to_owned())),
        "refresh" => Some(Directive::Refresh),
        "cd" if !argument.is_empty() => Some(Directive::Cd(argument.to_owned())),
        "select" if !argument.is_empty() => Some(Directive::Select(argument.to_owned())),
        "progress" => {
            let (done, total) = argument.split_once(" of ")?;
            Some(Directive::Progress {
                done: done.trim().parse().ok()?,
                total: total.trim().parse().ok()?,
            })
        }
        _ => None,
    }
}

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

/// The environment variables a script gets. `selection_file` names a file
/// with one selected path per line, which the host writes.
pub fn environment(context: &ScriptContext, selection_file: &Path) -> Vec<(String, String)> {
    let mut vars = vec![
        ("DIRO_ACTIVE".to_owned(), context.active.to_string()),
        ("DIRO_OTHER".to_owned(), context.other.to_string()),
        (
            "DIRO_SELECTION_FILE".to_owned(),
            selection_file.display().to_string(),
        ),
    ];
    if let Some(cursor) = &context.cursor {
        vars.push(("DIRO_CURSOR".to_owned(), cursor.to_string()));
    }
    vars
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
    fn directives_are_recognized() {
        assert_eq!(
            parse_directive("diro:announce Done, 3 made"),
            Some(Directive::Announce("Done, 3 made".into()))
        );
        assert_eq!(parse_directive("diro:refresh"), Some(Directive::Refresh));
        assert_eq!(
            parse_directive("diro:cd /tmp/x y"),
            Some(Directive::Cd("/tmp/x y".into()))
        );
        assert_eq!(
            parse_directive("diro:progress 3 of 10"),
            Some(Directive::Progress { done: 3, total: 10 })
        );
        assert_eq!(parse_directive("diro:progress three"), None);
        assert_eq!(parse_directive("plain output"), None);
        assert_eq!(parse_directive("diro:dance"), None);
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

    #[test]
    fn the_environment_names_the_panels() {
        let vars = environment(&context(), Path::new("sel.txt"));
        assert!(vars.contains(&("DIRO_ACTIVE".to_owned(), "/home/ada".to_owned())));
        assert!(vars.contains(&("DIRO_CURSOR".to_owned(), "/home/ada/a.png".to_owned())));
    }
}
