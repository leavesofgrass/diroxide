//! Lists that are not folders in the app (alpha 5, lane LA): this file
//! holds the lane's dispatch and collections; the flat view is in
//! `flat.rs`, recent files in `recent.rs`. Every list opens in the
//! general results lister (`App::open_list` with a `ListKind`).
//!
//! Reserved by the pre-wiring step (lane P5): the command ids, their
//! catalog rows and menu items, this dispatch, the dialogs' [`ListsWants`]
//! hosted under `Wants::Lists`, [`crate::lists::ListsNote`] (an
//! [`AppEvent`](crate::AppEvent) variant), [`crate::lists::ListsAsk`] (a
//! [`Request`](crate::Request) variant), and the trash key in a
//! collection's list ([`App::collection_remove_items`]). Every function
//! says "Not built yet: NAME." until lane LA fills it; LA works in this
//! file, `flat.rs`, `recent.rs`, `crate::lists` and `runtime/lists.rs`.

use diroxide_core::{Command, CommandId, Message, Place};

use super::{App, Effect};
use crate::commands::ids;
use crate::lists::ListsNote;

/// What a dialog of these lists is for, hosted under `Wants::Lists` (the
/// Collections list, a collection's name prompt, the removal question,
/// the flat view's settings, the keep-folders question). Lane LA adds the
/// variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ListsWants {}

impl ListsWants {
    /// The purpose's name for the window's test of every purpose.
    pub(super) fn purpose(&self) -> &'static str {
        match *self {}
    }
}

/// A command of these lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Flat,
    Collections,
    Recent,
    SaveAs,
    Collect,
    RemoveMissing,
    ClearRecent,
}

const ROUTES: [(CommandId, Do); 7] = [
    (ids::FLAT_VIEW, Do::Flat),
    (ids::COLLECTIONS, Do::Collections),
    (ids::RECENT_FILES, Do::Recent),
    (ids::SAVE_AS_COLLECTION, Do::SaveAs),
    (ids::COLLECT, Do::Collect),
    (ids::COLLECTION_REMOVE_MISSING, Do::RemoveMissing),
    (ids::RECENT_CLEAR, Do::ClearRecent),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// The commands of the flat view, collections and recent files;
    /// `None` for any other.
    pub(super) fn lists_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::Flat => self.flat_view_toggle(command),
            Do::Collections => self.collections_list(command),
            Do::Recent => self.recent_files_list(command),
            Do::SaveAs => self.save_as_collection(command),
            Do::Collect => self.collect(command),
            Do::RemoveMissing => self.collection_remove_missing(command),
            Do::ClearRecent => self.clear_recent_files(command),
        })
    }

    /// The Collections list: collections and saved searches.
    fn collections_list(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Save as collection: from the list on the screen or a panel's
    /// marks.
    fn save_as_collection(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Collect: into the default collection.
    fn collect(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Remove missing items, in a collection's list.
    fn collection_remove_missing(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// The trash key in a collection's list: these items leave the
    /// collection and no file is touched ("Removed 1 from Trip photos.",
    /// then "The file was not touched."). The general results lister
    /// calls it for `ListKind::Collection`.
    pub(super) fn collection_remove_items(
        &mut self,
        _collection: &str,
        _places: Vec<Place>,
    ) -> Vec<Effect> {
        // "Not built yet: Remove from collection." (39 cells).
        vec![self.say(Message::info(format!(
            "{}Remove from collection.",
            super::reserved::NOT_BUILT
        )))]
    }

    /// Enter or a typed answer in one of these dialogs.
    pub(super) fn lists_accept(
        &mut self,
        wants: ListsWants,
        _row: Option<String>,
        _text: &str,
    ) -> Vec<Effect> {
        match wants {}
    }

    /// Yes to one of these questions.
    pub(super) fn lists_yes(&mut self, wants: ListsWants) -> Vec<Effect> {
        match wants {}
    }

    /// Escape on one of these dialogs, after it closed.
    pub(super) fn lists_escape(&mut self, wants: ListsWants) -> Vec<Effect> {
        match wants {}
    }

    /// What the runtime answered a [`crate::lists::ListsAsk`].
    pub(super) fn lists_note(&mut self, note: ListsNote) -> Vec<Effect> {
        match note {}
    }
}
