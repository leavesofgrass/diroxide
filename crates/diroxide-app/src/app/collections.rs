//! Lists that are not folders in the app (alpha 5, lane LA): this file
//! holds the lane's dispatch, its dialogs, collections, the notes from
//! the runtime, and the entry point for `diro -`; the flat view is in
//! `flat.rs`, recent files in `recent.rs`. Every list opens in the
//! general results lister (`App::open_list` with a `ListKind`), one at a
//! time (question 130).
//!
//! - **The Collections list** (Go menu): collections and saved searches
//!   by name, "Trip photos, collection, 3 items" and "Large images, saved
//!   search, runs on open"; Enter opens a collection in the lister or
//!   runs the saved search. Two rows at its end rename a collection or
//!   remove one; the removal is a question that starts on No, and the
//!   collection's file goes to the trash through the ordinary plan.
//! - **In a collection's list** the trash key takes the item out of the
//!   collection and touches no file: "Removed 1 from Trip photos. The
//!   file was not touched." Remove missing items takes out the gone ones.
//! - **Save as collection** asks for a name and keeps the list on the
//!   screen, or the panel's marked entries, in it; **Collect** adds the
//!   marked or focused entries to the collection "Collected".
//!
//! Every read and write goes to the runtime as a
//! [`ListsAsk`](crate::lists::ListsAsk); its answer comes back as a
//! [`ListsNote`].

use std::path::PathBuf;
use std::sync::Arc;

use diroxide_core::format::{count, grouped};
use diroxide_core::names::{display_name, spell_out};
use diroxide_core::{Command, CommandId, ListModel, ListRow, Message, Place, Side};
use diroxide_search::{FlatOptions, SearchHit};

use super::searching::{ListKind, ListOpen};
use super::{App, Effect, Purpose};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::lists::{
    CollectionRow, DEFAULT_COLLECTION, ListPurpose, ListsAsk, ListsNote, place_of_row,
};
use crate::request::{Request, SharedClock};

/// The Collections list's row that renames a collection. Its id starts
/// with a space, which no collection's name does (names are trimmed).
const RENAME_ROW: &str = " rename";

/// The Collections list's row that removes a collection.
const REMOVE_ROW: &str = " remove";

/// Said when there is nothing to keep: 32 cells.
pub(super) const NOTHING_TO_SAVE: &str = "Nothing to save: mark entries first.";

/// What a dialog of these lists is for, hosted under `Wants::Lists`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ListsWants {
    /// The Collections list, open for one purpose.
    Collections {
        /// The panel that asked.
        side: Side,
        /// Open, rename or remove.
        purpose: ListPurpose,
    },
    /// Save as collection's name field.
    SaveName {
        /// What it keeps.
        places: Vec<Place>,
    },
    /// A collection's new name.
    RenameName {
        /// Its name now.
        old: String,
    },
    /// The question before a collection's file goes to the trash.
    RemoveCollection {
        /// The panel that asked.
        side: Side,
        /// Its name.
        name: String,
        /// Its file.
        path: PathBuf,
    },
    /// The flat view's settings, before it starts.
    FlatForm {
        /// The panel whose folder it lists.
        side: Side,
        /// That folder.
        root: Place,
        /// Depth, files only, hidden.
        options: FlatOptions,
    },
    /// The flat view's depth field.
    FlatDepth {
        /// The panel whose folder it lists.
        side: Side,
        /// That folder.
        root: Place,
        /// The other settings, kept.
        options: FlatOptions,
    },
}

impl ListsWants {
    /// The purpose's name for the window's test of every purpose.
    pub(super) fn purpose(&self) -> &'static str {
        match self {
            ListsWants::Collections { .. } => "Collections",
            ListsWants::SaveName { .. } => "CollectionName",
            ListsWants::RenameName { .. } => "CollectionRename",
            ListsWants::RemoveCollection { .. } => "CollectionRemove",
            ListsWants::FlatForm { .. } => "FlatView",
            ListsWants::FlatDepth { .. } => "FlatViewDepth",
        }
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

/// "no items", "1 item", "1,204 items".
fn items_words(n: u64) -> String {
    match n {
        0 => "no items".to_owned(),
        1 => "1 item".to_owned(),
        n => format!("{} items", grouped(n)),
    }
}

/// A collection's name as said.
fn said_name(name: &str) -> String {
    display_name(name).into_owned()
}

impl App {
    /// The commands of the flat view, collections and recent files;
    /// `None` for any other. Files opened with Enter and F4 are kept in
    /// the recent files by the hook after every event (`keep_recent`).
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

    /// Sends an ask to the runtime.
    pub(super) fn lists_ask(&mut self, ask: ListsAsk) {
        self.request(Request::Lists(Box::new(ask)));
    }

    /// Whether the results lister is the dialog on top.
    fn lister_on_top(&self) -> bool {
        matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Results)
        )
    }

    /// The places of the list on the screen: its marked rows, or all its
    /// rows; empty when the results lister is not on top.
    fn lister_places(&self, focused_only: bool) -> Vec<Place> {
        if !focused_only {
            return self.list_places();
        }
        if !self.lister_on_top() {
            return Vec::new();
        }
        let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last() else {
            return Vec::new();
        };
        list.focused()
            .and_then(|r| place_of_row(&r.id))
            .into_iter()
            .collect()
    }

    /// The Collections list: collections and saved searches, read from
    /// the state folder.
    fn collections_list(&mut self, _command: &Command) -> Vec<Effect> {
        let side = self.active;
        self.lists_ask(ListsAsk::Collections {
            side,
            purpose: ListPurpose::Open,
        });
        Vec::new()
    }

    /// The collections were read: the list opens for its purpose, or
    /// says there are none.
    fn collections_read(
        &mut self,
        side: Side,
        purpose: ListPurpose,
        rows: Vec<CollectionRow>,
        problems: Vec<String>,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        if rows.is_empty() {
            let line = match purpose {
                ListPurpose::Open => "No collections yet. Save as collection makes one.",
                ListPurpose::Rename => "No collection to rename.",
                ListPurpose::Remove => "No collection to remove.",
            };
            effects.push(self.say(Message::status(line)));
        } else {
            let collections = rows.iter().filter(|row| !row.live).count();
            let (title, help) = match purpose {
                ListPurpose::Open => (
                    format!("Collections, {}", count(rows.len() as u64, "list", "lists")),
                    "Enter opens the collection or runs the saved search, Escape closes.",
                ),
                ListPurpose::Rename => (
                    "Rename which collection".to_owned(),
                    "Enter asks for the new name, Escape closes.",
                ),
                ListPurpose::Remove => (
                    "Remove which collection".to_owned(),
                    "Enter asks first; its file goes to the trash. Escape closes.",
                ),
            };
            let mut list_rows: Vec<ListRow> = rows
                .into_iter()
                .map(|row| ListRow::new(row.name, row.words))
                .collect();
            if purpose == ListPurpose::Open && collections > 0 {
                list_rows.push(ListRow::new(RENAME_ROW, "Rename a collection"));
                list_rows.push(ListRow::new(REMOVE_ROW, "Remove a collection"));
            }
            let mut list = ListModel::new(title).with_rows(list_rows);
            list.set_help(help);
            effects.extend(self.open_organize(
                Dialog::Choose {
                    list,
                    on_enter: Command::new(ids::COLLECTIONS),
                },
                super::organize::Wants::Lists(ListsWants::Collections { side, purpose }),
            ));
        }
        if !problems.is_empty() {
            effects.push(self.say(Message::warning(problems.join(" "))));
        }
        effects
    }

    /// Enter on a row of the Collections list.
    fn collections_chosen(&mut self, side: Side, purpose: ListPurpose, row: &str) -> Vec<Effect> {
        let mut effects = self.close_all();
        match (purpose, row) {
            (ListPurpose::Open, RENAME_ROW) => self.lists_ask(ListsAsk::Collections {
                side,
                purpose: ListPurpose::Rename,
            }),
            (ListPurpose::Open, REMOVE_ROW) => self.lists_ask(ListsAsk::Collections {
                side,
                purpose: ListPurpose::Remove,
            }),
            (ListPurpose::Open, name) => self.lists_ask(ListsAsk::Open {
                side,
                name: name.to_owned(),
            }),
            (ListPurpose::Rename, name) => {
                let label = format!("New name for {}", said_name(name));
                let field = EntryField::new(label, CompletionSource::None).with_text(name);
                effects.extend(self.open_organize(
                    Dialog::edit(field),
                    super::organize::Wants::Lists(ListsWants::RenameName {
                        old: name.to_owned(),
                    }),
                ));
            }
            (ListPurpose::Remove, name) => self.lists_ask(ListsAsk::FileOf {
                name: name.to_owned(),
            }),
        }
        effects
    }

    /// A collection was read: it opens in the results lister, its count
    /// first, the missing items said.
    fn collection_read(
        &mut self,
        side: Side,
        name: String,
        hits: Vec<SearchHit>,
        gone: Vec<Place>,
        unknown: u64,
    ) -> Vec<Effect> {
        let shown = said_name(&name);
        let items = (hits.len() + gone.len()) as u64;
        let mut start = format!("{shown}, collection, {}", items_words(items));
        if !gone.is_empty() {
            start.push_str(&format!(", {} missing", grouped(gone.len() as u64)));
        }
        if unknown > 0 {
            start.push_str(&format!(", {} not checked", grouped(unknown)));
        }
        let open = ListOpen {
            kind: ListKind::Collection(name),
            side,
            title: format!("{shown}, collection"),
            root: None,
            hits,
            gone,
            notes: Vec::new(),
            start,
            streaming: false,
        };
        let (effects, _) = self.open_list(open);
        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut()
            && list.is_empty()
        {
            list.set_empty_text("This collection is empty");
        }
        effects
    }

    /// Save as collection: a name for the list on the screen (its marked
    /// rows, or all of them), or for the panel's marked entries.
    fn save_as_collection(&mut self, _command: &Command) -> Vec<Effect> {
        let mut places = self.lister_places(false);
        if places.is_empty() && !self.lister_on_top() {
            places = self.panel(self.active).targets();
        }
        self.save_places_as_collection(places)
    }

    /// Asks for a collection's name and keeps these places in it: Save as
    /// collection, and the lanes whose lists offer it (duplicates,
    /// compare). Nothing to keep is said.
    pub(crate) fn save_places_as_collection(&mut self, places: Vec<Place>) -> Vec<Effect> {
        if places.is_empty() {
            return vec![self.say(Message::status(NOTHING_TO_SAVE))];
        }
        let label = format!("Collection name for {}", items_words(places.len() as u64));
        let field = EntryField::new(label, CompletionSource::None);
        self.open_organize(
            Dialog::edit(field),
            super::organize::Wants::Lists(ListsWants::SaveName { places }),
        )
    }

    /// Collect: the focused row of the list on the screen, or the panel's
    /// marked or focused entries, into the collection "Collected".
    fn collect(&mut self, _command: &Command) -> Vec<Effect> {
        let mut places = self.lister_places(true);
        if places.is_empty() && !self.lister_on_top() {
            places = self.panel(self.active).targets();
        }
        if places.is_empty() {
            return vec![self.say(Message::status("Nothing to collect here."))];
        }
        let clock = SharedClock(Arc::clone(&self.clock));
        self.lists_ask(ListsAsk::Add {
            name: DEFAULT_COLLECTION.to_owned(),
            places,
            clock,
        });
        Vec::new()
    }

    /// Remove missing items, in a collection's list.
    fn collection_remove_missing(&mut self, _command: &Command) -> Vec<Effect> {
        match self.list_kind() {
            Some(ListKind::Collection(name)) => {
                let name = name.clone();
                self.lists_ask(ListsAsk::RemoveGone { name });
                Vec::new()
            }
            _ => vec![self.say(Message::status(
                "Open a collection first: Collections, in the Go menu.",
            ))],
        }
    }

    /// The trash key in a collection's list: these items leave the
    /// collection and no file is touched ("Removed 1 from Trip photos.
    /// The file was not touched."). The general results lister calls it
    /// for `ListKind::Collection`.
    pub(super) fn collection_remove_items(
        &mut self,
        collection: &str,
        places: Vec<Place>,
    ) -> Vec<Effect> {
        if places.is_empty() {
            return Vec::new();
        }
        self.lists_ask(ListsAsk::Remove {
            name: collection.to_owned(),
            places,
        });
        Vec::new()
    }

    /// Opens a list of places in the results lister: the entry point for
    /// `diro -`, whose paths come from standard input. Each place is
    /// looked up; the gone ones say "missing". `title` names the list
    /// ("Standard input"), `start` is said as it opens, its count first
    /// ("3 paths from standard input"). Call it after [`App::start`] and
    /// hand the effects to the runtime with the start's.
    pub fn open_place_list(&mut self, title: &str, start: &str, places: Vec<Place>) -> Vec<Effect> {
        if places.is_empty() {
            return vec![self.say(Message::status(format!("{start}: nothing to list.")))];
        }
        let side = self.active;
        self.lists_ask(ListsAsk::Places {
            side,
            title: title.to_owned(),
            start: start.to_owned(),
            places,
        });
        Vec::new()
    }

    /// Enter or a typed answer in one of these dialogs.
    pub(super) fn lists_accept(
        &mut self,
        wants: ListsWants,
        row: Option<String>,
        text: &str,
    ) -> Vec<Effect> {
        match wants {
            ListsWants::Collections { side, purpose } => match row {
                Some(row) => self.collections_chosen(side, purpose, &row),
                None => Vec::new(),
            },
            ListsWants::SaveName { places } => {
                let name = text.trim();
                if name.is_empty() {
                    return vec![self.say(Message::status("A collection needs a name."))];
                }
                // Only the name field closes: the list it was saved from
                // stays on the screen.
                let mut effects = self.close_top();
                let clock = SharedClock(Arc::clone(&self.clock));
                self.lists_ask(ListsAsk::Add {
                    name: name.to_owned(),
                    places,
                    clock,
                });
                effects.push(Effect::Redraw(super::Redraw::All));
                effects
            }
            ListsWants::RenameName { old } => {
                let new = text.trim();
                if new.is_empty() {
                    return vec![self.say(Message::status("A collection needs a name."))];
                }
                let effects = self.close_all();
                self.lists_ask(ListsAsk::Rename {
                    old,
                    new: new.to_owned(),
                });
                effects
            }
            ListsWants::RemoveCollection { .. } => self.dialog_escape(),
            ListsWants::FlatForm {
                side,
                root,
                options,
            } => match row {
                Some(row) => self.flat_form_row(side, root, options, &row),
                None => Vec::new(),
            },
            ListsWants::FlatDepth {
                side,
                root,
                options,
            } => self.flat_depth_typed(side, root, options, text),
        }
    }

    /// Yes to one of these questions: a collection's file goes to the
    /// trash through the ordinary plan.
    pub(super) fn lists_yes(&mut self, wants: ListsWants) -> Vec<Effect> {
        match wants {
            ListsWants::RemoveCollection { side, name, path } => {
                let Ok(place) = Place::parse(&path.to_string_lossy()) else {
                    return vec![self.say(Message::error(format!(
                        "Could not remove {}: its file's path does not read.",
                        said_name(&name)
                    )))];
                };
                let job = self.next_job_id();
                let request =
                    diroxide_core::JobRequest::new(job, diroxide_core::JobKind::Trash, vec![place]);
                let mut effects = self.close_all();
                effects.push(self.say(Message::info(format!(
                    "Removing {}. Its items' files stay.",
                    said_name(&name)
                ))));
                effects.extend(self.plan_job(side, request, None, super::Then::Report, false));
                effects
            }
            _ => Vec::new(),
        }
    }

    /// Escape on one of these dialogs, after it closed: the question says
    /// nothing changed.
    pub(super) fn lists_escape(&mut self, wants: ListsWants) -> Vec<Effect> {
        match wants {
            ListsWants::RemoveCollection { name, .. } => vec![self.say(Message::info(format!(
                "Nothing removed; {} stays.",
                said_name(&name)
            )))],
            // The depth field closed: the form again, on its row.
            ListsWants::FlatDepth {
                side,
                root,
                options,
            } => self.flat_form_again(side, root, options),
            _ => Vec::new(),
        }
    }

    /// The question before a collection's file goes to the trash: it
    /// starts on No and lists the file.
    fn ask_remove_collection(&mut self, name: String, path: PathBuf) -> Vec<Effect> {
        let side = self.active;
        let question = with_default_no(Message::question(&format!(
            "Remove collection {}",
            said_name(&name)
        )));
        let listing = vec![
            spell_out(&path.to_string_lossy()).into_owned(),
            "Its file goes to the trash; the items' files stay.".to_owned(),
        ];
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            super::organize::Wants::Lists(ListsWants::RemoveCollection { side, name, path }),
        )
    }

    /// What the runtime answered a [`crate::lists::ListsAsk`].
    pub(super) fn lists_note(&mut self, note: ListsNote) -> Vec<Effect> {
        match note {
            ListsNote::FlatDone { list, result } => self.flat_done(list, result),
            ListsNote::Collections {
                side,
                purpose,
                rows,
                problems,
            } => self.collections_read(side, purpose, rows, problems),
            ListsNote::Collection {
                side,
                name,
                hits,
                gone,
                unknown,
            } => self.collection_read(side, name, hits, gone, unknown),
            ListsNote::Live { saved } => {
                let name = said_name(&saved.name);
                match saved.to_query(&self.filter_options()) {
                    Ok(query) => self.start_query_search(query, Some(name)),
                    Err(error) => {
                        vec![self.say(Message::error(format!("Could not run {name}: {error}")))]
                    }
                }
            }
            ListsNote::Added { words } | ListsNote::Renamed { words } => {
                vec![self.say(Message::done(words))]
            }
            ListsNote::Removed {
                name,
                places,
                words,
            } => {
                let mut effects = Vec::new();
                if self.list_kind() == Some(&ListKind::Collection(name)) {
                    effects.extend(self.list_remove_places(&places));
                }
                effects.push(self.say(Message::done(words)));
                effects
            }
            ListsNote::FileOf { name, path } => self.ask_remove_collection(name, path),
            ListsNote::Recent {
                side,
                recent,
                hits,
                gone,
            } => self.recent_read(side, *recent, hits, gone),
            ListsNote::RecentCleared { cleared } => self.recent_cleared(cleared),
            ListsNote::Places {
                side,
                title,
                start,
                hits,
                gone,
            } => {
                let mut start = start;
                if !gone.is_empty() {
                    start.push_str(&format!(", {} missing", grouped(gone.len() as u64)));
                }
                let open = ListOpen {
                    kind: ListKind::Search,
                    side,
                    title,
                    root: None,
                    hits,
                    gone,
                    notes: Vec::new(),
                    start,
                    streaming: false,
                };
                self.open_list(open).0
            }
            ListsNote::Say(message) => vec![self.say(message)],
        }
    }
}

#[cfg(test)]
mod tests;
