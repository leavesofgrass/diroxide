//! Bookmarks: places the owner keeps (plan section 2, `research/
//! alpha3-plan.md`, "Labels, bookmarks, history, per-folder views"). Each
//! has a name, its place, a category, an alias letter for the jump, how
//! often and when it was last used, a sticky flag that keeps it at the top
//! of the list, and whether it is an item bookmark (going there opens the
//! item's folder with the item focused).
//!
//! The list reads meaning first: "Reports, bookmark, in work, used 12
//! times", or "Reports, alias r, in work" when the bookmark has an alias.
//! Its first row follows the hotlist rule: "Add this folder" when the
//! active panel's folder is not bookmarked, "Remove this folder" when it
//! is. The store is `bookmarks.toml` in the state folder, written
//! atomically on every change. The filter language's `bookmarked` fact
//! reads it through [`BookmarkLookup`].
//!
//! W3l owns this module. The app's commands are in `app::bookmarking`.

use diroxide_config::{ConfigProblem, Loaded, Paths};
use diroxide_core::filter::Lookup;
use diroxide_core::format::count;
use diroxide_core::{Entry, ListModel, ListRow, Place, Timestamp};
use serde::{Deserialize, Serialize};

use crate::labels::{place_key, state_file};
use crate::panel::place_leaf;

/// The bookmarks file in the state folder.
pub const FILE: &str = "bookmarks.toml";

/// The version of `bookmarks.toml` this build writes.
pub const FORMAT: u32 = 1;

/// The longest bookmark name or category, so a row keeps its meaning in
/// 40 cells.
pub const NAME_LIMIT: usize = 30;

/// The id of the list's first row, Add or Remove this folder.
pub const THIS_FOLDER_ROW: &str = "this-folder";

/// One bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    /// The name said first: the folder's own name unless changed.
    pub name: String,
    /// The place.
    pub place: Place,
    /// A category, or empty for none.
    pub category: String,
    /// The alias for the jump: one letter or digit, lowercase.
    pub alias: Option<char>,
    /// Kept at the top of the list.
    pub sticky: bool,
    /// An item bookmark: going there opens its folder, the item focused.
    pub item: bool,
    /// How many times it was used.
    pub uses: u32,
    /// When it was last used, in seconds since the epoch.
    pub last_used: Option<i64>,
}

impl Bookmark {
    /// A bookmark for a place, named after it, never used.
    pub fn new(place: Place, item: bool) -> Self {
        Self {
            name: place_leaf(&place),
            place,
            category: String::new(),
            alias: None,
            sticky: false,
            item,
            uses: 0,
            last_used: None,
        }
    }

    /// Where it is, in a few words: "in work", the folder above it.
    pub fn where_words(&self) -> String {
        match self.place.parent() {
            Some(parent) => format!("in {}", place_leaf(&parent)),
            None => "a top place".to_owned(),
        }
    }

    /// How often it was used: "used 12 times", "used once", "not used
    /// yet".
    pub fn uses_words(&self) -> String {
        match self.uses {
            0 => "not used yet".to_owned(),
            1 => "used once".to_owned(),
            n => format!("used {} times", n),
        }
    }

    /// Its row in the Bookmarks list, the meaning in the first 40 cells:
    /// "Reports, bookmark, in work, used 12 times", or "Reports, alias r,
    /// in work, used 12 times"; then the category, sticky, item, and the
    /// whole place last.
    pub fn row(&self) -> ListRow {
        let kind = match self.alias {
            Some(alias) => format!("alias {alias}"),
            None => "bookmark".to_owned(),
        };
        let mut details = vec![kind, self.where_words(), self.uses_words()];
        if !self.category.is_empty() {
            details.push(format!("category {}", self.category));
        }
        if self.sticky {
            details.push("sticky".to_owned());
        }
        if self.item {
            details.push("opens its folder on the item".to_owned());
        }
        details.push(display(&self.place));
        ListRow::new(self.place.to_string(), self.name.clone()).with_details(details)
    }

    /// What adding it says: "Bookmarked Reports, in work".
    pub fn added_sentence(&self) -> String {
        format!("Bookmarked {}, {}", self.name, self.where_words())
    }
}

/// A place as said, with hidden characters shown safely.
fn display(place: &Place) -> String {
    diroxide_core::names::display_name(&place.to_string()).into_owned()
}

/// Checks a bookmark name or a category: trimmed, at most
/// [`NAME_LIMIT`] characters, no control characters; a category may be
/// empty, a name may not. The error is a sentence.
pub fn check_text(text: &str, what: &str, may_be_empty: bool) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() && !may_be_empty {
        return Err(format!("A bookmark needs a {what}."));
    }
    if text.chars().count() > NAME_LIMIT {
        return Err(format!("A {what} is at most {NAME_LIMIT} characters."));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("A {what} cannot hold control characters."));
    }
    Ok(text.to_owned())
}

/// Reads an alias: one letter or digit, lowercase; empty is none. The
/// error is a sentence.
pub fn check_alias(text: &str) -> Result<Option<char>, String> {
    let text = text.trim();
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (None, _) => Ok(None),
        (Some(c), None) if c.is_alphanumeric() => Ok(c.to_lowercase().next()),
        _ => Err("An alias is one letter or digit.".to_owned()),
    }
}

/// The bookmarks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmarks {
    list: Vec<Bookmark>,
}

impl Bookmarks {
    /// No bookmarks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every bookmark, in the order they were added.
    pub fn all(&self) -> &[Bookmark] {
        &self.list
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The bookmarks as the list shows them: sticky ones first, then the
    /// most recently used, then those never used by name.
    pub fn ordered(&self) -> Vec<&Bookmark> {
        let mut ordered: Vec<&Bookmark> = self.list.iter().collect();
        ordered.sort_by(|a, b| {
            b.sticky
                .cmp(&a.sticky)
                .then_with(|| b.last_used.cmp(&a.last_used))
                .then_with(|| crate::panel::natural_cmp(&a.name, &b.name))
        });
        ordered
    }

    /// The bookmark for a place, the same way the place's file system
    /// compares names.
    pub fn find(&self, place: &Place) -> Option<&Bookmark> {
        let key = place_key(place);
        self.list.iter().find(|b| place_key(&b.place) == key)
    }

    fn index_of(&self, place: &Place) -> Option<usize> {
        let key = place_key(place);
        self.list.iter().position(|b| place_key(&b.place) == key)
    }

    /// Whether a place is bookmarked.
    pub fn is_bookmarked(&self, place: &Place) -> bool {
        self.find(place).is_some()
    }

    /// The bookmark with an alias.
    pub fn by_alias(&self, alias: char) -> Option<&Bookmark> {
        let alias = alias.to_lowercase().next()?;
        self.list.iter().find(|b| b.alias == Some(alias))
    }

    /// Adds a bookmark; a place already bookmarked is refused in words.
    pub fn add(&mut self, bookmark: Bookmark) -> Result<&Bookmark, String> {
        if let Some(there) = self.find(&bookmark.place) {
            return Err(format!(
                "Already bookmarked: {}, {}",
                there.name,
                there.where_words()
            ));
        }
        self.list.push(bookmark);
        let last = self.list.len() - 1;
        Ok(&self.list[last])
    }

    /// Takes a place's bookmark out; returns it.
    pub fn remove(&mut self, place: &Place) -> Option<Bookmark> {
        let index = self.index_of(place)?;
        Some(self.list.remove(index))
    }

    /// Notes a use of a place's bookmark: its count and its time.
    pub fn used(&mut self, place: &Place, now: Timestamp) -> bool {
        match self.index_of(place) {
            Some(index) => {
                let bookmark = &mut self.list[index];
                bookmark.uses = bookmark.uses.saturating_add(1);
                bookmark.last_used = Some(now.unix_seconds());
                true
            }
            None => false,
        }
    }

    /// Changes a place's bookmark with a function, checking the alias
    /// stays unique; the error is a sentence.
    pub fn edit(
        &mut self,
        place: &Place,
        change: impl FnOnce(&mut Bookmark),
    ) -> Result<&Bookmark, String> {
        let index = self
            .index_of(place)
            .ok_or_else(|| "That bookmark is gone.".to_owned())?;
        let mut edited = self.list[index].clone();
        change(&mut edited);
        if let Some(alias) = edited.alias
            && let Some(other) = self
                .list
                .iter()
                .enumerate()
                .find(|(i, b)| *i != index && b.alias == Some(alias))
                .map(|(_, b)| b)
        {
            return Err(format!("Alias {alias} is taken by {}.", other.name));
        }
        self.list[index] = edited;
        Ok(&self.list[index])
    }

    /// The categories in use, by name.
    pub fn categories(&self) -> Vec<String> {
        let mut categories: Vec<String> = self
            .list
            .iter()
            .filter(|b| !b.category.is_empty())
            .map(|b| b.category.clone())
            .collect();
        categories.sort_by(|a, b| crate::panel::natural_cmp(a, b));
        categories.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        categories
    }

    /// The Bookmarks list for a panel showing `here`: the first row by the
    /// hotlist rule ("Add this folder", or "Remove this folder" when it is
    /// bookmarked), then every bookmark, sticky ones first. A place inside
    /// an archive can be bookmarked too.
    pub fn list(&self, here: &Place) -> ListModel {
        let first = match self.find(here) {
            Some(bookmark) => ListRow::new(THIS_FOLDER_ROW, "Remove this folder")
                .with_details([bookmark.name.clone(), bookmark.where_words()]),
            None => ListRow::new(THIS_FOLDER_ROW, "Add this folder")
                .with_details([place_leaf(here), bookmark_where(here)]),
        };
        let mut rows = vec![first];
        rows.extend(self.ordered().into_iter().map(Bookmark::row));
        let mut list = ListModel::new(format!(
            "Bookmarks, {}",
            count(self.list.len() as u64, "bookmark", "bookmarks")
        ))
        .with_rows(rows);
        list.set_empty_text("No bookmarks");
        list
    }

    /// Reads `bookmarks.toml` text; a place that cannot be read, a second
    /// bookmark of one place and an alias used twice are left out and
    /// said.
    pub fn from_toml(text: &str) -> Result<(Self, Vec<String>), toml::de::Error> {
        let file: BookmarksFile = toml::from_str(text)?;
        let mut bookmarks = Self::new();
        let mut problems = Vec::new();
        for line in file.bookmark {
            let Ok(place) = Place::parse(&line.place) else {
                problems.push(format!("A bookmark's place cannot be read: {}", line.place));
                continue;
            };
            let mut bookmark = Bookmark::new(place, line.item);
            if let Ok(name) = check_text(&line.name, "name", false) {
                bookmark.name = name;
            }
            bookmark.category = check_text(&line.category, "category", true).unwrap_or_default();
            bookmark.alias = match check_alias(&line.alias) {
                Ok(Some(alias)) if bookmarks.by_alias(alias).is_some() => {
                    problems.push(format!(
                        "Alias {alias} is used twice; {} keeps none.",
                        bookmark.name
                    ));
                    None
                }
                Ok(alias) => alias,
                Err(_) => {
                    problems.push(format!(
                        "Bookmark {}: an alias is one letter or digit.",
                        bookmark.name
                    ));
                    None
                }
            };
            bookmark.sticky = line.sticky;
            bookmark.uses = line.uses;
            bookmark.last_used = (line.last_used > 0).then_some(line.last_used);
            if let Err(why) = bookmarks.add(bookmark) {
                problems.push(why);
            }
        }
        Ok((bookmarks, problems))
    }

    /// The bookmarks as `bookmarks.toml` text, in the order added.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let file = BookmarksFile {
            format: FORMAT,
            bookmark: self
                .list
                .iter()
                .map(|b| BookmarkLine {
                    name: b.name.clone(),
                    place: b.place.to_string(),
                    category: b.category.clone(),
                    alias: b.alias.map(String::from).unwrap_or_default(),
                    sticky: b.sticky,
                    item: b.item,
                    uses: b.uses,
                    last_used: b.last_used.unwrap_or(0),
                })
                .collect(),
        };
        toml::to_string(&file)
    }
}

/// Where a place is, as a bookmark of it would say: "in work".
fn bookmark_where(place: &Place) -> String {
    Bookmark::new(place.clone(), false).where_words()
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct BookmarksFile {
    format: u32,
    bookmark: Vec<BookmarkLine>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct BookmarkLine {
    name: String,
    place: String,
    category: String,
    alias: String,
    sticky: bool,
    item: bool,
    uses: u32,
    /// Seconds since the epoch; 0 is never.
    last_used: i64,
}

/// Reads `bookmarks.toml` from the state folder: none when there is no
/// file; when it cannot be read, none, a copy kept as
/// `bookmarks.toml.bad`, and one sentence that says so.
pub fn load(paths: &Paths) -> Loaded<Bookmarks> {
    match state_file::read(paths, FILE) {
        state_file::Read::Missing => Loaded::clean(Bookmarks::new()),
        state_file::Read::Failed(problem) => Loaded {
            value: Bookmarks::new(),
            problems: vec![problem],
        },
        state_file::Read::Text(text) => match Bookmarks::from_toml(&text) {
            Ok((bookmarks, problems)) => Loaded {
                value: bookmarks,
                problems: problems
                    .into_iter()
                    .map(|p| ConfigProblem::new(FILE, p))
                    .collect(),
            },
            Err(error) => Loaded {
                value: Bookmarks::new(),
                problems: vec![state_file::unreadable(
                    paths,
                    FILE,
                    &text,
                    &error,
                    "no bookmarks are in use",
                )],
            },
        },
    }
}

/// Writes `bookmarks.toml` to the state folder atomically. The error is
/// the cause in words.
pub fn save(paths: &Paths, bookmarks: &Bookmarks) -> Result<(), String> {
    let text = bookmarks.to_toml().map_err(|e| e.to_string())?;
    state_file::write(paths, FILE, &text)
}

/// The filter language's facts that need the bookmark store, for a
/// listing of one folder: an entry's whole path, and whether it is
/// bookmarked. The other facts answer "not known here"; the wiring lane
/// joins this with the panel's own (depth, selection).
#[derive(Clone, Copy, Debug)]
pub struct BookmarkLookup<'a> {
    folder: &'a Place,
    bookmarks: &'a Bookmarks,
}

impl<'a> BookmarkLookup<'a> {
    /// The facts for entries listed in `folder`.
    pub fn new(folder: &'a Place, bookmarks: &'a Bookmarks) -> Self {
        Self { folder, bookmarks }
    }

    /// Whether an entry of the folder is bookmarked.
    pub fn is_bookmarked(&self, entry: &Entry) -> Option<bool> {
        self.folder
            .join(&entry.name)
            .ok()
            .map(|place| self.bookmarks.is_bookmarked(&place))
    }
}

impl Lookup for BookmarkLookup<'_> {
    fn path(&self, entry: &Entry) -> Option<String> {
        self.folder.join(&entry.name).ok().map(|p| p.to_string())
    }

    fn bookmarked(&self, entry: &Entry) -> Option<bool> {
        self.is_bookmarked(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::{EntryKind, UtcOffset};

    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    fn now() -> Timestamp {
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap()
    }

    #[test]
    fn rows_say_the_name_the_kind_and_the_place_first() {
        let mut reports = Bookmark::new(p("/work/Reports"), false);
        reports.uses = 12;
        assert!(
            reports
                .row()
                .spoken()
                .starts_with("Reports, bookmark, in work, used 12 times")
        );
        assert!(within_cells(
            &reports.row().spoken(),
            "Reports, bookmark, in work"
        ));
        reports.alias = Some('r');
        assert!(
            reports
                .row()
                .spoken()
                .starts_with("Reports, alias r, in work")
        );
        assert_eq!(reports.added_sentence(), "Bookmarked Reports, in work");
        let fresh = Bookmark::new(p("/work/Fresh"), false);
        assert!(fresh.row().spoken().contains("not used yet"));
    }

    #[test]
    fn the_first_row_follows_the_hotlist_rule() {
        let mut bookmarks = Bookmarks::new();
        let here = p("/work/Reports");
        let list = bookmarks.list(&here);
        assert_eq!(list.rows()[0].label, "Add this folder");
        assert_eq!(list.title(), "Bookmarks, no bookmarks");
        bookmarks.add(Bookmark::new(here.clone(), false)).unwrap();
        let list = bookmarks.list(&here);
        assert_eq!(list.rows()[0].label, "Remove this folder");
        assert_eq!(list.title(), "Bookmarks, 1 bookmark");
        assert_eq!(list.len(), 2);
        let elsewhere = bookmarks.list(&p("/work"));
        assert_eq!(elsewhere.rows()[0].label, "Add this folder");
        assert!(bookmarks.add(Bookmark::new(here.clone(), false)).is_err());
    }

    #[test]
    fn sticky_ones_come_first_then_the_latest_used() {
        let mut bookmarks = Bookmarks::new();
        for name in ["/a", "/b", "/c"] {
            bookmarks.add(Bookmark::new(p(name), false)).unwrap();
        }
        bookmarks.used(&p("/b"), now());
        bookmarks.edit(&p("/c"), |b| b.sticky = true).unwrap();
        let order: Vec<&str> = bookmarks
            .ordered()
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(order, ["c", "b", "a"]);
        assert_eq!(bookmarks.find(&p("/b")).unwrap().uses, 1);
    }

    #[test]
    fn aliases_are_one_letter_and_unique() {
        let mut bookmarks = Bookmarks::new();
        bookmarks.add(Bookmark::new(p("/a"), false)).unwrap();
        bookmarks.add(Bookmark::new(p("/b"), false)).unwrap();
        assert_eq!(check_alias("R"), Ok(Some('r')));
        assert_eq!(check_alias(""), Ok(None));
        assert!(check_alias("rr").is_err());
        assert!(check_alias("!").is_err());
        bookmarks.edit(&p("/a"), |b| b.alias = Some('r')).unwrap();
        let taken = bookmarks.edit(&p("/b"), |b| b.alias = Some('r'));
        assert_eq!(taken.unwrap_err(), "Alias r is taken by a.");
        assert_eq!(bookmarks.by_alias('R').unwrap().name, "a");
        assert!(check_text("", "name", false).is_err());
        assert_eq!(check_text(" work ", "category", true).unwrap(), "work");
    }

    #[test]
    fn the_file_round_trips_and_says_what_it_left_out() {
        let mut bookmarks = Bookmarks::new();
        let mut reports = Bookmark::new(p("/work/Reports"), false);
        reports.alias = Some('r');
        reports.category = "work".into();
        reports.uses = 3;
        reports.last_used = Some(now().unix_seconds());
        bookmarks.add(reports).unwrap();
        bookmarks
            .add(Bookmark::new(p("/work/notes.txt"), true))
            .unwrap();
        let text = bookmarks.to_toml().unwrap();
        let (read, problems) = Bookmarks::from_toml(&text).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(read, bookmarks);
        assert_eq!(read.categories(), ["work"]);
        let doubled = format!("{text}\n[[bookmark]]\nplace = \"\"\n");
        let (_, problems) = Bookmarks::from_toml(&doubled).unwrap();
        assert!(problems[0].starts_with("A bookmark's place cannot be read"));
    }

    #[test]
    fn the_store_round_trips_through_an_injected_state_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        assert!(load(&paths).value.is_empty());
        let mut bookmarks = Bookmarks::new();
        bookmarks
            .add(Bookmark::new(p("/work/Reports"), false))
            .unwrap();
        save(&paths, &bookmarks).unwrap();
        assert_eq!(load(&paths).value, bookmarks);
    }

    #[test]
    fn the_lookup_answers_bookmarked_for_the_filter_language() {
        let mut bookmarks = Bookmarks::new();
        bookmarks
            .add(Bookmark::new(p("/work/Reports"), false))
            .unwrap();
        let folder = p("/work");
        let lookup = BookmarkLookup::new(&folder, &bookmarks);
        let reports = Entry::new("Reports", EntryKind::Folder);
        let notes = Entry::new("notes.txt", EntryKind::File);
        assert_eq!(lookup.bookmarked(&reports), Some(true));
        assert_eq!(lookup.bookmarked(&notes), Some(false));
        assert!(lookup.path(&reports).unwrap().ends_with("Reports"));
    }
}
