//! Recent files: the files opened or edited from diroxide, newest first,
//! at most [`RECENT_LIMIT`], in `recent.toml` in the state folder,
//! written atomically. Nothing is kept when the `recent_files` setting is
//! off, and a list left from before is emptied then. The list never
//! leaves the machine.

use std::collections::BTreeMap;

use diroxide_core::clock::describe_time;
use diroxide_core::format::grouped;
use diroxide_core::names::display_name;
use diroxide_core::{Clock, ListModel, ListRow, Place, Timestamp, UtcOffset};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::store::{self, ConfigProblem, Loaded, ReadText};

/// The recent files' file in the state folder.
pub const RECENT_FILE: &str = "recent.toml";

/// The most recent files kept.
pub const RECENT_LIMIT: usize = 50;

/// The format number the recent files' file is written with.
pub const RECENT_FORMAT: u32 = 1;

/// A moment as the file writes it, in UTC: `2026-10-17T14:30:00Z`.
fn utc_text(time: Timestamp) -> String {
    let c = time.to_civil(UtcOffset::default());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// The moment of a [`utc_text`] value, or `None` when it does not read.
fn utc_moment(text: &str) -> Option<Timestamp> {
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

/// A place's own name as said, or the whole place for a drive or share.
fn place_name(place: &Place) -> String {
    match place.name() {
        Some(name) => display_name(name).into_owned(),
        None => display_name(&place.to_string()).into_owned(),
    }
}

/// How a recent file was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecentAction {
    /// Opened with its program or the viewer.
    Opened,
    /// Opened in an editor.
    Edited,
}

impl RecentAction {
    /// The word: "opened", "edited".
    pub const fn word(self) -> &'static str {
        match self {
            RecentAction::Opened => "opened",
            RecentAction::Edited => "edited",
        }
    }
}

/// One recent file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentItem {
    /// Where it is.
    pub place: Place,
    /// How it was used last.
    pub action: RecentAction,
    /// When, in UTC: `2026-11-12T10:00:00Z`.
    pub when: String,
}

impl RecentItem {
    /// When it was used, or `None` when the file's text does not read.
    pub fn time(&self) -> Option<Timestamp> {
        utc_moment(&self.when)
    }

    /// The row, keyed by the place: "report.pdf, edited today, 10:00 AM,
    /// in D:\docs".
    pub fn row(&self, clock: &dyn Clock) -> ListRow {
        let mut details = Vec::new();
        match self.time() {
            Some(time) => details.push(format!(
                "{} {}",
                self.action.word(),
                describe_time(time, clock)
            )),
            None => details.push(self.action.word().to_owned()),
        }
        if let Some(parent) = self.place.parent() {
            details.push(format!("in {}", display_name(&parent.to_string())));
        }
        ListRow::new(self.place.to_string(), place_name(&self.place)).with_details(details)
    }
}

/// The files opened or edited from diroxide, newest first, at most
/// [`RECENT_LIMIT`]. Local only: the list never leaves the machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentFiles {
    /// The file's format number.
    pub format: u32,
    /// The files, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<RecentItem>,
    /// Keys this version does not know, kept as they were.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for RecentFiles {
    fn default() -> Self {
        Self {
            format: RECENT_FORMAT,
            items: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl RecentFiles {
    /// Whether recent files are kept: the `recent_files` setting.
    pub fn enabled(settings: &Settings) -> bool {
        settings.recent_files
    }

    /// Reads `recent.toml` from the state folder. A missing file is an
    /// empty list; a file that cannot be read is copied to
    /// `recent.toml.bad`, said in one sentence, and the list starts empty.
    pub fn load(paths: &Paths) -> Loaded<Self> {
        let path = paths.state_file(RECENT_FILE);
        match store::read_text(&path, RECENT_FILE) {
            ReadText::Missing => Loaded::clean(Self::default()),
            ReadText::Failed(problem) => Loaded {
                value: Self::default(),
                problems: vec![problem],
            },
            ReadText::Text(text) => match store::parse_typed::<Self>(RECENT_FILE, &text) {
                Ok(recent) => Loaded::clean(recent),
                Err(failure) => {
                    let backup = store::back_up(&path, &text);
                    Loaded {
                        value: Self::default(),
                        problems: vec![ConfigProblem::new(RECENT_FILE, failure.sentence(&backup))],
                    }
                }
            },
        }
    }

    /// Writes `recent.toml` to the state folder atomically.
    pub fn save(&self, paths: &Paths) -> Result<(), ConfigError> {
        let text = toml::to_string(self).map_err(|error| ConfigError::parse(RECENT_FILE, error))?;
        store::write_atomic(&paths.state_file(RECENT_FILE), &text)
            .map_err(|error| ConfigError::io(RECENT_FILE, &error))
    }

    /// Puts a file at the top, once, and drops the oldest past
    /// [`RECENT_LIMIT`]. Nothing is kept when `enabled` is false. Returns
    /// whether it was kept.
    pub fn record(
        &mut self,
        place: Place,
        action: RecentAction,
        now: Timestamp,
        enabled: bool,
    ) -> bool {
        if !enabled {
            return false;
        }
        self.items.retain(|item| item.place != place);
        self.items.insert(
            0,
            RecentItem {
                place,
                action,
                when: utc_text(now),
            },
        );
        self.items.truncate(RECENT_LIMIT);
        true
    }

    /// Forgets one file. Returns whether it was there.
    pub fn forget(&mut self, place: &Place) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.place != *place);
        self.items.len() != before
    }

    /// Empties the list. Returns how many files it held.
    pub fn clear(&mut self) -> usize {
        let held = self.items.len();
        self.items.clear();
        held
    }

    /// The cleared words: "Cleared 20 recent files.", "No recent files
    /// to clear."
    pub fn cleared_words(cleared: usize) -> String {
        match cleared {
            0 => "No recent files to clear.".to_owned(),
            1 => "Cleared 1 recent file.".to_owned(),
            n => format!("Cleared {} recent files.", grouped(n as u64)),
        }
    }

    /// The list's line, the meaning first: "Recent: 20 files, newest
    /// report.pdf, today", "Recent: no files", or "Recent files are off"
    /// when the setting is off.
    pub fn summary_line(&self, enabled: bool, clock: &dyn Clock) -> String {
        if !enabled {
            return "Recent files are off".to_owned();
        }
        let mut line = match self.items.len() {
            0 => "Recent: no files".to_owned(),
            1 => "Recent: 1 file".to_owned(),
            n => format!("Recent: {} files", grouped(n as u64)),
        };
        if let Some(newest) = self.items.first() {
            line.push_str(&format!(", newest {}", place_name(&newest.place)));
            if let Some(time) = newest.time() {
                let when = describe_time(time, clock);
                let day = match when.split_once(", ") {
                    Some((day, _)) if day == "today" || day == "yesterday" => day.to_owned(),
                    _ => when,
                };
                line.push_str(&format!(", {day}"));
            }
        }
        line
    }

    /// The list titled "Recent files", newest first, each row keyed by
    /// its place; empty when the setting is off.
    pub fn list_model(&self, enabled: bool, clock: &dyn Clock) -> ListModel {
        let rows = if enabled {
            self.items.iter().map(|item| item.row(clock)).collect()
        } else {
            Vec::new()
        };
        let mut list = ListModel::new("Recent files").with_rows(rows);
        list.set_empty_text(if enabled {
            "No recent files yet"
        } else {
            "Recent files are off in Settings"
        });
        list
    }
}

/// Records a file opened or edited from diroxide in `recent.toml`, when
/// the `recent_files` setting is on. When it is off, nothing is kept and
/// a list left from before is emptied. Returns whether it was kept.
pub fn record_recent(
    paths: &Paths,
    settings: &Settings,
    place: Place,
    action: RecentAction,
    now: Timestamp,
) -> Result<bool, ConfigError> {
    let mut recent = RecentFiles::load(paths).value;
    if !RecentFiles::enabled(settings) {
        if recent.clear() > 0 {
            recent.save(paths)?;
        }
        return Ok(false);
    }
    recent.record(place, action, now, true);
    recent.save(paths)?;
    Ok(true)
}

/// Empties `recent.toml` (the file stays, holding no files). Returns how
/// many files it held.
pub fn clear_recent(paths: &Paths) -> Result<usize, ConfigError> {
    let mut recent = RecentFiles::load(paths).value;
    let cleared = recent.clear();
    if cleared > 0 {
        recent.save(paths)?;
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::FixedClock;
    use diroxide_core::format::within_cells;
    use std::path::Path;

    /// A folder for a test that writes, under the build's own target
    /// folder, never a user's folder or the system's temporary folder.
    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().expect("the test's own path");
        let base = exe
            .parent()
            .and_then(Path::parent)
            .expect("the target folder")
            .join("config-unit-tmp");
        std::fs::create_dir_all(&base).expect("a scratch folder");
        tempfile::Builder::new()
            .prefix("recent-")
            .tempdir_in(base)
            .expect("a scratch folder")
    }

    fn paths(dir: &tempfile::TempDir) -> Paths {
        Paths::new(dir.path().join("config"), dir.path().join("state"))
    }

    fn place(text: &str) -> Place {
        let text = if cfg!(windows) {
            format!(r"D:\{}", text.replace('/', r"\"))
        } else {
            format!("/{text}")
        };
        Place::parse(&text).unwrap()
    }

    /// Thursday, November 12, 2026, 10:00 AM UTC.
    const NOW: Timestamp = Timestamp::from_unix(1_794_477_600, 0);

    #[test]
    fn recent_files_are_bounded_newest_first_and_cleared() {
        let dir = scratch();
        let paths = paths(&dir);
        let clock = FixedClock::new(NOW, UtcOffset::default());
        let mut recent = RecentFiles::default();
        assert_eq!(recent.summary_line(true, &clock), "Recent: no files");
        for n in 0..(RECENT_LIMIT + 5) {
            let at = NOW.plus_seconds(n as i64 - 100);
            assert!(recent.record(place(&format!("f{n}.txt")), RecentAction::Opened, at, true));
        }
        assert_eq!(recent.items.len(), RECENT_LIMIT);
        recent.record(place("docs/report.pdf"), RecentAction::Edited, NOW, true);
        recent.record(
            place("f54.txt"),
            RecentAction::Opened,
            NOW.plus_seconds(-1),
            true,
        );
        assert_eq!(recent.items.len(), RECENT_LIMIT);
        assert_eq!(recent.items[0].place, place("f54.txt"));
        assert_eq!(recent.items[1].place, place("docs/report.pdf"));
        recent.record(place("docs/report.pdf"), RecentAction::Edited, NOW, true);
        // 42 cells; the count and the newest name are in the first 40.
        let line = recent.summary_line(true, &clock);
        assert_eq!(line, "Recent: 50 files, newest report.pdf, today");
        assert!(within_cells(&line, "Recent: 50 files, newest report.pdf"));
        let row = recent.items[0].row(&clock).spoken();
        assert!(
            row.starts_with("report.pdf, edited today, 10:00 AM, in "),
            "{row}"
        );
        recent.save(&paths).unwrap();
        assert_eq!(RecentFiles::load(&paths), Loaded::clean(recent.clone()));
        assert_eq!(recent.summary_line(false, &clock), "Recent files are off");
        assert!(!recent.record(place("x.txt"), RecentAction::Opened, NOW, false));
        assert!(recent.forget(&place("docs/report.pdf")));
        assert_eq!(clear_recent(&paths).unwrap(), RECENT_LIMIT);
        assert!(RecentFiles::load(&paths).value.items.is_empty());
        assert!(
            paths.state_file(RECENT_FILE).is_file(),
            "emptied, not removed"
        );
        assert_eq!(RecentFiles::cleared_words(20), "Cleared 20 recent files.");
        assert_eq!(RecentFiles::cleared_words(0), "No recent files to clear.");
        let off = recent.list_model(false, &clock);
        assert!(off.is_empty());
        assert_eq!(off.empty_text(), "Recent files are off in Settings");
    }

    #[test]
    fn the_setting_turns_recent_files_off_and_empties_them() {
        let dir = scratch();
        let paths = paths(&dir);
        let mut settings = Settings::default();
        assert!(RecentFiles::enabled(&settings));
        assert!(
            record_recent(&paths, &settings, place("a.txt"), RecentAction::Opened, NOW).unwrap()
        );
        assert_eq!(RecentFiles::load(&paths).value.items.len(), 1);
        settings.recent_files = false;
        assert!(
            !record_recent(&paths, &settings, place("b.txt"), RecentAction::Opened, NOW).unwrap()
        );
        assert!(RecentFiles::load(&paths).value.items.is_empty());
        std::fs::write(paths.state_file(RECENT_FILE), "items = [\n").unwrap();
        let bad = RecentFiles::load(&paths);
        assert!(bad.value.items.is_empty());
        assert!(
            bad.problems[0]
                .text
                .starts_with("recent.toml could not be read: "),
            "{}",
            bad.problems[0].text
        );
        assert!(paths.state_file("recent.toml.bad").is_file());
    }
}
