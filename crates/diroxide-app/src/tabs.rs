//! Tabs (alpha 5, lane TB; research/modern-features.md, "Folder tabs"):
//! a panel holds a list of listers, one of them current, each with its
//! own place, sort, filter, columns, selection, cursor and history. This
//! module holds what the app's tab commands share and the frontends may
//! read: the words a tab is said with, and the saved tab sets.
//!
//! No tab bar is drawn: a tab's state is words. The tab number comes
//! after the name, as the list model's position comes last: "Documents,
//! tab 2 of 3". A lock is said after the number only when it holds:
//! "Documents, tab 2 of 3, locked".
//!
//! Saved tab sets live in the state folder as `tab-sets.toml`, each by
//! name, with both panels' tabs and which panel was active. The app holds
//! them in memory and asks the runtime to write them
//! ([`crate::Request::SaveTabSets`]); reading happens once, at start.

use std::fs;
use std::io;

use diroxide_config::{ConfigProblem, Loaded, Paths, TabLock, TabState, write_atomic};
use diroxide_core::Side;
use serde::{Deserialize, Serialize};

/// The saved tab sets' file in the state folder.
pub const FILE: &str = "tab-sets.toml";

/// The longest name a tab set may have, in characters, so its rows and
/// lines keep their meaning in a 40-cell line.
pub const NAME_LIMIT: usize = 24;

/// How many closed tabs Undo close tab can bring back, newest first.
pub const CLOSED_LIMIT: usize = 16;

/// A tab's position in words: "tab 2 of 3".
pub fn position_words(index: usize, count: usize) -> String {
    format!("tab {} of {}", index + 1, count.max(1))
}

/// A tab in words, name first: "Documents, tab 2 of 3", with its lock
/// after the number when one holds: "Documents, tab 2 of 3, locked".
pub fn tab_words(name: &str, index: usize, count: usize, lock: TabLock) -> String {
    let mut words = format!("{name}, {}", position_words(index, count));
    if let Some(lock) = lock.said() {
        words.push_str(&format!(", {lock}"));
    }
    words
}

/// Checks a tab set's name as typed: trimmed, not empty, at most
/// [`NAME_LIMIT`] characters, and no control characters. The error is
/// the reason in words, meaning first.
pub fn check_set_name(text: &str) -> Result<String, String> {
    let name = text.trim();
    if name.is_empty() {
        return Err("Type a name for the tab set.".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("A tab set's name cannot hold control characters.".to_owned());
    }
    if name.chars().count() > NAME_LIMIT {
        return Err(format!("Name too long: {NAME_LIMIT} characters at most."));
    }
    Ok(name.to_owned())
}

/// One saved tab set: its name, both panels' tabs, each panel's current
/// tab and the active panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabSet {
    /// The name it is saved and loaded by, kept inside the file, so the
    /// file's own name stays plain.
    pub name: String,
    /// The panel that was active.
    #[serde(default = "left")]
    pub active: Side,
    /// The left and right panels' tabs, in order.
    #[serde(default)]
    pub tabs: [Vec<TabState>; 2],
    /// Each panel's current tab's index.
    #[serde(default)]
    pub current: [usize; 2],
}

fn left() -> Side {
    Side::Left
}

impl TabSet {
    /// How many tabs the set holds, both panels together.
    pub fn tab_count(&self) -> usize {
        self.tabs.iter().map(Vec::len).sum()
    }

    /// One panel's tabs and its current one's index, kept in range;
    /// `None` when the set has no tab for it.
    pub fn tab_list(&self, side: Side) -> Option<(&[TabState], usize)> {
        let tabs = &self.tabs[side.index()];
        if tabs.is_empty() {
            return None;
        }
        Some((tabs, self.current[side.index()].min(tabs.len() - 1)))
    }

    /// The set in words for its row: "Work, 4 tabs".
    pub fn row_words(&self) -> String {
        format!(
            "{}, {}",
            self.name,
            diroxide_core::format::count(self.tab_count() as u64, "tab", "tabs")
        )
    }
}

/// The saved tab sets, in the order they were first saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabSets {
    #[serde(default, rename = "set")]
    sets: Vec<TabSet>,
}

impl TabSets {
    /// No sets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every set, in order.
    pub fn sets(&self) -> &[TabSet] {
        &self.sets
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// The set of this name, whatever its case.
    pub fn get(&self, name: &str) -> Option<&TabSet> {
        self.sets.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Saves a set, replacing one of the same name in its place; true
    /// when it replaced one.
    pub fn put(&mut self, set: TabSet) -> bool {
        match self
            .sets
            .iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(&set.name))
        {
            Some(old) => {
                *old = set;
                true
            }
            None => {
                self.sets.push(set);
                false
            }
        }
    }

    /// Forgets the set of this name; true when there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sets.len();
        self.sets.retain(|s| !s.name.eq_ignore_ascii_case(name));
        self.sets.len() != before
    }

    /// Reads the sets from `tab-sets.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// The sets as `tab-sets.toml` text.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }
}

/// Reads `tab-sets.toml` from the state folder: none when there is no
/// file; when it cannot be read, none, a copy kept as
/// `tab-sets.toml.bad`, and one sentence that says so.
pub fn load(paths: &Paths) -> Loaded<TabSets> {
    let path = paths.state_file(FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::clean(TabSets::new()),
        Err(e) => {
            return Loaded {
                value: TabSets::new(),
                problems: vec![ConfigProblem::new(
                    FILE,
                    format!("{FILE} could not be opened: {e}."),
                )],
            };
        }
    };
    match TabSets::from_toml(&text) {
        Ok(sets) => Loaded::clean(sets),
        Err(_) => {
            let backup = format!("{FILE}.bad");
            let kept = write_atomic(&paths.state_file(&backup), &text).is_ok();
            let sentence = if kept {
                format!(
                    "{FILE} could not be read. A copy is kept as {backup}; no tab sets are in use."
                )
            } else {
                format!("{FILE} could not be read. No tab sets are in use.")
            };
            Loaded {
                value: TabSets::new(),
                problems: vec![ConfigProblem::new(FILE, sentence)],
            }
        }
    }
}

/// Writes `tab-sets.toml` to the state folder atomically. The error is
/// the cause in words.
pub fn save(paths: &Paths, sets: &TabSets) -> Result<(), String> {
    let text = sets.to_toml().map_err(|e| e.to_string())?;
    write_atomic(&paths.state_file(FILE), &text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::Place;

    fn folder(name: &str) -> Place {
        let text = if cfg!(windows) {
            format!(r"D:\{name}")
        } else {
            format!("/{name}")
        };
        Place::parse(&text).unwrap()
    }

    #[test]
    fn a_tab_is_said_name_first_with_its_lock_only_when_held() {
        let free = tab_words("Documents", 1, 3, TabLock::Free);
        assert_eq!(free, "Documents, tab 2 of 3");
        assert_eq!(free.chars().count(), 21);
        let locked = tab_words("Documents", 1, 3, TabLock::Locked);
        assert_eq!(locked, "Documents, tab 2 of 3, locked");
        assert_eq!(locked.chars().count(), 29);
        let pinned = tab_words("Downloads", 0, 2, TabLock::Pinned);
        assert_eq!(pinned, "Downloads, tab 1 of 2, pinned");
        assert_eq!(pinned.chars().count(), 29);
        assert_eq!(position_words(0, 0), "tab 1 of 1");
    }

    #[test]
    fn a_set_name_is_checked_in_words() {
        assert_eq!(check_set_name("  Work  ").unwrap(), "Work");
        assert_eq!(
            check_set_name(" ").unwrap_err(),
            "Type a name for the tab set."
        );
        let long = check_set_name(&"x".repeat(25)).unwrap_err();
        assert_eq!(long, "Name too long: 24 characters at most.");
        assert!(long.chars().count() <= 40);
        assert!(check_set_name("a\u{7}b").is_err());
    }

    #[test]
    fn sets_are_kept_by_name_and_read_back() {
        let tab = |name: &str| TabState {
            place: Some(folder(name)),
            ..TabState::default()
        };
        let set = TabSet {
            name: "Work".to_owned(),
            active: Side::Right,
            tabs: [
                vec![tab("docs"), tab("mail"), tab("notes")],
                vec![tab("build")],
            ],
            current: [1, 7],
        };
        assert_eq!(set.tab_count(), 4);
        assert_eq!(set.tab_list(Side::Right).unwrap().1, 0, "kept in range");
        let row = set.row_words();
        assert_eq!(row, "Work, 4 tabs");
        assert_eq!(row.chars().count(), 12);
        let mut sets = TabSets::new();
        assert!(!sets.put(set.clone()));
        let mut again = set.clone();
        again.name = "work".to_owned();
        assert!(sets.put(again), "the same name in another case replaces");
        assert_eq!(sets.sets().len(), 1);
        assert!(sets.get("WORK").is_some());

        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        assert_eq!(load(&paths), Loaded::clean(TabSets::new()));
        save(&paths, &sets).unwrap();
        assert_eq!(load(&paths), Loaded::clean(sets.clone()));
        assert!(sets.remove("Work"));
        assert!(sets.is_empty());

        fs::write(paths.state_file(FILE), "set = [\n").unwrap();
        let bad = load(&paths);
        assert!(bad.value.is_empty());
        assert_eq!(
            bad.problems[0].text,
            "tab-sets.toml could not be read. A copy is kept as tab-sets.toml.bad; no tab sets are in use."
        );
        assert!(paths.state_file("tab-sets.toml.bad").exists());
    }
}
