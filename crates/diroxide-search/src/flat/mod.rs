//! The flat view: every entry below one folder in one list, as search
//! hits, so the results lister shows it with no list code of its own:
//! "report.pdf, in work/2026/q3, 2.1 KB".
//!
//! It is not a second walker. [`flat_query`] builds the search a flat
//! view is: no filter, the depth as the scope's `max_depth` (direct
//! children are 1), hidden entries as the panel has them, ignore files
//! read only when asked (a panel reads none), links never followed
//! unless asked and never out of the base folder, no row cap of the
//! search's own. [`flat_view`] runs it through [`run_search`], leaves the
//! folders out when the mode is files only, counts files and the folders
//! that hold them, and hands every hit on to the caller's [`SearchSink`]
//! in arrival order, so the results lister streams them as it streams a
//! search. The [`FlatSummary`] says it in words: "Flat view on, 14 files
//! in 4 folders", "Flat view of work: 1,204 files, depth 3".
//!
//! Each row's key is its path below the base folder ([`flat_key`]),
//! never its name, because names repeat in a flat view.

mod entry;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::ops::ControlFlow;

use diroxide_core::format::grouped;
use diroxide_core::names::display_name;
use diroxide_core::{EntryKind, Place, SystemClock};
use diroxide_vfs::Registry;

use crate::summary::counted;
use crate::{
    DEFAULT_ROW_CAP, Scope, SearchControl, SearchError, SearchHit, SearchProgress, SearchQuery,
    SearchSink, SearchSummary, run_search,
};

pub use entry::{flat_key, flat_list, flat_row, relative_folder};

/// How a flat view walks and what it lists.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlatOptions {
    /// How many folders deep to list; direct children are 1. `None` lists
    /// every level (the walk's own limit of 256 still applies, and is
    /// said).
    pub depth: Option<u32>,
    /// Whether folders are rows too; by default only files are listed,
    /// while every folder is still walked.
    pub folders: bool,
    /// Whether hidden entries are listed and hidden folders entered.
    pub hidden: bool,
    /// Whether `.gitignore` and `.ignore` files are read and what they
    /// name left out; off, as a panel reads none.
    pub ignore_files: bool,
    /// Whether links to folders inside the base folder are followed; off
    /// by default, and never out of it.
    pub follow_links: bool,
    /// Whether the walk stays on the base folder's drive.
    pub one_file_system: bool,
    /// The most rows handed on; the counts keep counting past it. `None`
    /// hands on every row.
    pub row_cap: Option<u64>,
    /// Threads that walk; `None` lets the walker choose.
    pub threads: Option<usize>,
}

impl Default for FlatOptions {
    fn default() -> Self {
        Self {
            depth: None,
            folders: false,
            hidden: false,
            ignore_files: false,
            follow_links: false,
            one_file_system: true,
            row_cap: Some(DEFAULT_ROW_CAP),
            threads: None,
        }
    }
}

impl FlatOptions {
    /// The defaults: files only, every level, hidden entries skipped,
    /// ignore files not read, links not followed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The options as a panel has them: its hidden setting; ignore files
    /// are not read, as a panel reads none.
    pub fn for_panel(show_hidden: bool) -> Self {
        Self::default().with_hidden(show_hidden)
    }

    /// The same options with a depth limit; direct children are 1, and
    /// `None` or 0 is every level.
    pub fn with_depth(mut self, depth: Option<u32>) -> Self {
        self.depth = depth.filter(|d| *d > 0);
        self
    }

    /// The same options, listing folders as rows or not.
    pub fn with_folders(mut self, folders: bool) -> Self {
        self.folders = folders;
        self
    }

    /// The same options, listing hidden entries or not.
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// The same options, reading ignore files or not.
    pub fn with_ignore_files(mut self, read: bool) -> Self {
        self.ignore_files = read;
        self
    }

    /// The same options, following links to folders inside the base
    /// folder or not.
    pub fn with_follow_links(mut self, follow: bool) -> Self {
        self.follow_links = follow;
        self
    }

    /// The same options with another row cap; `None` hands on every row.
    pub fn with_row_cap(mut self, cap: Option<u64>) -> Self {
        self.row_cap = cap;
        self
    }

    /// The same options with a thread count; `None` lets the walker
    /// choose.
    pub fn with_threads(mut self, threads: Option<usize>) -> Self {
        self.threads = threads;
        self
    }

    /// The search scope these options walk at `root`. The row cap is the
    /// flat view's own, since folders it leaves out are hits too, so the
    /// scope has none.
    pub fn scope(&self, root: Place) -> Scope {
        Scope::new(root)
            .with_hidden(self.hidden)
            .with_ignore_files(self.ignore_files)
            .with_follow_links(self.follow_links)
            .with_one_file_system(self.one_file_system)
            .with_max_depth(self.depth)
            .with_threads(self.threads)
            .with_row_cap(None)
    }

    /// The depth in words: "depth 3", "every level".
    pub fn depth_words(&self) -> String {
        match self.depth {
            Some(depth) => format!("depth {depth}"),
            None => "every level".to_owned(),
        }
    }

    /// The mode in words: "files only", "files and folders".
    pub fn mode_words(&self) -> &'static str {
        if self.folders {
            "files and folders"
        } else {
            "files only"
        }
    }
}

/// The search a flat view of `root` is: every entry, no filter, in the
/// options' scope. Run it with [`flat_view`], which also leaves folders
/// out in the files-only mode and counts.
pub fn flat_query(root: &Place, options: &FlatOptions) -> SearchQuery {
    SearchQuery::everything(root.clone()).with_scope(options.scope(root.clone()))
}

/// The progress line, the meaning first: "Flat view of Photos: 4,200
/// entries so far", "Flat view of Photos held: 4,200 entries so far".
/// The progress a flat view hands on counts its rows in `found`.
pub fn flat_progress_line(root: &Place, progress: &SearchProgress) -> String {
    let held = if progress.paused { " held" } else { "" };
    format!(
        "Flat view of {}{held}: {} so far",
        root_name(root),
        counted(progress.found, "entry", "entries")
    )
}

/// What a flat view listed, and what it skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlatSummary {
    /// The base folder.
    pub root: Place,
    /// The base folder's name, as said.
    pub root_name: String,
    /// The options it ran with.
    pub options: FlatOptions,
    /// Files found (past the row cap too).
    pub files: u64,
    /// Folders found below the base folder, rows or not.
    pub folders: u64,
    /// Folders that hold at least one file found, the base folder
    /// included.
    pub folders_with_files: u64,
    /// Rows handed on.
    pub shown: u64,
    /// Whether it was stopped before the end.
    pub cancelled: bool,
    /// The search's own summary: what was skipped, and why.
    pub search: SearchSummary,
}

impl FlatSummary {
    /// Entries found: files, and folders when they are rows.
    pub fn entries(&self) -> u64 {
        if self.options.folders {
            self.files + self.folders
        } else {
            self.files
        }
    }

    /// Whether rows were left out at the row cap.
    pub fn capped(&self) -> bool {
        self.shown < self.entries()
    }

    /// The count line, the meaning first: "Flat view of work: 1,204
    /// files, depth 3"; with folders as rows, "Flat view of work: 1,204
    /// files, 310 folders, every level".
    pub fn count_line(&self) -> String {
        let mut line = format!(
            "Flat view of {}: {}",
            self.root_name,
            counted(self.files, "file", "files")
        );
        if self.options.folders {
            line.push_str(", ");
            line.push_str(&counted(self.folders, "folder", "folders"));
        }
        line.push_str(", ");
        line.push_str(&self.options.depth_words());
        line
    }

    /// The announcement when the view is on: "Flat view on, 14 files in 4
    /// folders", or "Stopped at 4,200 entries" after a stop.
    pub fn on_words(&self) -> String {
        if self.cancelled {
            return format!("Stopped at {}", counted(self.entries(), "entry", "entries"));
        }
        format!(
            "Flat view on, {} in {}",
            counted(self.files, "file", "files"),
            counted(self.folders_with_files, "folder", "folders")
        )
    }

    /// The status line: the count line, or "Stopped at 4,200 entries".
    pub fn status_line(&self) -> String {
        if self.cancelled {
            self.on_words()
        } else {
            self.count_line()
        }
    }

    /// What was left out, one sentence each: the row cap, then the walk's
    /// skips ("Hidden files skipped.").
    pub fn skip_sentences(&self) -> Vec<String> {
        let mut sentences = Vec::new();
        if self.capped() {
            sentences.push(format!(
                "Showing the first {}. Set a depth to list fewer.",
                grouped(self.shown)
            ));
        }
        sentences.extend(self.search.skip_sentences());
        sentences
    }

    /// The whole summary as said when the view is ready: the status line
    /// and then what was skipped.
    pub fn sentence(&self) -> String {
        let mut parts = vec![format!("{}.", self.status_line())];
        parts.extend(self.skip_sentences());
        parts.join(" ")
    }
}

/// The announcement when the view is turned off: "Flat view off. Photos."
pub fn flat_off_words(root: &Place) -> String {
    format!("Flat view off. {}.", root_name(root))
}

/// A base folder's name as said: its own name, or the drive or share.
pub fn root_name(root: &Place) -> String {
    match root.name() {
        Some(name) => display_name(name).into_owned(),
        None => display_name(&root.to_string()).into_owned(),
    }
}

/// Lists every entry below `root` as one list: runs [`flat_query`],
/// hands each row on to `sink` as a [`SearchHit`] in arrival order (the
/// results lister's sink), and returns the summary. `control` stops,
/// holds and resumes it from any thread; a hit the sink answers with
/// `Break` stops it too. The progress handed on counts the rows in
/// `found`.
pub fn flat_view(
    root: &Place,
    options: &FlatOptions,
    registry: &Registry,
    control: &SearchControl,
    sink: &mut dyn SearchSink,
) -> Result<FlatSummary, SearchError> {
    let mut counter = Counter {
        root,
        options,
        sink,
        files: 0,
        folders: 0,
        shown: 0,
        homes: HashSet::new(),
        stop: false,
    };
    let search = run_search(
        &flat_query(root, options),
        registry,
        &SystemClock::utc(),
        control,
        &mut counter,
    )?;
    Ok(FlatSummary {
        root: root.clone(),
        root_name: root_name(root),
        options: options.clone(),
        files: counter.files,
        folders: counter.folders,
        folders_with_files: counter.homes.len() as u64,
        shown: counter.shown,
        cancelled: search.cancelled || counter.stop,
        search,
    })
}

/// Counts the search's hits and hands the flat view's rows on.
struct Counter<'a> {
    root: &'a Place,
    options: &'a FlatOptions,
    sink: &'a mut dyn SearchSink,
    files: u64,
    folders: u64,
    shown: u64,
    /// The folders, below the base, that hold a file found.
    homes: HashSet<String>,
    /// Whether the sink asked to stop.
    stop: bool,
}

impl SearchSink for Counter<'_> {
    fn found(&mut self, hit: SearchHit) -> ControlFlow<()> {
        // A hit already on its way when the sink said stop is not handed
        // on: the sink hears nothing after its own Break.
        if self.stop {
            return ControlFlow::Break(());
        }
        if hit.entry.kind == EntryKind::Folder {
            self.folders += 1;
            if !self.options.folders {
                return ControlFlow::Continue(());
            }
        } else {
            self.files += 1;
            let home = relative_folder(&hit.place, self.root);
            if !self.homes.contains(&home) {
                self.homes.insert(home);
            }
        }
        if self.options.row_cap.is_some_and(|cap| self.shown >= cap) {
            return ControlFlow::Continue(());
        }
        self.shown += 1;
        let flow = self.sink.found(hit);
        if flow.is_break() {
            self.stop = true;
        }
        flow
    }

    fn progress(&mut self, progress: &SearchProgress) {
        let mut progress = progress.clone();
        progress.found = if self.options.folders {
            self.files + self.folders
        } else {
            self.files
        };
        self.sink.progress(&progress);
    }
}
