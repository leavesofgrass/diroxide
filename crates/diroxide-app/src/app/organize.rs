//! The labels in the app (W3l,
//! plan commit 22): what the app holds for them, their commands'
//! routes, what their dialogs are for, and the hooks the app's dispatch,
//! keys, dialogs and undo call. The behavior is in `labeling`.
//!
//! The rules are the app's: every list and question is a dialog under the
//! contract (modal, named, Escape closes, the focus returns); every row
//! and sentence says its meaning in its first 40 cells; one announcement
//! per change; nothing is written but through the runtime, atomically, in
//! the state folder.

use diroxide_core::{Command, CommandId, Effect, ListModel, Message, Place, Side};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
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
    Favorite(usize),
}

const ROUTES: [(CommandId, Do); 10] = [
    (ids::LABEL_SET, Do::SetLabel),
    (ids::LABEL_TOGGLE, Do::ToggleLabel),
    (ids::LABEL_CLEAR, Do::ClearLabels),
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
            Do::Favorite(number) => self.favorite_label(side, number),
        })
    }

    /// Opens a dialog of these lists.
    pub(super) fn open_organize(&mut self, dialog: Dialog, wants: Wants) -> Vec<Effect> {
        self.open_dialog(dialog, Purpose::Organize(Box::new(wants)))
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
        }
    }

    /// The candidates of one of these fields for the text typed so far;
    /// `None` when the field has no list of its own.
    pub(super) fn organize_candidates(&self, wants: &Wants, text: &str) -> Option<ListModel> {
        match wants {
            Wants::Label { places, .. } => Some(self.label_choices(places, text)),
        }
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
