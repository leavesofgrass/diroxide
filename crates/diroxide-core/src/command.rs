//! Commands: what the user, a button, a plugin or `diro` asks for. A command
//! is an id with named arguments; the app's catalog gives each id its name,
//! keys and menu place.
//!
//! A command also has a text form, the one the banks, `diro run` and the
//! server's `command/run` share: the id, then each argument as
//! `name=value` (`file.copy dest="D:\My Backup"`). [`Command`]'s
//! `Display` writes it and [`Command::parse_text`] reads it back.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::{CoreError, PlaceError};
use crate::place::Place;

/// A command's id: lowercase words joined by dots, such as `file.copy` or
/// `panel.switch`. Words are ASCII letters and digits, with `-` or `_`
/// inside them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommandId(Cow<'static, str>);

impl CommandId {
    /// An id from a constant. The text is not checked here; the catalog's
    /// tests check every built-in id with [`CommandId::new`].
    pub const fn from_static(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// An id from text, checked.
    pub fn new(id: impl Into<String>) -> Result<Self, CoreError> {
        let id = id.into();
        if Self::is_valid(&id) {
            Ok(Self(Cow::Owned(id)))
        } else {
            Err(CoreError::BadCommandId(id))
        }
    }

    /// Whether text is a well-formed id.
    pub fn is_valid(id: &str) -> bool {
        !id.is_empty()
            && id.split('.').all(|word| {
                let bytes = word.as_bytes();
                !bytes.is_empty()
                    && bytes[0].is_ascii_alphanumeric()
                    && bytes[bytes.len() - 1].is_ascii_alphanumeric()
                    && bytes.iter().all(|&b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                    })
            })
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for CommandId {
    type Error = CoreError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<CommandId> for String {
    fn from(id: CommandId) -> Self {
        id.0.into_owned()
    }
}

/// The value of a command argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgValue {
    /// Text, such as a new name or a pattern.
    Text(String),
    /// A whole number, such as a count.
    Number(i64),
    /// Yes or no.
    Flag(bool),
    /// One place.
    Place(Place),
    /// Several places, such as a selection.
    Places(Vec<Place>),
}

/// A command with its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Which command.
    pub id: CommandId,
    /// Named arguments, in name order.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, ArgValue>,
}

impl Command {
    /// A command with no arguments.
    pub fn new(id: CommandId) -> Self {
        Self {
            id,
            args: BTreeMap::new(),
        }
    }

    /// The same command with an argument set.
    pub fn with_arg(mut self, name: impl Into<String>, value: ArgValue) -> Self {
        self.args.insert(name.into(), value);
        self
    }

    /// An argument by name.
    pub fn arg(&self, name: &str) -> Option<&ArgValue> {
        self.args.get(name)
    }

    /// A text argument by name.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.args.get(name) {
            Some(ArgValue::Text(text)) => Some(text),
            _ => None,
        }
    }

    /// A number argument by name.
    pub fn number(&self, name: &str) -> Option<i64> {
        match self.args.get(name) {
            Some(ArgValue::Number(n)) => Some(*n),
            _ => None,
        }
    }

    /// A flag argument by name.
    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.args.get(name) {
            Some(ArgValue::Flag(flag)) => Some(*flag),
            _ => None,
        }
    }

    /// A place argument by name.
    pub fn place(&self, name: &str) -> Option<&Place> {
        match self.args.get(name) {
            Some(ArgValue::Place(place)) => Some(place),
            _ => None,
        }
    }

    /// A places argument by name; a single place counts as one.
    pub fn places(&self, name: &str) -> Vec<&Place> {
        match self.args.get(name) {
            Some(ArgValue::Places(places)) => places.iter().collect(),
            Some(ArgValue::Place(place)) => vec![place],
            _ => Vec::new(),
        }
    }
}

impl From<CommandId> for Command {
    fn from(id: CommandId) -> Self {
        Self::new(id)
    }
}

/// Why command text could not be read. Its `Display` is the sentence the
/// banks' problem list and the server say, meaning first: "Not a command
/// id: File.Copy".
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CommandTextError {
    /// Nothing but spaces.
    #[error("No command: the text is empty")]
    Empty,
    /// The first word is not a command id.
    #[error("Not a command id: {0}")]
    BadId(String),
    /// An argument without `=value`.
    #[error("No value for {0}: write {0}=value")]
    NoValue(String),
    /// A value without a name before its `=`.
    #[error("No name before ={0}")]
    NoName(String),
    /// A quotation mark that is never closed.
    #[error("a quotation mark is not closed")]
    UnclosedQuote,
    /// A place argument whose value is not a full place.
    #[error("Not a place for {name}: {error}")]
    BadPlace {
        /// The argument's name.
        name: String,
        /// Why the value is not a place.
        error: PlaceError,
    },
    /// An argument whose value is not one of its kind, such as an attribute
    /// job's `recursion=all` ([`crate::access`]): "Not a value for
    /// recursion: all. Write this_item, files_and_folders, files_only or
    /// folders_only."
    #[error("Not a value for {name}: {value}. {error}")]
    BadValue {
        /// The argument's name.
        name: String,
        /// The value as written.
        value: String,
        /// What to write instead.
        error: crate::access::ValueTextError,
    },
}

impl Command {
    /// Argument names whose values are places in the text form; a name
    /// given more than once gathers its places into [`ArgValue::Places`].
    pub const PLACE_ARGUMENTS: [&'static str; 6] =
        ["place", "dest", "to", "from", "source", "target"];

    /// Reads a command from its text form (the `Display` of a command).
    /// Words are split at spaces; a quoted part stays whole, with a
    /// quotation mark inside written twice. A value of a
    /// [`Command::PLACE_ARGUMENTS`] name is a place; a quoted value is
    /// text; an unquoted one is a number, `true` or `false` when it reads
    /// as one, else text.
    pub fn parse_text(text: &str) -> Result<Self, CommandTextError> {
        let mut parts = words(text)?.into_iter();
        let Some((id, _)) = parts.next() else {
            return Err(CommandTextError::Empty);
        };
        let id = CommandId::new(id.clone()).map_err(|_| CommandTextError::BadId(id))?;
        let mut command = Command::new(id);
        for (part, was_quoted) in parts {
            let Some((name, raw)) = part.split_once('=') else {
                return Err(CommandTextError::NoValue(part));
            };
            if name.is_empty() {
                return Err(CommandTextError::NoName(raw.to_owned()));
            }
            let value = if Self::PLACE_ARGUMENTS.contains(&name) {
                let place = Place::parse(raw).map_err(|error| CommandTextError::BadPlace {
                    name: name.to_owned(),
                    error,
                })?;
                match command.args.remove(name) {
                    Some(ArgValue::Place(first)) => ArgValue::Places(vec![first, place]),
                    Some(ArgValue::Places(mut all)) => {
                        all.push(place);
                        ArgValue::Places(all)
                    }
                    _ => ArgValue::Place(place),
                }
            } else if was_quoted {
                ArgValue::Text(raw.to_owned())
            } else if let Ok(n) = raw.parse::<i64>() {
                ArgValue::Number(n)
            } else if raw == "true" || raw == "false" {
                ArgValue::Flag(raw == "true")
            } else {
                ArgValue::Text(raw.to_owned())
            };
            command.args.insert(name.to_owned(), value);
        }
        Ok(command)
    }
}

/// The text form: the id, then each argument as `name=value`, in name
/// order. Text that holds a space or a quotation mark, is empty, or reads
/// as a number or a flag is put in quotation marks, with a quotation mark
/// inside written twice; backslashes stay as they are, so Windows paths
/// read naturally. Several places are written as the name repeated.
impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id.as_str())?;
        for (name, value) in &self.args {
            let values: Vec<String> = match value {
                ArgValue::Text(t) => vec![quoted(t)],
                ArgValue::Number(n) => vec![n.to_string()],
                ArgValue::Flag(b) => vec![b.to_string()],
                ArgValue::Place(p) => vec![quoted(&p.to_string())],
                ArgValue::Places(places) => places.iter().map(|p| quoted(&p.to_string())).collect(),
            };
            for value in values {
                write!(f, " {name}={value}")?;
            }
        }
        Ok(())
    }
}

fn quoted(text: &str) -> String {
    let plain = !text.is_empty()
        && !text.chars().any(|c| c.is_whitespace() || c == '"')
        && text.parse::<i64>().is_err()
        && text != "true"
        && text != "false";
    if plain {
        text.to_owned()
    } else {
        format!("\"{}\"", text.replace('"', "\"\""))
    }
}

/// Splits command text into words, keeping quoted parts whole; each word
/// says whether any of it was quoted.
fn words(text: &str) -> Result<Vec<(String, bool)>, CommandTextError> {
    let mut words = Vec::new();
    let mut chars = text.chars().peekable();
    let mut word = String::new();
    let mut quoted_word = false;
    let mut in_word = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_word = true;
                quoted_word = true;
                loop {
                    match chars.next() {
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            word.push('"');
                        }
                        Some('"') => break,
                        Some(other) => word.push(other),
                        None => return Err(CommandTextError::UnclosedQuote),
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push((std::mem::take(&mut word), quoted_word));
                    quoted_word = false;
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push((word, quoted_word));
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_checked() {
        assert!(CommandId::new("file.copy").is_ok());
        assert!(CommandId::new("list.page-down").is_ok());
        assert!(CommandId::new("plugin.my_tool.run2").is_ok());
        for bad in [
            "",
            "File.copy",
            "file..copy",
            ".file",
            "file.",
            "file copy",
            "-x",
            "x-",
        ] {
            assert!(CommandId::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn arguments_by_name() {
        let place = Place::parse(if cfg!(windows) { "D:\\x" } else { "/x" }).unwrap();
        let c = Command::new(CommandId::from_static("file.rename"))
            .with_arg("name", ArgValue::Text("new.txt".into()))
            .with_arg("count", ArgValue::Number(3))
            .with_arg("force", ArgValue::Flag(false))
            .with_arg("to", ArgValue::Place(place.clone()));
        assert_eq!(c.text("name"), Some("new.txt"));
        assert_eq!(c.number("count"), Some(3));
        assert_eq!(c.flag("force"), Some(false));
        assert_eq!(c.place("to"), Some(&place));
        assert_eq!(c.places("to"), vec![&place]);
        assert_eq!(c.text("count"), None);
        assert!(c.places("missing").is_empty());
    }

    #[test]
    fn the_text_form_reads_back() {
        let place = Place::parse(if cfg!(windows) {
            r"D:\My Backup"
        } else {
            "/my backup"
        })
        .unwrap();
        let other = Place::parse(if cfg!(windows) { r"E:\x" } else { "/x" }).unwrap();
        let command = Command::new(CommandId::from_static("file.copy"))
            .with_arg("dest", ArgValue::Place(place.clone()))
            .with_arg("count", ArgValue::Number(3))
            .with_arg("keep_dates", ArgValue::Flag(false))
            .with_arg("note", ArgValue::Text("say \"hi\"".into()))
            .with_arg("plain", ArgValue::Text("12".into()))
            .with_arg("empty", ArgValue::Text(String::new()))
            .with_arg("source", ArgValue::Places(vec![place.clone(), other]));
        let text = command.to_string();
        assert_eq!(Command::parse_text(&text).unwrap(), command, "{text}");
        assert!(text.starts_with("file.copy count=3 dest=\""), "{text}");
        assert!(text.contains(" plain=\"12\""), "{text}");
        assert!(text.contains(" empty=\"\""), "{text}");
        assert!(text.contains(" note=\"say \"\"hi\"\"\""), "{text}");
        let bare = Command::new(CommandId::from_static("panel.switch"));
        assert_eq!(bare.to_string(), "panel.switch");
        assert_eq!(Command::parse_text("  panel.switch  ").unwrap(), bare);
    }

    #[test]
    fn bad_text_is_refused_in_words() {
        let cases = [
            (
                "   ",
                CommandTextError::Empty,
                "No command: the text is empty",
            ),
            (
                "File.Copy",
                CommandTextError::BadId("File.Copy".into()),
                "Not a command id: File.Copy",
            ),
            (
                "file.copy dest",
                CommandTextError::NoValue("dest".into()),
                "No value for dest: write dest=value",
            ),
            (
                "file.copy =3",
                CommandTextError::NoName("3".into()),
                "No name before =3",
            ),
            (
                "file.copy note=\"open",
                CommandTextError::UnclosedQuote,
                "a quotation mark is not closed",
            ),
        ];
        for (text, error, said) in cases {
            let got = Command::parse_text(text).unwrap_err();
            assert_eq!(got, error, "{text}");
            assert_eq!(got.to_string(), said);
        }
        let bad_place = Command::parse_text("file.copy dest=relative").unwrap_err();
        assert!(matches!(bad_place, CommandTextError::BadPlace { .. }));
        assert!(
            bad_place.to_string().starts_with("Not a place for dest: "),
            "{bad_place}"
        );
    }
}
