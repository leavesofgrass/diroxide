//! Lists that are not folders, their types in the app (alpha 5, lane
//! LA): the flat view, collections and recent files, over the library
//! halves in `diroxide_search` (the flat view, collections) and
//! `diroxide_config` (recent files). Each opens in the general results
//! lister (`App::open_list` with a `ListKind`).
//!
//! [`ListsAsk`] travels as [`Request::Lists`](crate::Request::Lists) to
//! the runtime, which answers in `runtime/lists.rs`, and [`ListsNote`]
//! comes back as [`AppEvent::Lists`](crate::AppEvent::Lists), handled in
//! `app/collections.rs`. Every read and write of the state folder happens
//! on the runtime's workers; the app keeps no copy of a collection.
//!
//! The words, each checked for its cells in the tests:
//!
//! - "Flat view on, 14 files in 4 folders" (35), "Stopped at 4,200
//!   entries" (24), "Flat view off. work." (20);
//! - "Trip photos, collection, 3 items" (32), "Large images, saved
//!   search, runs on open" (40);
//! - "Removed 1 from Trip photos. The file was not touched." (the
//!   library's sentence; its first 27 cells carry the meaning);
//! - "Recent: 2 files, newest report.md, today" and "Cleared 2 recent
//!   files." (the library's).

use std::path::PathBuf;

use diroxide_config::{RecentAction, RecentFiles};
use diroxide_core::{Message, Place, Side, Timestamp};
use diroxide_search::{FlatOptions, FlatSummary, Saved, SearchHit};

use crate::request::SharedClock;

/// The collection Collect adds to: made the first time it is used.
pub const DEFAULT_COLLECTION: &str = "Collected";

/// What the app asks the runtime for these lists.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ListsAsk {
    /// Walk a folder for the flat view, its rows streamed to the results
    /// lister's list `list` as a search's are; Escape stops it as it
    /// stops a search.
    Flat {
        /// The results lister's list number.
        list: u64,
        /// The base folder.
        root: Place,
        /// Depth, files only, hidden.
        options: FlatOptions,
    },
    /// Read every collection and saved search for the Collections list.
    Collections {
        /// The panel that asked.
        side: Side,
        /// What the list is for: opening one, renaming one, removing one.
        purpose: ListPurpose,
    },
    /// Open a collection (each item looked up) or run a saved search.
    Open {
        /// The panel that asked.
        side: Side,
        /// The collection's or saved search's name.
        name: String,
    },
    /// Add places to a collection, making it when it is missing.
    Add {
        /// The collection's name.
        name: String,
        /// The places, in order.
        places: Vec<Place>,
        /// The app's clock, for a new collection's date.
        clock: SharedClock,
    },
    /// Take items out of a collection; no file is touched.
    Remove {
        /// The collection's name.
        name: String,
        /// The items' places.
        places: Vec<Place>,
    },
    /// Take a collection's missing items out; no file is touched.
    RemoveGone {
        /// The collection's name.
        name: String,
    },
    /// Rename a collection.
    Rename {
        /// Its name now.
        old: String,
        /// The name typed.
        new: String,
    },
    /// Find the file that holds a collection, so it can go to the trash
    /// through the ordinary plan.
    FileOf {
        /// The collection's name.
        name: String,
    },
    /// Read the recent files, each looked up.
    Recent {
        /// The panel that asked.
        side: Side,
    },
    /// Keep a file opened with Enter or edited with F4 in the recent
    /// files; sent only while the setting is on.
    Record {
        /// The file.
        place: Place,
        /// Opened or edited.
        action: RecentAction,
        /// When, from the app's clock.
        now: Timestamp,
    },
    /// Forget every recent file.
    ClearRecent,
    /// Look up a list of places for the results lister: `diro -` and its
    /// paths from standard input ([`crate::App::open_place_list`]).
    Places {
        /// The panel the list belongs to.
        side: Side,
        /// The list's title.
        title: String,
        /// The line said as it opens, its count first.
        start: String,
        /// The places, in order.
        places: Vec<Place>,
    },
}

/// What the Collections list is open for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListPurpose {
    /// Enter opens the collection or runs the saved search.
    Open,
    /// Enter asks for the collection's new name.
    Rename,
    /// Enter asks whether to send the collection's file to the trash.
    Remove,
}

/// One row of the Collections list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionRow {
    /// The name, the row's id.
    pub name: String,
    /// Its words: "Trip photos, collection, 3 items", "Large images,
    /// saved search, runs on open".
    pub words: String,
    /// Whether it is a saved search, run on open.
    pub live: bool,
}

/// What the runtime answers a [`ListsAsk`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ListsNote {
    /// The flat view's walk ended, stopped or failed.
    FlatDone {
        /// The results lister's list number.
        list: u64,
        /// The summary, or why the walk failed.
        result: Result<Box<FlatSummary>, Message>,
    },
    /// The collections and saved searches were read.
    Collections {
        /// The panel that asked.
        side: Side,
        /// What the list is for.
        purpose: ListPurpose,
        /// The rows, by name ignoring case.
        rows: Vec<CollectionRow>,
        /// A sentence for each file that could not be read.
        problems: Vec<String>,
    },
    /// A collection was read and its items looked up.
    Collection {
        /// The panel that asked.
        side: Side,
        /// Its name.
        name: String,
        /// The items that are there.
        hits: Vec<SearchHit>,
        /// The items that are gone.
        gone: Vec<Place>,
        /// Items that could not be checked.
        unknown: u64,
    },
    /// A saved search to run.
    Live {
        /// The saved search.
        saved: Box<Saved>,
    },
    /// Places were added: "Added 3 to Trip photos."
    Added {
        /// The sentence.
        words: String,
    },
    /// Items left a collection: "Removed 1 from Trip photos. The file
    /// was not touched."
    Removed {
        /// The collection's name.
        name: String,
        /// The items' places.
        places: Vec<Place>,
        /// The sentence.
        words: String,
    },
    /// A collection was renamed: "Renamed Trip photos to Trip, 2026."
    Renamed {
        /// The sentence.
        words: String,
    },
    /// The file that holds a collection.
    FileOf {
        /// The collection's name.
        name: String,
        /// Its file in the state folder.
        path: PathBuf,
    },
    /// The recent files were read and looked up.
    Recent {
        /// The panel that asked.
        side: Side,
        /// The list, newest first.
        recent: Box<RecentFiles>,
        /// The files that are there.
        hits: Vec<SearchHit>,
        /// The files that are gone.
        gone: Vec<Place>,
    },
    /// The recent files were forgotten.
    RecentCleared {
        /// How many there were.
        cleared: usize,
    },
    /// A list of places was looked up.
    Places {
        /// The panel the list belongs to.
        side: Side,
        /// The list's title.
        title: String,
        /// The line said as it opens.
        start: String,
        /// The places that are there.
        hits: Vec<SearchHit>,
        /// The places that are gone.
        gone: Vec<Place>,
    },
    /// A sentence to say: why something could not be done, or that
    /// nothing needed doing ("Nothing missing in Trip photos.").
    Say(Message),
}

/// The place a results row stands for, from its id: a hit's id is its
/// place, with "#LINE" after it for a content match.
pub(crate) fn place_of_row(id: &str) -> Option<Place> {
    let place = match id.rsplit_once('#') {
        Some((before, line)) if !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) => {
            before
        }
        _ => id,
    };
    Place::parse(place).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rows_place_drops_a_line_number_only() {
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let file = here.join("notes.txt").unwrap();
        assert_eq!(place_of_row(&file.to_string()), Some(file.clone()));
        assert_eq!(place_of_row(&format!("{file}#12")), Some(file));
        let odd = here.join("a#b").unwrap();
        assert_eq!(place_of_row(&odd.to_string()), Some(odd));
    }

    #[test]
    fn the_default_collection_has_a_plain_name() {
        assert_eq!(DEFAULT_COLLECTION, "Collected");
    }
}
