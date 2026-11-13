//! Compare folders and synchronize in the app (alpha 5, lane SA): the
//! compare and its result, the plan view, the run, over the engine half
//! in `diroxide_ops::sync`.
//!
//! Reserved by the pre-wiring step (lane P5): the command ids, their
//! catalog rows and menu items, this dispatch, the dialogs' [`SyncWants`]
//! hosted under `Wants::Sync`, [`crate::sync_ui::SyncNote`] (an
//! [`AppEvent`](crate::AppEvent) variant) and [`crate::sync_ui::SyncAsk`]
//! (a [`Request`](crate::Request) variant). Every function below says
//! "Not built yet: NAME." until lane SA fills it; SA works in this file,
//! `crate::sync_ui` and `runtime/syncing.rs`. A compare's files open in
//! the general results lister (`App::open_list` with
//! `ListKind::Compared`).

use diroxide_core::{Command, CommandId};

use super::{App, Effect};
use crate::commands::ids;
use crate::sync_ui::SyncNote;

/// What a dialog of compare and synchronize is for, hosted under
/// `Wants::Sync` (the criteria list, the result, the mode list, the plan
/// view, the question before the run). Lane SA adds the variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SyncWants {}

impl SyncWants {
    /// The purpose's name for the window's test of every purpose.
    pub(super) fn purpose(&self) -> &'static str {
        match *self {}
    }
}

/// A compare or synchronize command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    CompareFolders,
    Synchronize,
    TowardHere,
    TowardThere,
    CompareAgain,
}

const ROUTES: [(CommandId, Do); 5] = [
    (ids::COMPARE_FOLDERS, Do::CompareFolders),
    (ids::SYNCHRONIZE, Do::Synchronize),
    (ids::SYNC_TOWARD_HERE, Do::TowardHere),
    (ids::SYNC_TOWARD_THERE, Do::TowardThere),
    (ids::COMPARE_AGAIN, Do::CompareAgain),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// The compare and synchronize commands; `None` for any other.
    pub(super) fn sync_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::CompareFolders => self.compare_folders(command),
            Do::Synchronize => self.synchronize(command),
            Do::TowardHere | Do::TowardThere | Do::CompareAgain => self.sync_list_action(command),
        })
    }

    /// Compare folders: the two panels' places, the criteria, the job.
    fn compare_folders(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Synchronize: the plan view from a compare's result.
    fn synchronize(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// A command of the result or the plan view: a row's direction,
    /// Compare again.
    fn sync_list_action(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Enter or a typed answer in one of these dialogs.
    pub(super) fn sync_accept(
        &mut self,
        wants: SyncWants,
        _row: Option<String>,
        _text: &str,
    ) -> Vec<Effect> {
        match wants {}
    }

    /// Yes to one of these questions ("Synchronize 2 items? y or n").
    pub(super) fn sync_yes(&mut self, wants: SyncWants) -> Vec<Effect> {
        match wants {}
    }

    /// Escape on one of these dialogs, after it closed.
    pub(super) fn sync_escape(&mut self, wants: SyncWants) -> Vec<Effect> {
        match wants {}
    }

    /// What the runtime answered a [`crate::sync_ui::SyncAsk`].
    pub(super) fn sync_note(&mut self, note: SyncNote) -> Vec<Effect> {
        match note {}
    }
}
