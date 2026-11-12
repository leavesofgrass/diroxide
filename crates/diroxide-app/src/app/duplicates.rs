//! Duplicates in the app (alpha 5, lane DA): the scope, the scan, the
//! groups lister, the keep rules and the trash through the engine, over
//! the engine half in `diroxide_ops::dups`.
//!
//! Reserved by the pre-wiring step (lane P5): the command ids, their
//! catalog rows and menu items, this dispatch, the dialogs' [`DupsWants`]
//! hosted under `Wants::Dups`, [`crate::dups_ui::DupsNote`] (an
//! [`AppEvent`](crate::AppEvent) variant) and [`crate::dups_ui::DupsAsk`]
//! (a [`Request`](crate::Request) variant). Every function below says
//! "Not built yet: NAME." until lane DA fills it; DA works in this file,
//! `crate::dups_ui` and `runtime/duplicates.rs`. A group's members open in
//! the general results lister (`App::open_list` with
//! `ListKind::DuplicateGroup`).

use diroxide_core::{Command, CommandId};

use super::{App, Effect};
use crate::commands::ids;
use crate::dups_ui::DupsNote;

/// What a dialog of duplicates is for, hosted under `Wants::Dups` (the
/// scope list, the options, the ask-first estimate, the groups, the keep
/// rules). Lane DA adds the variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DupsWants {}

impl DupsWants {
    /// The purpose's name for the window's test of every purpose.
    pub(super) fn purpose(&self) -> &'static str {
        match *self {}
    }
}

/// A duplicates command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Find,
    Keep,
}

const ROUTES: [(CommandId, Do); 2] = [(ids::FIND_DUPLICATES, Do::Find), (ids::DUPS_KEEP, Do::Keep)];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// The duplicates commands; `None` for any other.
    pub(super) fn dups_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::Find => self.find_duplicates(command),
            Do::Keep => self.keep_copies(command),
        })
    }

    /// Find duplicates: the scope, the options, the scan as a job.
    fn find_duplicates(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Choose copies to keep: the keep rules over the groups.
    fn keep_copies(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Enter or a typed answer in one of these dialogs.
    pub(super) fn dups_accept(
        &mut self,
        wants: DupsWants,
        _row: Option<String>,
        _text: &str,
    ) -> Vec<Effect> {
        match wants {}
    }

    /// Yes to one of these questions (the ask-first estimate).
    pub(super) fn dups_yes(&mut self, wants: DupsWants) -> Vec<Effect> {
        match wants {}
    }

    /// Escape on one of these dialogs, after it closed.
    pub(super) fn dups_escape(&mut self, wants: DupsWants) -> Vec<Effect> {
        match wants {}
    }

    /// What the runtime answered a [`crate::dups_ui::DupsAsk`].
    pub(super) fn dups_note(&mut self, note: DupsNote) -> Vec<Effect> {
        match note {}
    }
}
