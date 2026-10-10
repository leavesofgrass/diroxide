//! Commands: what the user, a button, a plugin or `diro` asks for. A command
//! is an id with named arguments; the app's catalog gives each id its name,
//! keys and menu place.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
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
}
