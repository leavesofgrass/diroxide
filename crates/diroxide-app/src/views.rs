//! Per-folder views and the History list (`research/alpha3-plan.md`,
//! "Labels, bookmarks, history, per-folder views").
//!
//! A folder view is a folder's own sort, filter, hidden files and
//! columns, saved by the owner with Save view for this folder. Entering
//! the folder again applies it and says so ("Sorted by size, largest
//! first; saved for this folder."); leaving it puts the panel's usual
//! view back. Nothing is saved or removed on its own: Forget this
//! folder's view takes one out. The views live in `folder-views.toml` in
//! the state folder, written atomically on every change.
//!
//! The History list reads the path jump store W1d keeps (the folders
//! visited in every session, with how often and how lately): its rows say
//! "reports, in work, used 12 times, today", most used first.
//!
//! W3l owns this module. The app's commands are in `app::folder_views`.

use diroxide_config::{ColumnKind, ConfigProblem, Loaded, Paths};
use diroxide_core::format::count;
use diroxide_core::{Clock, ListModel, ListRow, Place, SortSpec, Timestamp};
use serde::{Deserialize, Serialize};

use crate::labels::{place_key, state_file};
use crate::panel::place_leaf;
use crate::state::Visits;

/// The folder views file in the state folder.
pub const FILE: &str = "folder-views.toml";

/// The version of `folder-views.toml` this build writes.
pub const FORMAT: u32 = 1;

/// One folder's own view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderView {
    /// The folder.
    pub place: Place,
    /// How it sorts.
    pub sort: SortSpec,
    /// Its filter as typed, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Whether hidden entries are shown.
    #[serde(default)]
    pub show_hidden: bool,
    /// The details each row says after its name.
    #[serde(default)]
    pub columns: Vec<ColumnKind>,
}

impl FolderView {
    /// What entering the folder says, the sort first: "Sorted by size,
    /// largest first; saved for this folder." `usual_hidden` is the
    /// panel's own hidden setting, so the hidden files are said only when
    /// this view differs.
    pub fn sentence(&self, usual_hidden: bool) -> String {
        let mut parts = vec![self.sort.describe()];
        if let Some(filter) = &self.filter {
            parts.push(format!(
                "filter {}",
                diroxide_core::names::display_name(filter)
            ));
        }
        if self.show_hidden != usual_hidden {
            parts.push(if self.show_hidden {
                "hidden files shown".to_owned()
            } else {
                "hidden files not shown".to_owned()
            });
        }
        format!("{}; saved for this folder.", parts.join("; "))
    }
}

/// Every folder's view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderViews {
    views: Vec<FolderView>,
}

impl FolderViews {
    /// No views.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every view, in the order saved.
    pub fn all(&self) -> &[FolderView] {
        &self.views
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// A folder's view, compared as its file system compares names.
    pub fn get(&self, place: &Place) -> Option<&FolderView> {
        let key = place_key(place);
        self.views.iter().find(|v| place_key(&v.place) == key)
    }

    /// Saves a folder's view, replacing the one it had.
    pub fn set(&mut self, view: FolderView) {
        let key = place_key(&view.place);
        match self.views.iter_mut().find(|v| place_key(&v.place) == key) {
            Some(old) => *old = view,
            None => self.views.push(view),
        }
    }

    /// Forgets a folder's view; returns whether it had one.
    pub fn forget(&mut self, place: &Place) -> bool {
        let key = place_key(place);
        let before = self.views.len();
        self.views.retain(|v| place_key(&v.place) != key);
        self.views.len() != before
    }

    /// Reads `folder-views.toml` text; a view that cannot be read is left
    /// out and said.
    pub fn from_toml(text: &str) -> Result<(Self, Vec<String>), toml::de::Error> {
        let file: ViewsFile = toml::from_str(text)?;
        let mut views = Self::new();
        let mut problems = Vec::new();
        for value in file.view {
            match value.try_into::<FolderView>() {
                Ok(view) => views.set(view),
                Err(error) => problems.push(format!("A folder view cannot be read: {error}")),
            }
        }
        Ok((views, problems))
    }

    /// The views as `folder-views.toml` text.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        #[derive(Serialize)]
        struct Out<'a> {
            format: u32,
            view: &'a [FolderView],
        }
        toml::to_string(&Out {
            format: FORMAT,
            view: &self.views,
        })
    }
}

/// The file as read: each view kept as a table first, so one that cannot
/// be read is left out and the rest are used. The `format` key is read
/// by later versions; this one writes it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct ViewsFile {
    view: Vec<toml::Value>,
}

/// Reads `folder-views.toml` from the state folder: none when there is no
/// file; when it cannot be read, none, a copy kept as
/// `folder-views.toml.bad`, and one sentence that says so.
pub fn load(paths: &Paths) -> Loaded<FolderViews> {
    match state_file::read(paths, FILE) {
        state_file::Read::Missing => Loaded::clean(FolderViews::new()),
        state_file::Read::Failed(problem) => Loaded {
            value: FolderViews::new(),
            problems: vec![problem],
        },
        state_file::Read::Text(text) => match FolderViews::from_toml(&text) {
            Ok((views, problems)) => Loaded {
                value: views,
                problems: problems
                    .into_iter()
                    .map(|p| ConfigProblem::new(FILE, p))
                    .collect(),
            },
            Err(error) => Loaded {
                value: FolderViews::new(),
                problems: vec![state_file::unreadable(
                    paths,
                    FILE,
                    &text,
                    &error,
                    "no folder views are in use",
                )],
            },
        },
    }
}

/// Writes `folder-views.toml` to the state folder atomically. The error
/// is the cause in words.
pub fn save(paths: &Paths, views: &FolderViews) -> Result<(), String> {
    let text = views.to_toml().map_err(|e| e.to_string())?;
    state_file::write(paths, FILE, &text)
}

/// A day in words for a row, relative to the clock's today: "today",
/// "yesterday", "Tuesday" within the week, "October 3" this year,
/// "October 3, 2025" otherwise.
pub fn day_words(time: Timestamp, clock: &dyn Clock) -> String {
    let words = diroxide_core::clock::describe_time(time, clock);
    match words.split_once(", ") {
        Some((day @ ("today" | "yesterday"), _)) => day.to_owned(),
        _ => words,
    }
}

/// The History list over the path jump store: every folder visited in
/// any session, most used first, each row "reports, in work, used 12
/// times, today", the whole place last. Enter goes there; the row id is
/// the place.
pub fn history_list(visits: &Visits, clock: &dyn Clock) -> ListModel {
    let now = clock.now().unix_seconds();
    let rows = visits
        .ranked(now)
        .into_iter()
        .map(|visit| {
            let mut details = Vec::new();
            if let Some(parent) = visit.place.parent() {
                details.push(format!("in {}", place_leaf(&parent)));
            }
            details.push(match visit.count {
                1 => "used once".to_owned(),
                n => format!("used {n} times"),
            });
            details.push(day_words(Timestamp::from_unix(visit.last, 0), clock));
            details.push(diroxide_core::names::display_name(&visit.place.to_string()).into_owned());
            ListRow::new(visit.place.to_string(), place_leaf(&visit.place)).with_details(details)
        })
        .collect();
    let mut list = ListModel::new(format!(
        "History, {}",
        count(visits.len() as u64, "place", "places")
    ))
    .with_rows(rows);
    list.set_empty_text("No places in the history yet");
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::{FixedClock, SortKey, UtcOffset};

    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    fn clock() -> FixedClock {
        FixedClock::new(
            Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        )
    }

    fn by_size(place: Place) -> FolderView {
        FolderView {
            place,
            sort: SortSpec::by(SortKey::Size),
            filter: None,
            show_hidden: false,
            columns: vec![ColumnKind::Type, ColumnKind::Size],
        }
    }

    #[test]
    fn a_view_says_its_sort_first() {
        let mut view = by_size(p("/work"));
        assert_eq!(
            view.sentence(false),
            "Sorted by size, largest first; saved for this folder."
        );
        view.filter = Some("*.pdf".into());
        view.show_hidden = true;
        assert_eq!(
            view.sentence(false),
            "Sorted by size, largest first; filter *.pdf; hidden files shown; saved for this folder."
        );
        assert!(within_cells(&view.sentence(false), "Sorted by size"));
    }

    #[test]
    fn views_are_set_replaced_and_forgotten_by_place() {
        let mut views = FolderViews::new();
        views.set(by_size(p("/work")));
        let mut named = by_size(p("/work"));
        named.sort = SortSpec::by(SortKey::Name);
        views.set(named.clone());
        assert_eq!(views.all().len(), 1);
        assert_eq!(views.get(&p("/work")), Some(&named));
        if cfg!(windows) {
            assert!(views.get(&p("/WORK")).is_some());
        }
        assert!(views.forget(&p("/work")));
        assert!(!views.forget(&p("/work")));
        assert!(views.is_empty());
    }

    #[test]
    fn the_store_round_trips_through_an_injected_state_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        assert!(load(&paths).value.is_empty());
        let mut views = FolderViews::new();
        let mut view = by_size(p("/work"));
        view.filter = Some("*.pdf".into());
        views.set(view);
        views.set(by_size(p("/other")));
        save(&paths, &views).unwrap();
        let read = load(&paths);
        assert!(read.problems.is_empty(), "{:?}", read.problems);
        assert_eq!(read.value, views);
        let (_, problems) = FolderViews::from_toml("[[view]]\nplace = 3\n").unwrap();
        assert!(problems[0].starts_with("A folder view cannot be read"));
    }

    #[test]
    fn the_history_list_reads_meaning_first() {
        let clock = clock();
        let now = clock.now().unix_seconds();
        let mut visits = Visits::default();
        let reports = p("/work/reports");
        for _ in 0..12 {
            visits.record(&reports, now - 60, None);
        }
        visits.record(&p("/work/old"), now - 3 * 86_400, None);
        let list = history_list(&visits, &clock);
        assert_eq!(list.title(), "History, 2 places");
        assert!(
            list.rows()[0]
                .spoken()
                .starts_with("reports, in work, used 12 times, today"),
            "{}",
            list.rows()[0].spoken()
        );
        assert!(
            list.rows()[1]
                .spoken()
                .starts_with("old, in work, used once")
        );
        assert_eq!(list.rows()[0].id, reports.to_string());
        assert_eq!(
            day_words(clock.now().plus_seconds(-86_400), &clock),
            "yesterday"
        );
    }
}
