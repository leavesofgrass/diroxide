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

/// One panel's state.
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
}
