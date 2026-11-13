//! Batch rename in the app (alpha 5, lane RA): the rules list, the
//! preview list, apply, the editor door, presets and passes, over the
//! engine half in `diroxide_ops::rename_rules`.
//!
//! Reserved by the pre-wiring step (lane P5): the command ids, their
//! catalog rows and menu items, this dispatch, the dialogs' [`RenameWants`]
//! hosted under `Wants::Rename`, [`crate::rename_batch::RenameNote`] (an
//! [`AppEvent`](crate::AppEvent) variant) and
//! [`crate::rename_batch::RenameAsk`] (a [`Request`](crate::Request)
//! variant). Every function below says "Not built yet: NAME." until lane
//! RA fills it; RA works in this file, `crate::rename_batch` and
//! `runtime/renaming.rs`, and touches no shared file for these.

use diroxide_core::{Command, CommandId};

use super::{App, Effect};
use crate::commands::ids;
use crate::rename_batch::RenameNote;

/// What a dialog of batch rename is for, hosted under `Wants::Rename`
/// (the rules list, a rule's prompts, the preview list, the question
/// before the run, the presets). Lane RA adds the variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RenameWants {}

impl RenameWants {
    /// The purpose's name for the window's test of every purpose: one per
    /// variant, as `organize::Wants::purpose` gives the others.
    pub(super) fn purpose(&self) -> &'static str {
        match *self {}
    }
}

/// A batch rename command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Batch,
    OneByOne,
    InEditor,
    MakeUnique,
    SkipProblems,
    NextProblem,
    PreviousProblem,
    SayBoth,
    NextPass,
    Presets,
}

const ROUTES: [(CommandId, Do); 10] = [
    (ids::BATCH_RENAME, Do::Batch),
    (ids::RENAME_ONE_BY_ONE, Do::OneByOne),
    (ids::RENAME_IN_EDITOR, Do::InEditor),
    (ids::RENAME_MAKE_UNIQUE, Do::MakeUnique),
    (ids::RENAME_SKIP_PROBLEMS, Do::SkipProblems),
    (ids::RENAME_NEXT_PROBLEM, Do::NextProblem),
    (ids::RENAME_PREVIOUS_PROBLEM, Do::PreviousProblem),
    (ids::RENAME_SAY_BOTH, Do::SayBoth),
    (ids::RENAME_NEXT_PASS, Do::NextPass),
    (ids::RENAME_PRESETS, Do::Presets),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// The batch rename commands; `None` for any other.
    pub(super) fn rename_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::Batch => self.batch_rename(command),
            Do::OneByOne => self.rename_one_by_one(command),
            Do::InEditor
            | Do::MakeUnique
            | Do::SkipProblems
            | Do::NextProblem
            | Do::PreviousProblem
            | Do::SayBoth
            | Do::NextPass
            | Do::Presets => self.rename_list_action(command),
        })
    }

    /// Batch rename: the rules list over the marked entries.
    fn batch_rename(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Rename one by one: today's stepping through the marked entries
    /// (`app/files.rs`), as its own command (question 132).
    fn rename_one_by_one(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// A command of the rules or preview list: Edit names in editor, Make
    /// names unique, Skip problems, Next and Previous problem, Say both
    /// names, Next pass, Rename presets.
    fn rename_list_action(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Enter or a typed answer in one of the batch rename's dialogs.
    pub(super) fn rename_accept(
        &mut self,
        wants: RenameWants,
        _row: Option<String>,
        _text: &str,
    ) -> Vec<Effect> {
        match wants {}
    }

    /// Yes to one of the batch rename's questions.
    pub(super) fn rename_yes(&mut self, wants: RenameWants) -> Vec<Effect> {
        match wants {}
    }

    /// Escape on one of the batch rename's dialogs, after it closed.
    pub(super) fn rename_escape(&mut self, wants: RenameWants) -> Vec<Effect> {
        match wants {}
    }

    /// What the runtime answered a [`crate::rename_batch::RenameAsk`].
    pub(super) fn rename_note(&mut self, note: RenameNote) -> Vec<Effect> {
        match note {}
    }
}
