//! The labels in the app (W3l,
//! plan commits 22 and 23): what the app holds for them, their commands'
//! routes, what their dialogs are for, and the hooks the app's dispatch,
//! keys, dialogs and undo call. The behavior is in `labeling`.
//!
//! The rules are the app's: every list and question is a dialog under the
//! contract (modal, named, Escape closes, the focus returns); every row
//! and sentence says its meaning in its first 40 cells; one announcement
//! per change; nothing is written but through the runtime, atomically, in
//! the state folder.

use std::collections::HashSet;

use diroxide_core::{
    Command, CommandId, Effect, Key, Layer, ListModel, Message, Place, RequestId, Side,
};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::keymap::Lookup;
use crate::labels::{LabelStore, LabelUndo};

/// What the app holds for the labels.
#[derive(Clone, Debug, Default)]
pub(super) struct Organize {
    /// The labels and the labeled places.
    pub(super) labels: LabelStore,
    /// The label changes to undo, newest last, each with the newest job
    /// in the undo log when it was made, so Undo takes whichever came
    /// last.
    pub(super) label_undo: Vec<(u64, LabelUndo)>,
    /// The Labels list's check for missing items.
    pub(super) check: Option<Check>,
}

impl Organize {
    /// What the app starts with.
    pub(super) fn new(labels: LabelStore) -> Self {
        Self {
            labels,
            ..Self::default()
        }
    }
}

/// The Labels list's check for labeled items that are gone.
#[derive(Clone, Debug)]
pub(super) struct Check {
    /// The request the runtime answers.
    pub(super) request: RequestId,
    /// The keys of the places found gone, once the answer came.
    pub(super) missing: Option<HashSet<String>>,
    /// How many places could not be looked at.
    pub(super) unreadable: usize,
}

/// What a dialog of these lists is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Wants {
    /// Set or toggle a label on these places: a field with the labels
    /// listed under it.
    Label {
        side: Side,
        places: Vec<Place>,
        /// The item's name when there is one item.
        one: Option<String>,
        toggle: bool,
    },
    /// The Labels list.
    Labels,
    /// One label's items.
    Labeled { label: String },
}

impl Wants {
    /// Whether the dialog's list stays open while typing narrows it, as
    /// the palette's does: Escape then closes the dialog at once.
    pub(super) fn list_always(&self) -> bool {
        matches!(self, Wants::Label { .. })
    }
}

/// What a command of these lists does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    SetLabel,
    ToggleLabel,
    ClearLabels,
    Labels,
    Favorite(usize),
}

const ROUTES: [(CommandId, Do); 11] = [
    (ids::LABEL_SET, Do::SetLabel),
    (ids::LABEL_TOGGLE, Do::ToggleLabel),
    (ids::LABEL_CLEAR, Do::ClearLabels),
    (ids::LABELS, Do::Labels),
    (ids::LABEL_FAVORITE_1, Do::Favorite(1)),
    (ids::LABEL_FAVORITE_2, Do::Favorite(2)),
    (ids::LABEL_FAVORITE_3, Do::Favorite(3)),
    (ids::LABEL_FAVORITE_4, Do::Favorite(4)),
    (ids::LABEL_FAVORITE_5, Do::Favorite(5)),
    (ids::LABEL_FAVORITE_6, Do::Favorite(6)),
    (ids::LABEL_FAVORITE_7, Do::Favorite(7)),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// Runs a command of the labels;
    /// `None` when the command is another's. With a dialog or the viewer
    /// open it says what is in the way, as the panel's commands do.
    pub(super) fn organize_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        if self.viewing.is_some() {
            let close = self.key(&ids::CLOSE_VIEWER);
            let text = if close.is_empty() {
                "Close the viewer first.".to_owned()
            } else {
                format!("Close the viewer first. {close} closes it.")
            };
            return Some(vec![self.say(Message::status(text))]);
        }
        let side = self.active;
        Some(match action {
            Do::SetLabel => self.label_prompt(side, false, command),
            Do::ToggleLabel => self.label_prompt(side, true, command),
            Do::ClearLabels => self.clear_labels(side),
            Do::Labels => self.labels_list(),
            Do::Favorite(number) => self.favorite_label(side, number),
        })
    }

    /// The purpose of the dialog on top, when it is one of these lists'.
    pub(super) fn organize_top(&self) -> Option<Wants> {
        match self.opens.last().map(|o| &o.purpose) {
            Some(Purpose::Organize(wants)) => Some((**wants).clone()),
            _ => None,
        }
    }

    /// Opens a dialog of these lists.
    pub(super) fn open_organize(&mut self, dialog: Dialog, wants: Wants) -> Vec<Effect> {
        self.open_dialog(dialog, Purpose::Organize(Box::new(wants)))
    }

    /// A key in one of these lists before the dialog's own keys: the keys
    /// that trash a file take the label off the focused item of a label's
    /// items. `None` when the key is the dialog's.
    pub(super) fn organize_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let wants = self.organize_top()?;
        let in_choose = matches!(self.dialogs.last(), Some((_, Dialog::Choose { .. })));
        if !in_choose {
            return None;
        }
        let file_command = match self.keymap.lookup(&[Layer::Panel], key) {
            Lookup::Command(id) => Some(id),
            Lookup::NoMatch => None,
        };
        let removes = file_command
            .as_ref()
            .is_some_and(|id| *id == ids::TRASH || *id == ids::DELETE_PERMANENTLY);
        let row = self.top_row_id();
        match wants {
            Wants::Labeled { label } if removes => {
                let place = row.and_then(|r| Place::parse(&r).ok())?;
                Some(self.unlabel_one(&label, place))
            }
            _ => None,
        }
    }

    /// The focused row's id of the dialog on top.
    pub(super) fn top_row_id(&self) -> Option<String> {
        match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => list.focused().map(|r| r.id.clone()),
            _ => None,
        }
    }

    /// Enter in one of these dialogs.
    pub(super) fn organize_accept(
        &mut self,
        wants: Wants,
        row: Option<String>,
        text: String,
        in_list: bool,
    ) -> Vec<Effect> {
        match wants {
            Wants::Label {
                side,
                places,
                one,
                toggle,
            } => self.label_chosen(side, places, one, toggle, row, &text, in_list),
            Wants::Labels => match row {
                Some(label) => self.labeled_items(&label),
                None => Vec::new(),
            },
            Wants::Labeled { .. } => match row.and_then(|r| Place::parse(&r).ok()) {
                Some(place) => self.show_labeled(place),
                None => Vec::new(),
            },
        }
    }

    /// The candidates of one of these fields for the text typed so far;
    /// `None` when the field has no list of its own.
    pub(super) fn organize_candidates(&self, wants: &Wants, text: &str) -> Option<ListModel> {
        match wants {
            Wants::Label { places, .. } => Some(self.label_choices(places, text)),
            _ => None,
        }
    }

    /// Replaces the list of the open dialog of these lists that `pick`
    /// chooses, where it is in the stack, keeping its focus by row id.
    pub(super) fn replace_open_list(
        &mut self,
        pick: impl Fn(&Wants) -> bool,
        list: ListModel,
    ) -> bool {
        let index = self.opens.iter().rposition(|open| match &open.purpose {
            Purpose::Organize(wants) => pick(wants),
            _ => false,
        });
        let Some(index) = index else {
            return false;
        };
        if let Some((_, Dialog::Choose { list: open, .. })) = self.dialogs.get_mut(index) {
            let focus = open.focused().map(|r| r.id.clone());
            let position = open.focus_index();
            *open = list;
            let kept = focus.is_some_and(|id| open.focus_id(&id));
            if !kept && let Some(position) = position {
                let last = open.len().saturating_sub(1);
                open.move_focus(diroxide_core::Move::To(position.min(last)));
            }
            return true;
        }
        false
    }

    /// Saves the labels through the runtime.
    pub(super) fn save_labels(&mut self) {
        let store = self.organize.labels.clone();
        self.request(crate::request::Request::SaveLabels(Box::new(store)));
    }

    /// The labels' names, for the filter language's lints and completion
    /// (`LintContext` and `CompletionContext`): the wiring lane passes
    /// these.
    pub fn label_names(&self) -> Vec<String> {
        self.organize.labels.names()
    }

    /// The labels and the labeled places.
    pub fn labels(&self) -> &LabelStore {
        &self.organize.labels
    }
}
