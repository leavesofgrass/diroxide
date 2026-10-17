//! Saved searches, in the file shape alpha.8's collections will read: one
//! TOML file each in the state folder (`saved/NAME.toml`, a plain file
//! name made from the display name, written atomically), holding a
//! format number, the display name, the kind, and then either what a
//! search needs (its places, its expression as typed, the contents it
//! looks for, its scope's options, its sort, when it was made) or a
//! collection's items (places, each with an optional label word).
//!
//! A saved search runs on open: its expression is stored as typed, so
//! date words are read again against the clock each time. A stored list
//! is stale by nature, so [`stale_check`] answers for one row whether its
//! file is still there and unchanged, before the row is acted on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use diroxide_core::filter::{FilterError, FilterOptions};
use diroxide_core::names::display_name;
use diroxide_core::{CaseRule, Clock, Place, Timestamp, UtcOffset};
use diroxide_vfs::{OpContext, Registry, VfsError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ContentPattern, RowShape, Scope, SearchHit, SearchQuery};

/// The format number this version writes and reads.
pub const SAVED_FORMAT: u32 = 1;

/// The folder below the state folder that holds saved searches and
/// collections.
pub const SAVED_FOLDER: &str = "saved";

/// What a saved file holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedKind {
    /// A search, run each time it is opened.
    #[default]
    Search,
    /// A collection: a list of places kept as they are (alpha.8).
    Collection,
}

/// The contents a saved search looks for (the dialog's second field).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedContent {
    /// The text, or a regular expression when `regex` is set.
    pub text: String,
    /// Whether the text is a regular expression.
    #[serde(default)]
    pub regex: bool,
    /// How case decides: smart, ignore or match.
    #[serde(default = "smart")]
    pub case: String,
    /// Read only this many bytes of each file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_bytes: Option<u64>,
}

fn smart() -> String {
    "smart".to_owned()
}

fn yes() -> bool {
    true
}

/// A saved search's scope, beside its expression.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedOptions {
    /// Search hidden entries.
    #[serde(default)]
    pub hidden: bool,
    /// Read ignore files.
    #[serde(default)]
    pub ignore_files: bool,
    /// Follow links inside the start.
    #[serde(default)]
    pub follow_links: bool,
    /// Stay on one drive.
    #[serde(default = "yes")]
    pub one_file_system: bool,
    /// Search inside archives.
    #[serde(default)]
    pub archives: bool,
    /// How deep to go; direct children are 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// One row per file or per matching line: "file" or "line".
    #[serde(default = "per_file")]
    pub rows: String,
}

fn per_file() -> String {
    "file".to_owned()
}

impl Default for SavedOptions {
    fn default() -> Self {
        Self {
            hidden: false,
            ignore_files: false,
            follow_links: false,
            one_file_system: true,
            archives: false,
            depth: None,
            rows: per_file(),
        }
    }
}

/// How a saved list is sorted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSort {
    /// The column: name, folder, size, modified or match.
    pub column: String,
    /// ascending or descending.
    pub direction: String,
}

/// One item of a collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedItem {
    /// Where it is.
    pub place: Place,
    /// A label word, when the item has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A saved search or collection, as its file holds it. Keys this version
/// does not know are kept and written back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// The file's format number.
    pub format: u32,
    /// The display name, any characters.
    pub name: String,
    /// A search or a collection.
    pub kind: SavedKind,
    /// When it was saved, in UTC: `2026-10-17T14:30:00Z`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// A search's start places.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
    /// A search's expression, as typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// How case decides in the expression when it is not the owner's
    /// setting: smart, ignore or match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    /// A search's contents, beside the expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<SavedContent>,
    /// A search's scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SavedOptions>,
    /// How the list is sorted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<SavedSort>,
    /// A collection's items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<SavedItem>,
    /// Keys this version does not know, kept as they were.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// Why a saved file could not be read or written.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SavedError {
    /// The file could not be read or written.
    #[error("the file could not be {what}: {source}")]
    File {
        /// "read" or "written".
        what: &'static str,
        /// The cause.
        source: std::io::Error,
    },
    /// The file is not a saved search this version can read.
    #[error("it is not a saved search: {0}")]
    Shape(String),
    /// The format number is newer than this version knows.
    #[error("it was saved by a newer version (format {0})")]
    Newer(u32),
    /// The expression no longer reads.
    #[error(transparent)]
    Filter(#[from] FilterError),
    /// A collection has no search to run.
    #[error("a collection is not a search")]
    NotASearch,
}

/// A moment as the file writes it, in UTC: `2026-10-17T14:30:00Z`.
pub fn utc_text(time: Timestamp) -> String {
    let c = time.to_civil(UtcOffset::default());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// The moment of a `created` value, or `None` when it does not read.
pub fn utc_moment(text: &str) -> Option<Timestamp> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-').map(str::parse::<i64>);
    let mut time = time.split(':').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (hour, minute, second) = (time.next()?.ok()?, time.next()?.ok()?, time.next()?.ok()?);
    Timestamp::from_civil(
        i32::try_from(year).ok()?,
        u8::try_from(month).ok()?,
        u8::try_from(day).ok()?,
        u8::try_from(hour).ok()?,
        u8::try_from(minute).ok()?,
        u8::try_from(second).ok()?,
        UtcOffset::default(),
    )
}

fn case_word(rule: CaseRule) -> &'static str {
    match rule {
        CaseRule::Ignore => "ignore",
        CaseRule::Match => "match",
        _ => "smart",
    }
}

impl Saved {
    /// A saved search of a query, made now.
    pub fn search(name: &str, query: &SearchQuery, clock: &dyn Clock) -> Self {
        let scope = &query.scope;
        let options = SavedOptions {
            hidden: scope.hidden,
            ignore_files: scope.ignore_files,
            follow_links: scope.follow_links,
            one_file_system: scope.one_file_system,
            archives: scope.archives,
            depth: scope.max_depth,
            rows: match query.rows {
                RowShape::PerFile => "file".to_owned(),
                RowShape::PerLine => "line".to_owned(),
            },
        };
        let case = case_word(query.options().case);
        Self {
            format: SAVED_FORMAT,
            name: name.to_owned(),
            kind: SavedKind::Search,
            created: Some(utc_text(clock.now())),
            places: scope.places.clone(),
            expression: query.filter.as_ref().map(|f| f.text().to_owned()),
            case: (case != "smart").then(|| case.to_owned()),
            content: None,
            options: Some(options),
            sort: None,
            items: Vec::new(),
            extra: BTreeMap::new(),
        }
    }

    /// A collection of places, made now.
    pub fn collection(name: &str, items: Vec<SavedItem>, clock: &dyn Clock) -> Self {
        Self {
            format: SAVED_FORMAT,
            name: name.to_owned(),
            kind: SavedKind::Collection,
            created: Some(utc_text(clock.now())),
            places: Vec::new(),
            expression: None,
            case: None,
            content: None,
            options: None,
            sort: None,
            items,
            extra: BTreeMap::new(),
        }
    }

    /// Reads a saved file's text.
    pub fn from_toml(text: &str) -> Result<Self, SavedError> {
        let saved: Saved =
            toml::from_str(text).map_err(|error| SavedError::Shape(error.message().to_owned()))?;
        if saved.format > SAVED_FORMAT {
            return Err(SavedError::Newer(saved.format));
        }
        Ok(saved)
    }

    /// The file's text.
    pub fn to_toml(&self) -> Result<String, SavedError> {
        toml::to_string(self).map_err(|error| SavedError::Shape(error.to_string()))
    }

    /// Reads a saved file.
    pub fn load(path: &Path) -> Result<Self, SavedError> {
        let text = std::fs::read_to_string(path).map_err(|source| SavedError::File {
            what: "read",
            source,
        })?;
        Self::from_toml(&text)
    }

    /// Writes the file into `folder` atomically: the text goes to an
    /// unfinished name beside the target, then takes the target's name, so
    /// the file is always whole. The name is made from the display name
    /// ([`file_name_for`]); a file of another saved search with that name
    /// is never replaced (a number is added). Returns where it went.
    pub fn save(&self, folder: &Path) -> Result<PathBuf, SavedError> {
        let written = |source| SavedError::File {
            what: "written",
            source,
        };
        std::fs::create_dir_all(folder).map_err(written)?;
        let base = file_stem_for(&self.name);
        let mut number = 1;
        let target = loop {
            let stem = if number == 1 {
                base.clone()
            } else {
                format!("{base}-{number}")
            };
            let candidate = folder.join(format!("{stem}.toml"));
            match Self::load(&candidate) {
                Err(SavedError::File { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    break candidate;
                }
                Ok(other) if other.name == self.name => break candidate,
                _ => number += 1,
            }
        };
        let text = self.to_toml()?;
        let mut unfinished = target.clone().into_os_string();
        unfinished.push(".part");
        let unfinished = PathBuf::from(unfinished);
        std::fs::write(&unfinished, text).map_err(written)?;
        std::fs::rename(&unfinished, &target).map_err(written)?;
        Ok(target)
    }

    /// Every saved file in `folder`, by name, each read or with why not.
    pub fn list(folder: &Path) -> Vec<(PathBuf, Result<Saved, SavedError>)> {
        let Ok(items) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = items
            .filter_map(Result::ok)
            .map(|item| item.path())
            .filter(|path| path.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let saved = Self::load(&path);
                (path, saved)
            })
            .collect()
    }

    /// The row a list of saved searches shows: "Large videos, saved
    /// search, runs on open", "Taxes, collection, 12 items".
    pub fn row_words(&self) -> String {
        let name = display_name(&self.name);
        match self.kind {
            SavedKind::Search => format!("{name}, saved search, runs on open"),
            SavedKind::Collection => format!(
                "{name}, collection, {}",
                crate::summary::counted(self.items.len() as u64, "item", "items")
            ),
        }
    }

    /// The search to run when it is opened, its date words read again
    /// against the clock when it runs. `options` are the owner's filter
    /// settings; a case the file names wins.
    pub fn to_query(&self, options: &FilterOptions) -> Result<SearchQuery, SavedError> {
        if self.kind != SavedKind::Search {
            return Err(SavedError::NotASearch);
        }
        let mut options = *options;
        match self.case.as_deref() {
            Some("ignore") => options.case = CaseRule::Ignore,
            Some("match") => options.case = CaseRule::Match,
            Some("smart") => options.case = CaseRule::Smart,
            _ => {}
        }
        let first = self
            .places
            .first()
            .cloned()
            .ok_or_else(|| SavedError::Shape("it names no place to search".to_owned()))?;
        let mut query =
            SearchQuery::parse_with(first, self.expression.as_deref().unwrap_or(""), &options)?;
        let saved = self.options.clone().unwrap_or_default();
        query.scope = Scope::of(self.places.clone())
            .with_hidden(saved.hidden)
            .with_ignore_files(saved.ignore_files)
            .with_follow_links(saved.follow_links)
            .with_one_file_system(saved.one_file_system)
            .with_archives(saved.archives)
            .with_max_depth(saved.depth);
        if saved.rows == "line" {
            query.rows = RowShape::PerLine;
        }
        if let Some(content) = &self.content {
            let case_sensitive = match content.case.as_str() {
                "match" => true,
                "ignore" => false,
                _ => content.text.chars().any(char::is_uppercase),
            };
            let pattern = ContentPattern {
                regex: if content.regex {
                    content.text.clone()
                } else {
                    regex::escape(&content.text)
                },
                case_sensitive,
                max_bytes: content.first_bytes,
            };
            query = query.with_content(&pattern)?;
        }
        Ok(query)
    }
}

/// A plain file stem for a display name: lowercase letters and digits,
/// everything else a hyphen, at most 60 characters, never empty and never
/// a name Windows keeps for a device. "Large videos" is `large-videos`.
pub fn file_stem_for(name: &str) -> String {
    let mut stem = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() && !c.is_control() {
            stem.push(c);
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
        if stem.chars().count() >= 60 {
            break;
        }
    }
    let stem = stem.trim_end_matches('-').to_owned();
    let device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if stem.is_empty() {
        "search".to_owned()
    } else if device {
        format!("{stem}-search")
    } else {
        stem
    }
}

/// The file name for a display name: [`file_stem_for`] and `.toml`.
pub fn file_name_for(name: &str) -> String {
    format!("{}.toml", file_stem_for(name))
}

/// Whether a stored row's file is still as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Freshness {
    /// There, with the same size and time.
    Fresh,
    /// There, but its size or time changed.
    Changed,
    /// Gone since the list was made.
    Gone,
    /// It could not be checked.
    Unknown,
}

impl Freshness {
    /// The row's words when it is not fresh, the name first: "report.txt,
    /// gone since this list was made"; `None` when it is fresh.
    pub fn words(self, name: &str) -> Option<String> {
        let name = display_name(name);
        match self {
            Freshness::Fresh => None,
            Freshness::Changed => Some(format!("{name}, changed since this list was made")),
            Freshness::Gone => Some(format!("{name}, gone since this list was made")),
            Freshness::Unknown => Some(format!("{name}, could not be checked")),
        }
    }

    /// Whether an action may go ahead on the row: not when it is gone.
    pub fn allows_action(self) -> bool {
        self != Freshness::Gone
    }
}

/// The stale rule for one stored row (a search result, a collection
/// item): its file is read once, when the row takes focus and before an
/// action, and compared with what the list holds.
pub fn stale_check(hit: &SearchHit, registry: &Registry, ctx: &OpContext) -> Freshness {
    let Ok(provider) = registry.resolve(&hit.place) else {
        return Freshness::Unknown;
    };
    match provider.stat(&hit.place, ctx) {
        Ok(now) => {
            let same = now.kind == hit.entry.kind
                && (hit.entry.size.is_none() || now.size == hit.entry.size)
                && (hit.entry.modified.is_none() || now.modified == hit.entry.modified);
            if same {
                Freshness::Fresh
            } else {
                Freshness::Changed
            }
        }
        Err(VfsError::NotFound) => Freshness::Gone,
        Err(_) => Freshness::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_plain() {
        assert_eq!(file_stem_for("Large videos"), "large-videos");
        assert_eq!(file_stem_for("  Q3: report/*?  "), "q3-report");
        assert_eq!(file_stem_for("Café notes"), "café-notes");
        assert_eq!(file_stem_for("***"), "search");
        assert_eq!(file_stem_for("CON"), "con-search");
        assert_eq!(file_stem_for("com1"), "com1-search");
        assert_eq!(file_stem_for("commas"), "commas");
        assert_eq!(file_name_for(&"x".repeat(100)).len(), 65);
    }

    #[test]
    fn moments_read_back() {
        let moment = Timestamp::from_unix(1_791_000_000, 0);
        assert_eq!(utc_text(moment), "2026-10-03T04:00:00Z");
        assert_eq!(utc_moment(&utc_text(moment)), Some(moment));
        assert_eq!(utc_moment("October 3"), None);
    }

    #[test]
    fn stale_words_put_the_name_first() {
        assert_eq!(
            Freshness::Gone.words("report.txt").unwrap(),
            "report.txt, gone since this list was made"
        );
        assert_eq!(Freshness::Fresh.words("a"), None);
        assert!(!Freshness::Gone.allows_action());
        assert!(Freshness::Changed.allows_action());
    }
}
