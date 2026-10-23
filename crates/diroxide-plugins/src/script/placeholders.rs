//! Placeholders in a manifest-only command's `argv` (R3 section 5.7, R22
//! sections 2.6 and 3), filled **per argument, after the list is split**,
//! never inside a shell string: a name with spaces, quotes or `$(...)` is
//! inert because no shell reads it.
//!
//! - `{file}`: the cursor's entry. `{files}`: every selected entry, each
//!   its own argument; it must stand alone. `{name}`: the cursor's bare
//!   name, with `./` in front when it starts with a dash, so a tool never
//!   reads it as an option.
//! - `{dir}`: the active panel's place. `{other}` and `{dest}`: the other
//!   panel's. `{other_file}`: the other panel's cursor entry.
//! - `{input}`: the answer to the command's `prompt` (a previewer's or a
//!   converter's input file, for those). `{output}`: a converter's
//!   staging path. `{max_rows}`: a lister's cap.
//! - `{selection_file}` and `{selection_file0}`: the two list files.
//! - `{setting.KEY}`: a plugin setting; one of kind `argv` is a list and,
//!   like `{files}`, must stand alone.
//!
//! A `{files}` list too long for one command line is split into batches,
//! like `xargs`, and the host says "Running in 3 batches."; every other
//! argument repeats in each batch.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

use diroxide_core::names::spell_out;
use serde_json::Value;

/// The longest command line on Windows, in UTF-16 units, less room for
/// the program's own path.
pub const WINDOWS_COMMAND_LINE: usize = 32_000;

/// The command-line budget elsewhere: well under the usual `ARG_MAX`,
/// which the environment also uses.
pub const UNIX_COMMAND_LINE: usize = 128 * 1024;

/// The command-line budget of this system.
pub fn default_budget() -> usize {
    if cfg!(windows) {
        WINDOWS_COMMAND_LINE
    } else {
        UNIX_COMMAND_LINE
    }
}

/// Why a placeholder could not be filled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceholderError {
    /// Nothing fills it: no cursor for `{file}`, an empty `{files}`, an
    /// empty setting.
    Empty(String),
    /// A list placeholder inside other text.
    NotAlone(String),
    /// Not a placeholder this command has.
    Unknown(String),
    /// One file alone is longer than a command line.
    TooLong,
}

impl PlaceholderError {
    /// The cause, meaning first, for "Could not run NAME: CAUSE."
    pub fn cause(&self) -> String {
        match self {
            PlaceholderError::Empty(name) => format!("{{{name}}} has nothing to fill it"),
            PlaceholderError::NotAlone(name) => {
                format!("{{{name}}} must be an argument of its own")
            }
            PlaceholderError::Unknown(name) => format!("{{{name}}} is not a placeholder"),
            PlaceholderError::TooLong => "one name is longer than a command line".to_owned(),
        }
    }

    /// The sentence when a setting is what is missing: "Choose a diff
    /// tool in the plugin's settings." comes from the manifest's own
    /// description; this is the plain form.
    pub fn is_setting(&self) -> Option<&str> {
        match self {
            PlaceholderError::Empty(name) => name.strip_prefix("setting."),
            _ => None,
        }
    }
}

/// The values the placeholders of one run take.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fill {
    single: BTreeMap<String, OsString>,
    lists: BTreeMap<String, Vec<OsString>>,
}

impl Fill {
    /// No values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a one-value placeholder.
    pub fn set(&mut self, name: &str, value: impl Into<OsString>) -> &mut Self {
        self.single.insert(name.to_owned(), value.into());
        self
    }

    /// Sets a list placeholder.
    pub fn set_list(&mut self, name: &str, values: Vec<OsString>) -> &mut Self {
        self.lists.insert(name.to_owned(), values);
        self
    }

    /// Sets `{name}` from a bare name: `./` in front when it starts with a
    /// dash.
    pub fn set_name(&mut self, name: &str) -> &mut Self {
        let guarded = if name.starts_with('-') {
            format!("./{name}")
        } else {
            name.to_owned()
        };
        self.set("name", guarded)
    }

    /// Sets `{setting.KEY}` for each setting: text, numbers and flags as
    /// one value, lists (kind `argv`) as a list.
    pub fn set_settings(&mut self, settings: &serde_json::Map<String, Value>) -> &mut Self {
        for (key, value) in settings {
            let name = format!("setting.{key}");
            match value {
                Value::Array(items) => {
                    let list = items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(OsString::from)
                        .collect();
                    self.set_list(&name, list);
                }
                Value::String(text) => {
                    self.set(&name, text.clone());
                }
                Value::Number(n) => {
                    self.set(&name, n.to_string());
                }
                Value::Bool(b) => {
                    self.set(&name, b.to_string());
                }
                Value::Null | Value::Object(_) => {}
            }
        }
        self
    }

    fn list(&self, name: &str) -> Option<&Vec<OsString>> {
        self.lists.get(name)
    }
}

/// The placeholder names in an argument: `{name}` where the name is
/// lowercase letters, digits, `_` and `.`. Other braces (`{}` in a `find`
/// argument) are left alone.
pub fn names_in(arg: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = arg;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        let name = &after[..close];
        if is_name(name) {
            names.push(name.to_owned());
            rest = &after[close + 1..];
        } else {
            rest = after;
        }
    }
    names
}

fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

/// The placeholders every command may use, besides `setting.KEY`.
pub const KNOWN: &[&str] = &[
    "file",
    "files",
    "name",
    "dir",
    "other",
    "dest",
    "other_file",
    "input",
    "output",
    "max_rows",
    "selection_file",
    "selection_file0",
];

/// Placeholders in a list of arguments that no command has, for `diro
/// plugin check`; `settings` are the plugin's setting keys.
pub fn unknown_names(args: &[String], settings: &BTreeSet<String>) -> Vec<String> {
    args.iter()
        .flat_map(|a| names_in(a))
        .filter(|n| {
            !KNOWN.contains(&n.as_str())
                && !n
                    .strip_prefix("setting.")
                    .is_some_and(|key| settings.contains(key))
        })
        .collect()
}

/// Fills one argument list. A list placeholder standing alone becomes its
/// values, each its own argument; any other placeholder is replaced
/// inside its argument.
pub fn expand(args: &[String], fill: &Fill) -> Result<Vec<OsString>, PlaceholderError> {
    let mut out = Vec::new();
    for arg in args {
        let names = names_in(arg);
        if names.len() == 1 && *arg == format!("{{{}}}", names[0]) {
            let name = &names[0];
            if let Some(values) = fill.list(name) {
                if values.is_empty() {
                    return Err(PlaceholderError::Empty(name.clone()));
                }
                out.extend(values.iter().cloned());
                continue;
            }
        }
        let mut text = arg.clone();
        for name in names {
            if fill.list(&name).is_some() {
                return Err(PlaceholderError::NotAlone(name));
            }
            let Some(value) = fill.single.get(&name) else {
                return Err(
                    if KNOWN.contains(&name.as_str()) || name.starts_with("setting.") {
                        PlaceholderError::Empty(name)
                    } else {
                        PlaceholderError::Unknown(name)
                    },
                );
            };
            if value.is_empty() && name.starts_with("setting.") {
                return Err(PlaceholderError::Empty(name));
            }
            text = text.replace(&format!("{{{name}}}"), &value.to_string_lossy());
        }
        // An argument that was only one placeholder keeps its exact bytes.
        if let [only] = names_in(arg).as_slice()
            && *arg == format!("{{{only}}}")
            && let Some(value) = fill.single.get(only)
        {
            out.push(value.clone());
        } else {
            out.push(OsString::from(text));
        }
    }
    Ok(out)
}

/// The length an argument takes on a command line, with room for quotes
/// and a space.
fn cost(arg: &OsString) -> usize {
    arg.len() + 3
}

/// Fills an argument list, splitting `{files}` into batches when the
/// whole would pass `budget`; every other argument repeats in each batch.
pub fn expand_in_batches(
    args: &[String],
    fill: &Fill,
    budget: usize,
) -> Result<Vec<Vec<OsString>>, PlaceholderError> {
    let whole = expand(args, fill)?;
    let total: usize = whole.iter().map(cost).sum();
    let Some(files) = fill.list("files") else {
        return Ok(vec![whole]);
    };
    if total <= budget || !args.iter().any(|a| a == "{files}") {
        return Ok(vec![whole]);
    }
    let mut without = fill.clone();
    without
        .lists
        .insert("files".to_owned(), vec![OsString::new()]);
    let fixed: usize =
        expand(args, &without)?.iter().map(cost).sum::<usize>() - cost(&OsString::new());
    let room = budget.saturating_sub(fixed);
    let mut batches = Vec::new();
    let mut current: Vec<OsString> = Vec::new();
    let mut used = 0;
    for file in files {
        let c = cost(file);
        if c > room {
            return Err(PlaceholderError::TooLong);
        }
        if used + c > room && !current.is_empty() {
            batches.push(std::mem::take(&mut current));
            used = 0;
        }
        used += c;
        current.push(file.clone());
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
        .into_iter()
        .map(|batch| {
            let mut one = fill.clone();
            one.lists.insert("files".to_owned(), batch);
            expand(args, &one)
        })
        .collect()
}

/// The argument lists in words, for `--explain`: what would run, and
/// nothing runs. Hidden characters are spelled out.
pub fn explain(program: &str, runs: &[Vec<OsString>]) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(first) = runs.first() else {
        return vec![format!(
            "Would run nothing: {program} has no arguments to fill."
        )];
    };
    if runs.len() == 1 {
        lines.push(format!(
            "Would run {program} with {}.",
            count_words(first.len(), "argument")
        ));
    } else {
        lines.push(format!(
            "Would run {program} {} times, in batches.",
            runs.len()
        ));
    }
    for (index, run) in runs.iter().enumerate() {
        if runs.len() > 1 {
            lines.push(format!(
                "Batch {} of {}: {}.",
                index + 1,
                runs.len(),
                count_words(run.len(), "argument")
            ));
        }
        for (n, arg) in run.iter().enumerate() {
            let text = arg.to_string_lossy();
            let shown = if text.is_empty() {
                "(empty)".to_owned()
            } else {
                spell_out(&text).into_owned()
            };
            lines.push(format!("Argument {}: {shown}", n + 1));
        }
    }
    lines.push("Nothing was run.".to_owned());
    lines
}

fn count_words(n: usize, noun: &str) -> String {
    match n {
        0 => format!("no {noun}s"),
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn fill() -> Fill {
        let mut fill = Fill::new();
        fill.set("file", "/home/ada/a b.png")
            .set("dir", "/home/ada")
            .set("other", "/backup")
            .set("dest", "/backup")
            .set("input", "report")
            .set("max_rows", "5000")
            .set_name("-starts.txt")
            .set_list(
                "files",
                vec![
                    "/home/ada/a b.png".into(),
                    "/home/ada/say \"hi\".png".into(),
                ],
            );
        fill
    }

    #[test]
    fn files_become_separate_arguments() {
        let args = strings(&["-resize", "50%", "--", "{files}", "{dest}/thumbs"]);
        let out = expand(&args, &fill()).unwrap();
        assert_eq!(
            out,
            [
                "-resize",
                "50%",
                "--",
                "/home/ada/a b.png",
                "/home/ada/say \"hi\".png",
                "/backup/thumbs"
            ]
        );
    }

    #[test]
    fn a_name_starting_with_a_dash_gets_dot_slash() {
        let out = expand(&strings(&["{name}"]), &fill()).unwrap();
        assert_eq!(out, ["./-starts.txt"]);
        let mut plain = Fill::new();
        plain.set_name("notes.txt");
        assert_eq!(
            expand(&strings(&["{name}"]), &plain).unwrap(),
            ["notes.txt"]
        );
    }

    #[test]
    fn mistakes_are_named() {
        assert_eq!(
            expand(&strings(&["x{files}"]), &fill()),
            Err(PlaceholderError::NotAlone("files".into()))
        );
        assert_eq!(
            expand(&strings(&["{colour}"]), &fill()),
            Err(PlaceholderError::Unknown("colour".into()))
        );
        assert_eq!(
            expand(&strings(&["{other_file}"]), &fill()),
            Err(PlaceholderError::Empty("other_file".into()))
        );
        let mut empty = Fill::new();
        empty.set_list("files", Vec::new());
        assert_eq!(
            expand(&strings(&["{files}"]), &empty).unwrap_err().cause(),
            "{files} has nothing to fill it"
        );
        // Braces that are not a placeholder stay.
        assert_eq!(
            expand(&strings(&["-exec", "{}", "{ x }"]), &fill()).unwrap(),
            ["-exec", "{}", "{ x }"]
        );
    }

    #[test]
    fn settings_fill_as_text_or_as_a_list() {
        let settings = serde_json::json!({
            "tool": ["code", "--diff"],
            "width": 640,
            "empty": [],
        });
        let mut fill = fill();
        fill.set_settings(settings.as_object().unwrap());
        let out = expand(
            &strings(&["{setting.tool}", "{file}", "--width={setting.width}"]),
            &fill,
        )
        .unwrap();
        assert_eq!(out, ["code", "--diff", "/home/ada/a b.png", "--width=640"]);
        let missing = expand(&strings(&["{setting.empty}"]), &fill).unwrap_err();
        assert_eq!(missing.is_setting(), Some("empty"));
        let known = BTreeSet::from(["tool".to_owned()]);
        assert_eq!(
            unknown_names(
                &strings(&["{setting.tool}", "{setting.nope}", "{dir}", "{wat}"]),
                &known
            ),
            ["setting.nope", "wat"]
        );
    }

    #[test]
    fn a_long_list_splits_into_batches() {
        let files: Vec<OsString> = (0..100)
            .map(|n| OsString::from(format!("/f/{n:04}.txt")))
            .collect();
        let mut fill = Fill::new();
        fill.set_list("files", files.clone());
        let args = strings(&["-v", "{files}", "--end"]);
        let runs = expand_in_batches(&args, &fill, 400).unwrap();
        assert!(runs.len() > 1);
        let mut seen = Vec::new();
        for run in &runs {
            assert_eq!(run.first().unwrap(), "-v");
            assert_eq!(run.last().unwrap(), "--end");
            assert!(run.iter().map(cost).sum::<usize>() <= 400);
            seen.extend(run[1..run.len() - 1].iter().cloned());
        }
        assert_eq!(seen, files);
        let one = expand_in_batches(&args, &fill, default_budget()).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(
            expand_in_batches(&args, &fill, 20),
            Err(PlaceholderError::TooLong)
        );
    }

    #[test]
    fn explain_says_the_arguments_in_words() {
        let runs = vec![vec![
            OsString::from("-n"),
            OsString::from("5000"),
            OsString::from(""),
            OsString::from("right\u{202e}txt"),
        ]];
        assert_eq!(
            explain("es", &runs),
            [
                "Would run es with 4 arguments.",
                "Argument 1: -n",
                "Argument 2: 5000",
                "Argument 3: (empty)",
                "Argument 4: right[U+202E]txt",
                "Nothing was run."
            ]
        );
        let two = vec![vec![OsString::from("a")], vec![OsString::from("b")]];
        let lines = explain("du", &two);
        assert_eq!(lines[0], "Would run du 2 times, in batches.");
        assert_eq!(lines[1], "Batch 1 of 2: 1 argument.");
    }

    proptest! {
        #[test]
        fn batches_keep_every_file_once_and_in_order(
            names in prop::collection::vec("[a-z /\"-]{1,40}", 0..300),
            budget in 200usize..4000,
        ) {
            let files: Vec<OsString> = names.iter().map(OsString::from).collect();
            let mut fill = Fill::new();
            fill.set_list("files", files.clone()).set("dir", "/d");
            let args = strings(&["--", "{dir}", "{files}"]);
            match expand_in_batches(&args, &fill, budget) {
                Ok(runs) => {
                    let mut seen = Vec::new();
                    for run in &runs {
                        prop_assert_eq!(&run[0], "--");
                        prop_assert_eq!(&run[1], "/d");
                        seen.extend(run[2..].iter().cloned());
                    }
                    prop_assert_eq!(seen, files);
                }
                Err(PlaceholderError::Empty(name)) => {
                    prop_assert!(files.is_empty());
                    prop_assert_eq!(name, "files");
                }
                Err(PlaceholderError::TooLong) => {
                    prop_assert!(files.iter().any(|f| cost(f) + 12 > budget));
                }
                Err(other) => prop_assert!(false, "{other:?}"),
            }
        }

        #[test]
        fn expansion_never_joins_or_splits_a_name(name in "[^{}]{0,60}") {
            let mut fill = Fill::new();
            fill.set("file", name.clone());
            let out = expand(&strings(&["{file}"]), &fill).unwrap();
            prop_assert_eq!(out, vec![OsString::from(name)]);
        }
    }
}
