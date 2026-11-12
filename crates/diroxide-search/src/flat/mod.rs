//! The flat view: every entry below one folder in one list, each row's
//! folder said after its name ("report.pdf, in 2026/q3, 2.1 MB").
//!
//! It runs over the search walker, so hidden entries, ignore files,
//! snapshots and links follow the one set of rules searches have: links
//! are not followed unless [`FlatOptions::follow_links`] says so, and
//! never out of the base folder. It is a streaming job like a search:
//! [`flat_view`] hands rows to a [`FlatSink`] in batches, the first one
//! small so a panel shows rows at once, then a batch at most every
//! [`FlatOptions::batch_interval`]; the [`SearchControl`] stops it from any
//! thread, and the [`FlatSummary`] says what was listed and what was
//! skipped, in words: "Flat view of work: 1,204 files, depth 3".
//!
//! The settings of the mode are a depth limit (direct children are 1),
//! files only or files and folders, hidden entries and ignore files as
//! the panel has them ([`FlatOptions::for_panel`]).

mod entry;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use diroxide_core::format::grouped;
use diroxide_core::names::display_name;
use diroxide_core::{EntryKind, ListModel, Place, SystemClock};
use diroxide_vfs::Registry;

use crate::summary::counted;
use crate::{
    DEFAULT_ROW_CAP, Scope, SearchControl, SearchError, SearchHit, SearchProgress, SearchQuery,
    SearchSink, SearchSummary, run_search,
};

pub use entry::{AT_THE_TOP, FlatEntry, relative_folder};

/// Rows in the first batch, so a panel shows rows at once.
pub const FIRST_BATCH: usize = 32;

/// Rows in a later batch, at most.
pub const BATCH_ROWS: usize = 512;

/// The longest a found row waits before its batch is handed over.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(100);

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
    /// name left out.
    pub ignore_files: bool,
    /// Whether links to folders inside the base folder are followed; off
    /// by default, and never out of it.
    pub follow_links: bool,
    /// Whether the walk stays on the base folder's drive.
    pub one_file_system: bool,
    /// The most rows handed over; the counts keep counting past it.
    /// `None` hands over every row.
    pub row_cap: Option<u64>,
    /// Rows in a batch, at most (the first batch is [`FIRST_BATCH`]).
    pub batch_rows: usize,
    /// The longest a found row waits before its batch is handed over.
    pub batch_interval: Duration,
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
            batch_rows: BATCH_ROWS,
            batch_interval: BATCH_INTERVAL,
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

    /// The options as a panel has them: its hidden setting and the
    /// `search_respect_ignore` setting.
    pub fn for_panel(show_hidden: bool, respect_ignore: bool) -> Self {
        Self::default()
            .with_hidden(show_hidden)
            .with_ignore_files(respect_ignore)
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

    /// The same options with another row cap; `None` hands over every row.
    pub fn with_row_cap(mut self, cap: Option<u64>) -> Self {
        self.row_cap = cap;
        self
    }

    /// The same options with another batch size and interval.
    pub fn with_batches(mut self, rows: usize, interval: Duration) -> Self {
        self.batch_rows = rows.max(1);
        self.batch_interval = interval;
        self
    }

    /// The same options with a thread count; `None` lets the walker
    /// choose.
    pub fn with_threads(mut self, threads: Option<usize>) -> Self {
        self.threads = threads;
        self
    }

    /// The search scope these options walk at `root`. The row cap is the
    /// flat view's own, so the scope has none.
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

/// Where a flat view's rows go, on the thread that runs it.
pub trait FlatSink {
    /// A batch of rows, in the order found. `Break` stops the view, as a
    /// cancel does.
    fn batch(&mut self, rows: Vec<FlatEntry>) -> ControlFlow<()>;

    /// How far it has got, at most ten times a second. The app says
    /// [`FlatProgress::line`] at the progress interval, never on every
    /// call.
    fn progress(&mut self, progress: &FlatProgress) {
        let _ = progress;
    }
}

/// A sink made of a closure that takes every batch.
pub struct FnFlatSink<F>(pub F);

impl<F: FnMut(Vec<FlatEntry>)> FlatSink for FnFlatSink<F> {
    fn batch(&mut self, rows: Vec<FlatEntry>) -> ControlFlow<()> {
        (self.0)(rows);
        ControlFlow::Continue(())
    }
}

/// How far a flat view has got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlatProgress {
    /// The base folder's name, as said.
    pub root_name: String,
    /// Files found so far.
    pub files: u64,
    /// Folders found so far.
    pub folders: u64,
    /// Rows handed over so far.
    pub shown: u64,
    /// Milliseconds since it started.
    pub elapsed_ms: u64,
    /// Whether it is held.
    pub paused: bool,
}

impl FlatProgress {
    /// Entries found so far: files, and folders when they are rows.
    pub fn entries(&self) -> u64 {
        self.files + self.folders
    }

    /// The progress line, the meaning first: "Flat view of Photos: 4,200
    /// entries so far", "Flat view of Photos held: 4,200 entries so far".
    pub fn line(&self) -> String {
        let held = if self.paused { " held" } else { "" };
        format!(
            "Flat view of {}{held}: {} so far",
            self.root_name,
            counted(self.entries(), "entry", "entries")
        )
    }
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
    /// Rows handed over.
    pub shown: u64,
    /// Whether it was stopped before the end.
    pub cancelled: bool,
    /// The walk's own summary: what was skipped, and why.
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

    /// The announcement when the view is on: "Flat view on, 4,200 files
    /// in 310 folders", or "Stopped at 4,200 entries" after a stop.
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

/// The rows as a list titled "Flat view of Photos", each row the name,
/// then its folder, then its size.
pub fn flat_list(root: &Place, rows: &[FlatEntry]) -> ListModel {
    let mut list = ListModel::new(format!("Flat view of {}", root_name(root)))
        .with_rows(rows.iter().map(FlatEntry::row).collect());
    list.set_empty_text("No files below this folder");
    list
}

/// Lists every entry below `root` as one list, handing rows to `sink` in
/// batches as they are found, and returns the summary. `control` stops,
/// holds and resumes it from any thread; a batch the sink answers with
/// `Break` stops it too.
pub fn flat_view(
    root: &Place,
    options: &FlatOptions,
    registry: &Registry,
    control: &SearchControl,
    sink: &mut dyn FlatSink,
) -> Result<FlatSummary, SearchError> {
    let query = SearchQuery::everything(root.clone()).with_scope(options.scope(root.clone()));
    let name = root_name(root);
    let mut batcher = Batcher {
        root,
        options,
        control,
        sink,
        root_name: name.clone(),
        buffer: Vec::new(),
        last_flush: Instant::now(),
        batches: 0,
        files: 0,
        folders: 0,
        shown: 0,
        homes: HashSet::new(),
        stop: false,
    };
    let search = run_search(&query, registry, &SystemClock::utc(), control, &mut batcher)?;
    let stopped = batcher.stop;
    // The last rows; every entry is found, so a stop asked for now
    // changes nothing.
    let _ = batcher.flush();
    Ok(FlatSummary {
        root: root.clone(),
        root_name: name,
        options: options.clone(),
        files: batcher.files,
        folders: batcher.folders,
        folders_with_files: batcher.homes.len() as u64,
        shown: batcher.shown,
        cancelled: search.cancelled || stopped,
        search,
    })
}

/// Turns the search's hits into flat rows and hands them over in batches.
struct Batcher<'a> {
    root: &'a Place,
    options: &'a FlatOptions,
    control: &'a SearchControl,
    sink: &'a mut dyn FlatSink,
    root_name: String,
    buffer: Vec<FlatEntry>,
    last_flush: Instant,
    batches: u64,
    files: u64,
    folders: u64,
    shown: u64,
    /// The folders, below the base, that hold a file found.
    homes: HashSet<String>,
    /// Whether the sink asked to stop.
    stop: bool,
}

impl Batcher<'_> {
    /// Whether the rows waiting should go now.
    fn due(&self) -> bool {
        let size = if self.batches == 0 {
            FIRST_BATCH.min(self.options.batch_rows)
        } else {
            self.options.batch_rows
        };
        self.buffer.len() >= size || self.last_flush.elapsed() >= self.options.batch_interval
    }

    /// Hands the waiting rows over. Returns `Break` when the sink asks to
    /// stop.
    fn flush(&mut self) -> ControlFlow<()> {
        self.last_flush = Instant::now();
        if self.buffer.is_empty() || self.stop {
            return ControlFlow::Continue(());
        }
        self.batches += 1;
        let rows = std::mem::take(&mut self.buffer);
        match self.sink.batch(rows) {
            ControlFlow::Continue(()) => ControlFlow::Continue(()),
            ControlFlow::Break(()) => {
                self.stop = true;
                ControlFlow::Break(())
            }
        }
    }
}

impl SearchSink for Batcher<'_> {
    fn found(&mut self, hit: SearchHit) -> ControlFlow<()> {
        if self.stop {
            return ControlFlow::Break(());
        }
        let row = FlatEntry::from_hit(hit, self.root);
        if row.entry.kind == EntryKind::Folder {
            self.folders += 1;
            if !self.options.folders {
                return ControlFlow::Continue(());
            }
        } else {
            self.files += 1;
            if !self.homes.contains(&row.folder) {
                self.homes.insert(row.folder.clone());
            }
        }
        if self.options.row_cap.is_some_and(|cap| self.shown >= cap) {
            return ControlFlow::Continue(());
        }
        self.shown += 1;
        self.buffer.push(row);
        if self.due() {
            self.flush()
        } else {
            ControlFlow::Continue(())
        }
    }

    fn progress(&mut self, progress: &SearchProgress) {
        if !self.buffer.is_empty()
            && self.last_flush.elapsed() >= self.options.batch_interval
            && self.flush().is_break()
        {
            // The search hears the stop at its next hit; the cancel makes
            // it hear it even when no hit comes.
            self.control.cancel();
        }
        let progress = FlatProgress {
            root_name: self.root_name.clone(),
            files: self.files,
            folders: if self.options.folders {
                self.folders
            } else {
                0
            },
            shown: self.shown,
            elapsed_ms: progress.elapsed_ms,
            paused: progress.paused,
        };
        self.sink.progress(&progress);
    }
}
