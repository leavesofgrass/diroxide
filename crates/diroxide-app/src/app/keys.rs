//! Keys in the app: Change key's capture, its checks and its questions,
//! the Keys list, the presets and the single-letter switch (plan section
//! 5.7, "Customizing keys"; research/alpha3-plan.md, "The Keys list").
//!
//! The pure parts live in the keymap's modules: the capture engine
//! ([`crate::keymap::capture`]), the conflict analysis
//! ([`crate::keymap::conflicts`]), the Keys list's rows
//! ([`crate::keymap::keys_list`]) and the `keymap.toml` writer
//! ([`crate::keymap::file`]). This module holds what the app keeps while
//! they run, turns their answers into dialogs and sentences, and asks the
//! runtime to write each change; `list` holds the Keys list's own flows.
//!
//! The keys the app runs on are layers: the defaults, then the owner's
//! `keymap.toml` lines, then the single-letter setting. Every change
//! writes the owner's lines it touched and builds the keys again from the
//! layers, so what runs now is what the next start reads.
//!
//! W3k owns this module.

mod list;

use diroxide_config::KeymapConfig;
use diroxide_core::{
    Command, CommandId, Effect, Key, KeyColumn, Layer, ListModel, ListRow, Message, Redraw,
};

use super::{App, Purpose};
use crate::commands::{self, ids};
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::capture::{self, Capture, Captured, KeyReports};
use crate::keymap::conflicts::{self, AskKind, Finding, Taken};
use crate::keymap::file::{self, KeyLine, KeymapEdit};
use crate::keymap::presets::{self, Preset};
use crate::keymap::{KeyChoice, Keymap, written};
use crate::request::Request;

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
    /// The Keys list: a field that narrows it, and the list.
    List,
    /// A row's actions.
    Actions(CommandId),
    /// Which of a command's keys to remove.
    RemoveWhich(CommandId),
    /// "Reset all keys? y or n".
    ResetAll,
    /// The preset chooser.
    Presets,
    /// "Apply Total Commander keys? y or n", by the preset's id.
    ApplyPreset(String),
}

impl KeysPurpose {
    /// Whether this is the Keys list, whose field narrows it.
    pub(super) fn is_list(&self) -> bool {
        matches!(self, KeysPurpose::List)
    }
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
    /// The owner's `keymap.toml` lines, as the file holds them after every
    /// change the app asked the runtime to write.
    pub(super) owner: KeymapConfig,
}

impl KeysState {
    /// The state with the owner's lines read at start.
    pub(super) fn new(owner: KeymapConfig) -> Self {
        Self {
            owner,
            ..Self::default()
        }
    }
}

/// Whether a command is one of the keys commands this module runs.
pub(super) fn is_keys_command(id: &CommandId) -> bool {
    [
        ids::KEYS_CHANGE,
        ids::KEYS_LIST,
        ids::KEYS_RESET_ALL,
        ids::KEYS_SINGLE_LETTERS,
        ids::KEYS_PRESET,
    ]
    .contains(id)
}

impl App {
    /// Runs a keys command: the Keys list, Change a key (with a `command`
    /// argument, that command's capture; without, the Keys list), Reset
    /// all keys and the single-letter switch.
    pub(super) fn keys_dispatch(&mut self, command: &Command) -> Vec<Effect> {
        if !self.dialogs.is_empty() && self.top_keys_purpose().is_none() {
            return vec![self.say(Message::status("Close this dialog first."))];
        }
        match &command.id {
            id if *id == ids::KEYS_CHANGE => {
                let Some(text) = command.text("command") else {
                    return self.open_keys_list();
                };
                match CommandId::new(text)
                    .ok()
                    .filter(|t| commands::info(t).is_some())
                {
                    Some(target) => self.change_key(&target),
                    None => vec![self.say(Message::error(format!(
                        "No command has the id {text}. Nothing changed."
                    )))],
                }
            }
            id if *id == ids::KEYS_LIST => self.open_keys_list(),
            id if *id == ids::KEYS_RESET_ALL => self.reset_all_question(),
            id if *id == ids::KEYS_SINGLE_LETTERS => self.setting_chosen("single_letters"),
            id if *id == ids::KEYS_PRESET => self.preset_chooser(),
            _ => Vec::new(),
        }
    }

    /// One column's keys from the layers: the defaults for this system,
    /// then the owner's lines; every key, whatever the single-letter
    /// setting.
    pub(super) fn layered(&self, column: KeyColumn) -> Keymap {
        self.base_keymap(column).with_overrides(&self.keys.owner)
    }

    /// One column's keys under the owner's lines: the defaults, then the
    /// chosen preset.
    pub(super) fn base_keymap(&self, column: KeyColumn) -> Keymap {
        self.preset().keymap(column, self.keymap.platform())
    }

    /// The preset the `preset` setting chooses, or the diroxide defaults.
    pub(super) fn preset(&self) -> &'static Preset {
        presets::chosen(&self.settings.preset)
    }

    /// Builds the keys the app runs on again from the layers and the
    /// single-letter setting.
    pub(super) fn rebuild_keymap(&mut self) {
        self.keymap = self
            .layered(self.keymap.column())
            .with_single_letters(self.settings.single_letters);
    }

    /// Change key for a command, as the Keys list's Change key does: the
    /// capture prompt opens in this keymap's column and waits for one key
    /// ("Press the new key now. Escape cancels."). App-side hook for the
    /// gui and the tests; the Keys list calls it too.
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

    /// What the checks find for a change.
    fn findings(&self, change: &Change, key: &Key) -> Vec<Finding> {
        let Some(info) = commands::info(&change.command) else {
            return Vec::new();
        };
        conflicts::analyze(
            &self.layered(change.column),
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
    /// the GUI column only?" moves the change to the GUI column and
    /// checks it there again.
    pub(super) fn keys_yes(&mut self, purpose: KeysPurpose) -> Vec<Effect> {
        if purpose == KeysPurpose::ResetAll {
            return self.reset_all();
        }
        if let KeysPurpose::ApplyPreset(id) = &purpose {
            return self.apply_preset(id);
        }
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
        if kind == AskKind::GuiOnly {
            change.column = KeyColumn::Gui;
            let again = self.findings(&change, &key);
            return self.run_checks(change, key, again, Choice::default());
        }
        self.run_checks(change, key, rest, choice)
    }

    /// Writes the owner's lines a change touched: each command's keys in
    /// its layer as the changed keymap has them, or the line taken away
    /// when they are the keys under the owner's lines again. The runtime
    /// writes `keymap.toml`; the app's keys are built again from the
    /// layers.
    pub(super) fn persist(
        &mut self,
        column: KeyColumn,
        changed_keymap: &Keymap,
        changed: &[(Layer, CommandId)],
    ) {
        let base = self.base_keymap(column);
        let sorted = |mut keys: Vec<Key>| {
            keys.sort();
            keys
        };
        let lines = |explicit: bool| -> Vec<KeyLine> {
            changed
                .iter()
                .map(|(layer, command)| {
                    let now = changed_keymap.keys_in(*layer, command);
                    let back = sorted(now.clone()) == sorted(base.keys_in(*layer, command));
                    KeyLine {
                        column,
                        layer: *layer,
                        command: command.clone(),
                        keys: (explicit || !back).then_some(now),
                    }
                })
                .collect()
        };
        // A line taken away must give back what the change made; when
        // another line of the owner's would take a key again, every line
        // is written as it is.
        let mut edit = KeymapEdit::Lines(lines(false));
        let mut trial = self.keys.owner.clone();
        file::apply_to(&mut trial, &edit);
        let rebuilt = base.clone().with_overrides(&trial);
        let same = changed.iter().all(|(layer, command)| {
            sorted(rebuilt.keys_in(*layer, command))
                == sorted(changed_keymap.keys_in(*layer, command))
        });
        if !same {
            edit = KeymapEdit::Lines(lines(true));
        }
        file::apply_to(&mut self.keys.owner, &edit);
        self.request(Request::EditKeymap(Box::new(edit)));
        if column == self.keymap.column() {
            self.rebuild_keymap();
        }
    }

    /// Makes a change in its column, writes it and says it: "Ctrl+F9
    /// saved for Sort by size."
    fn make_change(&mut self, change: &Change, key: Key, choice: &Choice) -> Vec<Effect> {
        let Some(info) = commands::info(&change.command) else {
            return Vec::new();
        };
        let platform = self.keymap.platform();
        let k = written(&key, platform);
        let name = info.name;
        let mut keymap = self.layered(change.column);
        let changed = keymap.bind(
            info.layer,
            key,
            &change.command,
            change.add,
            choice.from.clone(),
            choice.choice,
        );
        self.persist(change.column, &keymap, &changed);
        let mut said = if change.column == self.keymap.column() {
            format!("{k} saved for {name}.")
        } else {
            format!("{k} saved for {name} in the {} column.", change.column)
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
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
        effects.extend(self.refresh_keys_list(Some(&change.command)));
        effects
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
            KeysPurpose::List => self.keys_list_enter(row),
            KeysPurpose::Actions(command) => self.action_chosen(&command, row),
            KeysPurpose::RemoveWhich(command) => self.remove_chosen(&command, row),
            KeysPurpose::Presets => self.preset_chosen(row),
            // Enter on a yes-or-no question answers No.
            KeysPurpose::ResetAll | KeysPurpose::ApplyPreset(_) => self.dialog_escape(),
        }
    }

    /// Escape on a keys dialog: nothing changes. Leaving the Keys list or
    /// a row's actions says nothing more than the dialog's closing.
    pub(super) fn keys_escape(&mut self, purpose: KeysPurpose) -> Vec<Effect> {
        match purpose {
            KeysPurpose::List
            | KeysPurpose::Actions(_)
            | KeysPurpose::RemoveWhich(_)
            | KeysPurpose::Presets => Vec::new(),
            KeysPurpose::Capture(_) => {
                self.keys.capture = None;
                vec![self.say(Message::info(capture::CANCELLED))]
            }
            _ => vec![self.say(Message::info(capture::CANCELLED))],
        }
    }
}
