//! What search offers beyond Find files (W3a, plan commit 38): saved
//! searches, the Space list, the largest items below a folder, folder
//! sizes and Size every folder, and newest files with Earlier day.
//!
//! What is said, one line a change: "Search saved as Large videos";
//! "Saved searches, 2 searches, Large videos, saved search, runs on open,
//! 1 of 2"; "Total 66.4 GB. Largest 3 hold 81 percent" before the Space
//! list's rows; "Backup: 2.1 MB in 40 files and 3 folders" or "At least
//! 12.3 GB, 2 folders unreadable"; "Sized 3 folders, 4.2 MB in all. Sort
//! by size? No is the default. y or n"; "Newest files: 12 changed on
//! October 8, 2026". The walks run on the runtime's workers; the largest
//! items and newest files open as search results, so every result's key
//! works on them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use diroxide_core::format::{count, human_size};
use diroxide_core::{
    Command, CommandId, Effect, Importance, Key, Layer, ListModel, ListRow, Message, MessageKind,
    Place, RequestId, Side,
};
use diroxide_search::{Day, Grouping, Largest, Newest, Saved, Scope, SearchQuery, SpaceList};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::Lookup as KeyLookup;
use crate::panel::place_leaf;
use crate::request::{Request, SharedClock};

/// How many items Largest items below here lists.
const LARGEST_LIMIT: usize = 50;

/// What the search tools wait for.
#[derive(Clone, Debug, Default)]
pub(super) struct ToolsState {
    saved: Option<RequestId>,
    space: Option<(RequestId, Side)>,
    sizes: Option<(RequestId, Side, bool)>,
}

/// A saved search in the list, with its file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SavedRow {
    pub path: PathBuf,
    pub saved: Saved,
}

/// A command of the search tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Save,
    Saved,
    Space(Grouping),
    Largest,
    Sizes,
    SizeAll,
    Newest,
    Earlier,
}

const ROUTES: [(CommandId, Do); 10] = [
    (ids::SEARCH_SAVE, Do::Save),
    (ids::SEARCH_SAVED, Do::Saved),
    (ids::SPACE_LIST, Do::Space(Grouping::Items)),
    (ids::SPACE_BY_KIND, Do::Space(Grouping::Kind)),
    (ids::SPACE_BY_AGE, Do::Space(Grouping::Age)),
    (ids::LARGEST_BELOW, Do::Largest),
    (ids::DIR_SIZES, Do::Sizes),
    (ids::SIZE_EVERY_FOLDER, Do::SizeAll),
    (ids::NEWEST_FILES, Do::Newest),
    (ids::EARLIER_DAY, Do::Earlier),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// Runs a search tool's command; `None` when the command is another's.
    pub(super) fn tools_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        // Earlier day and saving work from the results they act on.
        let from_results = self.results_shown() && matches!(action, Do::Save | Do::Earlier);
        if !self.dialogs.is_empty() && !from_results {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        let side = self.active;
        Some(match action {
            Do::Save => self.save_search_prompt(),
            Do::Saved => self.saved_searches(),
            Do::Space(grouping) => {
                let place = self.panel(side).place().clone();
                self.space_list(side, place, grouping)
            }
            Do::Largest => self.largest_below(side),
            Do::Sizes => self.folder_sizes(side, false),
            Do::SizeAll => self.folder_sizes(side, true),
            Do::Newest => self.newest_files(side, None),
            Do::Earlier => self.earlier_day(),
        })
    }

    /// Save this search: the name, the last search's expression offered.
    fn save_search_prompt(&mut self) -> Vec<Effect> {
        let Some((typed, _)) = self.last_search_query() else {
            let line = "No search to save. Find files starts one.";
            return vec![self.say(Message::status(line))];
        };
        let field = EntryField::new("Save the search as", CompletionSource::None).with_text(typed);
        self.open_dialog(Dialog::edit(field), Purpose::SaveSearchName)
    }

    /// The name typed: the search is written to the state folder.
    pub(super) fn save_search_named(&mut self, name: &str) -> Vec<Effect> {
        let name = name.trim();
        if name.is_empty() {
            return vec![self.say(Message::error("Type a name, such as Large videos"))];
        }
        let Some((_, query)) = self.last_search_query() else {
            return self.close_top();
        };
        let saved = Saved::search(name, &query, self.clock.as_ref());
        let mut effects = self.close_top();
        self.request(Request::SaveSearch(Box::new(saved)));
        let open = match self.key(&ids::SEARCH_SAVED) {
            key if key.is_empty() => "Saved searches runs it".to_owned(),
            key => format!("{key} lists the saved searches"),
        };
        let line = format!(
            "Search saved as {}. {open}.",
            diroxide_core::names::display_name(name)
        );
        effects.push(self.say(Message::info(line)));
        effects
    }

    /// Saved searches: the list, read from the state folder.
    fn saved_searches(&mut self) -> Vec<Effect> {
        let request = self.ids.request();
        self.tools.saved = Some(request);
        self.request(Request::ListSaved { request });
        Vec::new()
    }

    /// The saved searches were read: the list opens.
    pub(super) fn saved_listed(
        &mut self,
        request: RequestId,
        found: Vec<(PathBuf, Result<Saved, String>)>,
    ) -> Vec<Effect> {
        if self.tools.saved.take() != Some(request) {
            return Vec::new();
        }
        let mut rows = Vec::new();
        let mut saved = Vec::new();
        let mut problems = 0;
        for (path, result) in found {
            match result {
                Ok(search) => {
                    let id = path.to_string_lossy().into_owned();
                    rows.push(ListRow::new(id, search.row_words()));
                    saved.push(SavedRow {
                        path,
                        saved: search,
                    });
                }
                Err(_) => problems += 1,
            }
        }
        let title = format!(
            "Saved searches, {}",
            count(saved.len() as u64, "search", "searches")
        );
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_empty_text("No saved searches. Save this search keeps one.");
        list.set_help(
            "Up and Down choose, Enter runs the search, the trash key deletes it after asking, Escape closes.",
        );
        let mut effects = self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SEARCH_SAVED),
            },
            Purpose::SavedList(saved),
        );
        if problems > 0 {
            let line = format!(
                "{} could not be read",
                count(problems, "saved file", "saved files")
            );
            effects.push(self.say(Message::warning(line)));
        }
        effects
    }

    /// Enter on a saved search: it runs, its date words read again now.
    pub(super) fn saved_chosen(&mut self, rows: Vec<SavedRow>, row: &str) -> Vec<Effect> {
        let Some(found) = rows.into_iter().find(|r| r.path.to_string_lossy() == row) else {
            return Vec::new();
        };
        match found.saved.to_query(&self.filter_options()) {
            Ok(query) => {
                let mut effects = self.close_top();
                let name = diroxide_core::names::display_name(&found.saved.name).into_owned();
                effects.extend(self.start_query_search(query, Some(name)));
                effects
            }
            Err(error) => {
                let name = diroxide_core::names::display_name(&found.saved.name).into_owned();
                vec![self.say(Message::error(format!("Could not run {name}: {error}")))]
            }
        }
    }

    /// The trash key in the saved searches: the question first.
    pub(super) fn saved_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let Some(Purpose::SavedList(rows)) = self.opens.last().map(|o| o.purpose.clone()) else {
            return None;
        };
        let removes = match self.keymap.lookup(&[Layer::Panel], key) {
            KeyLookup::Command(id) => id == ids::TRASH || id == ids::DELETE_PERMANENTLY,
            KeyLookup::NoMatch => false,
        };
        if !removes {
            return None;
        }
        let row = self.top_row_id()?;
        let found = rows.into_iter().find(|r| r.path.to_string_lossy() == row)?;
        let name = diroxide_core::names::display_name(&found.saved.name).into_owned();
        let question = with_default_no(Message::question(&format!("Delete saved search {name}")));
        Some(self.open_dialog(
            Dialog::Confirm {
                question,
                listing: vec![
                    diroxide_core::names::spell_out(&found.path.to_string_lossy()).into_owned(),
                ],
                on_yes: Command::new(ids::YES),
            },
            Purpose::DeleteSaved(found.path, name),
        ))
    }

    /// Yes to deleting a saved search: its file goes; the list closes.
    pub(super) fn delete_saved(&mut self, path: PathBuf, name: &str) -> Vec<Effect> {
        let mut effects = self.close_all();
        self.request(Request::DeleteSaved(path));
        effects.push(self.say(Message::info(format!("Deleted saved search {name}"))));
        effects
    }

    /// The Space list of a folder, by its items, by kind or by age.
    fn space_list(&mut self, side: Side, place: Place, grouping: Grouping) -> Vec<Effect> {
        let request = self.ids.request();
        self.tools.space = Some((request, side));
        self.request(Request::SpaceList {
            request,
            place: place.clone(),
            grouping,
            clock: SharedClock(Arc::clone(&self.clock)),
        });
        let line = format!("Counting the space in {}", place_leaf(&place));
        vec![self.say(Message::info(line))]
    }

    /// The Space list was counted: the sentence first, then the rows.
    pub(super) fn space_listed(
        &mut self,
        request: RequestId,
        result: Result<SpaceList, Message>,
    ) -> Vec<Effect> {
        if self.tools.space.map(|(r, _)| r) != Some(request) {
            return Vec::new();
        }
        self.tools.space = None;
        let space = match result {
            Ok(space) => space,
            Err(message) => return vec![self.say(message)],
        };
        let mut line = space.summary();
        let notes = space.notes();
        if !notes.is_empty() {
            line.push('.');
        }
        for note in notes {
            line.push_str(&format!(" {note}"));
        }
        let mut list = space.list();
        list.set_help(
            "Up and Down choose, Enter goes into a folder, the key that goes up goes back, Escape closes.",
        );
        // A Space list opened from inside another goes over it.
        let mut effects = Vec::new();
        if matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Space(_))
        ) {
            effects.extend(self.close_top());
        }
        effects.extend(self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SPACE_LIST),
            },
            Purpose::Space(Box::new(space)),
            Message::info(line),
        ));
        effects
    }

    /// Enter on a Space list row: a folder's own Space list.
    pub(super) fn space_row(&mut self, space: SpaceList, row: &str) -> Vec<Effect> {
        let found = space.rows.iter().find(|r| r.id == row).cloned();
        let side = self.active;
        match found.and_then(|r| r.place.map(|p| (p, r.what))) {
            Some((place, what)) if what == "folder" => {
                self.space_list(side, place, Grouping::Items)
            }
            _ => vec![self.say(Message::status("Not a folder: only folders open here."))],
        }
    }

    /// The key that goes up in a Space list: the folder above.
    pub(super) fn space_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let Some(Purpose::Space(space)) = self.opens.last().map(|o| o.purpose.clone()) else {
            return None;
        };
        let up = match self.keymap.lookup(&[Layer::Panel], key) {
            KeyLookup::Command(id) => id == ids::UP,
            KeyLookup::NoMatch => false,
        };
        if !up {
            return None;
        }
        let side = self.active;
        let Some(parent) = space.place.parent() else {
            return Some(vec![self.say(Message::status("At the top already"))]);
        };
        Some(self.space_list(side, parent, space.grouping))
    }

    /// Largest items below here, as search results.
    fn largest_below(&mut self, side: Side) -> Vec<Effect> {
        let place = self.panel(side).place().clone();
        let leaf = place_leaf(&place);
        let query = SearchQuery::everything(place.clone());
        let title = format!("Largest {LARGEST_LIMIT} below {leaf}");
        let start = format!("Looking for the largest items below {leaf}");
        let search = self.begin_results(query, Some(place.clone()), title, start.clone());
        self.request(Request::Largest {
            search,
            place,
            limit: LARGEST_LIMIT,
        });
        self.open_results_saying(start)
    }

    /// The largest items were found.
    pub(super) fn largest_found(
        &mut self,
        search: u64,
        result: Result<Largest, Message>,
    ) -> Vec<Effect> {
        match result {
            Ok(largest) => {
                let line = format!(
                    "Done: the largest {} below {}.",
                    count(largest.hits.len() as u64, "item", "items"),
                    place_leaf(&largest.root)
                );
                self.finish_results(search, largest.hits, line, None)
            }
            Err(message) => self.finish_results_failed(search, message),
        }
    }

    /// Folder sizes: the marked folders, or the focused one; with `all`,
    /// every folder in the panel. The panel's rows then say the sizes.
    fn folder_sizes(&mut self, side: Side, all: bool) -> Vec<Effect> {
        let panel = self.panel(side);
        let folders: Vec<Place> = if all {
            panel
                .entries()
                .iter()
                .filter(|e| e.kind == diroxide_core::EntryKind::Folder)
                .filter_map(|e| panel.place().join(&e.name).ok())
                .collect()
        } else {
            let marked: Vec<Place> = panel
                .selected_entries()
                .into_iter()
                .filter(|e| e.kind == diroxide_core::EntryKind::Folder)
                .filter_map(|e| panel.place().join(&e.name).ok())
                .collect();
            if marked.is_empty() {
                match panel.focused_entry() {
                    Some(e) if e.kind == diroxide_core::EntryKind::Folder => {
                        panel.focused_place().into_iter().collect()
                    }
                    _ => Vec::new(),
                }
            } else {
                marked
            }
        };
        if folders.is_empty() {
            return vec![self.say(Message::status("No folder to size here"))];
        }
        let request = self.ids.request();
        self.tools.sizes = Some((request, side, all));
        let line = format!(
            "Sizing {}",
            count(folders.len() as u64, "folder", "folders")
        );
        self.request(Request::DirSizes {
            request,
            places: folders,
        });
        vec![self.say(Message::info(line))]
    }

    /// The folders were sized: one is said in full; several in all, and
    /// Size every folder offers the size sort.
    pub(super) fn folders_sized(
        &mut self,
        request: RequestId,
        sizes: Vec<(Place, Result<diroxide_search::DirSize, Message>)>,
    ) -> Vec<Effect> {
        let Some((waited, side, all)) = self.tools.sizes else {
            return Vec::new();
        };
        if waited != request {
            return Vec::new();
        }
        self.tools.sizes = None;
        let mut counted: HashMap<String, u64> = HashMap::new();
        let mut total = 0u64;
        let mut failed = Vec::new();
        let mut first = None;
        for (place, result) in &sizes {
            let name = place.name().unwrap_or_default().to_owned();
            match result {
                Ok(size) => {
                    counted.insert(name.clone(), size.bytes);
                    total += size.bytes;
                    if first.is_none() {
                        first = Some((name, *size));
                    }
                }
                Err(message) => failed.push(message.text.clone()),
            }
        }
        let clock = Arc::clone(&self.clock);
        self.panels[side.index()].set_folder_sizes(&counted, clock.as_ref());
        let mut effects = vec![self.redraw_panel(side)];
        if sizes.len() == 1 && !all {
            let line = match (&first, failed.first()) {
                (Some((name, size)), _) => {
                    let mut line = format!(
                        "{}: {}",
                        diroxide_core::names::display_name(name),
                        size.sentence()
                    );
                    if let Some(cloud) = size.cloud_words() {
                        line.push_str(&format!(". {cloud}"));
                    }
                    line
                }
                (None, Some(failed)) => failed.clone(),
                (None, None) => String::new(),
            };
            effects.push(self.say(Message::info(line)));
            return effects;
        }
        let mut line = format!(
            "Sized {}, {} in all",
            count(counted.len() as u64, "folder", "folders"),
            human_size(total)
        );
        if !failed.is_empty() {
            line.push_str(&format!(", {} could not be read", failed.len()));
        }
        if !all {
            effects.push(self.say(Message::info(format!("{line}."))));
            return effects;
        }
        let question = with_default_no(Message::new(
            format!("{line}. Sort by size? y or n"),
            Importance::High,
            MessageKind::Question,
        ));
        effects.extend(self.open_dialog(
            Dialog::Confirm {
                question,
                listing: Vec::new(),
                on_yes: Command::new(ids::YES),
            },
            Purpose::OfferSizeSort,
        ));
        effects
    }

    /// Yes to the size sort after Size every folder.
    pub(super) fn sort_by_size_now(&mut self) -> Vec<Effect> {
        let side = self.active;
        if self.panel(side).sort().key == diroxide_core::SortKey::Size {
            let text = self.panel(side).sort().describe();
            return vec![self.say(Message::info(text))];
        }
        self.sort_by(&ids::SORT_SIZE)
    }

    /// Newest files: the files of the latest day that has any, below the
    /// active folder, as search results; Earlier day goes back a day.
    fn newest_files(&mut self, side: Side, before: Option<Day>) -> Vec<Effect> {
        let place = self.panel(side).place().clone();
        let scope = Scope::new(place.clone()).with_hidden(self.panel(side).shows_hidden());
        let leaf = place_leaf(&place);
        let start = match before {
            None => format!("Looking for the newest files below {leaf}"),
            Some(day) => format!("Looking for files changed before {}", day.words()),
        };
        let query = SearchQuery::everything(place.clone()).with_scope(scope.clone());
        let search =
            self.begin_results(query, Some(place), "Newest files".to_owned(), start.clone());
        self.request(Request::Newest {
            search,
            scope: Box::new(scope),
            before,
            clock: SharedClock(Arc::clone(&self.clock)),
        });
        let mut effects = Vec::new();
        if self.results_shown() {
            effects.extend(self.close_top());
        }
        effects.extend(self.open_results_saying(start));
        effects
    }

    /// Newest files were found: the title says the day.
    pub(super) fn newest_found(
        &mut self,
        search: u64,
        result: Result<Newest, Message>,
    ) -> Vec<Effect> {
        match result {
            Ok(newest) => {
                let title = newest.title();
                let line = format!("Done. {title}.");
                self.finish_results(search, newest.hits, line, Some((title, newest.day)))
            }
            Err(message) => self.finish_results_failed(search, message),
        }
    }

    /// Earlier day: the newest files of the day before the one listed.
    fn earlier_day(&mut self) -> Vec<Effect> {
        let Some(day) = self.newest_day() else {
            let line = "Earlier day works in Newest files' results.";
            return vec![self.say(Message::status(line))];
        };
        let side = self.active;
        self.newest_files(side, Some(day))
    }
}
