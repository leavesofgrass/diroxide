//! What the app remembers between runs, in the state folder: each
//! prompt's history (plan section 5.19), newest first, one file per prompt
//! under `history/`; and the path jump store, the folders visited with how
//! often and how lately, so the go-to-place list and path jump rank them.
//!
//! Nothing here is ever a secret: the masked prompt keeps no history
//! (ADR-0009). The app holds these in memory and asks the runtime to write
//! them ([`crate::Request::SaveHistory`]); reading happens once, at start.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use diroxide_config::{Paths, write_atomic};
use diroxide_core::Place;
use serde::{Deserialize, Serialize};

/// The most entries a prompt's history keeps.
pub const HISTORY_LIMIT: usize = 100;

/// The history key of the path jump store.
pub const VISITS_KEY: &str = "visits";

/// The history keys of the prompts, each a plain file name.
pub mod keys {
    /// Go to a path.
    pub const GO_PATH: &str = "go-path";
    /// Go to a place, and path jump.
    pub const PLACES: &str = "places";
    /// Rename.
    pub const RENAME: &str = "rename";
    /// Make folder.
    pub const MAKE_FOLDER: &str = "make-folder";
    /// Duplicate.
    pub const DUPLICATE: &str = "duplicate";
    /// The name filter.
    pub const FILTER: &str = "filter";
    /// Select and unselect by pattern.
    pub const PATTERN: &str = "select-pattern";
    /// The run-command prompt.
    pub const RUN_COMMAND: &str = "run-command";
    /// Copy or move to a place: the recent destinations.
    pub const DESTINATIONS: &str = "destinations";
    /// The palette's recent commands.
    pub const PALETTE: &str = "palette";
}

#[derive(Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<String>,
}

/// Whether a key is a plain file name: lowercase letters, digits and `-`.
fn plain_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn history_path(paths: &Paths, key: &str) -> Option<PathBuf> {
    plain_key(key).then(|| paths.history_dir().join(format!("{key}.toml")))
}

/// Writes a prompt's history, newest first, atomically. The error is the
/// cause in words.
pub fn save_history(paths: &Paths, key: &str, entries: &[String]) -> Result<(), String> {
    let path = history_path(paths, key).ok_or_else(|| format!("{key} is not a plain name"))?;
    let file = HistoryFile {
        entries: entries.iter().take(HISTORY_LIMIT).cloned().collect(),
    };
    let text = toml::to_string(&file).map_err(|e| e.to_string())?;
    write_atomic(&path, &text).map_err(|e| e.to_string())
}

/// Reads a prompt's history, newest first; empty when there is none or the
/// file cannot be read.
pub fn load_history(paths: &Paths, key: &str) -> Vec<String> {
    history_path(paths, key)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| toml::from_str::<HistoryFile>(&text).ok())
        .map(|file| file.entries)
        .unwrap_or_default()
}

/// Reads every prompt's history the app knows, by key.
pub fn load_histories(paths: &Paths) -> BTreeMap<String, Vec<String>> {
    [
        keys::GO_PATH,
        keys::PLACES,
        keys::RENAME,
        keys::MAKE_FOLDER,
        keys::DUPLICATE,
        keys::FILTER,
        keys::PATTERN,
        keys::RUN_COMMAND,
        keys::DESTINATIONS,
        keys::PALETTE,
        VISITS_KEY,
    ]
    .into_iter()
    .map(|key| (key.to_owned(), load_history(paths, key)))
    .filter(|(_, entries)| !entries.is_empty())
    .collect()
}

/// Puts an entry first in a history, once, and keeps the limit.
pub fn remember(history: &mut Vec<String>, entry: &str) {
    if entry.is_empty() {
        return;
    }
    history.retain(|e| e != entry);
    history.insert(0, entry.to_owned());
    history.truncate(HISTORY_LIMIT);
}

/// One folder of the path jump store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    /// The folder.
    pub place: Place,
    /// How many times it was opened.
    pub count: u32,
    /// When it was last opened, in seconds since the epoch.
    pub last: i64,
}

/// The path jump store: folders by how often and how lately they were
/// visited (zoxide's rule, R13), kept as a history of lines
/// "count last place" so it shares the history files' shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visits {
    visits: Vec<Visit>,
}

/// The most folders the store keeps.
pub const VISITS_LIMIT: usize = 500;

impl Visits {
    /// The store from its history lines; lines that cannot be read are
    /// dropped.
    pub fn from_lines(lines: &[String]) -> Self {
        let visits = lines
            .iter()
            .filter_map(|line| {
                let mut parts = line.splitn(3, ' ');
                let count = parts.next()?.parse().ok()?;
                let last = parts.next()?.parse().ok()?;
                let place = Place::parse(parts.next()?).ok()?;
                Some(Visit { place, count, last })
            })
            .collect();
        Self { visits }
    }

    /// The store as history lines, most recent first.
    pub fn to_lines(&self) -> Vec<String> {
        let mut visits = self.visits.clone();
        visits.sort_by_key(|v| std::cmp::Reverse(v.last));
        visits
            .iter()
            .map(|v| format!("{} {} {}", v.count, v.last, v.place))
            .collect()
    }

    /// Notes a visit. The home folder is not kept (R13: it is always one
    /// key away).
    pub fn record(&mut self, place: &Place, now: i64, home: Option<&Place>) {
        if home == Some(place) {
            return;
        }
        match self.visits.iter_mut().find(|v| &v.place == place) {
            Some(visit) => {
                visit.count = visit.count.saturating_add(1);
                visit.last = now;
            }
            None => self.visits.push(Visit {
                place: place.clone(),
                count: 1,
                last: now,
            }),
        }
        if self.visits.len() > VISITS_LIMIT {
            self.visits.sort_by_key(|v| std::cmp::Reverse(v.last));
            self.visits.truncate(VISITS_LIMIT);
        }
    }

    /// Every folder, best first: the score is the count weighted by how
    /// recent the last visit is (within the hour four times, the day
    /// twice, the week as is, older a quarter).
    pub fn ranked(&self, now: i64) -> Vec<&Visit> {
        let score = |v: &Visit| {
            let age = now - v.last;
            let weight = if age < 3600 {
                16
            } else if age < 86_400 {
                8
            } else if age < 604_800 {
                4
            } else {
                1
            };
            u64::from(v.count) * weight
        };
        let mut ranked: Vec<&Visit> = self.visits.iter().collect();
        ranked.sort_by(|a, b| score(b).cmp(&score(a)).then_with(|| b.last.cmp(&a.last)));
        ranked
    }

    /// The folders that match typed terms, best first (path jump, R13):
    /// every term appears in the path in order, ignoring case, and the last
    /// term in the last folder's name.
    pub fn matching(&self, terms: &str, now: i64) -> Vec<&Visit> {
        let terms: Vec<String> = terms.split_whitespace().map(str::to_lowercase).collect();
        self.ranked(now)
            .into_iter()
            .filter(|visit| jump_matches(&visit.place, &terms))
            .collect()
    }

    /// How many folders are kept.
    pub fn len(&self) -> usize {
        self.visits.len()
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }
}

/// Whether a place matches path jump terms: each term after the last one
/// matched, the last term within the place's own name.
pub fn jump_matches(place: &Place, terms: &[String]) -> bool {
    let Some(last) = terms.last() else {
        return true;
    };
    let path = place.to_string().to_lowercase();
    let mut from = 0;
    for term in terms {
        match path[from..].find(term.as_str()) {
            Some(at) => from += at + term.len(),
            None => return false,
        }
    }
    place
        .name()
        .is_some_and(|name| name.to_lowercase().contains(last.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A place in the platform's own style: "/home/ada" is "D:\home\ada"
    /// on Windows.
    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    #[test]
    fn histories_keep_the_newest_first_once() {
        let mut history = vec!["b".to_owned(), "a".to_owned()];
        remember(&mut history, "a");
        assert_eq!(history, ["a", "b"]);
        remember(&mut history, "");
        assert_eq!(history.len(), 2);
        assert!(plain_key("go-path"));
        assert!(!plain_key("../x"));
        assert!(!plain_key("Go"));
    }

    #[test]
    fn visits_rank_by_count_and_how_recent() {
        let mut visits = Visits::default();
        let home = p("/home/ada");
        visits.record(&home, 100, Some(&home));
        assert!(visits.is_empty());
        let work = p("/home/ada/work");
        let reports = p("/home/ada/work/reports");
        visits.record(&work, 100, Some(&home));
        visits.record(&reports, 100, Some(&home));
        visits.record(&reports, 200, Some(&home));
        let ranked: Vec<&Place> = visits.ranked(300).iter().map(|v| &v.place).collect();
        assert_eq!(ranked, [&reports, &work]);
        let found: Vec<&Place> = visits
            .matching("work rep", 300)
            .iter()
            .map(|v| &v.place)
            .collect();
        assert_eq!(found, [&reports]);
        assert!(visits.matching("rep work", 300).is_empty());
        let lines = visits.to_lines();
        assert_eq!(Visits::from_lines(&lines).to_lines(), lines);
        assert_eq!(lines[0], format!("2 200 {reports}"));
    }
}
