//! Keys in the app: Change key's capture, its checks and its questions,
//! the Keys list, the presets and the single-letter switch (plan section
//! 5.7, "Customizing keys"; research/alpha3-plan.md, "The Keys list").
//!
//! The pure parts live in the keymap's modules: the capture engine
//! ([`crate::keymap::capture`]). This module holds what the app keeps
//! while they run and turns their answers into dialogs and sentences.
//!
//! W3k owns this module.

use diroxide_core::{Command, CommandId, Effect, Key, KeyColumn, Message, Redraw};

use super::{App, Purpose};
use crate::commands::{self, ids};
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::KeyChoice;
use crate::keymap::capture::{self, Capture, Captured, KeyReports};

/// A change asked for: a command's key in one column, replaced or added
/// to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Change {
    /// The command.
    pub(super) command: CommandId,
    /// The column changed.
    pub(super) column: KeyColumn,
    /// Whether the key is added beside the command's keys rather than
    /// replacing them.
    pub(super) add: bool,
}

/// What a keys dialog is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeysPurpose {
    /// The capture prompt, while a capture waits for a key.
    Capture(Change),
    /// The typed name's field.
    TypedName(Change),
}

/// What the app keeps for keys between events.
#[derive(Clone, Debug, Default)]
pub(super) struct KeysState {
    /// The capture waiting for a key, and the change it is for.
    pub(super) capture: Option<(Capture, Change)>,
    /// What the frontend knows of the terminal.
    pub(super) reports: KeyReports,
}

/// Whether a command is one of the keys commands this module runs.
pub(super) fn is_keys_command(id: &CommandId) -> bool {
    [ids::KEYS_CHANGE].contains(id)
}

impl App {
    /// Runs a keys command: Change a key with its `command` argument.
    pub(super) fn keys_dispatch(&mut self, command: &Command) -> Vec<Effect> {
        if !self.dialogs.is_empty() && self.top_keys_purpose().is_none() {
            return vec![self.say(Message::status("Close this dialog first."))];
        }
        if command.id == ids::KEYS_CHANGE {
            let target = command.text("command").and_then(|t| CommandId::new(t).ok());
            return match target {
                Some(target) if commands::info(&target).is_some() => self.change_key(&target),
                Some(target) => vec![self.say(Message::error(format!(
                    "No command has the id {target}. Nothing changed."
                )))],
                None => vec![self.say(Message::error(
                    "Change a key needs a command: keys.change command=view.sort-size.",
                ))],
            };
        }
        Vec::new()
    }

    /// Change key for a command, as the Keys list's Change key does: the
    /// capture prompt opens in this keymap's column and waits for one key
    /// ("Press the new key now. Escape cancels."). App-side hook for the
    /// window and the tests; the Keys list calls it too.
    pub fn change_key(&mut self, command: &CommandId) -> Vec<Effect> {
        let change = Change {
            command: command.clone(),
            column: self.keymap.column(),
            add: false,
        };
        self.start_capture(change)
    }

    /// Whether a capture waits for a key: the frontend sends every key as
    /// it came, and turns the keyboard enhancement protocol on while it
    /// waits, when the terminal has it.
    pub fn capturing_key(&self) -> bool {
        self.keys.capture.is_some()
    }

    /// What the frontend knows of how its terminal reports keys; the
    /// capture prompt says it.
    pub fn set_key_reports(&mut self, reports: KeyReports) {
        self.keys.reports = reports;
    }

    /// Text the terminal pasted where no text field has the focus: a
    /// capture refuses it in words and goes on waiting; anywhere else the
    /// paste is refused, since its letters would run as keys.
    pub fn pasted(&mut self, _text: &str) -> Vec<Effect> {
        let now = self.now();
        if let Some((capture, _)) = self.keys.capture.as_mut() {
            let captured = capture.paste(now);
            return self.captured(captured);
        }
        vec![self.say(Message::info(
            "Text paste ignored here. Paste files with Paste in the Edit menu.",
        ))]
    }

    /// Opens the capture prompt for a change.
    pub(super) fn start_capture(&mut self, change: Change) -> Vec<Effect> {
        let capture = Capture::new(
            self.now(),
            self.settings.dialog_settle_ms,
            self.keymap.platform(),
        )
        .with_altgr(cfg!(windows))
        .with_reports(self.keys.reports);
        let lines = capture.lines();
        self.keys.capture = Some((capture, change.clone()));
        self.open_dialog(
            Dialog::Read {
                title: capture::PROMPT.to_owned(),
                lines,
            },
            Purpose::Keys(Box::new(KeysPurpose::Capture(change))),
        )
    }

    /// A key while a capture waits.
    pub(super) fn capture_press(&mut self, key: Key) -> Vec<Effect> {
        let now = self.now();
        let Some((capture, _)) = self.keys.capture.as_mut() else {
            return Vec::new();
        };
        let captured = capture.press(key, now);
        self.captured(captured)
    }

    /// Ten seconds without a key: the capture ends in words and the typed
    /// name is offered. The frontend's timer calls this through
    /// [`App::end_key_test`].
    pub(super) fn capture_timed_out(&mut self) -> Vec<Effect> {
        let Some((_, change)) = self.keys.capture.take() else {
            return Vec::new();
        };
        let mut effects = self.close_capture_prompt();
        effects.push(self.say(Message::info(capture::NO_KEY)));
        effects.extend(self.typed_name_field(change));
        effects
    }

    fn captured(&mut self, captured: Captured) -> Vec<Effect> {
        match captured {
            Captured::Waiting(None) => Vec::new(),
            Captured::Waiting(Some(words)) => vec![self.say(Message::info(words))],
            Captured::Cancelled => {
                self.keys.capture = None;
                let mut effects = self.close_capture_prompt();
                effects.push(self.say(Message::info(capture::CANCELLED)));
                effects
            }
            Captured::TimedOut => self.capture_timed_out(),
            Captured::Key { key, said } => {
                let Some((_, change)) = self.keys.capture.take() else {
                    return Vec::new();
                };
                let mut effects = self.close_capture_prompt();
                effects.extend(self.key_chosen(change, key, said));
                effects
            }
        }
    }

    /// Closes the capture prompt when it is the dialog on top.
    fn close_capture_prompt(&mut self) -> Vec<Effect> {
        let on_top = matches!(self.top_keys_purpose(), Some(KeysPurpose::Capture(_)));
        if on_top { self.close_top() } else { Vec::new() }
    }

    fn top_keys_purpose(&self) -> Option<&KeysPurpose> {
        match self.opens.last().map(|o| &o.purpose) {
            Some(Purpose::Keys(purpose)) => Some(purpose),
            _ => None,
        }
    }

    /// The typed name's field, the equal way in.
    pub(super) fn typed_name_field(&mut self, change: Change) -> Vec<Effect> {
        let field = EntryField::new(capture::TYPE_THE_NAME, CompletionSource::None);
        self.open_dialog(
            Dialog::edit(field),
            Purpose::Keys(Box::new(KeysPurpose::TypedName(change))),
        )
    }

    /// Enter in the typed name's field.
    fn typed_name_entered(&mut self, change: Change, text: &str) -> Vec<Effect> {
        match capture::typed_key(text) {
            Ok(key) => {
                let mut effects = self.close_top();
                let said = capture::say_back(&key, self.keymap.platform());
                effects.extend(self.key_chosen(change, key, said));
                effects
            }
            Err(problem) => vec![self.say(Message::error(problem))],
        }
    }

    /// A key was chosen, by a press or by its name: it is said back, and
    /// the change is made in this column's keymap.
    fn key_chosen(&mut self, change: Change, key: Key, said: String) -> Vec<Effect> {
        let mut effects = vec![self.say(Message::info(said))];
        effects.extend(self.make_change(&change, key, KeyChoice::Take));
        effects
    }

    /// Makes a change in the keymap the app runs on, and says it.
    fn make_change(&mut self, change: &Change, key: Key, choice: KeyChoice) -> Vec<Effect> {
        let Some(info) = commands::info(&change.command) else {
            return Vec::new();
        };
        if change.column != self.keymap.column() {
            return Vec::new();
        }
        let said = self.keymap.assign(info.layer, key, &change.command, choice);
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))]
    }

    /// Enter on a keys dialog.
    pub(super) fn keys_accept(&mut self, purpose: KeysPurpose, text: &str) -> Vec<Effect> {
        match purpose {
            KeysPurpose::Capture(_) => Vec::new(),
            KeysPurpose::TypedName(change) => self.typed_name_entered(change, text),
        }
    }

    /// Escape on a keys dialog: nothing changes.
    pub(super) fn keys_escape(&mut self, purpose: KeysPurpose) -> Vec<Effect> {
        match purpose {
            KeysPurpose::Capture(_) => {
                self.keys.capture = None;
                vec![self.say(Message::info(capture::CANCELLED))]
            }
            KeysPurpose::TypedName(_) => vec![self.say(Message::info(capture::CANCELLED))],
        }
    }
}
