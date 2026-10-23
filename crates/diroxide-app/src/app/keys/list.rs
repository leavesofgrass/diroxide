//! The Keys list's flows: opening it, typing to narrow it, a row's
//! actions, Remove a key, Reset this key, Reset all keys, and the
//! list's own rows (research/alpha3-plan.md, "The Keys list").
//!
//! The list is a text field with its list always open, as the palette is:
//! typing narrows it silently but for the count on the status line, Down
//! enters the list, Enter on a row opens its actions, Escape closes it.

use diroxide_core::format::count;
use diroxide_core::{CommandId, Effect, Key, KeyColumn, ListModel, ListRow, Message, Redraw};

use super::{Change, KeysPurpose};
use crate::app::{App, Purpose};
use crate::commands::{self, CommandInfo, ids};
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::conflicts::is_critical;
use crate::keymap::file::{self, KeyLine, KeymapEdit};
use crate::keymap::keys_list::{KeysView, actions};
use crate::keymap::presets;
use crate::keymap::written;
use crate::request::Request;

/// The list's own row that resets every key.
pub(crate) const ROW_RESET_ALL: &str = "\u{1}reset-all";
/// The list's own row for the single-letter switch.
pub(crate) const ROW_LETTERS: &str = "\u{1}letters";
/// The list's own row that chooses a preset.
pub(crate) const ROW_PRESET: &str = "\u{1}preset";

impl App {
    /// The Keys list for what was typed, built from the layers, with the
    /// list's own rows first while nothing is typed.
    fn keys_list_model(&self, query: &str) -> (String, ListModel) {
        let terminal = self.layered(KeyColumn::Terminal);
        let gui = self.layered(KeyColumn::Gui);
        let preset = self.preset();
        let view = KeysView {
            terminal: &terminal,
            gui: &gui,
            here: self.keymap.column(),
            owner: &self.keys.owner,
            preset: &preset.lines,
            preset_name: &preset.name,
        };
        let own = vec![
            ListRow::new(ROW_PRESET, "Choose a preset").with_details([preset.name.clone()]),
            ListRow::new(ROW_RESET_ALL, "Reset all keys")
                .with_details([format!("{} changed", view.changed_count())]),
            ListRow::new(ROW_LETTERS, "Single letters")
                .with_details([self.settings.value_words("single_letters")]),
        ];
        let mut list = view.list_with(query, own);
        // The plugins' commands, after the built-in ones (lane A2).
        let plugin_rows = self.plugin_key_rows(query);
        if !plugin_rows.is_empty() {
            let mut rows = list.rows().to_vec();
            rows.extend(plugin_rows);
            list.set_rows(rows);
            list.move_focus(diroxide_core::Move::Home);
        }
        (view.title(), list)
    }

    /// A row's actions, built from the layers.
    fn actions_model(&self, info: &CommandInfo) -> ListModel {
        let terminal = self.layered(KeyColumn::Terminal);
        let gui = self.layered(KeyColumn::Gui);
        let preset = self.preset();
        KeysView {
            terminal: &terminal,
            gui: &gui,
            here: self.keymap.column(),
            owner: &self.keys.owner,
            preset: &preset.lines,
            preset_name: &preset.name,
        }
        .actions(info)
    }

    /// Opens the Keys list: "Keys, preset diroxide defaults, 0 changed".
    pub(crate) fn open_keys_list(&mut self) -> Vec<Effect> {
        let (title, list) = self.keys_list_model("");
        let field = EntryField::new(title, CompletionSource::None);
        self.open_dialog(
            Dialog::Edit {
                field,
                candidates: Some(list),
                in_list: false,
            },
            Purpose::Keys(Box::new(KeysPurpose::List)),
        )
    }

    /// Builds the Keys list on top again after a change, the title's count
    /// included, the focus kept on `focus` (or on the row it was on).
    pub(crate) fn refresh_keys_list(&mut self, focus: Option<&CommandId>) -> Vec<Effect> {
        if !self.top_keys_purpose().is_some_and(KeysPurpose::is_list) {
            return Vec::new();
        }
        let text = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => field.text().to_owned(),
            _ => return Vec::new(),
        };
        let (title, mut list) = self.keys_list_model(&text);
        if let Some((
            _,
            Dialog::Edit {
                field, candidates, ..
            },
        )) = self.dialogs.last_mut()
        {
            let was = candidates
                .as_ref()
                .and_then(|l| l.focused())
                .map(|r| r.id.clone());
            let wanted = focus.map(|c| c.as_str().to_owned()).or(was);
            if let Some(id) = wanted {
                list.focus_id(&id);
            }
            if field.label() != title {
                *field = EntryField::new(title, CompletionSource::None).with_text(text);
            }
            *candidates = Some(list);
        }
        vec![Effect::Redraw(Redraw::All)]
    }

    /// The field's text changed: the list narrows, and the count is said
    /// with the first row, as the palette does.
    pub(crate) fn keys_list_typed(&mut self) -> Vec<Effect> {
        let text = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => field.text().to_owned(),
            _ => return Vec::new(),
        };
        let (_, list) = self.keys_list_model(&text);
        let shown = list.len() as u64;
        let said = match list.focused() {
            Some(first) if !text.trim().is_empty() => {
                format!("{}. {}", first.spoken(), count(shown, "match", "matches"))
            }
            _ => count(shown, "match", "matches"),
        };
        if let Some((_, Dialog::Edit { candidates, .. })) = self.dialogs.last_mut() {
            *candidates = Some(list);
        }
        vec![
            Effect::Redraw(Redraw::All),
            Effect::Announce(Message::status(said)),
        ]
    }

    /// Enter in the Keys list: a command's actions, or the list's own row.
    pub(crate) fn keys_list_enter(&mut self, row: Option<&str>) -> Vec<Effect> {
        match row {
            None => vec![self.say(Message::status("No command matches"))],
            Some(ROW_RESET_ALL) => self.reset_all_question(),
            Some(ROW_LETTERS) => self.setting_chosen("single_letters"),
            Some(ROW_PRESET) => self.preset_chooser(),
            Some(id) => {
                if let Some(effects) = CommandId::new(id)
                    .ok()
                    .and_then(|id| self.plugin_key_row_entered(&id))
                {
                    return effects;
                }
                let Some(info) = CommandId::new(id).ok().and_then(|id| commands::info(&id)) else {
                    return Vec::new();
                };
                let list = self.actions_model(info);
                self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: diroxide_core::Command::new(ids::KEYS_LIST),
                    },
                    Purpose::Keys(Box::new(KeysPurpose::Actions(info.id.clone()))),
                )
            }
        }
    }

    /// Enter on one of a row's actions.
    pub(crate) fn action_chosen(&mut self, command: &CommandId, row: Option<&str>) -> Vec<Effect> {
        let Some(info) = commands::info(command) else {
            return Vec::new();
        };
        let here = self.keymap.column();
        let other = match here {
            KeyColumn::Terminal => KeyColumn::Gui,
            KeyColumn::Gui => KeyColumn::Terminal,
        };
        let change = |column, add| Change {
            command: command.clone(),
            column,
            add,
        };
        match row {
            Some(actions::CHANGE) => {
                let mut effects = self.close_top();
                effects.extend(self.start_capture(change(here, false)));
                effects
            }
            Some(actions::OTHER) => {
                let mut effects = self.close_top();
                effects.extend(self.typed_name_field(change(other, false)));
                effects
            }
            Some(actions::ADD) => {
                let mut effects = self.close_top();
                effects.extend(self.start_capture(change(here, true)));
                effects
            }
            Some(actions::REMOVE) => {
                let keys = self.layered(here).keys_in(info.layer, command);
                match keys.as_slice() {
                    [] => vec![self.say(Message::info(format!("{} has no key.", info.name)))],
                    [one] => {
                        let one = *one;
                        let mut effects = self.close_top();
                        effects.extend(self.remove_key(command, one));
                        effects
                    }
                    many => {
                        let platform = self.keymap.platform();
                        let rows = many
                            .iter()
                            .map(|k| ListRow::new(file_name(k), written(k, platform)))
                            .collect();
                        let mut list = ListModel::new(format!("Remove which key of {}", info.name))
                            .with_rows(rows);
                        list.set_help("Up and Down choose, Enter removes it, Escape goes back.");
                        self.open_dialog(
                            Dialog::Choose {
                                list,
                                on_enter: diroxide_core::Command::new(ids::KEYS_LIST),
                            },
                            Purpose::Keys(Box::new(KeysPurpose::RemoveWhich(command.clone()))),
                        )
                    }
                }
            }
            Some(actions::RESET) => {
                let mut effects = self.close_top();
                effects.extend(self.reset_command(command));
                effects
            }
            Some(actions::TEST) => {
                let mut effects = self.close_top();
                effects.extend(self.start_key_test());
                effects
            }
            _ => Vec::new(),
        }
    }

    /// Enter in "Remove which key".
    pub(crate) fn remove_chosen(&mut self, command: &CommandId, row: Option<&str>) -> Vec<Effect> {
        let Some(key) = row.and_then(|r| Key::parse(r).ok()) else {
            return Vec::new();
        };
        // The list of keys, then the row's actions.
        let mut effects = self.close_top();
        if self
            .top_keys_purpose()
            .is_some_and(|p| matches!(p, KeysPurpose::Actions(_)))
        {
            effects.extend(self.close_top());
        }
        effects.extend(self.remove_key(command, key));
        effects
    }

    /// Takes one key away from a command in this column and writes it:
    /// "Ctrl+F9 removed from Sort by size." A critical command keeps its
    /// last key.
    fn remove_key(&mut self, command: &CommandId, key: Key) -> Vec<Effect> {
        let Some(info) = commands::info(command) else {
            return Vec::new();
        };
        let here = self.keymap.column();
        let mut keymap = self.layered(here);
        let k = written(&key, self.keymap.platform());
        if is_critical(command) && keymap.keys_in(info.layer, command).len() <= 1 {
            return vec![self.say(Message::error(format!(
                "{} needs a key, so {k} stays. Change it instead.",
                info.name
            )))];
        }
        if !keymap.unbind(info.layer, &key, command) {
            return Vec::new();
        }
        self.persist(here, &keymap, &[(info.layer, command.clone())]);
        let mut said = format!("{k} removed from {}.", info.name);
        if keymap.keys_in(info.layer, command).is_empty() {
            said.push_str(" It has no key now; the palette still runs it.");
        }
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
        effects.extend(self.refresh_keys_list(Some(command)));
        effects
    }

    /// Takes the owner's lines for a command away, in both columns, so it
    /// has its default keys again: "Sort by size back to Ctrl+F6."
    fn reset_command(&mut self, command: &CommandId) -> Vec<Effect> {
        let Some(info) = commands::info(command) else {
            return Vec::new();
        };
        let lines: Vec<KeyLine> = self
            .keys
            .owner
            .overrides
            .iter()
            .filter(|o| o.command == *command)
            .map(|o| KeyLine {
                column: o.column,
                layer: o.layer,
                command: o.command.clone(),
                keys: None,
            })
            .collect();
        if lines.is_empty() {
            return vec![self.say(Message::info(format!(
                "{} has no change of yours to reset.",
                info.name
            )))];
        }
        let edit = KeymapEdit::Lines(lines);
        file::apply_to(&mut self.keys.owner, &edit);
        self.request(Request::EditKeymap(Box::new(edit)));
        self.rebuild_keymap();
        let keys = self
            .layered(self.keymap.column())
            .keys_in(info.layer, command);
        let platform = self.keymap.platform();
        let now = if keys.is_empty() {
            "no key".to_owned()
        } else {
            keys.iter()
                .map(|k| written(k, platform))
                .collect::<Vec<_>>()
                .join(" or ")
        };
        let mut effects = vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::info(format!("{} back to {now}.", info.name))),
        ];
        effects.extend(self.refresh_keys_list(Some(command)));
        effects
    }

    /// The number of built-in commands the owner's lines change.
    fn changed_commands(&self) -> usize {
        let mut seen: Vec<&CommandId> = Vec::new();
        for line in &self.keys.owner.overrides {
            if commands::info(&line.command).is_some() && !seen.contains(&&line.command) {
                seen.push(&line.command);
            }
        }
        seen.len()
    }

    /// "Reset all keys? No is the default. y or n", or, with nothing
    /// changed, that there is nothing to reset.
    pub(crate) fn reset_all_question(&mut self) -> Vec<Effect> {
        if self.changed_commands() == 0 {
            return vec![self.say(Message::info("No key was changed. Nothing to reset."))];
        }
        self.open_dialog(
            Dialog::Confirm {
                question: with_default_no(Message::question("Reset all keys")),
                listing: Vec::new(),
                on_yes: diroxide_core::Command::new(ids::KEYS_RESET_ALL),
            },
            Purpose::Keys(Box::new(KeysPurpose::ResetAll)),
        )
    }

    /// Yes to Reset all keys: every line of the owner's for a built-in
    /// command goes; the file's comments and unknown lines stay.
    pub(crate) fn reset_all(&mut self) -> Vec<Effect> {
        let changed = self.changed_commands() as u64;
        let edit = KeymapEdit::ResetAll;
        file::apply_to(&mut self.keys.owner, &edit);
        self.request(Request::EditKeymap(Box::new(edit)));
        self.rebuild_keymap();
        let said = format!(
            "All keys reset: {} back to the default.",
            count(changed, "command", "commands")
        );
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
        effects.extend(self.refresh_keys_list(None));
        effects
    }
}

impl App {
    /// The preset chooser: each preset's name, then its description, the
    /// one in use said as current and focused.
    pub(crate) fn preset_chooser(&mut self) -> Vec<Effect> {
        let current = self.preset().id.clone();
        let rows = presets::presets()
            .iter()
            .map(|p| {
                let row = ListRow::new(p.id.clone(), p.name.clone());
                if p.id == current {
                    row.with_details(["current".to_owned(), p.description.clone()])
                } else {
                    row.with_details([p.description.clone()])
                }
            })
            .collect();
        let mut list = ListModel::new("Key presets").with_rows(rows);
        list.focus_id(&current);
        list.set_help(
            "Up and Down choose, Enter asks before applying; your own keys stay on top. Escape closes.",
        );
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: diroxide_core::Command::new(ids::KEYS_PRESET),
            },
            Purpose::Keys(Box::new(KeysPurpose::Presets)),
        )
    }

    /// Enter in the preset chooser: the question, starting on No.
    pub(crate) fn preset_chosen(&mut self, row: Option<&str>) -> Vec<Effect> {
        let Some(preset) = row.and_then(presets::preset) else {
            return Vec::new();
        };
        if preset.id == self.preset().id {
            return vec![self.say(Message::info(format!(
                "{} are in use already.",
                preset.keys_name()
            )))];
        }
        let mut effects = self.close_top();
        effects.extend(self.open_dialog(
            Dialog::Confirm {
                question: with_default_no(Message::question(&preset.question())),
                listing: Vec::new(),
                on_yes: diroxide_core::Command::new(ids::KEYS_PRESET),
            },
            Purpose::Keys(Box::new(KeysPurpose::ApplyPreset(preset.id.clone()))),
        ));
        effects
    }

    /// Yes to "Apply Total Commander keys?": the `preset` setting is
    /// written and nothing is cleared, so the owner's own lines stay on
    /// top; a preset whose letters run commands turns type-ahead off, and
    /// one whose letters jump turns it back on after one that ran them.
    pub(crate) fn apply_preset(&mut self, id: &str) -> Vec<Effect> {
        let Some(preset) = presets::preset(id) else {
            return Vec::new();
        };
        self.settings.preset = preset.id.clone();
        self.request(Request::SaveSetting {
            key: "preset".to_owned(),
            value: Some(toml::Value::String(preset.id.clone())),
        });
        let letters = presets::letters_after(preset, self.settings.single_letters);
        if let Some(letters) = letters {
            self.settings.single_letters = letters;
            self.request(Request::SaveSetting {
                key: "single_letters".to_owned(),
                value: Some(toml::Value::String(
                    presets::letters_value(letters).to_owned(),
                )),
            });
        }
        self.rebuild_keymap();
        let mut said = format!("{} applied.", preset.keys_name());
        let mine = self.changed_commands();
        if mine > 0 {
            let verb = if mine == 1 { "applies" } else { "apply" };
            said.push_str(&format!(
                " Your {} still {verb}.",
                count(mine as u64, "change", "changes")
            ));
        }
        if letters.is_some() {
            said.push_str(&format!(
                " Single letters: {}.",
                self.settings.value_words("single_letters")
            ));
        }
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
        effects.extend(self.refresh_keys_list(None));
        effects
    }
}

/// A key as `keymap.toml` writes it, for a row's id.
fn file_name(key: &Key) -> String {
    crate::keymap::for_file(key)
}
