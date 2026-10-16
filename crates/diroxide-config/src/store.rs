//! Reading and writing the files: every write atomic and only on an
//! explicit change, the owner's comments and order kept, a file that
//! cannot be read backed up and reported in words, never overwritten.
//!
//! - A missing file means the defaults, and says nothing.
//! - A file that is not valid TOML is copied to `NAME.bad` (or `NAME.bad2`
//!   and on, never over an earlier backup that differs), the defaults are
//!   used, and one sentence says so: "config.toml could not be read: line
//!   3 is not valid TOML. A copy is kept as config.toml.bad." The file
//!   itself stays where it is, so nothing the owner wrote is lost, and a
//!   save refuses to write over it until it is fixed.
//! - A write goes to `NAME.new` beside the file, is flushed to the disk,
//!   and is renamed over the old file, so a crash leaves the old file or
//!   the new one, never half of either.
//! - A change to one setting edits that one value through `toml_edit`, so
//!   comments, blank lines, order and keys this version does not know all
//!   survive.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table as EditTable};

use crate::error::ConfigError;
use crate::paths::Paths;

/// One problem found while reading a file, in words, meaning first, so it
/// fits a 40-cell Braille line: "Unknown command: file.copi, line 4."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigProblem {
    /// The file's name, such as `keymap.toml`.
    pub file: String,
    /// The sentence.
    pub text: String,
}

impl ConfigProblem {
    /// A problem in a file.
    pub fn new(file: &str, text: impl Into<String>) -> Self {
        Self {
            file: file.to_owned(),
            text: text.into(),
        }
    }
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// A value read from a file, with the problems found on the way. The value
/// is always usable: what could not be read takes its default.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded<T> {
    /// What was read.
    pub value: T,
    /// What was wrong, in words, in file order.
    pub problems: Vec<ConfigProblem>,
}

impl<T> Loaded<T> {
    /// A value with no problems.
    pub fn clean(value: T) -> Self {
        Self {
            value,
            problems: Vec::new(),
        }
    }
}

/// The startup line for a set of problems, the file first: "keymap.toml:
/// 2 problems". Empty when there are none.
pub fn problems_line(problems: &[ConfigProblem]) -> String {
    let mut files: Vec<(&str, usize)> = Vec::new();
    for problem in problems {
        match files.iter_mut().find(|(file, _)| *file == problem.file) {
            Some((_, n)) => *n += 1,
            None => files.push((&problem.file, 1)),
        }
    }
    files
        .iter()
        .map(|(file, n)| {
            let noun = if *n == 1 { "problem" } else { "problems" };
            format!("{file}: {n} {noun}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What reading a file gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReadText {
    /// The file is not there.
    Missing,
    /// The file's text.
    Text(String),
    /// It could not be read; the problem says why.
    Failed(ConfigProblem),
}

/// Reads a file as text.
pub(crate) fn read_text(path: &Path, name: &str) -> ReadText {
    match fs::read_to_string(path) {
        Ok(text) => ReadText::Text(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => ReadText::Missing,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => ReadText::Failed(ConfigProblem::new(
            name,
            format!("{name} could not be read: it is not UTF-8 text. The defaults are in use."),
        )),
        Err(e) => ReadText::Failed(ConfigProblem::new(
            name,
            format!("{name} could not be opened: {e}. The defaults are in use."),
        )),
    }
}

/// The line a TOML error points at, counted from 1.
pub(crate) fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset.min(text.len()))
        .map_or(1, |before| before.matches('\n').count() + 1)
}

/// Why a file's text could not be used as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseFailure {
    name: String,
    line: Option<usize>,
    syntax: bool,
}

impl ParseFailure {
    fn new(name: &str, text: &str, error: &toml::de::Error, syntax: bool) -> Self {
        Self {
            name: name.to_owned(),
            line: error.span().map(|span| line_of(text, span.start)),
            syntax,
        }
    }

    /// The sentence, meaning first, with the backup named: "keymap.toml
    /// could not be read: line 3 is not valid TOML. A copy is kept as
    /// keymap.toml.bad; the defaults are in use."
    pub(crate) fn sentence(&self, backup: &str) -> String {
        let name = &self.name;
        let what = if self.syntax {
            "is not valid TOML"
        } else {
            "has a value of the wrong kind"
        };
        let place = match self.line {
            Some(line) => format!("line {line} {what}"),
            None => format!("it {what}"),
        };
        if backup.is_empty() {
            format!("{name} could not be read: {place}. The defaults are in use.")
        } else {
            format!(
                "{name} could not be read: {place}. A copy is kept as {backup}; the defaults are in use."
            )
        }
    }
}

/// Parses text as TOML, then as `T`, telling a syntax mistake from a
/// value of the wrong kind.
pub(crate) fn parse_typed<T: serde::de::DeserializeOwned>(
    name: &str,
    text: &str,
) -> Result<T, ParseFailure> {
    if let Err(error) = toml::from_str::<toml::Table>(text) {
        return Err(ParseFailure::new(name, text, &error, true));
    }
    toml::from_str::<T>(text).map_err(|error| ParseFailure::new(name, text, &error, false))
}

/// Parses text as a TOML table, the syntax only.
pub(crate) fn parse_table(name: &str, text: &str) -> Result<toml::Table, ParseFailure> {
    toml::from_str::<toml::Table>(text).map_err(|error| ParseFailure::new(name, text, &error, true))
}

/// Copies a file that could not be read to `NAME.bad`, or the first of
/// `NAME.bad2`, `NAME.bad3` and on that is free or already holds the same
/// text, and returns the backup's file name. The file itself is left as it
/// is. An empty name means no copy could be made.
pub(crate) fn back_up(path: &Path, text: &str) -> String {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return String::new();
    };
    for n in 1..100 {
        let candidate = if n == 1 {
            format!("{name}.bad")
        } else {
            format!("{name}.bad{n}")
        };
        let backup = path.with_file_name(&candidate);
        match fs::read_to_string(&backup) {
            Ok(existing) if existing == text => return candidate,
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return match write_atomic(&backup, text) {
                    Ok(()) => candidate,
                    Err(_) => String::new(),
                };
            }
            Err(_) => continue,
        }
    }
    String::new()
}

/// Writes a file atomically: to `NAME.new` beside it, flushed to the disk,
/// then renamed over the old file. The folder is made when it is missing.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent()
        && !folder.as_os_str().is_empty()
    {
        fs::create_dir_all(folder)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}

/// Writes one of the read-only references beside the owner's files, such
/// as `keymap-defaults.toml`, only when its text differs from what is on
/// disk. Returns whether it was written.
pub fn write_reference(paths: &Paths, name: &str, text: &str) -> Result<bool, ConfigError> {
    let path = paths.config_file(name);
    if fs::read_to_string(&path).is_ok_and(|on_disk| on_disk == text) {
        return Ok(false);
    }
    write_atomic(&path, text).map_err(|e| ConfigError::io(name, &e))?;
    Ok(true)
}

/// Converts a plain TOML value into an editable one.
pub(crate) fn editable(value: &toml::Value) -> toml_edit::Value {
    match value {
        toml::Value::String(text) => toml_edit::Value::from(text.as_str()),
        toml::Value::Integer(n) => toml_edit::Value::from(*n),
        toml::Value::Float(x) => toml_edit::Value::from(*x),
        toml::Value::Boolean(b) => toml_edit::Value::from(*b),
        toml::Value::Datetime(when) => toml_edit::Value::from(when.to_string()),
        toml::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(editable(item));
            }
            toml_edit::Value::Array(array)
        }
        toml::Value::Table(table) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, item) in table {
                inline.insert(key, editable(item));
            }
            toml_edit::Value::InlineTable(inline)
        }
    }
}

/// Sets one value in a document by its dotted key (`theme`, `sort.key`,
/// `colors.folder`), making the tables on the way when they are missing.
/// The key's comments and the value's trailing comment are kept. A `None`
/// value removes the key.
pub(crate) fn set_in_document(
    document: &mut DocumentMut,
    dotted: &str,
    value: Option<&toml::Value>,
) {
    let parts: Vec<&str> = dotted.split('.').collect();
    let Some((last, tables)) = parts.split_last() else {
        return;
    };
    let mut table: &mut EditTable = document.as_table_mut();
    for part in tables {
        let entry = table.entry(part).or_insert_with(|| {
            let mut fresh = EditTable::new();
            fresh.set_implicit(false);
            Item::Table(fresh)
        });
        if !entry.is_table() {
            if let Some(inline) = entry.as_inline_table().cloned() {
                *entry = Item::Table(inline.into_table());
            } else {
                *entry = Item::Table(EditTable::new());
            }
        }
        let Some(next) = entry.as_table_mut() else {
            return;
        };
        table = next;
    }
    match value {
        None => {
            table.remove(last);
        }
        Some(value) => {
            let mut fresh = editable(value);
            if let Some(old) = table.get(last).and_then(Item::as_value) {
                *fresh.decor_mut() = old.decor().clone();
            }
            match table.get_mut(last) {
                Some(item) => *item = Item::Value(fresh),
                None => {
                    table.insert(last, Item::Value(fresh));
                }
            }
        }
    }
}

/// Changes values in one of the config folder's files, editing only those
/// values: comments, blank lines, order and unknown keys survive. A file
/// that cannot be read is not written over; the error says so.
pub fn edit_file(
    paths: &Paths,
    name: &str,
    changes: &[(String, Option<toml::Value>)],
) -> Result<(), ConfigError> {
    if changes.is_empty() {
        return Ok(());
    }
    let path = paths.config_file(name);
    let text = match read_text(&path, name) {
        ReadText::Missing => String::new(),
        ReadText::Text(text) => text,
        ReadText::Failed(problem) => {
            return Err(ConfigError::Io {
                file: name.to_owned(),
                message: problem.text,
            });
        }
    };
    let mut document: DocumentMut = text.parse().map_err(|_| {
        ConfigError::bad_entry(name, "the file is not valid TOML, so nothing was saved")
    })?;
    for (key, value) in changes {
        set_in_document(&mut document, key, value.as_ref());
    }
    write_atomic(&path, &document.to_string()).map_err(|e| ConfigError::io(name, &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_write_is_atomic_and_makes_its_folder() {
        let dir = folder();
        let path = dir.path().join("deeper").join("config.toml");
        write_atomic(&path, "theme = \"nord\"\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme = \"nord\"\n");
        assert!(!dir.path().join("deeper").join("config.toml.new").exists());
        write_atomic(&path, "theme = \"dracula\"\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme = \"dracula\"\n");
    }

    #[test]
    fn one_change_keeps_comments_order_and_unknown_keys() {
        let dir = folder();
        let paths = Paths::new(dir.path().to_path_buf(), dir.path().join("state"));
        let before = "# My settings, by hand.\n\n# The colors I like.\ntheme = \"nord\" # dark\nshow_hidden = false\nfuture_option = 3\n\n[sort]\n# Newest first.\nkey = \"modified\"\n";
        fs::write(paths.config_file(Paths::SETTINGS), before).unwrap();
        edit_file(
            &paths,
            Paths::SETTINGS,
            &[
                (
                    "theme".to_owned(),
                    Some(toml::Value::String("dracula".into())),
                ),
                (
                    "sort.folders".to_owned(),
                    Some(toml::Value::String("last".into())),
                ),
            ],
        )
        .unwrap();
        let after = fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap();
        assert_eq!(
            after,
            "# My settings, by hand.\n\n# The colors I like.\ntheme = \"dracula\" # dark\nshow_hidden = false\nfuture_option = 3\n\n[sort]\n# Newest first.\nkey = \"modified\"\nfolders = \"last\"\n"
        );
    }

    #[test]
    fn a_new_table_is_made_and_a_key_can_be_removed() {
        let mut document: DocumentMut = "theme = \"nord\"\n".parse().unwrap();
        set_in_document(
            &mut document,
            "colors.folder",
            Some(&toml::Value::String("#ffaa00".into())),
        );
        set_in_document(&mut document, "theme", None);
        assert_eq!(document.to_string(), "[colors]\nfolder = \"#ffaa00\"\n");
    }

    #[test]
    fn a_corrupt_file_is_backed_up_once_and_never_written_over() {
        let dir = folder();
        let paths = Paths::new(dir.path().to_path_buf(), dir.path().join("state"));
        let path = paths.config_file(Paths::SETTINGS);
        fs::write(&path, "theme = \n").unwrap();
        assert_eq!(back_up(&path, "theme = \n"), "config.toml.bad");
        assert_eq!(back_up(&path, "theme = \n"), "config.toml.bad");
        assert_eq!(back_up(&path, "other = \n"), "config.toml.bad2");
        let err = edit_file(
            &paths,
            Paths::SETTINGS,
            &[("theme".to_owned(), Some(toml::Value::String("nord".into())))],
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("config.toml has a bad entry"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme = \n");
    }

    #[test]
    fn a_reference_is_written_only_when_it_changes() {
        let dir = folder();
        let paths = Paths::new(dir.path().to_path_buf(), dir.path().join("state"));
        assert!(write_reference(&paths, Paths::KEYMAP_DEFAULTS, "# keys\n").unwrap());
        assert!(!write_reference(&paths, Paths::KEYMAP_DEFAULTS, "# keys\n").unwrap());
        assert!(write_reference(&paths, Paths::KEYMAP_DEFAULTS, "# keys, new\n").unwrap());
    }

    #[test]
    fn the_problems_line_counts_per_file() {
        let problems = [
            ConfigProblem::new("keymap.toml", "Unknown key: hyper+q, line 2."),
            ConfigProblem::new("keymap.toml", "Unknown command: x.y, line 3."),
            ConfigProblem::new("config.toml", "Unknown setting: foo. Kept as it is."),
        ];
        assert_eq!(
            problems_line(&problems),
            "keymap.toml: 2 problems, config.toml: 1 problem"
        );
        assert_eq!(problems_line(&[]), "");
        assert_eq!(line_of("a\nb\nc", 4), 3);
    }
}
