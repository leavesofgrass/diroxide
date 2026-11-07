//! A recipe's placeholders (R28 5.3), filled **per argument, after the
//! list is split**, never inside a shell string: a name with spaces,
//! quotes or `$(...)` is inert because no shell reads it.
//!
//! - `{input}`: the file converted, its full path. `{file}` is the same,
//!   the name the typed actions use for it.
//! - `{typed_input}`: the same path behind the format diroxide's own
//!   detection found, `PNG:/photos/a.png`, so ImageMagick reads the file
//!   as what it is and never guesses from the name or treats it as one of
//!   its special names (spike V1).
//! - `{output}`: where the tool writes: the staging name in the
//!   destination folder (`.diro-part-` and eight hexadecimal digits and
//!   the output's extension), renamed to its real name only when the tool
//!   succeeded.
//! - `{stem}`: the input's name without its last extension: `song` for
//!   `song.flac`.
//! - `{name}`: the input's bare name, with `./` in front when it starts
//!   with a dash, so a tool never reads it as an option; the tool runs in
//!   the input's folder.
//! - `{dir}`: the input's folder. `{dest}` and `{other}`: the folder the
//!   output goes to.
//!
//! `{{` and `}}` stand for a brace. The typed actions' `{files}` is not a
//! recipe's: a recipe takes one file at a time.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

/// Every placeholder a recipe may use, without the braces.
pub const PLACEHOLDERS: [&str; 9] = [
    "input",
    "typed_input",
    "output",
    "stem",
    "name",
    "file",
    "dir",
    "dest",
    "other",
];

/// The placeholders that name the file converted; a recipe uses at least
/// one.
pub const INPUTS: [&str; 4] = ["input", "typed_input", "file", "name"];

/// One piece of an argument: text as written, or a placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    Brace(char),
    Slot(&'a str),
}

/// Cuts an argument into text and placeholders. A `{` that does not open
/// a placeholder of letters and underscores is text.
fn pieces(argument: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = argument;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("{{") {
            out.push(Piece::Brace('{'));
            rest = after;
            continue;
        }
        if let Some(after) = rest.strip_prefix("}}") {
            out.push(Piece::Brace('}'));
            rest = after;
            continue;
        }
        if let Some(after) = rest.strip_prefix('{')
            && let Some(end) = after.find('}')
            && end > 0
            && after[..end]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit())
        {
            out.push(Piece::Slot(&after[..end]));
            rest = &after[end + 1..];
            continue;
        }
        let next = rest[1..].find(['{', '}']).map_or(rest.len(), |at| at + 1);
        out.push(Piece::Text(&rest[..next]));
        rest = &rest[next..];
    }
    out
}

/// The placeholders an argument names, in order, without the braces.
pub fn named(argument: &str) -> Vec<&str> {
    pieces(argument)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Slot(name) => Some(name),
            _ => None,
        })
        .collect()
}

/// The first placeholder in these arguments that is not a recipe's, with
/// its braces, for "Recipe png-to-jpeg: {size} is not a placeholder."
pub fn first_unknown(args: &[String]) -> Option<String> {
    args.iter()
        .flat_map(|argument| named(argument))
        .find(|name| !PLACEHOLDERS.contains(name))
        .map(|name| format!("{{{name}}}"))
}

/// Whether the arguments name the file converted and the output.
pub fn has_input_and_output(args: &[String]) -> (bool, bool) {
    let names: Vec<&str> = args.iter().flat_map(|a| named(a)).collect();
    (
        names.iter().any(|name| INPUTS.contains(name)),
        names.contains(&"output"),
    )
}

/// What each placeholder stands for in one run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fill {
    values: BTreeMap<&'static str, OsString>,
}

impl Fill {
    /// Nothing filled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fills one placeholder; a name that is not a recipe's is ignored.
    pub fn set(&mut self, name: &str, value: impl AsRef<OsStr>) -> &mut Self {
        if let Some(known) = PLACEHOLDERS.iter().find(|known| **known == name) {
            self.values.insert(known, value.as_ref().to_owned());
        }
        self
    }

    /// The value of one placeholder.
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.values.get(name).map(OsString::as_os_str)
    }
}

/// The arguments with every placeholder filled, each argument whole.
/// `Err` names the first placeholder nothing fills, with its braces.
pub fn expand(args: &[String], fill: &Fill) -> Result<Vec<OsString>, String> {
    args.iter()
        .map(|argument| {
            let mut out = OsString::new();
            for piece in pieces(argument) {
                match piece {
                    Piece::Text(text) => out.push(text),
                    Piece::Brace(brace) => out.push(brace.to_string()),
                    Piece::Slot(name) => match fill.get(name) {
                        Some(value) => out.push(value),
                        None => return Err(format!("{{{name}}}")),
                    },
                }
            }
            Ok(out)
        })
        .collect()
}

/// A bare name as `{name}` gives it: `./` in front when it starts with a
/// dash, so no tool reads it as an option.
pub fn dash_guarded(name: &str) -> String {
    if name.starts_with('-') {
        format!("./{name}")
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn placeholders_are_found_inside_arguments_and_braces_can_be_written() {
        assert_eq!(named("{typed_input}"), ["typed_input"]);
        assert_eq!(named("file:{input}"), ["input"]);
        assert_eq!(named("--metadata=title:{stem}.{dir}"), ["stem", "dir"]);
        assert!(named("1600x1600>").is_empty());
        assert!(named("{{not}}").is_empty());
        assert!(named("{ spaced }").is_empty());
        assert!(named("{}").is_empty());
        assert_eq!(
            first_unknown(&strings(&["{input}", "-resize", "{size}"])),
            Some("{size}".to_owned())
        );
        assert_eq!(
            first_unknown(&strings(&["{files}"])),
            Some("{files}".to_owned())
        );
        assert_eq!(first_unknown(&strings(&["{input}", "{output}"])), None);
        assert_eq!(
            has_input_and_output(&strings(&["{typed_input}", "{output}"])),
            (true, true)
        );
        assert_eq!(
            has_input_and_output(&strings(&["-i", "{output}"])),
            (false, true)
        );
    }

    /// Each argument stays whole, whatever its characters: no shell ever
    /// reads it, so a name with spaces, quotes or `$(...)` is inert.
    #[test]
    fn filling_keeps_each_argument_whole() {
        let mut fill = Fill::new();
        fill.set("input", "/in/it's $(here) a.png")
            .set("output", "/out/.diro-part-0000002a.jpg")
            .set("stem", "a")
            .set("not_a_placeholder", "ignored");
        let args = strings(&["PNG:{input}", "-quality", "90", "{output}", "{{x}}"]);
        let filled = expand(&args, &fill).unwrap();
        assert_eq!(filled.len(), 5);
        assert_eq!(filled[0], OsString::from("PNG:/in/it's $(here) a.png"));
        assert_eq!(filled[3], OsString::from("/out/.diro-part-0000002a.jpg"));
        assert_eq!(filled[4], OsString::from("{x}"));
        assert_eq!(fill.get("not_a_placeholder"), None);
        assert_eq!(expand(&strings(&["{dir}"]), &fill), Err("{dir}".to_owned()));
    }

    #[test]
    fn a_name_starting_with_a_dash_is_guarded() {
        assert_eq!(dash_guarded("-rf.png"), "./-rf.png");
        assert_eq!(dash_guarded("song.flac"), "song.flac");
    }
}
