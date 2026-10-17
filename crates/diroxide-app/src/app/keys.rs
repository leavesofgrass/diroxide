//! Keys in the app: Change key's capture, its checks and its questions,
//! the Keys list, the presets and the single-letter switch (plan section
//! 5.7, "Customizing keys"; research/alpha3-plan.md, "The Keys list").
//!
//! The pure parts live in the keymap's modules: the capture engine
//! ([`crate::keymap::capture`]). This module holds what the app keeps
//! while they run and turns their answers into dialogs and sentences.
//!
//! W3k owns this module.

use diroxide_core::{
    Command, CommandId, Effect, Key, KeyColumn, Layer, ListModel, ListRow, Message, Redraw,
};

use super::{App, Purpose};
use crate::commands::{self, ids};
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::capture::{self, Capture, Captured, KeyReports};
use crate::keymap::conflicts::{self, AskKind, Finding, Taken};
use crate::keymap::{KeyChoice, Keymap, written};

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
    /// A yes-or-no question of the checks, starting on No; the findings
    /// after it wait.
    Check {
        /// The change.
        change: Change,
        /// The key.
        key: Key,
        /// What it asks.
        kind: AskKind,
        /// The findings still to say.
        rest: Vec<Finding>,
        /// The choice made at a "Take it, swap, or cancel?" before it.
        choice: Choice,
    },
    /// "Take it, swap, or cancel?"; the findings after it wait.
    Taken {
        /// The change.
        change: Change,
        /// The key.
        key: Key,
        /// Who holds it.
        taken: Taken,
        /// The findings still to say.
        rest: Vec<Finding>,
    },
}

/// What was chosen about a key another command holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    /// Take or swap; take when no command held the key.
    pub(crate) choice: KeyChoice,
    /// The binding the key comes from, when another command held it.
    pub(crate) from: Option<(Layer, CommandId)>,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            choice: KeyChoice::Take,
            from: None,
        }
    }
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

    /// The keymap of a column as the checks see it.
    pub(super) fn column_keymap(&self, column: KeyColumn) -> Keymap {
        if column == self.keymap.column() {
            return self.keymap.clone();
        }
        Keymap::defaults_for(column).for_platform(self.keymap.platform())
    }

    /// What the checks find for a change.
    fn findings(&self, change: &Change, key: &Key) -> Vec<Finding> {
        let Some(info) = commands::info(&change.command) else {
            return Vec::new();
        };
        conflicts::analyze(
            &self.column_keymap(change.column),
            info.layer,
            key,
            &change.command,
            self.settings.single_letters,
        )
    }

    /// A key was chosen, by a press or by its name: it is said back, then
    /// the checks run, one sentence each, a question where one is needed.
    fn key_chosen(&mut self, change: Change, key: Key, said: String) -> Vec<Effect> {
        let mut effects = vec![self.say(Message::info(said))];
        let findings = self.findings(&change, &key);
        effects.extend(self.run_checks(change, key, findings, Choice::default()));
        effects
    }

    /// Says the findings in order until one asks a question, which opens a
    /// dialog starting on No with the rest waiting; with none left, the
    /// change is made.
    fn run_checks(
        &mut self,
        change: Change,
        key: Key,
        mut findings: Vec<Finding>,
        choice: Choice,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        while !findings.is_empty() {
            let finding = findings.remove(0);
            match finding {
                Finding::Refused(line) => {
                    effects.push(self.say(Message::error(line)));
                    return effects;
                }
                Finding::Note(line) => effects.push(self.say(Message::info(line))),
                Finding::Ask { line, kind } => {
                    let asked = format!("{line} {}", kind.words());
                    let question = with_default_no(Message::question(&asked));
                    effects.extend(self.open_dialog(
                        Dialog::Confirm {
                            question,
                            listing: Vec::new(),
                            on_yes: Command::new(ids::KEYS_CHANGE),
                        },
                        Purpose::Keys(Box::new(KeysPurpose::Check {
                            change,
                            key,
                            kind,
                            rest: findings,
                            choice,
                        })),
                    ));
                    return effects;
                }
                Finding::Taken(taken) => {
                    if !taken.take && taken.swap.is_none() {
                        let line = Finding::Taken(taken).said();
                        effects.push(self.say(Message::error(line)));
                        return effects;
                    }
                    effects.extend(self.taken_question(change, key, taken, findings));
                    return effects;
                }
            }
        }
        effects.extend(self.make_change(&change, key, &choice));
        effects
    }

    /// "Take it, swap, or cancel?", the focus on Cancel.
    fn taken_question(
        &mut self,
        change: Change,
        key: Key,
        taken: Taken,
        rest: Vec<Finding>,
    ) -> Vec<Effect> {
        let platform = self.keymap.platform();
        let other = commands::name_of(&taken.other);
        let mut rows = Vec::new();
        if taken.take {
            rows.push(
                ListRow::new("take", "Take it")
                    .with_details([format!("{other} loses {}", written(&key, platform))]),
            );
        }
        if let Some(old) = taken.swap {
            rows.push(
                ListRow::new("swap", "Swap")
                    .with_details([format!("{other} gets {}", written(&old, platform))]),
            );
        }
        rows.push(ListRow::new("cancel", "Cancel").with_details(["nothing changes"]));
        let mut choices = ListModel::new(taken.question()).with_rows(rows);
        choices.focus_id("cancel");
        choices.set_help("Up and Down choose, Enter answers, Escape cancels.");
        let line = Finding::Taken(taken.clone()).said();
        self.open_dialog(
            Dialog::Question {
                lines: vec![line],
                choices,
            },
            Purpose::Keys(Box::new(KeysPurpose::Taken {
                change,
                key,
                taken,
                rest,
            })),
        )
    }

    /// Yes to a check's question: the next findings follow. "Use it for
    /// the window column only?" moves the change to the window column and
    /// checks it there again.
    pub(super) fn keys_yes(&mut self, purpose: KeysPurpose) -> Vec<Effect> {
        let KeysPurpose::Check {
            mut change,
            key,
            kind,
            rest,
            choice,
        } = purpose
        else {
            return Vec::new();
        };
        if kind == AskKind::WindowOnly {
            change.column = KeyColumn::Window;
            let again = self.findings(&change, &key);
            return self.run_checks(change, key, again, Choice::default());
        }
        self.run_checks(change, key, rest, choice)
    }

    /// Makes a change in the keymap of its column, and says it: "Ctrl+F9
    /// now runs Sort by size."
    fn make_change(&mut self, change: &Change, key: Key, choice: &Choice) -> Vec<Effect> {
        let Some(info) = commands::info(&change.command) else {
            return Vec::new();
        };
        let platform = self.keymap.platform();
        let k = written(&key, platform);
        let name = info.name;
        let mut keymap = self.column_keymap(change.column);
        keymap.bind(
            info.layer,
            key,
            &change.command,
            change.add,
            choice.from.clone(),
            choice.choice,
        );
        let mut said = if change.column == self.keymap.column() {
            format!("{k} now runs {name}.")
        } else {
            format!("{k} now runs {name} in the {} column.", change.column)
        };
        if let Some((layer, other)) = &choice.from {
            let left = keymap.keys_in(*layer, other);
            let other_name = commands::name_of(other);
            let tail = match (choice.choice, left.first()) {
                (KeyChoice::Swap, Some(moved)) => {
                    format!("{other_name} moved to {}.", written(moved, platform))
                }
                (_, None) => format!("{other_name} has no key now."),
                (_, Some(_)) => {
                    let keys: Vec<String> = left.iter().map(|k| written(k, platform)).collect();
                    format!("{other_name} keeps {}.", keys.join(" or "))
                }
            };
            said.push(' ');
            said.push_str(&tail);
        }
        if change.column == self.keymap.column() {
            self.keymap = keymap;
        }
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))]
    }

    /// Enter on a keys dialog.
    pub(super) fn keys_accept(
        &mut self,
        purpose: KeysPurpose,
        row: Option<&str>,
        text: &str,
    ) -> Vec<Effect> {
        match purpose {
            KeysPurpose::Capture(_) => Vec::new(),
            KeysPurpose::TypedName(change) => self.typed_name_entered(change, text),
            // Enter on a yes-or-no question answers No.
            KeysPurpose::Check { .. } => self.dialog_escape(),
            KeysPurpose::Taken {
                change,
                key,
                taken,
                rest,
            } => {
                if self.settling() {
                    return Vec::new();
                }
                let choice = match row {
                    Some("take") => KeyChoice::Take,
                    Some("swap") => KeyChoice::Swap,
                    _ => return self.dialog_escape(),
                };
                let mut effects = self.close_top();
                let from = Some((taken.layer, taken.other.clone()));
                effects.extend(self.run_checks(change, key, rest, Choice { choice, from }));
                effects
            }
        }
    }

    /// Escape on a keys dialog: nothing changes.
    pub(super) fn keys_escape(&mut self, purpose: KeysPurpose) -> Vec<Effect> {
        if let KeysPurpose::Capture(_) = purpose {
            self.keys.capture = None;
        }
        vec![self.say(Message::info(capture::CANCELLED))]
    }
}
