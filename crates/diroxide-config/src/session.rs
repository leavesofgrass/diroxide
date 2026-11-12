//! The session: what the program restores when it starts again (R19):
//! each panel's place, its focused entry, its sort, whether it shows
//! hidden entries and its history; the layout and which panel was active.
//! It lives in the state folder as `session.toml`, written atomically, and
//! belongs to the configuration crate, not the app, so the app's core
//! stays free of I/O.
//!
//! The file is written atomically on every change worth keeping; a file
//! that cannot be read is copied to `session.toml.bad`, said in one
//! sentence, and the start goes on from the defaults. Tests pass their own
//! state folder through [`Paths::new`].

use diroxide_core::{Place, Side, SortSpec};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths::Paths;
use crate::store::{self, ConfigProblem, Loaded, ReadText};

/// How the two panels are laid out (ADR-0007): side by side, the default
/// and primary layout; stacked, one above the other, the fallback for
/// screen-reader users and narrow terminals; or the active panel alone.
/// The `layout` setting of W1c takes these values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// The two panels side by side.
    #[default]
    SideBySide,
    /// One panel above the other, each row at full width.
    Stacked,
    /// The active panel alone; Tab switches which one is shown.
    Single,
}

impl Layout {
    /// The layout in words: "side by side", "stacked", "single panel".
    pub const fn word(self) -> &'static str {
        match self {
            Layout::SideBySide => "side by side",
            Layout::Stacked => "stacked",
            Layout::Single => "single panel",
        }
    }
}

/// A tab's lock (alpha 5, lane TB; research/modern-features.md, "Folder
/// tabs"), said in words after its name: free, the default; locked, where
/// closing asks first and a folder change opens a new tab; pinned, free to
/// change folder but remembering a base folder to go back to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabLock {
    /// Nothing held: the tab closes and changes folder.
    #[default]
    Free,
    /// Closing asks first; a folder change opens a new tab.
    Locked,
    /// Folder changes are free; the tab remembers a base folder.
    Pinned,
}

impl TabLock {
    /// The lock in words: "free", "locked", "pinned".
    pub const fn word(self) -> &'static str {
        match self {
            TabLock::Free => "free",
            TabLock::Locked => "locked",
            TabLock::Pinned => "pinned",
        }
    }

    /// The word said after a tab's name: none for a free tab, since only
    /// a lock that holds is worth hearing.
    pub const fn said(self) -> Option<&'static str> {
        match self {
            TabLock::Free => None,
            other => Some(other.word()),
        }
    }
}

/// One tab of a panel (alpha 5, lane TB): its place, cursor, sort,
/// hidden files, history, lock, and a pinned tab's base folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    /// The folder or other place shown.
    pub place: Option<Place>,
    /// The focused entry's name, put back under the cursor.
    pub focused: Option<String>,
    /// How the tab sorts.
    pub sort: SortSpec,
    /// Whether hidden entries are shown.
    pub show_hidden: bool,
    /// The places visited, oldest first, for back and forward.
    pub history: Vec<Place>,
    /// Free, locked or pinned.
    pub lock: TabLock,
    /// A pinned tab's base folder, which Go to base folder returns to.
    pub base: Option<Place>,
}

impl TabState {
    /// The tab a panel's state describes, free.
    pub fn of_panel(panel: &PanelState) -> Self {
        Self {
            place: panel.place.clone(),
            focused: panel.focused.clone(),
            sort: panel.sort.clone(),
            show_hidden: panel.show_hidden,
            history: panel.history.clone(),
            ..Self::default()
        }
    }

    /// The panel's state this tab gives when it is current.
    pub fn as_panel(&self) -> PanelState {
        PanelState {
            place: self.place.clone(),
            focused: self.focused.clone(),
            sort: self.sort.clone(),
            show_hidden: self.show_hidden,
            history: self.history.clone(),
        }
    }

    /// Whether the tab holds nothing a panel's state does not: free, with
    /// no base folder.
    pub fn is_plain(&self) -> bool {
        self.lock == TabLock::Free && self.base.is_none()
    }
}

/// One panel's state: its current tab's, so an older build reading a
/// session with tabs finds the current tab where it looks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelState {
    /// The folder or other place shown; `None` for the start folder.
    pub place: Option<Place>,
    /// The focused entry's name, put back under the cursor.
    pub focused: Option<String>,
    /// How the panel sorts.
    pub sort: SortSpec,
    /// Whether hidden entries are shown.
    pub show_hidden: bool,
    /// The places visited, oldest first, for back and forward.
    pub history: Vec<Place>,
}

/// What is restored at the next start. Version 2 (alpha 5, lane TB) adds
/// each panel's tabs beside the two panels, and reads version 1 as one
/// tab a panel; `panels` stays each panel's current tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    /// The file's version, so a later shape can read an older one.
    pub version: u32,
    /// The layout.
    pub layout: Layout,
    /// The active panel.
    pub active: Side,
    /// The left and right panels, each its current tab.
    pub panels: [PanelState; 2],
    /// Each panel's tabs in order, the current one among them; empty for
    /// a panel of one plain tab.
    pub tabs: [Vec<TabState>; 2],
    /// Each panel's current tab's index in `tabs`.
    pub current: [usize; 2],
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            layout: Layout::default(),
            active: Side::Left,
            panels: [PanelState::default(), PanelState::default()],
            tabs: [Vec::new(), Vec::new()],
            current: [0, 0],
        }
    }
}

impl SessionState {
    /// The version this build writes.
    pub const VERSION: u32 = 2;

    /// One panel's state.
    pub fn panel(&self, side: Side) -> &PanelState {
        &self.panels[side.index()]
    }

    /// One panel's state, to change.
    pub fn panel_mut(&mut self, side: Side) -> &mut PanelState {
        &mut self.panels[side.index()]
    }

    /// A panel's tabs in order and the current one's index: one tab, the
    /// panel's own, when the file has none (version 1, or one plain tab);
    /// the index kept in range.
    pub fn tab_list(&self, side: Side) -> (Vec<TabState>, usize) {
        let tabs = &self.tabs[side.index()];
        if tabs.is_empty() {
            return (vec![TabState::of_panel(self.panel(side))], 0);
        }
        let current = self.current[side.index()].min(tabs.len() - 1);
        (tabs.clone(), current)
    }

    /// Sets a panel's tabs and its current one: the panel's state becomes
    /// the current tab's, and the tabs are kept only when there is more
    /// than one or the one holds a lock or a base folder.
    pub fn set_tabs(&mut self, side: Side, tabs: Vec<TabState>, current: usize) {
        let i = side.index();
        let current = current.min(tabs.len().saturating_sub(1));
        if let Some(now) = tabs.get(current) {
            self.panels[i] = now.as_panel();
        }
        if tabs.len() > 1 || tabs.first().is_some_and(|t| !t.is_plain()) {
            self.tabs[i] = tabs;
            self.current[i] = current;
        } else {
            self.tabs[i] = Vec::new();
            self.current[i] = 0;
        }
    }

    /// Reads `session.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::SESSION, e))
    }

    /// Writes the session as TOML text.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(|e| ConfigError::parse(Paths::SESSION, e))
    }
}

/// Reads the session from the state folder. A missing file gives the
/// defaults and says nothing; a file that cannot be read gives the
/// defaults, is copied to `session.toml.bad`, and says so in one sentence.
pub fn load_session(paths: &Paths) -> Loaded<SessionState> {
    let name = Paths::SESSION;
    let path = paths.state_file(name);
    match store::read_text(&path, name) {
        ReadText::Missing => Loaded::clean(SessionState::default()),
        ReadText::Failed(problem) => Loaded {
            value: SessionState::default(),
            problems: vec![problem],
        },
        ReadText::Text(text) => match store::parse_typed::<SessionState>(name, &text) {
            Ok(session) => Loaded::clean(session),
            Err(failure) => {
                let backup = store::back_up(&path, &text);
                Loaded {
                    value: SessionState::default(),
                    problems: vec![ConfigProblem::new(name, failure.sentence(&backup))],
                }
            }
        },
    }
}

/// Writes the session to the state folder atomically: to a temporary file
/// beside it, flushed, then a rename over the old one.
pub fn save_session(paths: &Paths, session: &SessionState) -> Result<(), ConfigError> {
    let text = session.to_toml()?;
    store::write_atomic(&paths.state_file(Paths::SESSION), &text)
        .map_err(|e| ConfigError::io(Paths::SESSION, &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{SortKey, SortSpec};

    #[test]
    fn a_session_reads_back_as_written() {
        let mut session = SessionState {
            layout: Layout::Stacked,
            active: Side::Right,
            ..SessionState::default()
        };
        let place = Place::parse(if cfg!(windows) { r"D:\docs" } else { "/docs" }).unwrap();
        let left = session.panel_mut(Side::Left);
        left.place = Some(place.clone());
        left.focused = Some("report.pdf".to_owned());
        left.sort = SortSpec::by(SortKey::Size);
        left.history = vec![place];
        let text = session.to_toml().unwrap();
        assert_eq!(SessionState::from_toml(&text).unwrap(), session);
        assert_eq!(
            SessionState::from_toml("").unwrap(),
            SessionState::default()
        );
        assert_eq!(Layout::default().word(), "side by side");
    }

    #[test]
    fn the_session_survives_a_restart_and_a_bad_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let empty = load_session(&paths);
        assert_eq!(empty.value, SessionState::default());
        assert!(empty.problems.is_empty());
        let mut session = SessionState::default();
        session.panel_mut(Side::Right).focused = Some("notes.txt".to_owned());
        save_session(&paths, &session).unwrap();
        assert_eq!(load_session(&paths), Loaded::clean(session));
        std::fs::write(paths.state_file(Paths::SESSION), "version = [\n").unwrap();
        let bad = load_session(&paths);
        assert_eq!(bad.value, SessionState::default());
        assert_eq!(
            bad.problems[0].text,
            "session.toml could not be read: line 1 is not valid TOML. A copy is kept as session.toml.bad; the defaults are in use."
        );
        assert!(paths.state_file("session.toml.bad").exists());
    }

    fn folder(name: &str) -> Place {
        let text = if cfg!(windows) {
            format!(r"D:\{name}")
        } else {
            format!("/{name}")
        };
        Place::parse(&text).unwrap()
    }

    #[test]
    fn a_session_written_before_tabs_reads_as_one_tab() {
        let place = folder("docs");
        let text = format!(
            "version = 1\nactive = \"left\"\n\n[[panels]]\nplace = {:?}\nfocused = \"a.txt\"\n\n[[panels]]\n",
            place.to_string()
        );
        let session = SessionState::from_toml(&text).unwrap();
        assert_eq!(session.version, 1);
        let (tabs, current) = session.tab_list(Side::Left);
        assert_eq!(current, 0);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].place.as_ref(), Some(&place));
        assert_eq!(tabs[0].focused.as_deref(), Some("a.txt"));
        assert_eq!(tabs[0].lock, TabLock::Free);
        assert_eq!(session.tab_list(Side::Right).0.len(), 1);
        assert_eq!(SessionState::default().version, 2);
    }

    #[test]
    fn a_panel_of_three_tabs_reads_back_with_its_locks() {
        let tab = |name: &str, lock| TabState {
            place: Some(folder(name)),
            focused: Some(format!("{name}.txt")),
            lock,
            ..TabState::default()
        };
        let tabs = vec![
            tab("docs", TabLock::Pinned),
            tab("music", TabLock::Free),
            tab("work", TabLock::Locked),
        ];
        let mut tabs = tabs;
        tabs[0].base = Some(folder("docs"));
        let mut session = SessionState::default();
        session.set_tabs(Side::Right, tabs.clone(), 2);
        // The panel's own state is the current tab's, for older builds.
        let right = session.panel(Side::Right);
        assert_eq!(right.place.as_ref(), Some(&folder("work")));
        assert_eq!(right.focused.as_deref(), Some("work.txt"));
        let text = session.to_toml().unwrap();
        let back = SessionState::from_toml(&text).unwrap();
        assert_eq!(back, session);
        assert_eq!(back.tab_list(Side::Right), (tabs, 2));
        // One plain tab writes no list of tabs; a lone locked one does.
        session.set_tabs(Side::Left, vec![tab("docs", TabLock::Free)], 0);
        assert!(session.tabs[0].is_empty());
        assert_eq!(session.tab_list(Side::Left).0.len(), 1);
        session.set_tabs(Side::Left, vec![tab("docs", TabLock::Locked)], 0);
        assert_eq!(session.tabs[0].len(), 1);
        // A current index past the end is kept in range.
        session.current[1] = 9;
        assert_eq!(session.tab_list(Side::Right).1, 2);
        assert_eq!(TabLock::Pinned.word(), "pinned");
        assert_eq!(TabLock::Free.said(), None);
        assert_eq!(TabLock::Locked.said(), Some("locked"));
    }
}
