//! Labels in the app (plan commits 22 and 23): Set label (a field with
//! the labels listed under it, a new name making a new label), Toggle
//! label, Clear labels and the favorite keys act on the marked items, or
//! the focused one, and say one sentence ("Label urgent added to 3
//! items"); Undo takes the last label change back as one step ("Undid
//! label urgent on 3 items") when it came after the last job. The Labels
//! list says each label's count and, once checked, how many of its items
//! are gone ("Urgent, label, 14 items, 2 missing"); Enter lists the
//! label's items, and Enter on one shows it in its folder.

use std::collections::HashSet;
use std::sync::Arc;

use diroxide_core::format::count;
use diroxide_core::{Command, Effect, JobId, ListModel, ListRow, Message, Place, Side};

use super::App;
use super::organize::{Check, Wants};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::labels::{LabelChange, place_key};
use crate::panel::place_leaf;

/// How many label changes Undo keeps.
const LABEL_UNDO_LIMIT: usize = 20;

/// The row id of "New label" in the Set label list.
const NEW_LABEL_ROW: &str = "new-label";

/// The favorite keys' commands, 1 to 7.
const FAVORITE_COMMANDS: [diroxide_core::CommandId; 7] = [
    ids::LABEL_FAVORITE_1,
    ids::LABEL_FAVORITE_2,
    ids::LABEL_FAVORITE_3,
    ids::LABEL_FAVORITE_4,
    ids::LABEL_FAVORITE_5,
    ids::LABEL_FAVORITE_6,
    ids::LABEL_FAVORITE_7,
];

impl App {
    /// The places a label command acts on, the marked items or the focused
    /// one, and the item's name when there is one.
    fn label_targets(&self, side: Side) -> Option<(Vec<Place>, Option<String>)> {
        let (places, _) = self.frozen(side)?;
        let one = match places.as_slice() {
            [one] => one
                .name()
                .map(|n| diroxide_core::names::display_name(n).into_owned()),
            _ => None,
        };
        Some((places, one))
    }

    /// "3 items", or the item's name.
    fn targets_words(places: &[Place], one: Option<&str>) -> String {
        match (places.len(), one) {
            (1, Some(name)) => name.to_owned(),
            (n, _) => count(n as u64, "item", "items"),
        }
    }

    /// Set label or Toggle label: a command with a `label` applies it at
    /// once; otherwise the field opens with the labels listed under it.
    pub(super) fn label_prompt(
        &mut self,
        side: Side,
        toggle: bool,
        command: &Command,
    ) -> Vec<Effect> {
        let Some((places, one)) = self.label_targets(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if let Some(label) = command.text("label") {
            let label = match crate::labels::check_name(label) {
                Ok(label) => label,
                Err(why) => return vec![self.say(Message::error(why))],
            };
            let change = if toggle {
                LabelChange::Toggle(label)
            } else {
                LabelChange::Add(label)
            };
            return self.change_labels(side, change, places, one);
        }
        let verb = if toggle {
            "Toggle label on"
        } else {
            "Set label on"
        };
        let label = format!("{verb} {}", Self::targets_words(&places, one.as_deref()));
        let field = EntryField::new(label, CompletionSource::None);
        let mut effects = self.open_organize(
            Dialog::edit(field),
            Wants::Label {
                side,
                places,
                one,
                toggle,
            },
        );
        effects.extend(self.refresh_candidates());
        effects
    }

    /// The Set label list for what was typed: the labels that start with
    /// it, then those that contain it, each with its color and how many
    /// of the items have it; then "New label" with the typed name.
    pub(super) fn label_choices(&self, places: &[Place], text: &str) -> ListModel {
        let typed = text.trim();
        let wanted = typed.to_lowercase();
        let store = &self.organize.labels;
        let mut starting = Vec::new();
        let mut containing = Vec::new();
        let mut exact = false;
        for label in store.labels() {
            let name = label.name.to_lowercase();
            exact |= name == wanted;
            let on = places
                .iter()
                .filter(|p| {
                    store
                        .labels_of(p)
                        .iter()
                        .any(|l| l.eq_ignore_ascii_case(&label.name))
                })
                .count();
            let mut details = vec!["label".to_owned(), label.color.word().to_owned()];
            match (on, places.len()) {
                (0, _) => {}
                (1, 1) => details.push("already on it".to_owned()),
                (on, all) if on == all => details.push(format!("on all {all}")),
                (on, all) => details.push(format!("on {on} of {all}")),
            }
            let row = ListRow::new(label.name.clone(), label.name.clone()).with_details(details);
            if name.starts_with(&wanted) {
                starting.push(row);
            } else if name.contains(&wanted) {
                containing.push(row);
            }
        }
        starting.extend(containing);
        if !exact {
            let new = if typed.is_empty() {
                ListRow::new(NEW_LABEL_ROW, "New label").with_details(["type its name first"])
            } else {
                ListRow::new(NEW_LABEL_ROW, format!("New label {typed}"))
            };
            starting.push(new);
        }
        let mut list = ListModel::new("Labels").with_rows(starting);
        list.set_help("Type to narrow, Up and Down choose, Enter sets the label, Escape closes.");
        list
    }

    /// Enter in the Set label field: the label chosen in the list, the
    /// label named exactly, the first match, or a new label of the typed
    /// name.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn label_chosen(
        &mut self,
        side: Side,
        places: Vec<Place>,
        one: Option<String>,
        toggle: bool,
        row: Option<String>,
        text: &str,
        in_list: bool,
    ) -> Vec<Effect> {
        let typed = text.trim();
        let exact = self.organize.labels.label(typed).map(|l| l.name.clone());
        let chosen = match (in_list, exact) {
            (false, Some(name)) => Some(name),
            _ => match row.as_deref() {
                Some(NEW_LABEL_ROW) | None => (!typed.is_empty()).then(|| typed.to_owned()),
                Some(name) => Some(name.to_owned()),
            },
        };
        let Some(name) = chosen else {
            return vec![self.say(Message::status("Type the new label's name first."))];
        };
        if let Err(why) = crate::labels::check_name(&name) {
            return vec![self.say(Message::error(why))];
        }
        let mut effects = self.close_top();
        let change = if toggle {
            LabelChange::Toggle(name)
        } else {
            LabelChange::Add(name)
        };
        effects.extend(self.change_labels(side, change, places, one));
        effects
    }

    /// Clear labels on the marked items, or the focused one.
    pub(super) fn clear_labels(&mut self, side: Side) -> Vec<Effect> {
        let Some((places, one)) = self.label_targets(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        self.change_labels(side, LabelChange::Clear, places, one)
    }

    /// A favorite key: toggles the label in that place of the Labels list.
    pub(super) fn favorite_label(&mut self, side: Side, number: usize) -> Vec<Effect> {
        let Some(name) = self
            .organize
            .labels
            .favorite(number)
            .map(|l| l.name.clone())
        else {
            let have = self.organize.labels.labels().len() as u64;
            let text = format!(
                "No label {number} yet. The Labels list has {}.",
                count(have, "label", "labels")
            );
            return vec![self.say(Message::info(text))];
        };
        let Some((places, one)) = self.label_targets(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        self.change_labels(side, LabelChange::Toggle(name), places, one)
    }

    /// The newest job in the undo log, 0 for none: label changes and jobs
    /// are undone newest first between them.
    fn newest_job(&self) -> u64 {
        self.undo
            .last()
            .and_then(|record| record.job)
            .map_or(0, JobId::get)
    }

    /// Changes labels, keeps the undo step, saves, shows the new labels in
    /// both panels, and says the one sentence.
    pub(super) fn change_labels(
        &mut self,
        side: Side,
        change: LabelChange,
        places: Vec<Place>,
        one: Option<String>,
    ) -> Vec<Effect> {
        let before = self.organize.labels.labels().len();
        let after_job = self.newest_job();
        let applied = self.organize.labels.apply(&change, &places, one.as_deref());
        let made = self.organize.labels.labels().get(before).cloned();
        if let Some(undo) = applied.undo.clone() {
            let undo_log = &mut self.organize.label_undo;
            undo_log.push((after_job, undo));
            if undo_log.len() > LABEL_UNDO_LIMIT {
                undo_log.remove(0);
            }
        }
        let mut text = applied.sentence();
        if let Some(label) = &made {
            text.push_str(&format!("; a new label, {}", label.color.word()));
        }
        if applied.changed > 0 || made.is_some() {
            self.save_labels();
        }
        let mut effects = self.relabel_panels();
        effects.push(self.redraw_panel(side));
        effects.push(self.say(Message::info(text)));
        effects
    }

    /// Shows the store's labels on both panels' entries: the rows are
    /// rebuilt, the focus and the marks kept by id.
    pub(super) fn relabel_panels(&mut self) -> Vec<Effect> {
        let clock = Arc::clone(&self.clock);
        let mut effects = Vec::new();
        for side in Side::BOTH {
            let panel = &self.panels[side.index()];
            if !panel.is_listed() {
                continue;
            }
            let mut entries = panel.all_entries().to_vec();
            let place = panel.place().clone();
            self.organize.labels.apply_to_entries(&place, &mut entries);
            if entries
                .iter()
                .map(|e| &e.labels)
                .eq(panel.all_entries().iter().map(|e| &e.labels))
            {
                continue;
            }
            self.panels[side.index()].set_entries(entries, &self.types, clock.as_ref());
            if side == self.active || !self.screen_reader {
                effects.push(self.redraw_panel(side));
            }
        }
        effects
    }

    /// Undo, when the last label change came after the last job: puts the
    /// labels back as they were and says so ("Undid label urgent on 3
    /// items"). `None` leaves Undo to the job log.
    pub(super) fn undo_label_change(&mut self) -> Option<Vec<Effect>> {
        let (after_job, _) = self.organize.label_undo.last()?;
        if self.newest_job() > *after_job {
            return None;
        }
        let (_, undo) = self.organize.label_undo.pop()?;
        self.organize.labels.restore(&undo);
        self.save_labels();
        let mut effects = self.relabel_panels();
        effects.push(self.redraw_panel(self.active));
        effects.push(self.say(Message::info(undo.sentence())));
        Some(effects)
    }

    /// The Labels list: each label with its count, and how many of its
    /// items are gone once the runtime has looked ("Urgent, label, 14
    /// items, 2 missing"). Opening it starts that look.
    pub(super) fn labels_list(&mut self) -> Vec<Effect> {
        let places: Vec<Place> = self
            .organize
            .labels
            .items()
            .map(|item| item.place.clone())
            .collect();
        self.organize.check = None;
        if !places.is_empty() {
            let request = self.ids.request();
            self.organize.check = Some(Check {
                request,
                missing: None,
                unreadable: 0,
            });
            self.request(crate::request::Request::CheckPlaces { request, places });
        }
        let list = self.labels_list_model();
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::LABELS),
            },
            Wants::Labels,
        )
    }

    /// The keys of the labeled places found gone, once known.
    fn missing_keys(&self) -> Option<&HashSet<String>> {
        self.organize.check.as_ref()?.missing.as_ref()
    }

    /// The Labels list's rows, in the labels' order.
    fn labels_list_model(&self) -> ListModel {
        let store = &self.organize.labels;
        let missing = self.missing_keys();
        let rows = store
            .labels()
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let places = store.places_with(&label.name);
                let mut details = vec![
                    "label".to_owned(),
                    count(places.len() as u64, "item", "items"),
                ];
                if !places.is_empty() {
                    details.push(match missing {
                        Some(gone) => {
                            let n = places
                                .iter()
                                .filter(|p| gone.contains(&place_key(p)))
                                .count();
                            format!("{n} missing")
                        }
                        None => "checking".to_owned(),
                    });
                }
                details.push(label.color.word().to_owned());
                if let Some(command) = FAVORITE_COMMANDS.get(index) {
                    let key = self.key(command);
                    if !key.is_empty() {
                        details.push(key);
                    }
                }
                ListRow::new(label.name.clone(), label.name.clone()).with_details(details)
            })
            .collect();
        let mut list = ListModel::new(format!(
            "Labels, {}",
            count(store.labels().len() as u64, "label", "labels")
        ))
        .with_rows(rows);
        list.set_empty_text("No labels. Set label makes one.");
        list.set_help("Enter lists the label's items, Escape closes.");
        list
    }

    /// The runtime looked for the labeled places: the Labels list and an
    /// open label's items say how many are gone, on the status line only.
    pub(super) fn places_checked(
        &mut self,
        request: diroxide_core::RequestId,
        missing: Vec<Place>,
        unreadable: usize,
    ) -> Vec<Effect> {
        let Some(check) = self.organize.check.as_mut() else {
            return Vec::new();
        };
        if check.request != request {
            return Vec::new();
        }
        let gone: HashSet<String> = missing.iter().map(place_key).collect();
        let gone_count = gone.len();
        check.missing = Some(gone);
        check.unreadable = unreadable;
        let list = self.labels_list_model();
        let shown = self.replace_open_list(|w| matches!(w, Wants::Labels), list);
        if let Some(label) = self.organize_top_labeled() {
            let list = self.labeled_model(&label);
            self.replace_open_list(|w| matches!(w, Wants::Labeled { .. }), list);
        }
        if !shown {
            return Vec::new();
        }
        let checked = self.organize.labels.items().count() as u64;
        let mut text = format!(
            "Checked {}: {gone_count} missing",
            count(checked, "labeled item", "labeled items")
        );
        if unreadable > 0 {
            text.push_str(&format!(", {unreadable} could not be looked at"));
        }
        vec![
            Effect::Redraw(diroxide_core::Redraw::All),
            self.say(Message::status(text)),
        ]
    }

    /// The label whose items list is open, if one is.
    fn organize_top_labeled(&self) -> Option<String> {
        self.opens
            .iter()
            .rev()
            .find_map(|open| match &open.purpose {
                super::Purpose::Organize(wants) => match wants.as_ref() {
                    Wants::Labeled { label } => Some(label.clone()),
                    _ => None,
                },
                _ => None,
            })
    }

    /// A label's items: each place by name, "missing" when it is gone,
    /// where it is, and its whole place last.
    fn labeled_model(&self, label: &str) -> ListModel {
        let store = &self.organize.labels;
        let missing = self.missing_keys();
        let places = store.places_with(label);
        let rows = places
            .iter()
            .map(|place| {
                let mut details = Vec::new();
                if missing.is_some_and(|gone| gone.contains(&place_key(place))) {
                    details.push("missing".to_owned());
                }
                if let Some(parent) = place.parent() {
                    details.push(format!("in {}", place_leaf(&parent)));
                }
                details.push(diroxide_core::names::display_name(&place.to_string()).into_owned());
                ListRow::new(place.to_string(), place_leaf(place)).with_details(details)
            })
            .collect();
        let name = store
            .label(label)
            .map_or_else(|| label.to_owned(), |l| l.name.clone());
        let mut list = ListModel::new(format!(
            "{name}, {}",
            count(places.len() as u64, "item", "items")
        ))
        .with_rows(rows);
        list.set_empty_text("No items have this label");
        let remove = self.key(&ids::TRASH);
        let help = if remove.is_empty() {
            "Enter shows the item in its folder, Escape goes back.".to_owned()
        } else {
            format!(
                "Enter shows the item in its folder, {remove} takes the label off, Escape goes back."
            )
        };
        list.set_help(help);
        list
    }

    /// Enter on the Labels list: the label's items, as a list.
    pub(super) fn labeled_items(&mut self, label: &str) -> Vec<Effect> {
        let list = self.labeled_model(label);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::LABELS),
            },
            Wants::Labeled {
                label: label.to_owned(),
            },
        )
    }

    /// Enter on a labeled item: its folder in the active panel, the item
    /// focused; a missing item says so and stays.
    pub(super) fn show_labeled(&mut self, place: Place) -> Vec<Effect> {
        let gone = self
            .missing_keys()
            .is_some_and(|gone| gone.contains(&place_key(&place)));
        let name = place_leaf(&place);
        if gone {
            let remove = self.key(&ids::TRASH);
            let text = if remove.is_empty() {
                format!("Missing: {name}. It was moved or removed.")
            } else {
                format!("Missing: {name}. It was moved or removed; {remove} takes its label off.")
            };
            return vec![self.say(Message::info(text))];
        }
        let side = self.active;
        let mut effects = self.close_all();
        match (place.parent(), place.name()) {
            (Some(parent), Some(name)) => {
                let name = name.to_owned();
                effects.extend(self.open(side, parent, Some(name)));
            }
            _ => effects.extend(self.open(side, place, None)),
        }
        effects
    }

    /// The key that trashes, in a label's items: takes the label off the
    /// focused item (one undo step), and the list loses its row.
    pub(super) fn unlabel_one(&mut self, label: &str, place: Place) -> Vec<Effect> {
        let one = place
            .name()
            .map(|n| diroxide_core::names::display_name(n).into_owned());
        let side = self.active;
        let mut effects = self.change_labels(
            side,
            LabelChange::Remove(label.to_owned()),
            vec![place],
            one,
        );
        let list = self.labeled_model(label);
        self.replace_open_list(|w| matches!(w, Wants::Labeled { .. }), list);
        let labels = self.labels_list_model();
        self.replace_open_list(|w| matches!(w, Wants::Labels), labels);
        effects.push(Effect::Redraw(diroxide_core::Redraw::All));
        effects
    }

    /// The label words of a sort by label when no entry has one.
    pub(super) fn no_labels_here_note(&self) -> String {
        let set = self.key(&ids::LABEL_SET);
        if set.is_empty() {
            "No entry here has a label".to_owned()
        } else {
            format!("No entry here has a label; {set} sets one")
        }
    }
}
