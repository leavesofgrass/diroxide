//! The runtime half of the flat view, collections and recent files
//! (alpha 5, lane LA): walks, reads and writes on workers, answered as
//! [`crate::lists::ListsNote`].
//!
//! `Request::Lists` reaches [`Runtime::lists_request`]. The flat view's
//! walk takes the search's place in the runtime, so the results lister's
//! Escape stops it and its rows arrive as a search's do; the collections
//! and the recent files are read and written in the state folder only.

use std::sync::Arc;
use std::time::Instant;

use diroxide_config::{Paths, RecentFiles};
use diroxide_core::{Entry, EntryKind, Message, Place};
use diroxide_search::{
    CollectionError, Collections, Freshness, SearchControl, SearchHit, item_freshness,
};
use diroxide_vfs::{OpContext, Registry, VfsError};

use super::{BatchSink, Runtime};
use crate::app::AppEvent;
use crate::lists::{CollectionRow, ListPurpose, ListsAsk, ListsNote};

/// The sentence when there is no state folder (a test, or a run that
/// could not make one).
const NO_STATE: &str = "Lists need a state folder, and this run has none.";

/// How places looked up came out: there, gone, not checked.
#[derive(Default)]
struct Looked {
    hits: Vec<SearchHit>,
    gone: Vec<Place>,
    unknown: u64,
}

/// Looks each place up through its provider: the ones there with their
/// entries, the gone ones, and a count of those that could not be
/// checked (listed as plain files, so they still show).
fn look_up(registry: &Registry, places: Vec<Place>) -> Looked {
    let ctx = OpContext::new();
    let mut looked = Looked::default();
    for place in places {
        match registry
            .resolve(&place)
            .and_then(|provider| provider.stat(&place, &ctx))
        {
            Ok(entry) => looked.hits.push(SearchHit::new(place, entry)),
            Err(VfsError::NotFound) => looked.gone.push(place),
            Err(_) => {
                looked.unknown += 1;
                let name = place.name().unwrap_or_default().to_owned();
                looked
                    .hits
                    .push(SearchHit::new(place, Entry::new(name, EntryKind::File)));
            }
        }
    }
    looked
}

fn failed(error: &CollectionError) -> ListsNote {
    ListsNote::Say(Message::error(error.to_string()))
}

impl Runtime {
    /// Sends a note back to the app.
    fn lists_send(shared: &Arc<super::Shared>, note: ListsNote) {
        shared.send(AppEvent::Lists(Box::new(note)));
    }

    /// Carries out one ask of lane LA.
    pub(super) fn lists_request(&mut self, ask: ListsAsk) {
        if let ListsAsk::Flat {
            list,
            root,
            options,
        } = ask
        {
            self.flat_walk(list, root, options);
            return;
        }
        if let ListsAsk::Places {
            side,
            title,
            start,
            places,
        } = ask
        {
            let registry = Arc::clone(&self.registry);
            self.worker(move |shared| {
                let looked = look_up(&registry, places);
                let note = ListsNote::Places {
                    side,
                    title,
                    start,
                    hits: looked.hits,
                    gone: looked.gone,
                };
                Self::lists_send(shared, note);
            });
            return;
        }
        let Some(paths) = self.paths.clone() else {
            // Recording a recent file without a state folder is quiet.
            if !matches!(ask, ListsAsk::Record { .. }) {
                let note = ListsNote::Say(Message::status(NO_STATE));
                self.queue.push_back(AppEvent::Lists(Box::new(note)));
            }
            return;
        };
        let registry = Arc::clone(&self.registry);
        self.worker(move |shared| {
            if let Some(note) = state_work(&paths, &registry, ask) {
                Self::lists_send(shared, note);
            }
        });
    }

    /// The flat view's walk: it takes the search's place, so Escape in
    /// the results lister stops it, and its rows reach the list as a
    /// search's do.
    fn flat_walk(&mut self, list: u64, root: Place, options: diroxide_search::FlatOptions) {
        if let Some((_, old)) = self.search.take() {
            old.cancel();
        }
        let control = SearchControl::new();
        self.search = Some((list, control.clone()));
        let registry = Arc::clone(&self.registry);
        self.worker(move |shared| {
            let mut sink = BatchSink {
                shared: Arc::clone(shared),
                search: list,
                batch: Vec::new(),
                sent: Instant::now(),
                progress_sent: None,
            };
            let result =
                diroxide_search::flat_view(&root, &options, &registry, &control, &mut sink);
            sink.flush();
            let leaf = crate::panel::place_leaf(&root);
            let result = result
                .map(Box::new)
                .map_err(|error| Message::from_error("list", &leaf, &error));
            Self::lists_send(shared, ListsNote::FlatDone { list, result });
        });
    }
}

/// The asks that read or write the state folder, on a worker. `None`
/// when there is nothing to say (a recent file kept).
fn state_work(paths: &Paths, registry: &Registry, ask: ListsAsk) -> Option<ListsNote> {
    let collections = Collections::in_state_folder(&paths.state_dir);
    let note = match ask {
        ListsAsk::Collections { side, purpose } => {
            let (listed, problems) = collections.list();
            let rows = listed
                .into_iter()
                .filter(|item| purpose == ListPurpose::Open || !item.is_live())
                .map(|item| CollectionRow {
                    words: item.saved.row_words(),
                    live: item.is_live(),
                    name: item.saved.name,
                })
                .collect();
            ListsNote::Collections {
                side,
                purpose,
                rows,
                problems,
            }
        }
        ListsAsk::Open { side, name } => match collections.find(&name) {
            None => failed(&CollectionError::NotFound(name)),
            Some(found) if found.is_live() => ListsNote::Live {
                saved: Box::new(found.saved),
            },
            Some(found) => {
                let places = found.saved.items.iter().map(|i| i.place.clone()).collect();
                let looked = look_up(registry, places);
                ListsNote::Collection {
                    side,
                    name: found.saved.name,
                    hits: looked.hits,
                    gone: looked.gone,
                    unknown: looked.unknown,
                }
            }
        },
        ListsAsk::Add {
            name,
            places,
            clock,
        } => match collections.add(&name, places, clock.0.as_ref()) {
            Ok((listed, added)) => ListsNote::Added {
                words: listed.saved.added_words(added),
            },
            Err(error) => failed(&error),
        },
        ListsAsk::Remove { name, places } => match collections.remove(&name, &places) {
            Ok((listed, removed)) => ListsNote::Removed {
                words: listed.saved.removed_words(removed),
                name: listed.saved.name,
                places,
            },
            Err(error) => failed(&error),
        },
        ListsAsk::RemoveGone { name } => remove_gone(&collections, registry, &name),
        ListsAsk::Rename { old, new } => match collections.rename(&old, &new) {
            Ok(listed) => ListsNote::Renamed {
                words: Collections::renamed_words(&old, &listed),
            },
            Err(error) => failed(&error),
        },
        ListsAsk::FileOf { name } => match collections.load(&name) {
            Ok(listed) => ListsNote::FileOf {
                name: listed.saved.name,
                path: listed.path,
            },
            Err(error) => failed(&error),
        },
        ListsAsk::Recent { side } => {
            let loaded = RecentFiles::load(paths);
            let places = loaded.value.items.iter().map(|i| i.place.clone()).collect();
            let looked = look_up(registry, places);
            ListsNote::Recent {
                side,
                recent: Box::new(loaded.value),
                hits: looked.hits,
                gone: looked.gone,
            }
        }
        ListsAsk::Record { place, action, now } => {
            let mut recent = RecentFiles::load(paths).value;
            recent.record(place, action, now, true);
            match recent.save(paths) {
                Ok(()) => return None,
                Err(error) => {
                    ListsNote::Say(Message::warning(format!("Recent files not kept: {error}")))
                }
            }
        }
        ListsAsk::ClearRecent => match diroxide_config::clear_recent(paths) {
            Ok(cleared) => ListsNote::RecentCleared { cleared },
            Err(error) => ListsNote::Say(Message::error(format!(
                "Could not clear the recent files: {error}"
            ))),
        },
        // Answered on the runtime's own thread above.
        ListsAsk::Flat { .. } | ListsAsk::Places { .. } => return None,
    };
    Some(note)
}

/// Remove missing items: the gone ones found first, so the list on the
/// screen loses exactly those rows; the ones that could not be checked
/// stay.
fn remove_gone(collections: &Collections, registry: &Registry, name: &str) -> ListsNote {
    let listed = match collections.load(name) {
        Ok(listed) => listed,
        Err(error) => return failed(&error),
    };
    let ctx = OpContext::new();
    let gone: Vec<Place> = listed
        .saved
        .items
        .iter()
        .filter(|item| item_freshness(item, registry, &ctx) == Freshness::Gone)
        .map(|item| item.place.clone())
        .collect();
    if gone.is_empty() {
        let name = diroxide_core::names::display_name(&listed.saved.name).into_owned();
        return ListsNote::Say(Message::status(format!("Nothing missing in {name}.")));
    }
    match collections.remove(name, &gone) {
        Ok((listed, removed)) => ListsNote::Removed {
            words: listed.saved.removed_words(removed),
            name: listed.saved.name,
            places: gone,
        },
        Err(error) => failed(&error),
    }
}
