//! The window's look in the app (lane A6, G3's contract requests 1, 2
//! and 3): full screen and focus mode as commands, and a theme said
//! with its contrast verdict however it is set.
//!
//! - Full screen (`view.full-screen`, F11 in the GUI column) is the
//!   window's. The app keeps whether the window should fill the screen
//!   and says "Full screen on." or "Full screen off."; the GUI reads
//!   [`App::full_screen`] and carries it out, and tells the app with
//!   [`App::set_full_screen`] when the window left it by itself. In a
//!   terminal the command changes nothing and says why: "Full screen is
//!   the window's; the terminal program may have its own."
//! - Focus mode (`view.focus-mode`) turns the `focus_mode` setting on or
//!   off and writes it, as the Settings list does, and says "Focus mode
//!   on: the bars fade after three seconds." or "Focus mode off." The
//!   same words follow the setting set through the Settings list.
//! - A theme set through the Settings list is said as Next theme says it,
//!   its contrast verdict after its name: "Theme Galaxy Light, meets AA".

use diroxide_core::{Command, Effect, KeyColumn, Message, Redraw};

use super::App;
use crate::commands::ids;
use crate::settings;

/// Said when full screen comes on in the window.
pub const FULL_SCREEN_ON: &str = "Full screen on.";

/// Said when the window gives the room back.
pub const FULL_SCREEN_OFF: &str = "Full screen off.";

/// Said in a terminal, where full screen is the terminal program's.
pub const FULL_SCREEN_IN_TERMINAL: &str =
    "Full screen is the window's; the terminal program may have its own.";

/// Said when focus mode comes on in the window.
pub const FOCUS_MODE_ON: &str = "Focus mode on: the bars fade after three seconds.";

/// Said when focus mode comes on from a terminal: the setting is the
/// window's, and the terminal keeps its bars.
pub const FOCUS_MODE_ON_IN_TERMINAL: &str =
    "Focus mode on, for the window; the terminal keeps its bars.";

/// Said when focus mode goes off.
pub const FOCUS_MODE_OFF: &str = "Focus mode off.";

/// The look's state: whether the window should fill the screen.
#[derive(Clone, Debug, Default)]
pub(super) struct LookState {
    full_screen: bool,
}

impl App {
    /// Whether the window should fill the screen: Full screen turned it
    /// on. The GUI reads it after each event and carries it out; a
    /// terminal never sets it.
    pub fn full_screen(&self) -> bool {
        self.look.full_screen
    }

    /// The window's own full screen state, when it changed without the
    /// command: the system took the window out of full screen, or a
    /// window that runs in the background never goes full screen. Says
    /// nothing; the next Full screen starts from it.
    pub fn set_full_screen(&mut self, on: bool) {
        self.look.full_screen = on;
    }

    /// The chosen theme and its contrast verdict with the owner's colors,
    /// as Next theme and the Settings list say it: "Theme Galaxy, meets
    /// AA".
    pub fn theme_line(&self) -> String {
        settings::theme_line(&self.settings, None)
    }

    /// Runs Full screen and Focus mode; `None` when the command is
    /// another's.
    pub(super) fn look_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::FULL_SCREEN && command.id != ids::FOCUS_MODE {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(if command.id == ids::FULL_SCREEN {
            self.toggle_full_screen()
        } else {
            self.toggle_focus_mode()
        })
    }

    /// Full screen: on or off in the window, said; in a terminal, why
    /// not.
    fn toggle_full_screen(&mut self) -> Vec<Effect> {
        if self.keymap.column() == KeyColumn::Terminal {
            return vec![self.say(Message::info(FULL_SCREEN_IN_TERMINAL))];
        }
        self.look.full_screen = !self.look.full_screen;
        let line = if self.look.full_screen {
            FULL_SCREEN_ON
        } else {
            FULL_SCREEN_OFF
        };
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
    }

    /// Focus mode: the setting flipped and written, as the Settings list
    /// flips it, with its own words.
    fn toggle_focus_mode(&mut self) -> Vec<Effect> {
        let on = !self.settings.focus_mode;
        self.apply_setting("focus_mode", Some(toml::Value::Boolean(on)))
    }

    /// What a changed setting of the look says in place of the plain
    /// "Theme: nord" or "Focus mode: on": the theme with its verdict, and
    /// focus mode with what it does. `None` for the other settings.
    pub(super) fn look_setting_line(&self, key: &str) -> Option<String> {
        match key {
            "theme" => Some(self.theme_line()),
            "focus_mode" => Some(
                match (self.settings.focus_mode, self.keymap.column()) {
                    (false, _) => FOCUS_MODE_OFF,
                    (true, KeyColumn::Gui) => FOCUS_MODE_ON,
                    (true, KeyColumn::Terminal) => FOCUS_MODE_ON_IN_TERMINAL,
                }
                .to_owned(),
            ),
            _ => None,
        }
    }
}
