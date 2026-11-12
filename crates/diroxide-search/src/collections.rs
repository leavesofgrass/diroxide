//! Collections, on the saved file shape ([`Saved`] with
//! [`SavedKind::Collection`] and its [`SavedItem`]s, one file each in
//! `saved/` below the state folder): add, remove the item and never the
//! file, rename, list, the stale items said in words, and the paths read
//! from standard input for `diro -`.
//!
//! - **A saved search is the live collection.** It sits in the same
//!   folder, is listed beside the collections by [`Collections::list`]
//!   ("Large videos, saved search, runs on open"), and regenerates each
//!   time it is opened through [`Saved::to_query`]. Nothing here writes
//!   one; the search dialog does.
//! - **Removing an item never touches its file:** "Removed 3 from Trip
//!   photos. The files were not touched."
//! - **Stale items** are checked one by one through the providers, when a
//!   row takes focus or before an action ([`item_freshness`]), and said
//!   in [`Freshness`]'s words after the name: "report.pdf, gone since
//!   this list was made".
//! - **One path-list reader** ([`read_path_list`]) serves `diro -` and
//!   `diro ls -`: NUL-separated by default, one a line when asked, nnn's
//!   limits (16,384 paths, 64 MiB), each path's text kept or said not
//!   to be Unicode by its position.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use diroxide_core::format::grouped;
use diroxide_core::names::display_name;
use diroxide_core::{Clock, Explain, ListModel, ListRow, Place};
use diroxide_vfs::{OpContext, Registry, VfsError};
use thiserror::Error;

use crate::saved::{
    Freshness, SAVED_FOLDER, Saved, SavedError, SavedItem, SavedKind, file_name_for,
};
use crate::summary::counted;

/// The longest display name a collection takes, in characters.
pub const NAME_LIMIT: usize = 100;

/// Why a collection could not be read, made or changed. Each sentence
/// names the collection first.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CollectionError {
    /// No collection or saved search has that name.
    #[error("No collection is named {0}.")]
    NotFound(String),
    /// A collection or saved search already has that name.
    #[error("{0} is taken: a list has that name.")]
    Taken(String),
    /// The name is a saved search, which is not changed here.
    #[error("{0} is a saved search, not a collection.")]
    Live(String),
    /// The name is empty, too long or holds hidden characters.
    #[error("That name cannot be used: {0}.")]
    BadName(String),
    /// The file could not be read or written.
    #[error("{name} could not be read or saved: {source}")]
    Saved {
        /// The collection's name, or the file's.
        name: String,
        /// The cause.
        source: Box<SavedError>,
    },
}

impl Explain for CollectionError {
    fn next_step(&self) -> String {
        match self {
            CollectionError::NotFound(_) => "Choose one from the Collections list.".to_owned(),
            CollectionError::Taken(_) => "Choose another name.".to_owned(),
            CollectionError::Live(_) => "Change it from the search dialog.".to_owned(),
            CollectionError::BadName(_) => "Type a name of up to 100 characters.".to_owned(),
            CollectionError::Saved { .. } => {
                "Check that the state folder can be written.".to_owned()
            }
        }
    }
}

fn saved_error(name: &str, source: SavedError) -> CollectionError {
    CollectionError::Saved {
        name: display_name(name).into_owned(),
        source: Box::new(source),
    }
}

/// Checks a display name: trimmed, not empty, at most [`NAME_LIMIT`]
/// characters, no hidden characters.
pub fn check_name(name: &str) -> Result<String, CollectionError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CollectionError::BadName("it is empty".to_owned()));
    }
    if name.chars().count() > NAME_LIMIT {
        return Err(CollectionError::BadName(format!(
            "it is longer than {NAME_LIMIT} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(CollectionError::BadName(
            "it holds a hidden character".to_owned(),
        ));
    }
    Ok(name.to_owned())
}

/// What adding places to a collection did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Added {
    /// Places added.
    pub added: u64,
    /// Places that were in it already, left as they were.
    pub already: u64,
}

/// A place's own name as said, or the whole place for a drive or share.
fn place_name(place: &Place) -> String {
    match place.name() {
        Some(name) => display_name(name).into_owned(),
        None => display_name(&place.to_string()).into_owned(),
    }
}

impl Saved {
    /// Whether a collection holds a place.
    pub fn holds(&self, place: &Place) -> bool {
        self.items.iter().any(|item| item.place == *place)
    }

    /// Adds places at the end of a collection, each once; a place already
    /// in it stays where it is.
    pub fn add_places(&mut self, places: impl IntoIterator<Item = Place>) -> Added {
        let mut result = Added::default();
        for place in places {
            if self.holds(&place) {
                result.already += 1;
            } else {
                self.items.push(SavedItem { place, label: None });
                result.added += 1;
            }
        }
        result
    }

    /// Removes the items at these places from the list. The files are
    /// never touched. Returns how many were removed.
    pub fn remove_places(&mut self, places: &[Place]) -> u64 {
        let before = self.items.len();
        self.items.retain(|item| !places.contains(&item.place));
        (before - self.items.len()) as u64
    }

    /// What an add said: "Added 3 to Trip photos.", "Added 2 to Trip
    /// photos. 1 was there already."
    pub fn added_words(&self, added: Added) -> String {
        let mut words = format!(
            "Added {} to {}.",
            grouped(added.added),
            display_name(&self.name)
        );
        match added.already {
            0 => {}
            1 => words.push_str(" 1 was there already."),
            n => words.push_str(&format!(" {} were there already.", grouped(n))),
        }
        words
    }

    /// What a remove said: "Removed 3 from Trip photos. The files were
    /// not touched."
    pub fn removed_words(&self, removed: u64) -> String {
        let files = if removed == 1 {
            "The file was not touched."
        } else {
            "The files were not touched."
        };
        format!(
            "Removed {} from {}. {files}",
            grouped(removed),
            display_name(&self.name)
        )
    }

    /// An item's row: the name, then its state when it is not fresh, then
    /// its label, then its folder: "report.pdf, gone since this list was
    /// made, in D:\docs". The name stays first so letters jump to it; the
    /// key is the place.
    pub fn item_row(&self, index: usize, state: Option<Freshness>) -> Option<ListRow> {
        let item = self.items.get(index)?;
        let mut details = Vec::new();
        match state {
            Some(Freshness::Gone) => details.push("gone since this list was made".to_owned()),
            Some(Freshness::Unknown) => details.push("could not be checked".to_owned()),
            Some(Freshness::Changed) | Some(Freshness::Fresh) | None => {}
        }
        if let Some(label) = &item.label {
            details.push(format!("label {}", display_name(label)));
        }
        if let Some(parent) = item.place.parent() {
            details.push(format!("in {}", display_name(&parent.to_string())));
        }
        Some(ListRow::new(item.place.to_string(), place_name(&item.place)).with_details(details))
    }

    /// A collection as a list titled "Trip photos, collection", each
    /// item's state from `states` in item order (none said when missing).
    pub fn items_list(&self, states: &[Option<Freshness>]) -> ListModel {
        let rows = (0..self.items.len())
            .filter_map(|index| self.item_row(index, states.get(index).copied().flatten()))
            .collect();
        let mut list =
            ListModel::new(format!("{}, collection", display_name(&self.name))).with_rows(rows);
        list.set_empty_text("This collection is empty");
        list
    }
}

/// Whether a collection's item is still there: one `stat` through its
/// provider, [`Freshness::Fresh`] when it is, [`Freshness::Gone`] when it
/// is not, [`Freshness::Unknown`] when it could not be checked. A stored
/// item keeps no size or time, so it is never [`Freshness::Changed`].
pub fn item_freshness(item: &SavedItem, registry: &Registry, ctx: &OpContext) -> Freshness {
    let Ok(provider) = registry.resolve(&item.place) else {
        return Freshness::Unknown;
    };
    match provider.stat(&item.place, ctx) {
        Ok(_) => Freshness::Fresh,
        Err(VfsError::NotFound) => Freshness::Gone,
        Err(_) => Freshness::Unknown,
    }
}

/// Which items of a collection are stale, in item order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staleness {
    /// The collection's name as said.
    pub name: String,
    /// Each item's state, in item order.
    pub states: Vec<Freshness>,
}

impl Staleness {
    /// Checks every item of a collection through its provider.
    pub fn check(saved: &Saved, registry: &Registry, ctx: &OpContext) -> Self {
        Self {
            name: display_name(&saved.name).into_owned(),
            states: saved
                .items
                .iter()
                .map(|item| item_freshness(item, registry, ctx))
                .collect(),
        }
    }

    /// Items gone.
    pub fn gone(&self) -> u64 {
        self.count(Freshness::Gone)
    }

    /// Items that could not be checked.
    pub fn unknown(&self) -> u64 {
        self.count(Freshness::Unknown)
    }

    fn count(&self, state: Freshness) -> u64 {
        self.states.iter().filter(|s| **s == state).count() as u64
    }

    /// The check in one sentence, the count first: "2 of 214 items gone
    /// from Trip photos.", "No items gone from Trip photos.", with "3
    /// could not be checked." after it when some could not.
    pub fn sentence(&self) -> String {
        let gone = self.gone();
        let mut words = if gone == 0 {
            format!("No items gone from {}.", self.name)
        } else {
            format!(
                "{} of {} gone from {}.",
                grouped(gone),
                counted(self.states.len() as u64, "item", "items"),
                self.name
            )
        };
        let unknown = self.unknown();
        if unknown > 0 {
            words.push_str(&format!(" {} could not be checked.", grouped(unknown)));
        }
        words
    }
}

/// One file in the saved folder, as the Collections list shows it.
#[derive(Debug)]
pub struct Listed {
    /// The file.
    pub path: PathBuf,
    /// What it holds.
    pub saved: Saved,
}

impl Listed {
    /// Whether it is a live collection: a saved search, run on open.
    pub fn is_live(&self) -> bool {
        self.saved.kind == SavedKind::Search
    }
}

/// The collections and saved searches in one folder: `saved/` below the
/// state folder, or a folder a test names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collections {
    folder: PathBuf,
}

impl Collections {
    /// The collections in a state folder: its `saved/` folder.
    pub fn in_state_folder(state: &Path) -> Self {
        Self {
            folder: state.join(SAVED_FOLDER),
        }
    }

    /// The collections in a folder of their own.
    pub fn in_folder(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// The folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Every collection and saved search, by name ignoring case, each
    /// row's words [`Saved::row_words`]; a file that cannot be read is a
    /// sentence in the second list, the file first, and is left as it is.
    pub fn list(&self) -> (Vec<Listed>, Vec<String>) {
        let mut listed = Vec::new();
        let mut problems = Vec::new();
        for (path, saved) in Saved::list(&self.folder) {
            match saved {
                Ok(saved) => listed.push(Listed { path, saved }),
                Err(error) => {
                    let file = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    problems.push(format!(
                        "{file} could not be read: {error}. It is left as it is."
                    ));
                }
            }
        }
        listed.sort_by_key(|item| item.saved.name.to_lowercase());
        (listed, problems)
    }

    /// The file that holds a name, matched exactly and then ignoring case.
    pub fn find(&self, name: &str) -> Option<Listed> {
        let name = name.trim();
        let (listed, _) = self.list();
        let exact = listed.iter().position(|item| item.saved.name == name);
        let index = exact.or_else(|| {
            listed
                .iter()
                .position(|item| item.saved.name.to_lowercase() == name.to_lowercase())
        })?;
        listed.into_iter().nth(index)
    }

    /// Reads a collection by name. A saved search is refused: it runs on
    /// open through [`Saved::to_query`].
    pub fn load(&self, name: &str) -> Result<Listed, CollectionError> {
        let found = self
            .find(name)
            .ok_or_else(|| CollectionError::NotFound(name.trim().to_owned()))?;
        if found.is_live() {
            return Err(CollectionError::Live(found.saved.name));
        }
        Ok(found)
    }

    /// Makes an empty collection and writes it. A name another list has,
    /// ignoring case, is refused.
    pub fn create(&self, name: &str, clock: &dyn Clock) -> Result<Listed, CollectionError> {
        let name = check_name(name)?;
        if let Some(taken) = self.find(&name) {
            return Err(CollectionError::Taken(taken.saved.name));
        }
        let saved = Saved::collection(&name, Vec::new(), clock);
        let path = self.write_new(&saved)?;
        Ok(Listed { path, saved })
    }

    fn write_new(&self, saved: &Saved) -> Result<PathBuf, CollectionError> {
        saved
            .save(&self.folder)
            .map_err(|source| saved_error(&saved.name, source))
    }

    /// Writes a collection back over its own file, whole: to an
    /// unfinished name beside it, then over it.
    fn write_over(&self, path: &Path, saved: &Saved) -> Result<(), CollectionError> {
        let failed = |source| saved_error(&saved.name, source);
        let text = saved.to_toml().map_err(failed)?;
        let mut unfinished = path.as_os_str().to_owned();
        unfinished.push(".part");
        let unfinished = PathBuf::from(unfinished);
        let written = |source| SavedError::File {
            what: "written",
            source,
        };
        fs::write(&unfinished, text)
            .map_err(written)
            .map_err(failed)?;
        fs::rename(&unfinished, path)
            .map_err(written)
            .map_err(failed)
    }

    /// Adds places to a collection, making it when it is missing, and
    /// writes it. Returns the collection and what was added.
    pub fn add(
        &self,
        name: &str,
        places: impl IntoIterator<Item = Place>,
        clock: &dyn Clock,
    ) -> Result<(Listed, Added), CollectionError> {
        let mut listed = match self.load(name) {
            Ok(listed) => listed,
            Err(CollectionError::NotFound(_)) => self.create(name, clock)?,
            Err(error) => return Err(error),
        };
        let added = listed.saved.add_places(places);
        if added.added > 0 {
            self.write_over(&listed.path, &listed.saved)?;
        }
        Ok((listed, added))
    }

    /// Removes items from a collection, never their files, and writes it.
    /// Returns the collection and how many were removed.
    pub fn remove(&self, name: &str, places: &[Place]) -> Result<(Listed, u64), CollectionError> {
        let mut listed = self.load(name)?;
        let removed = listed.saved.remove_places(places);
        if removed > 0 {
            self.write_over(&listed.path, &listed.saved)?;
        }
        Ok((listed, removed))
    }

    /// Removes the items that are gone, checked through their providers,
    /// and writes the collection; items that could not be checked stay.
    pub fn remove_gone(
        &self,
        name: &str,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<(Listed, u64), CollectionError> {
        let mut listed = self.load(name)?;
        let before = listed.saved.items.len();
        listed
            .saved
            .items
            .retain(|item| item_freshness(item, registry, ctx) != Freshness::Gone);
        let removed = (before - listed.saved.items.len()) as u64;
        if removed > 0 {
            self.write_over(&listed.path, &listed.saved)?;
        }
        Ok((listed, removed))
    }

    /// Renames a collection: the name inside the file changes, and the
    /// file takes the new plain name when no other file has it. A name
    /// another list has, ignoring case, is refused; a change of case only
    /// is allowed.
    pub fn rename(&self, old: &str, new: &str) -> Result<Listed, CollectionError> {
        let new = check_name(new)?;
        let mut listed = self.load(old)?;
        if let Some(taken) = self.find(&new)
            && taken.path != listed.path
        {
            return Err(CollectionError::Taken(taken.saved.name));
        }
        listed.saved.name = new;
        self.write_over(&listed.path, &listed.saved)?;
        let wanted = self.folder.join(file_name_for(&listed.saved.name));
        if wanted != listed.path && !wanted.exists() {
            fs::rename(&listed.path, &wanted).map_err(|source| {
                saved_error(
                    &listed.saved.name,
                    SavedError::File {
                        what: "written",
                        source,
                    },
                )
            })?;
            listed.path = wanted;
        }
        Ok(listed)
    }

    /// The renamed words: "Renamed Trip photos to Trip, 2026."
    pub fn renamed_words(old: &str, listed: &Listed) -> String {
        format!(
            "Renamed {} to {}.",
            display_name(old.trim()),
            display_name(&listed.saved.name)
        )
    }
}

/// The most paths [`read_path_list`] reads by default (nnn's limit).
pub const PATH_LIST_MAX_PATHS: usize = 16_384;

/// The most bytes [`read_path_list`] reads by default: 64 MiB.
pub const PATH_LIST_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// How the paths of a list are separated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathSeparator {
    /// NUL, the one byte no name can hold: the default.
    #[default]
    Nul,
    /// Line breaks, with a carriage return before one dropped, for names
    /// known to hold none.
    Lines,
}

/// The limits of [`read_path_list`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathLimits {
    /// The most paths kept; reading stops there and says so.
    pub max_paths: usize,
    /// The most bytes read; a longer list is refused whole.
    pub max_bytes: u64,
}

impl Default for PathLimits {
    fn default() -> Self {
        Self {
            max_paths: PATH_LIST_MAX_PATHS,
            max_bytes: PATH_LIST_MAX_BYTES,
        }
    }
}

/// Why a path list was refused whole. Each sentence says the meaning
/// first.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PathListError {
    /// The list is longer than the byte limit.
    #[error("List is too long: over {0}.")]
    TooLong(String),
    /// NUL-separated was asked for, and the list has line breaks and no
    /// NUL.
    #[error("No NUL separators found. Use --lines.")]
    NoNul,
    /// One a line was asked for, and the list has NUL bytes.
    #[error("The list has NUL separators. Leave out --lines.")]
    HasNul,
    /// It could not be read.
    #[error("standard input could not be read: {0}")]
    Read(#[from] io::Error),
}

/// One path of a list, by its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathItem {
    /// Its position in the list, from 1, empty items not counted.
    pub number: usize,
    /// Its text, or `None` when it is not valid Unicode.
    pub text: Option<String>,
}

impl PathItem {
    /// How a path is named in a sentence: its text, or "item 2".
    pub fn what(&self) -> String {
        match &self.text {
            Some(text) => text.clone(),
            None => format!("item {}", self.number),
        }
    }
}

/// A path list as read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathList {
    /// The paths, in order, empty ones left out.
    pub items: Vec<PathItem>,
    /// Whether reading stopped at the path limit with more to read.
    pub stopped_at_limit: bool,
    /// The path limit it read with.
    pub max_paths: usize,
}

/// What [`PathList::places`] made of a list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacesRead {
    /// The places, each once, in the order read.
    pub places: Vec<Place>,
    /// The paths that were not Unicode or not a place, with why.
    pub refused: Vec<(PathItem, String)>,
    /// Paths read more than once, kept once.
    pub repeated: u64,
    /// Whether reading stopped at the path limit.
    pub stopped_at_limit: bool,
    /// The path limit.
    pub max_paths: usize,
}

impl PathList {
    /// Whether the list holds no paths.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The paths as places: a relative path is read against `base`, a
    /// path that is not Unicode or not a place is refused with why, and
    /// each place is kept once.
    pub fn places(&self, base: &Place) -> PlacesRead {
        let mut read = PlacesRead {
            stopped_at_limit: self.stopped_at_limit,
            max_paths: self.max_paths,
            ..PlacesRead::default()
        };
        let mut seen = std::collections::HashSet::new();
        for item in &self.items {
            let Some(text) = &item.text else {
                read.refused
                    .push((item.clone(), "the name is not valid Unicode".to_owned()));
                continue;
            };
            match Place::resolve(text, base) {
                Ok(place) if seen.insert(place.clone()) => read.places.push(place),
                Ok(_) => read.repeated += 1,
                Err(error) => read.refused.push((item.clone(), error.to_string())),
            }
        }
        read
    }
}

impl PlacesRead {
    /// What was read, the count first: "214 paths from standard input",
    /// then ". 2 could not be read" and ". Stopped at the limit of 16,384
    /// paths" when they apply.
    pub fn words(&self) -> String {
        let mut words = format!(
            "{} from standard input",
            counted(self.places.len() as u64, "path", "paths")
        );
        if !self.refused.is_empty() {
            words.push_str(&format!(
                ". {} could not be read",
                grouped(self.refused.len() as u64)
            ));
        }
        if self.stopped_at_limit {
            words.push_str(&format!(
                ". Stopped at the limit of {} paths",
                grouped(self.max_paths as u64)
            ));
        }
        words
    }
}

fn size_words(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", grouped(bytes / MIB))
    } else {
        format!("{} bytes", grouped(bytes))
    }
}

/// Reads a list of paths, as `find -print0` writes them (or one a line
/// with [`PathSeparator::Lines`]): empty items are skipped, an item that
/// is not Unicode is kept by its position for the caller to say, reading
/// stops at the path limit and says so, and a list over the byte limit is
/// refused whole. A list with line breaks and no NUL, read as
/// NUL-separated, is refused with "Use --lines"; a list of one path with
/// no separator at all is a list of one. This is the one reader `diro -`
/// and `diro ls -` share.
pub fn read_path_list(
    reader: impl Read,
    separator: PathSeparator,
    limits: PathLimits,
) -> Result<PathList, PathListError> {
    let mut bytes = Vec::new();
    reader
        .take(limits.max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limits.max_bytes {
        return Err(PathListError::TooLong(size_words(limits.max_bytes)));
    }
    let parts: Vec<&[u8]> = match separator {
        PathSeparator::Nul => {
            if !bytes.contains(&0) && bytes.contains(&b'\n') {
                return Err(PathListError::NoNul);
            }
            bytes.split(|b| *b == 0).collect()
        }
        PathSeparator::Lines => {
            if bytes.contains(&0) {
                return Err(PathListError::HasNul);
            }
            bytes
                .split(|b| *b == b'\n')
                .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
                .collect()
        }
    };
    let mut list = PathList {
        max_paths: limits.max_paths,
        ..PathList::default()
    };
    for part in parts.into_iter().filter(|part| !part.is_empty()) {
        if list.items.len() >= limits.max_paths {
            list.stopped_at_limit = true;
            break;
        }
        list.items.push(PathItem {
            number: list.items.len() + 1,
            text: std::str::from_utf8(part).ok().map(str::to_owned),
        });
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::{BRAILLE_CELLS, within_cells};
    use diroxide_core::{FixedClock, Timestamp, UtcOffset};

    use crate::test_support::scratch;

    fn clock() -> FixedClock {
        // Thursday, November 12, 2026, 10:00 AM UTC.
        FixedClock::new(Timestamp::from_unix(1_794_477_600, 0), UtcOffset::default())
    }

    fn place(text: &str) -> Place {
        let text = if cfg!(windows) {
            format!(r"D:\{}", text.replace('/', r"\"))
        } else {
            format!("/{text}")
        };
        Place::parse(&text).unwrap()
    }

    #[test]
    fn a_collection_is_made_added_to_and_read_back_in_the_saved_shape() {
        let dir = scratch();
        let lists = Collections::in_state_folder(dir.path());
        let clock = clock();
        let made = lists.create("Trip photos, 2026", &clock).unwrap();
        assert!(made.saved.items.is_empty());
        assert_eq!(made.path, lists.folder().join("trip-photos-2026.toml"));
        let (trip, added) = lists
            .add(
                "trip photos, 2026",
                [place("p/a.jpg"), place("p/b.jpg"), place("p/a.jpg")],
                &clock,
            )
            .unwrap();
        assert_eq!(
            added,
            Added {
                added: 2,
                already: 1
            }
        );
        // 29 cells, then the note.
        assert_eq!(
            trip.saved.added_words(added),
            "Added 2 to Trip photos, 2026. 1 was there already."
        );
        let again = Saved::load(&trip.path).unwrap();
        assert_eq!(again.kind, SavedKind::Collection);
        assert_eq!(again.items, trip.saved.items);
        // 38 cells.
        let row = again.row_words();
        assert_eq!(row, "Trip photos, 2026, collection, 2 items");
        assert!(row.chars().count() <= BRAILLE_CELLS);
        let (fresh, added) = lists.add("Taxes", [place("t/2025.pdf")], &clock).unwrap();
        assert_eq!(fresh.saved.added_words(added), "Added 1 to Taxes.");
        assert!(matches!(
            lists.create("TAXES", &clock),
            Err(CollectionError::Taken(name)) if name == "Taxes"
        ));
        assert!(matches!(
            lists.create("  ", &clock),
            Err(CollectionError::BadName(_))
        ));
        assert!(check_name("a\tb").is_err());
        assert!(check_name(&"x".repeat(101)).is_err());
    }

    #[test]
    fn removing_an_item_never_touches_its_file() {
        let dir = scratch();
        let lists = Collections::in_state_folder(dir.path());
        let real = dir.path().join("kept.txt");
        fs::write(&real, "kept").unwrap();
        let real_place = Place::parse(&real.to_string_lossy()).unwrap();
        lists
            .add("Keep", [real_place.clone(), place("x/y.txt")], &clock())
            .unwrap();
        let (keep, removed) = lists
            .remove("Keep", std::slice::from_ref(&real_place))
            .unwrap();
        assert_eq!(removed, 1);
        assert!(real.is_file(), "the file stays");
        assert!(!keep.saved.holds(&real_place));
        assert!(!Saved::load(&keep.path).unwrap().holds(&real_place));
        // 20 cells, then the reassurance.
        assert_eq!(
            keep.saved.removed_words(1),
            "Removed 1 from Keep. The file was not touched."
        );
        let trip = Saved::collection("Trip photos", Vec::new(), &clock());
        let words = trip.removed_words(3);
        assert_eq!(
            words,
            "Removed 3 from Trip photos. The files were not touched."
        );
        assert!(within_cells(&words, "Removed 3 from Trip photos."));
        assert!(matches!(
            lists.remove("Nothing", &[]),
            Err(CollectionError::NotFound(_))
        ));
    }

    #[test]
    fn gone_items_are_found_through_the_providers_and_said_in_words() {
        let dir = scratch();
        let lists = Collections::in_state_folder(dir.path());
        let here = dir.path().join("here.txt");
        fs::write(&here, "here").unwrap();
        let here = Place::parse(&here.to_string_lossy()).unwrap();
        let gone = Place::parse(&dir.path().join("gone.txt").to_string_lossy()).unwrap();
        let (trip, _) = lists
            .add("Trip photos", [here.clone(), gone.clone()], &clock())
            .unwrap();
        let registry = Registry::with_local();
        let ctx = OpContext::new();
        let stale = Staleness::check(&trip.saved, &registry, &ctx);
        assert_eq!(stale.states, [Freshness::Fresh, Freshness::Gone]);
        // 35 cells.
        assert_eq!(stale.sentence(), "1 of 2 items gone from Trip photos.");
        let row = trip
            .saved
            .item_row(1, Some(Freshness::Gone))
            .unwrap()
            .spoken();
        assert!(
            row.starts_with("gone.txt, gone since this list was made, in "),
            "{row}"
        );
        assert!(within_cells(&row, "gone.txt, gone since this list was"));
        let unknown = Staleness {
            name: "Trip photos".to_owned(),
            states: vec![Freshness::Fresh, Freshness::Unknown],
        };
        assert_eq!(
            unknown.sentence(),
            "No items gone from Trip photos. 1 could not be checked."
        );
        let (kept, removed) = lists.remove_gone("Trip photos", &registry, &ctx).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(kept.saved.items.len(), 1);
        assert_eq!(kept.saved.items[0].place, here);
        let list = kept.saved.items_list(&[]);
        assert_eq!(list.title(), "Trip photos, collection");
        assert_eq!(list.empty_text(), "This collection is empty");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn a_rename_keeps_the_items_and_the_file_name_follows() {
        let dir = scratch();
        let lists = Collections::in_state_folder(dir.path());
        let clock = clock();
        lists
            .add("Trip photos", [place("p/a.jpg")], &clock)
            .unwrap();
        lists.create("Taxes", &clock).unwrap();
        let renamed = lists.rename("Trip photos", "Trip, 2026").unwrap();
        assert_eq!(renamed.saved.items.len(), 1);
        assert_eq!(renamed.path, lists.folder().join("trip-2026.toml"));
        assert!(!lists.folder().join("trip-photos.toml").exists());
        // 34 cells.
        assert_eq!(
            Collections::renamed_words("Trip photos", &renamed),
            "Renamed Trip photos to Trip, 2026."
        );
        assert!(matches!(
            lists.rename("Trip, 2026", "taxes"),
            Err(CollectionError::Taken(name)) if name == "Taxes"
        ));
        assert_eq!(lists.rename("taxes", "TAXES").unwrap().saved.name, "TAXES");
        let names: Vec<String> = lists.list().0.into_iter().map(|l| l.saved.name).collect();
        assert_eq!(names, ["TAXES", "Trip, 2026"]);
    }

    #[test]
    fn a_saved_search_is_the_live_collection_and_is_never_changed_here() {
        let dir = scratch();
        let lists = Collections::in_state_folder(dir.path());
        let clock = clock();
        let videos = Place::parse(if cfg!(windows) {
            r"D:\videos"
        } else {
            "/videos"
        })
        .unwrap();
        let query = crate::SearchQuery::parse(videos, "size > 1 GB").unwrap();
        let search_path = Saved::search("Large videos", &query, &clock)
            .save(lists.folder())
            .unwrap();
        let before = fs::read_to_string(&search_path).unwrap();
        fs::write(lists.folder().join("broken.toml"), "name = [\n").unwrap();
        lists
            .add("Trip photos", [place("p/a.jpg")], &clock)
            .unwrap();
        let (listed, problems) = lists.list();
        let rows: Vec<String> = listed.iter().map(|l| l.saved.row_words()).collect();
        // 40 cells and 31 cells.
        assert_eq!(
            rows,
            [
                "Large videos, saved search, runs on open",
                "Trip photos, collection, 1 item"
            ]
        );
        assert_eq!(rows[0].chars().count(), BRAILLE_CELLS);
        assert!(listed[0].is_live());
        assert!(listed[0].saved.to_query(&Default::default()).is_ok());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with("broken.toml could not be read: "),
            "{}",
            problems[0]
        );
        assert!(matches!(
            lists.load("Large videos"),
            Err(CollectionError::Live(name)) if name == "Large videos"
        ));
        assert!(
            lists
                .add("Large videos", [place("v/a.mkv")], &clock)
                .is_err()
        );
        assert!(lists.rename("Large videos", "Big").is_err());
        assert_eq!(fs::read_to_string(&search_path).unwrap(), before);
        assert_eq!(
            fs::read_to_string(lists.folder().join("broken.toml")).unwrap(),
            "name = [\n",
            "a broken file is left as it is"
        );
    }

    #[test]
    fn the_path_list_reader_serves_both_doors() {
        let list = read_path_list(
            &b"two words.txt\0\0notes\0\xff\xfe\0"[..],
            PathSeparator::Nul,
            PathLimits::default(),
        )
        .unwrap();
        assert_eq!(list.items.len(), 3);
        assert_eq!(list.items[0].text.as_deref(), Some("two words.txt"));
        assert_eq!(list.items[2].text, None);
        assert_eq!(list.items[2].what(), "item 3");
        assert!(matches!(
            read_path_list(&b"a\nb\n"[..], PathSeparator::Nul, PathLimits::default()),
            Err(PathListError::NoNul)
        ));
        assert_eq!(
            PathListError::NoNul.to_string(),
            "No NUL separators found. Use --lines."
        );
        assert!(matches!(
            read_path_list(&b"a\0b"[..], PathSeparator::Lines, PathLimits::default()),
            Err(PathListError::HasNul)
        ));
        let lines = read_path_list(
            &b"file2.txt\r\nfile10.txt\n\n"[..],
            PathSeparator::Lines,
            PathLimits::default(),
        )
        .unwrap();
        assert_eq!(lines.items.len(), 2);
        assert_eq!(lines.items[0].text.as_deref(), Some("file2.txt"));
        // One path with no separator at all is a list of one.
        let one =
            read_path_list(&b"file2.txt"[..], PathSeparator::Nul, PathLimits::default()).unwrap();
        assert_eq!(one.items.len(), 1);
        assert!(
            read_path_list(&b"\0\0"[..], PathSeparator::Nul, PathLimits::default())
                .unwrap()
                .is_empty()
        );
        let small = PathLimits {
            max_paths: 2,
            max_bytes: 8,
        };
        let error = read_path_list(&b"aaaa\0bbbb\0"[..], PathSeparator::Nul, small).unwrap_err();
        // 33 cells.
        assert_eq!(error.to_string(), "List is too long: over 8 bytes.");
        assert_eq!(
            PathListError::TooLong(size_words(PATH_LIST_MAX_BYTES)).to_string(),
            "List is too long: over 64 MiB."
        );
        let capped = PathLimits {
            max_paths: 2,
            max_bytes: 1024,
        };
        let list = read_path_list(&b"a\0b\0c\0"[..], PathSeparator::Nul, capped).unwrap();
        assert_eq!(list.items.len(), 2);
        assert!(list.stopped_at_limit);
    }

    #[test]
    fn paths_become_places_for_a_collection_lister() {
        let base = place("work");
        let input: &[u8] = if cfg!(windows) {
            b"D:\\a.txt\0b.txt\0D:\\a.txt\0\xff\xfe\0sub\\c.txt\0"
        } else {
            b"/a.txt\0b.txt\0/a.txt\0\xff\xfe\0sub/c.txt\0"
        };
        let list = read_path_list(input, PathSeparator::Nul, PathLimits::default()).unwrap();
        let read = list.places(&base);
        assert_eq!(read.places.len(), 3);
        assert_eq!(read.places[0], place("a.txt"));
        assert_eq!(read.places[1], base.join("b.txt").unwrap());
        assert_eq!(read.repeated, 1);
        assert_eq!(read.refused.len(), 1);
        assert_eq!(read.refused[0].0.what(), "item 4");
        assert_eq!(read.refused[0].1, "the name is not valid Unicode");
        // 28 cells, then the note.
        assert_eq!(
            read.words(),
            "3 paths from standard input. 1 could not be read"
        );
        let capped = read_path_list(
            input,
            PathSeparator::Nul,
            PathLimits {
                max_paths: 2,
                max_bytes: 1024,
            },
        )
        .unwrap()
        .places(&base);
        assert_eq!(
            capped.words(),
            "2 paths from standard input. Stopped at the limit of 2 paths"
        );
        assert_eq!(
            PathList::default().places(&base).words(),
            "no paths from standard input"
        );
    }
}
