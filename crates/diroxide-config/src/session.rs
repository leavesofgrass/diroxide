//! The session: what the program restores when it starts again (R19):
//! each panel's tabs, and each tab's place, focused entry, sort, whether it shows
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

/// A tab's lock (alpha 5, lane TB), said in words after its name: free,
/// the default; locked, where the tab refuses to close and a folder
/// change opens a new tab; pinned, kept at the start of the panel's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabLock {
    /// Nothing held: the tab closes and changes folder.
    #[default]
    Free,
    /// The tab refuses to close; a folder change opens a new tab.
    Locked,
    /// The tab is kept at the start of the panel's tabs.
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
/// hidden files, history and lock.
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
}

/// One panel's state. Its own fields are the current tab's, so a file
/// written before tabs, which has no `tabs`, reads as one tab, and an
/// older build reading a newer file finds the current tab where it looks.
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
    /// The current tab's lock.
    pub lock: TabLock,
    /// Every tab in order, the current one among them, when there is
    /// more than one; empty for a panel of one tab (alpha 5, lane TB).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabState>,
    /// The current tab's index in `tabs`.
    pub current_tab: usize,
}

impl PanelState {
    /// The current tab as a panel of one tab describes it.
    pub fn as_tab(&self) -> TabState {
        TabState {
            place: self.place.clone(),
            focused: self.focused.clone(),
            sort: self.sort.clone(),
            show_hidden: self.show_hidden,
            history: self.history.clone(),
            lock: self.lock,
        }
    }

    /// Every tab in order and the current one's index: the one tab of a
    /// file written before tabs, or `tabs` with the index kept in range.
    pub fn tab_list(&self) -> (Vec<TabState>, usize) {
        if self.tabs.is_empty() {
            return (vec![self.as_tab()], 0);
        }
        let current = self.current_tab.min(self.tabs.len() - 1);
        (self.tabs.clone(), current)
    }

    /// A panel's state from its tabs in order and the current one's
    /// index: the panel's own fields are the current tab's, and `tabs` is
    /// written only when there is more than one.
    pub fn from_tabs(tabs: Vec<TabState>, current: usize) -> Self {
        let current = current.min(tabs.len().saturating_sub(1));
        let now = tabs.get(current).cloned().unwrap_or_default();
        let many = tabs.len() > 1;
        Self {
            place: now.place,
            focused: now.focused,
            sort: now.sort,
            show_hidden: now.show_hidden,
            history: now.history,
            lock: now.lock,
            tabs: if many { tabs } else { Vec::new() },
            current_tab: if many { current } else { 0 },
        }
    }
}

/// What is restored at the next start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    /// The file's version, so a later shape can read an older one.
    pub version: u32,
    /// The layout.
    pub layout: Layout,
    /// The active panel.
    pub active: Side,
    /// The left and right panels.
    pub panels: [PanelState; 2],
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            layout: Layout::default(),
            active: Side::Left,
            panels: [PanelState::default(), PanelState::default()],
        }
    }
}

impl SessionState {
    /// The version this build writes.
    pub const VERSION: u32 = 1;

    /// One panel's state.
    pub fn panel(&self, side: Side) -> &PanelState {
        &self.panels[side.index()]
    }

    /// One panel's state, to change.
    pub fn panel_mut(&mut self, side: Side) -> &mut PanelState {
        &mut self.panels[side.index()]
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
        let (tabs, current) = session.panel(Side::Left).tab_list();
        assert_eq!(current, 0);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].place.as_ref(), Some(&place));
        assert_eq!(tabs[0].focused.as_deref(), Some("a.txt"));
        assert_eq!(tabs[0].lock, TabLock::Free);
        assert_eq!(session.panel(Side::Right).tab_list().0.len(), 1);
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
        let mut session = SessionState::default();
        *session.panel_mut(Side::Right) = PanelState::from_tabs(tabs.clone(), 2);
        let right = session.panel(Side::Right);
        // The panel's own fields are the current tab's, for older builds.
        assert_eq!(right.place.as_ref(), Some(&folder("work")));
        assert_eq!(right.lock, TabLock::Locked);
        let text = session.to_toml().unwrap();
        let back = SessionState::from_toml(&text).unwrap();
        assert_eq!(back, session);
        assert_eq!(back.panel(Side::Right).tab_list(), (tabs, 2));
        // One tab writes no list of tabs.
        let one = PanelState::from_tabs(vec![tab("docs", TabLock::Free)], 0);
        assert!(one.tabs.is_empty());
        assert_eq!(one.tab_list().0.len(), 1);
        // A current index past the end is kept in range.
        let mut far = PanelState::from_tabs(vec![tab("a", TabLock::Free); 2], 1);
        far.current_tab = 9;
        assert_eq!(far.tab_list().1, 1);
        assert_eq!(TabLock::Pinned.word(), "pinned");
        assert_eq!(TabLock::Free.said(), None);
        assert_eq!(TabLock::Locked.said(), Some("locked"));
    }
}
