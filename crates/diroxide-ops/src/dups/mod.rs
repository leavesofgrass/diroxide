//! Duplicates, the library half (alpha 5, lane DU; R2's duplicates
//! section in `research/modern-features.md`, R23's keeper check in
//! `research/file-management-lessons.md`).
//!
//! - **Finding** ([`find_duplicates`]): every file under the roots,
//!   through each root's provider with the walk's boundary rule; grouped
//!   by size, then by a quick check of the first and last blocks, then by
//!   a full hash of the whole file, then, on request, by comparing the
//!   bytes. The quick check only drops files that cannot match: a file is
//!   never called a duplicate on it. Empty files and hard links to one
//!   file are said and left out by default. The search is cancellable and
//!   pausable through the [`JobControl`](crate::JobControl), says its
//!   progress at the interval the options name, and asks first
//!   ([`DupEstimate`]) when the full hash would read more than the options
//!   allow.
//! - **The result** ([`DupResult`]): groups, the largest waste first; a
//!   count line first ("8 groups, 21 files, 1.4 GB to free"); each group
//!   a row ("Group 3 of 41, 3 copies, 2.1 MB each, 4.2 MB to free") and
//!   each file a row with its name first and its folder after.
//! - **Selection helpers** ([`select`]): keep the oldest (the default),
//!   the newest, the shortest path, the first by name, the first found,
//!   or the copies in one folder; mark the rest. A tie goes to the
//!   shortest path, then the first found. A helper never marks a whole
//!   group; a group it cannot decide is left alone and said.
//! - **Scope:** the roots are folders, the marked entries, or a
//!   collection's places; a file reached twice is counted once.
//! - **The check before the trash** ([`recheck_selection`]): every
//!   keeper and every marked file is read again; a group whose keeper is
//!   gone or changed is refused whole, and a marked file that changed is
//!   dropped.
//!
//! Nothing here deletes. The app sends the selection to the trash through
//! the engine, with the plan's listing.
//!
//! The full hash is the caller's [`ContentHasher`]: the app and the
//! command line give the BLAKE3 the Checksum command computes;
//! [`Sha256Hasher`] is this crate's own.

mod find;
mod hash;
mod recheck;
mod select;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_shaped;
mod words;

use std::time::Duration;

use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::{Place, Timestamp};
use diroxide_vfs::Registry;

use crate::pace::Pacer;

pub use find::find_duplicates;
pub use hash::{ContentHasher, STEP, Sha256Hasher};
pub use recheck::{Rechecked, Refusal, recheck_selection};
pub use select::{GroupChoice, KeepRule, Selection, select};

/// What the search looks at, and how hard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupOptions {
    /// Group empty files too; by default they are counted and left out.
    pub include_empty: bool,
    /// Keep every name of a hard-linked file as a member, said as "same
    /// file as"; by default only the first name found takes part and the
    /// others are left out in words.
    pub include_hard_links: bool,
    /// Files smaller than this are counted and left out; 0 takes every
    /// size.
    pub min_size: u64,
    /// Read hidden files and the files in hidden folders (below a root;
    /// a root named itself is always read). Off, they are counted and
    /// left out before anything is hashed, so the ask-first estimate does
    /// not count them; the app passes the panel's setting (question
    /// 136). On by default.
    pub include_hidden: bool,
    /// Compare the bytes of files whose full hashes match, as the last
    /// step.
    pub byte_compare: bool,
    /// The quick check's block, read at the start and at the end of each
    /// file; at least 1 byte.
    pub partial_block: u64,
    /// Ask first ([`DupSink::confirm`]) when the full hash would read more
    /// than this many bytes; `None` never asks.
    pub ask_above: Option<u64>,
    /// How deep the walk goes below each root.
    pub max_depth: usize,
    /// The shortest time between two progress reports of one step; a new
    /// step is said at once.
    pub progress_interval: Duration,
}

impl DupOptions {
    /// The quick check's default block: 64 KiB.
    pub const PARTIAL_BLOCK: u64 = 64 * 1024;
    /// The default size above which the search asks first: 10 GB, a
    /// starting value for the owner to set.
    pub const ASK_ABOVE: u64 = 10_000_000_000;
}

impl Default for DupOptions {
    fn default() -> Self {
        Self {
            include_empty: false,
            include_hard_links: false,
            min_size: 0,
            include_hidden: true,
            byte_compare: false,
            partial_block: Self::PARTIAL_BLOCK,
            ask_above: Some(Self::ASK_ABOVE),
            max_depth: 256,
            progress_interval: Duration::from_millis(500),
        }
    }
}

/// What a search runs with: the places, the frontend, the hash and the
/// clock.
#[derive(Clone, Copy)]
pub struct DupEnv<'a> {
    /// Serves every root and every file under it.
    pub registry: &'a Registry,
    /// Told the progress, and asked before a large read.
    pub sink: &'a dyn DupSink,
    /// The full hash.
    pub hasher: &'a dyn ContentHasher,
    /// The clock the progress interval is measured on, and the waits
    /// while paused.
    pub pacer: &'a dyn Pacer,
}

impl std::fmt::Debug for DupEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DupEnv")
            .field("hasher", &self.hasher.name())
            .finish_non_exhaustive()
    }
}

/// The frontend of a search.
pub trait DupSink {
    /// Takes a progress report. A new step comes at once; the rest at
    /// most once per [`DupOptions::progress_interval`].
    fn progress(&self, progress: &DupProgress);

    /// Answers the question before a large read: `true` goes on. The
    /// default is No.
    fn confirm(&self, estimate: &DupEstimate) -> bool {
        let _ = estimate;
        false
    }
}

/// A sink that hears nothing and answers No.
#[derive(Clone, Copy, Debug, Default)]
pub struct QuietSink;

impl DupSink for QuietSink {
    fn progress(&self, _progress: &DupProgress) {}
}

/// The step a search is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DupStage {
    /// Listing the folders.
    Listing,
    /// Reading the first and last blocks of files of the same size.
    QuickCheck,
    /// Hashing whole files.
    Hashing,
    /// Comparing the bytes of files whose hashes match.
    Comparing,
    /// Reading the keepers and the marked files again before the trash.
    Rechecking,
}

impl DupStage {
    /// The step in words, lowercase: "listing", "quick check", "hashing",
    /// "comparing", "checking again".
    pub const fn word(self) -> &'static str {
        match self {
            DupStage::Listing => "listing",
            DupStage::QuickCheck => "quick check",
            DupStage::Hashing => "hashing",
            DupStage::Comparing => "comparing",
            DupStage::Rechecking => "checking again",
        }
    }
}

/// How far a search has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupProgress {
    /// The step.
    pub stage: DupStage,
    /// Whether Pause holds it.
    pub paused: bool,
    /// Files finished in this step (found, while listing).
    pub files_done: u64,
    /// Files in this step; 0 while listing.
    pub files_total: u64,
    /// Bytes read in this step.
    pub bytes_done: u64,
    /// Bytes this step will read.
    pub bytes_total: u64,
    /// The file or folder at work.
    pub current: Option<String>,
}

impl DupProgress {
    /// A step at its start.
    pub fn start(stage: DupStage, files_total: u64, bytes_total: u64) -> Self {
        Self {
            stage,
            paused: false,
            files_done: 0,
            files_total,
            bytes_done: 0,
            bytes_total,
            current: None,
        }
    }

    /// The share of the step done, in whole percent: by bytes when the
    /// step reads any, else by files.
    pub fn percent(&self) -> u64 {
        let (done, total) = if self.bytes_total > 0 {
            (self.bytes_done, self.bytes_total)
        } else {
            (self.files_done, self.files_total)
        };
        if total == 0 {
            return 0;
        }
        (u128::from(done.min(total)) * 100 / u128::from(total)) as u64
    }

    /// The jobs line, within 40 cells: "Duplicates: listed 1,200 files",
    /// "Duplicates: hashing, 30 percent", "Duplicates paused at 30
    /// percent".
    pub fn describe(&self) -> String {
        if self.paused {
            return format!("Duplicates paused at {} percent", self.percent());
        }
        match self.stage {
            DupStage::Listing => format!(
                "Duplicates: listed {}",
                grouped_count(self.files_done, "file", "files")
            ),
            stage => format!("Duplicates: {}, {} percent", stage.word(), self.percent()),
        }
    }

    /// The detail line, the step first: "Hashing 12,000 of 45,000 files,
    /// 3.1 of 8.0 GB".
    pub fn detail(&self) -> String {
        match self.stage {
            DupStage::Listing => match &self.current {
                Some(current) => format!("Listing {current}"),
                None => "Listing".to_owned(),
            },
            stage => {
                let word = match stage {
                    DupStage::QuickCheck => "Quick check of",
                    DupStage::Hashing => "Hashing",
                    DupStage::Comparing => "Comparing",
                    _ => "Checking again",
                };
                let mut text = format!(
                    "{word} {} of {}",
                    grouped(self.files_done),
                    grouped_count(self.files_total, "file", "files")
                );
                if self.bytes_total > 0 {
                    text.push_str(", ");
                    text.push_str(&words::bytes_of(self.bytes_done, self.bytes_total));
                }
                text
            }
        }
    }
}

/// The question before a large read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DupEstimate {
    /// Files the full hash would read.
    pub files: u64,
    /// Bytes it would read.
    pub bytes: u64,
}

impl DupEstimate {
    /// The question, the size first and the default last: "Read 8.0 GB
    /// to find duplicates? 1,204 files. No is the default. y or n".
    pub fn question(&self) -> String {
        format!(
            "Read {} to find duplicates? {}. No is the default. y or n",
            human_size(self.bytes),
            grouped_count(self.files, "file", "files")
        )
    }
}

/// Why a search stopped without a result.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DupError {
    /// No roots were given.
    #[error("Nothing to search: no folders were given")]
    NoRoots,
    /// Cancel stopped it.
    #[error("Finding duplicates was cancelled")]
    Cancelled,
    /// The question before a large read was answered No.
    #[error("Stopped before reading {}: not confirmed", human_size(.0.bytes))]
    Declined(DupEstimate),
}

/// One file of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupFile {
    /// Where it is.
    pub place: Place,
    /// Its size in bytes when it was hashed, so the trash plan can check
    /// it again before anything moves.
    pub size: u64,
    /// When its contents last changed, when known, for the same check.
    pub modified: Option<Timestamp>,
    /// Its place in the order the search found files, from 0: what "first
    /// found" means.
    pub found: usize,
    /// With [`DupOptions::include_hard_links`]: the member of the same
    /// group this name is a hard link of, by index. Marking it frees
    /// nothing while that member stays.
    pub same_file_as: Option<usize>,
}

impl DupFile {
    /// The row, the name first and the folder after: "report.pdf, in
    /// D:\a".
    pub fn row(&self) -> String {
        self.place.title()
    }
}

/// Files with the same contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupGroup {
    /// The size of each file, in bytes.
    pub size: u64,
    /// The files, in the order found.
    pub files: Vec<DupFile>,
    /// The full hash all of them share; empty for a group of empty files.
    pub digest: Vec<u8>,
    /// Whether their bytes were compared too.
    pub bytes_compared: bool,
}

impl DupGroup {
    /// The files that are not hard links of another member.
    pub fn distinct(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.same_file_as.is_none())
            .count()
    }

    /// The bytes freed by keeping one: the size times the other distinct
    /// copies.
    pub fn waste(&self) -> u64 {
        self.size
            .saturating_mul(self.distinct().saturating_sub(1) as u64)
    }

    /// The group's row, the number first: "Group 3 of 41, 3 copies, 2.1 MB
    /// each, 4.2 MB to free", or "Group 1 of 3, 5 copies, empty files".
    pub fn row(&self, number: usize, of: usize) -> String {
        let copies = grouped_count(self.distinct() as u64, "copy", "copies");
        let (number, of) = (grouped(number as u64), grouped(of as u64));
        if self.size == 0 {
            return format!("Group {number} of {of}, {copies}, empty files");
        }
        format!(
            "Group {number} of {of}, {copies}, {} each, {} to free",
            human_size(self.size),
            human_size(self.waste())
        )
    }

    /// The member's row in the flat form, the name first: "report.pdf,
    /// copy 1 of 3, group 3, in D:\a"; a hard link says "same file as
    /// copy 1" after its number.
    pub fn file_row(&self, file: usize, number: usize) -> Option<String> {
        let member = self.files.get(file)?;
        let name = member.place.name().unwrap_or_default().to_owned();
        let mut text = format!(
            "{}, copy {} of {}, group {number}",
            diroxide_core::names::display_name(&name),
            file + 1,
            self.files.len()
        );
        if let Some(of) = member.same_file_as {
            text.push_str(&format!(", same file as copy {}", of + 1));
        }
        if let Some(folder) = member.place.parent() {
            text.push_str(&format!(", in {folder}"));
        }
        Some(text)
    }
}

/// Something the search left out, and why in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// What.
    pub place: Place,
    /// Why: "could not be read: access is denied", "same file as
    /// report.pdf, 2 links".
    pub why: String,
}

impl LeftOut {
    /// The row, the name first: "photo.jpg, in D:\a: same file as
    /// report.pdf, 2 links".
    pub fn row(&self) -> String {
        format!("{}: {}", self.place.title(), self.why)
    }
}

/// What a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupResult {
    /// The groups, the largest waste first.
    pub groups: Vec<DupGroup>,
    /// Every file the walk found.
    pub files_seen: u64,
    /// Files and folders left out, each with its reason.
    pub left_out: Vec<LeftOut>,
    /// Empty files counted and left out.
    pub empty_left_out: u64,
    /// Files under [`DupOptions::min_size`] counted and left out.
    pub small_left_out: u64,
    /// Hidden files, and the files in hidden folders, counted and left
    /// out when [`DupOptions::include_hidden`] is off: never hashed and
    /// never in the ask-first estimate.
    pub hidden_left_out: u64,
    /// Extra names of hard-linked files left out.
    pub hard_links_left_out: u64,
    /// Files that could not be read, also in `left_out`.
    pub unreadable: u64,
    /// Files reached a second time (a junction, a folder alias,
    /// overlapping roots), counted once and also in `left_out`.
    pub reached_twice: u64,
    /// Files whose place has no random read, so the quick check was
    /// skipped and the full hash read them whole.
    pub no_quick: u64,
    /// The full hash's name: "BLAKE3", "SHA-256".
    pub hasher: &'static str,
}

impl DupResult {
    /// Every file in every group.
    pub fn files(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum()
    }

    /// The bytes freed by keeping one of each group.
    pub fn waste(&self) -> u64 {
        self.groups.iter().map(DupGroup::waste).sum()
    }

    /// The count line, first: "8 groups, 21 files, 1.4 GB to free", or
    /// "No duplicates in 1,200 files".
    pub fn count_line(&self) -> String {
        if self.groups.is_empty() {
            return format!(
                "No duplicates in {}",
                grouped_count(self.files_seen, "file", "files")
            );
        }
        format!(
            "{}, {}, {} to free",
            grouped_count(self.groups.len() as u64, "group", "groups"),
            grouped_count(self.files() as u64, "file", "files"),
            human_size(self.waste())
        )
    }

    /// The lines after the count line, one per thing left out by kind:
    /// "5 empty files left out", "2 hard links left out: same file".
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.unreadable > 0 {
            notes.push(format!(
                "{} could not be read",
                grouped_count(self.unreadable, "file", "files")
            ));
        }
        if self.empty_left_out > 0 {
            notes.push(format!(
                "{} left out",
                grouped_count(self.empty_left_out, "empty file", "empty files")
            ));
        }
        if self.hard_links_left_out > 0 {
            notes.push(format!(
                "{} left out: same file",
                grouped_count(self.hard_links_left_out, "hard link", "hard links")
            ));
        }
        if self.small_left_out > 0 {
            notes.push(format!(
                "{} left out: too small",
                grouped_count(self.small_left_out, "file", "files")
            ));
        }
        if self.hidden_left_out > 0 {
            notes.push(format!(
                "{} left out",
                grouped_count(self.hidden_left_out, "hidden file", "hidden files")
            ));
        }
        if self.reached_twice > 0 {
            notes.push(format!(
                "{} reached twice, counted once",
                grouped_count(self.reached_twice, "file", "files")
            ));
        }
        if self.no_quick > 0 {
            notes.push(format!(
                "{} read whole: no quick check there",
                grouped_count(self.no_quick, "file", "files")
            ));
        }
        let others = (self.left_out.len() as u64)
            .saturating_sub(self.unreadable)
            .saturating_sub(self.hard_links_left_out)
            .saturating_sub(self.reached_twice);
        if others > 0 {
            notes.push(format!(
                "{} left out by the walk",
                grouped_count(others, "place", "places")
            ));
        }
        notes
    }

    /// The groups' rows in order.
    pub fn group_rows(&self) -> Vec<String> {
        let of = self.groups.len();
        self.groups
            .iter()
            .enumerate()
            .map(|(at, group)| group.row(at + 1, of))
            .collect()
    }

    /// Every file as one list, the Save as collection form: "report.pdf,
    /// copy 1 of 3, group 3, in D:\a".
    pub fn flat_rows(&self) -> Vec<String> {
        self.groups
            .iter()
            .enumerate()
            .flat_map(|(at, group)| {
                (0..group.files.len()).filter_map(move |file| group.file_row(file, at + 1))
            })
            .collect()
    }
}

/// A count with commas: "1,204 files", "1 file", "no files".
pub(crate) fn grouped_count(n: u64, singular: &str, plural: &str) -> String {
    match n {
        0 | 1 => count(n, singular, plural),
        _ => format!("{} {plural}", grouped(n)),
    }
}
