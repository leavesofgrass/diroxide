//! Selection, type-ahead, sorting, the name filter and hidden files.
//!
//! Marks are said by count first ("3 items selected"), Space says the row
//! and the new count ("Selected report.pdf. 1 selected."), and typing
//! jumps to the first row that starts with what was typed, with the
//! matches counted ("report.pdf, 1 of 3 matches"; the owner's answer 74).

use std::collections::HashSet;
use std::sync::Arc;

use diroxide_core::format::count;
use diroxide_core::{Command, CommandId, Effect, Message, Move, Pattern, Redraw, Side, SortKey};

use super::{App, KEPT_LIMIT, Kept, Purpose, Quick};
use crate::commands::{self, ids};
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::millis_between;
use crate::state;

/// How long type-ahead waits for the next letter before it starts over.
const TYPE_AHEAD_PAUSE_MS: u64 = 1000;

impl App {
    pub(super) fn toggle_select(&mut self) -> Vec<Effect> {
        let side = self.active;
        let list = self.panel_mut(side).list_mut();
        let Some(selected) = list.toggle_selected() else {
            let empty = list.empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let label = list.focused().map(|r| r.label.clone()).unwrap_or_default();
        let total = list.selected_count();
        list.move_focus(Move::Down);
        let text = if selected {
            format!("Selected {label}. {total} selected.")
        } else {
            format!("Not selected {label}. {total} selected.")
        };
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    fn selection_said(&mut self, side: Side) -> Vec<Effect> {
        let selected = self.panel(side).list().selected_count() as u64;
        let text = if selected == 0 {
            "Nothing selected".to_owned()
        } else {
            format!("{} selected", count(selected, "item", "items"))
        };
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    pub(super) fn select_all(&mut self) -> Vec<Effect> {
        let side = self.active;
        self.keep_selection(side);
        self.panel_mut(side).list_mut().select_all();
        self.selection_said(side)
    }

    pub(super) fn select_none(&mut self) -> Vec<Effect> {
        let side = self.active;
        self.keep_selection(side);
        self.panel_mut(side).list_mut().select_none();
        self.selection_said(side)
    }

    pub(super) fn select_invert(&mut self) -> Vec<Effect> {
        let side = self.active;
        self.keep_selection(side);
        self.panel_mut(side).list_mut().invert_selection();
        self.selection_said(side)
    }

    /// Keeps a panel's marks, if it has any, for Restore selection, before
    /// a wholesale change replaces them: select all, none or invert, a
    /// pattern, a finished job, or leaving the folder. One kept selection
    /// a folder, for the newest 16 folders.
    pub(super) fn keep_selection(&mut self, side: Side) {
        let panel = self.panel(side);
        let ids: Vec<String> = panel
            .list()
            .selected_ids()
            .into_iter()
            .map(str::to_owned)
            .collect();
        if ids.is_empty() {
            return;
        }
        let place = panel.place().clone();
        self.kept.retain(|kept| kept.place != place);
        self.kept.push(Kept { place, ids });
        if self.kept.len() > KEPT_LIMIT {
            self.kept.remove(0);
        }
    }

    /// Restore selection: the marks the last wholesale change in this
    /// folder replaced come back, and the marks they replace are kept in
    /// turn, so a second Restore swaps back. "Selection restored: 31
    /// selected." Rows no longer in the list are counted, never guessed.
    pub(super) fn restore_selection(&mut self) -> Vec<Effect> {
        let side = self.active;
        let place = self.panel(side).place().clone();
        let Some(index) = self.kept.iter().position(|kept| kept.place == place) else {
            let text = "No earlier selection in this folder to restore";
            return vec![self.say(Message::status(text))];
        };
        let wanted: HashSet<&str> = self.kept[index].ids.iter().map(String::as_str).collect();
        let present = self
            .panel(side)
            .list()
            .rows()
            .iter()
            .filter(|row| wanted.contains(row.id.as_str()))
            .count();
        let missing = wanted.len().saturating_sub(present);
        if present == 0 {
            let text = format!(
                "Nothing restored: the earlier selection's {} not in the list",
                count(missing as u64, "item is", "items are")
            );
            return vec![self.say(Message::status(text))];
        }
        let kept = self.kept.remove(index);
        self.keep_selection(side);
        let wanted: HashSet<String> = kept.ids.into_iter().collect();
        let list = self.panel_mut(side).list_mut();
        list.select_none();
        list.select_matching(|row| wanted.contains(&row.id));
        let total = list.selected_count();
        let mut text = format!("Selection restored: {total} selected.");
        if missing > 0 {
            let gone = count(missing as u64, "is", "are");
            text = format!("{text} {gone} not in the list.");
        }
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    /// Select or unselect by pattern: a command with a `pattern` applies
    /// it; otherwise the prompt opens on `*`, so Enter alone is all.
    pub(super) fn pattern_prompt(&mut self, select: bool, command: &Command) -> Vec<Effect> {
        let side = self.active;
        if let Some(pattern) = command.text("pattern") {
            let pattern = pattern.to_owned();
            return self.apply_pattern(side, select, &pattern);
        }
        let label = if select {
            "Select by pattern"
        } else {
            "Unselect by pattern"
        };
        let field = EntryField::new(label, CompletionSource::History)
            .with_text("*")
            .with_history(state::keys::PATTERN);
        self.open_dialog(
            crate::dialog::Dialog::edit(field),
            Purpose::Pattern { side, select },
        )
    }

    pub(super) fn apply_pattern(&mut self, side: Side, select: bool, text: &str) -> Vec<Effect> {
        let pattern = match Pattern::parse(text) {
            Ok(pattern) => pattern,
            Err(error) => {
                use diroxide_core::Explain;
                let line = format!("{}. {}", error.line(), error.next_step());
                return vec![self.say(Message::error(line))];
            }
        };
        self.remember(state::keys::PATTERN, text);
        self.keep_selection(side);
        let matching: HashSet<String> = self
            .panel(side)
            .entries()
            .iter()
            .filter(|e| pattern.matches(&e.name, e.is_folder_like()))
            .map(|e| e.name.clone())
            .collect();
        let list = self.panel_mut(side).list_mut();
        let changed = if select {
            let before = list.selected_count();
            list.select_matching(|row| matching.contains(&row.id));
            list.selected_count() - before
        } else {
            let keep: HashSet<String> = list
                .selected_ids()
                .into_iter()
                .filter(|id| !matching.contains(*id))
                .map(str::to_owned)
                .collect();
            let before = list.selected_count();
            list.select_none();
            list.select_matching(|row| keep.contains(&row.id));
            before - list.selected_count()
        };
        let total = list.selected_count();
        let verb = if select { "Selected" } else { "Unselected" };
        let text = format!(
            "{verb} {changed} matching {}. {total} selected.",
            pattern.text()
        );
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    pub(super) fn next_selected(&mut self, forward: bool) -> Vec<Effect> {
        let side = self.active;
        let list = self.panel(side).list();
        let focus = list.focus_index().unwrap_or(0);
        let rows = list.rows();
        let found = if forward {
            (focus + 1..rows.len()).find(|&i| rows[i].selected)
        } else {
            (0..focus).rev().find(|&i| rows[i].selected)
        };
        match found {
            Some(index) => {
                self.panel_mut(side).list_mut().move_focus(Move::To(index));
                self.status.clear();
                self.focus_effects(side)
            }
            None => {
                let text = if forward {
                    "No selected row below"
                } else {
                    "No selected row above"
                };
                vec![self.say(Message::status(text))]
            }
        }
    }

    /// Type-ahead: the typed text so far jumps to the first row, from the
    /// focus on, that starts with it; the same letter again goes to the
    /// next such row.
    pub(super) fn type_ahead(&mut self, letter: char) -> Vec<Effect> {
        let side = self.active;
        let now = self.now();
        let mut text = match &self.quick {
            Some(quick) if millis_between(quick.at, now) < TYPE_AHEAD_PAUSE_MS => {
                quick.text.clone()
            }
            _ => String::new(),
        };
        text.push(letter);
        let lower: Vec<char> = text.to_lowercase().chars().collect();
        let repeated = lower.len() > 1 && lower.iter().all(|c| *c == lower[0]);
        let mut matches = self.panel(side).prefix_matches(&text);
        let mut cycle = false;
        if matches.is_empty() && repeated {
            text = letter.to_string();
            matches = self.panel(side).prefix_matches(&text);
            cycle = true;
        } else if repeated {
            cycle = true;
        }
        if matches.is_empty() {
            self.quick = None;
            let text = format!("No name starts with {text}");
            return vec![self.say(Message::status(text))];
        }
        self.quick = Some(Quick {
            text: text.clone(),
            at: now,
        });
        let focus = self.panel(side).list().focus_index().unwrap_or(0);
        let fresh = text.chars().count() == 1;
        let target = if cycle || fresh {
            matches
                .iter()
                .copied()
                .find(|&i| i > focus)
                .unwrap_or(matches[0])
        } else if matches.contains(&focus) {
            focus
        } else {
            matches
                .iter()
                .copied()
                .find(|&i| i > focus)
                .unwrap_or(matches[0])
        };
        self.panel_mut(side).list_mut().move_focus(Move::To(target));
        let position = matches.iter().position(|&i| i == target).unwrap_or(0) + 1;
        let label = self
            .panel(side)
            .list()
            .focused()
            .map(|r| r.label.clone())
            .unwrap_or_default();
        let said = format!("{label}, {position} of {} matches", matches.len());
        vec![self.redraw_panel(side), self.say(Message::info(said))]
    }

    pub(super) fn toggle_hidden(&mut self) -> Vec<Effect> {
        let side = self.active;
        let show = !self.panel(side).shows_hidden();
        let clock = Arc::clone(&self.clock);
        self.panels[side.index()].set_show_hidden(show, clock.as_ref());
        let text = if show {
            "Hidden files shown"
        } else {
            "Hidden files not shown"
        };
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    /// The name filter: a command with a `pattern` applies it; otherwise
    /// the filter prompt opens and narrows the panel as you type.
    pub(super) fn filter_prompt(&mut self, command: &Command) -> Vec<Effect> {
        let side = self.active;
        if let Some(pattern) = command.text("pattern") {
            let pattern = pattern.to_owned();
            self.filter_live(side, &pattern);
            return self.filter_done(side, &pattern);
        }
        let previous = self.panel(side).filter().cloned();
        let text = previous
            .as_ref()
            .map(|p| p.text().to_owned())
            .unwrap_or_default();
        let field = EntryField::new("Filter", CompletionSource::History)
            .with_text(text)
            .with_history(state::keys::FILTER);
        self.open_dialog(
            crate::dialog::Dialog::edit(field),
            Purpose::Filter(side, previous),
        )
    }

    /// Applies the filter as typed; a pattern that cannot be read yet
    /// leaves the panel as it was. Returns the count line.
    pub(super) fn filter_live(&mut self, side: Side, text: &str) -> Option<String> {
        let clock = Arc::clone(&self.clock);
        if text.trim().is_empty() {
            self.panels[side.index()].set_filter(None, clock.as_ref());
        } else {
            let pattern = Pattern::parse(text).ok()?;
            self.panels[side.index()].set_filter(Some(pattern), clock.as_ref());
        }
        let (shown, all) = self.panel(side).filter_counts();
        Some(format!("{shown} of {all} match"))
    }

    pub(super) fn filter_done(&mut self, side: Side, text: &str) -> Vec<Effect> {
        if text.trim().is_empty() {
            return self.filter_clear();
        }
        match Pattern::parse(text) {
            Ok(pattern) => {
                self.remember(state::keys::FILTER, text);
                let (shown, all) = self.panel(side).filter_counts();
                let line = format!("Filter {}: {shown} of {all} shown", pattern.text());
                vec![
                    self.redraw_panel(side),
                    Effect::Redraw(Redraw::TitleLine),
                    self.say(Message::info(line)),
                ]
            }
            Err(error) => {
                use diroxide_core::Explain;
                let line = format!("{}. {}", error.line(), error.next_step());
                vec![self.say(Message::error(line))]
            }
        }
    }

    pub(super) fn filter_clear(&mut self) -> Vec<Effect> {
        let side = self.active;
        let clock = Arc::clone(&self.clock);
        self.panels[side.index()].set_filter(None, clock.as_ref());
        let shown = self.panel(side).list().len() as u64;
        let text = format!("Filter cleared: {}", count(shown, "item", "items"));
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    /// Sorts the active panel by a sort command's key; the same key again
    /// reverses it. The cursor stays on its entry, and the result is said
    /// first: "Sorted by size, largest first".
    pub(super) fn sort_by(&mut self, id: &CommandId) -> Vec<Effect> {
        let Some(key) = commands::sort_key_for(id) else {
            return Vec::new();
        };
        let side = self.active;
        let spec = self.panel(side).sort().choose(key.clone());
        let clock = Arc::clone(&self.clock);
        self.panels[side.index()].set_sort(spec.clone(), clock.as_ref());
        let mut text = spec.describe();
        let entries = self.panel(side).entries();
        let has = |entry: &diroxide_core::Entry| match &key {
            SortKey::Created => entry.created.is_some(),
            SortKey::Accessed => entry.accessed.is_some(),
            SortKey::Owner => entry.owner.is_some(),
            SortKey::Label => !entry.labels.is_empty(),
            SortKey::Attributes => entry.attributes.is_some(),
            _ => true,
        };
        if !entries.is_empty() && !entries.iter().any(has) {
            let note = match key {
                SortKey::Label => self.no_labels_here_note(),
                other => format!("No entry here has its {}", other.word()),
            };
            text = format!("{text}. {note}.");
        }
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    /// The Sort list: every key, the current one marked with its
    /// direction; Enter sorts, the same key again reverses.
    pub(super) fn sort_list(&mut self) -> Vec<Effect> {
        let side = self.active;
        let list = commands::sort_list(self.panel(side).sort());
        self.open_dialog(
            crate::dialog::Dialog::Choose {
                list,
                on_enter: Command::new(ids::SORT_LIST),
            },
            Purpose::SortList,
        )
    }
}
