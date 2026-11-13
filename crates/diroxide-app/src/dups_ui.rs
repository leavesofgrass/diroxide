//! Duplicates, their types and words in the app (alpha 5, lane DA): the
//! scope, the options, the scan, the groups and their marks, over the
//! engine half in `diroxide_ops::dups`; the full hash is BLAKE3 through
//! `diroxide-meta`, supplied by the runtime.
//!
//! [`DupsAsk`] travels as [`Request::Duplicates`](crate::Request::Duplicates)
//! to the runtime, which answers in `runtime/duplicates.rs`, and
//! [`DupsNote`] comes back as
//! [`AppEvent::Duplicates`](crate::AppEvent::Duplicates), handled in
//! `app/duplicates.rs`. The scan and the check before the trash run as
//! `JobKind::FindDuplicates` jobs, cancellable from the Jobs list.
//!
//! The marks live in a [`DupView`]: nothing is marked when the groups
//! open; a keep rule marks every copy but its keeper in each group
//! ([`DupView::apply`]); Enter on a copy changes its mark
//! ([`DupView::toggle`]), and the last unmarked copy of a group can never
//! be marked: "One copy must stay." Nothing here deletes: the marked
//! copies go to the trash through the engine's plan, after the engine's
//! check of every keeper and every marked copy.

use std::time::Duration;

use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::names::display_name;
use diroxide_core::{Entry, EntryKind, JobId, Place, Side};
use diroxide_ops::dups::{
    DupError, DupFile, DupOptions, DupProgress, DupResult, DupStage, GroupChoice, KeepRule,
    Rechecked, Selection, select,
};

/// Said when marking would leave a group with no copy: 19 cells.
pub const ONE_COPY_MUST_STAY: &str = "One copy must stay.";

/// Said by Choose copies to keep when no duplicates are open: 36 cells.
pub const NO_GROUPS_OPEN: &str = "No duplicates open: Find them first.";

/// Said when Trash is chosen with nothing marked: 38 cells.
pub const NOTHING_MARKED: &str = "Nothing marked: choose copies to keep.";

/// Said when the trash question is answered No: 27 cells.
pub const NOTHING_TRASHED: &str = "Nothing moved to the trash.";

/// Said when the keep rules are closed without one: 31 cells.
pub const MARKS_UNCHANGED: &str = "No rule chosen; the marks stay.";

/// Said when the scope list is closed: 32 cells.
pub const NOTHING_SEARCHED: &str = "Nothing searched for duplicates.";

/// The list's title for where to look: 21 cells.
pub const SCOPE_TITLE: &str = "Find duplicates where";

/// The options list's title: 23 cells.
pub const OPTIONS_TITLE: &str = "Find duplicates options";

/// The keep rules' title: 21 cells.
pub const RULES_TITLE: &str = "Choose copies to keep";

/// The least sizes the options step through: any size, 1 KB, 1 MB, 100
/// MB.
pub const LEAST_SIZES: [u64; 4] = [0, 1_000, 1_000_000, 100_000_000];

/// Where Find duplicates looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DupScope {
    /// The active panel's folder, and every folder below it.
    ThisFolder,
    /// The active panel's marked entries: files, and folders searched
    /// whole.
    Marked,
    /// Both panels' folders, as one search.
    BothPanels,
}

impl DupScope {
    /// The row id in the scope list.
    pub fn id(self) -> &'static str {
        match self {
            DupScope::ThisFolder => "this",
            DupScope::Marked => "marked",
            DupScope::BothPanels => "both",
        }
    }

    /// The scope a row id names.
    pub fn from_id(id: &str) -> Option<Self> {
        [DupScope::ThisFolder, DupScope::Marked, DupScope::BothPanels]
            .into_iter()
            .find(|scope| scope.id() == id)
    }
}

/// A search's scope and options, as chosen in the lists before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupSetup {
    /// The panel it was asked from: the trash plan's panel, and "the one
    /// in this folder".
    pub side: Side,
    /// Where it looks.
    pub scope: DupScope,
    /// The places searched: folders, and files for a marked file.
    pub roots: Vec<Place>,
    /// The least size, by its index in [`LEAST_SIZES`].
    pub least: usize,
    /// Whether whole files are compared byte by byte after their hashes
    /// match.
    pub byte_compare: bool,
    /// Whether hidden files and the files in hidden folders are read.
    pub hidden: bool,
    /// Above this many bytes to hash, the search stops and asks first;
    /// `None` never asks.
    pub ask_above: Option<u64>,
    /// The shortest time between two progress lines, in milliseconds.
    pub interval_ms: u64,
}

impl DupSetup {
    /// The engine's options for this setup.
    pub fn options(&self) -> DupOptions {
        DupOptions {
            min_size: self.least_size(),
            byte_compare: self.byte_compare,
            include_hidden: self.hidden,
            ask_above: self.ask_above,
            progress_interval: Duration::from_millis(self.interval_ms),
            ..DupOptions::default()
        }
    }

    /// The least size in bytes.
    pub fn least_size(&self) -> u64 {
        LEAST_SIZES.get(self.least).copied().unwrap_or_default()
    }

    /// The options list's rows, the action first: each (id, words).
    pub fn option_rows(&self) -> Vec<(&'static str, String)> {
        let least = match self.least_size() {
            0 => "any size".to_owned(),
            bytes => format!("{} or more", human_size(bytes)),
        };
        let bytes = if self.byte_compare {
            "on, slower"
        } else {
            "off, hashes only"
        };
        let hidden = if self.hidden { "read" } else { "left out" };
        vec![
            ("find", format!("Find now: {}", self.where_words())),
            ("least", format!("Least size: {least}")),
            ("bytes", format!("Byte check: {bytes}")),
            ("hidden", format!("Hidden files: {hidden}")),
        ]
    }

    /// Where it looks, in words: "D:\photos", "3 marked entries", "both
    /// panels".
    pub fn where_words(&self) -> String {
        match self.scope {
            DupScope::ThisFolder => self
                .roots
                .first()
                .map(ToString::to_string)
                .unwrap_or_default(),
            DupScope::Marked => count(self.roots.len() as u64, "marked entry", "marked entries"),
            DupScope::BothPanels => "both panels".to_owned(),
        }
    }

    /// The jobs line as the search starts: "Finding duplicates in
    /// D:\photos".
    pub fn start_line(&self) -> String {
        format!("Finding duplicates in {}", self.where_words())
    }
}

/// What the app asks the runtime for duplicates.
#[derive(Clone, Debug, PartialEq)]
pub enum DupsAsk {
    /// Search, as a job.
    Find {
        /// The job.
        job: JobId,
        /// The scope and options, given back with the answer.
        setup: Box<DupSetup>,
    },
    /// Read every keeper and every marked copy again before the trash, as
    /// a job.
    Recheck {
        /// The job.
        job: JobId,
        /// The groups and their marks, given back with the answer.
        view: Box<DupView>,
    },
}

/// What the runtime answers a [`DupsAsk`].
#[derive(Debug)]
pub enum DupsNote {
    /// How far a search or a check has got, at the setup's interval.
    Progress {
        /// The job.
        job: JobId,
        /// The step and the counts.
        progress: DupProgress,
    },
    /// A search ended.
    Found {
        /// The job.
        job: JobId,
        /// The scope and options it ran with.
        setup: Box<DupSetup>,
        /// The groups, or why there are none.
        result: Result<Box<DupResult>, DupError>,
        /// Copies left out after the search because they are hidden or
        /// in a hidden folder.
        hidden_left_out: u64,
    },
    /// The check before the trash ended.
    Rechecked {
        /// The job.
        job: JobId,
        /// The groups and their marks it checked.
        view: Box<DupView>,
        /// What may go, and what was held back.
        result: Result<Rechecked, DupError>,
    },
}

/// The progress line, the step first and within 40 cells: "Hashing 120
/// of 450 files" (24 cells), "Listing, 1,200 files found".
pub fn progress_line(progress: &DupProgress) -> String {
    if progress.paused {
        return progress.describe();
    }
    let of = |word: &str| {
        format!(
            "{word} {} of {}",
            grouped(progress.files_done),
            files(progress.files_total)
        )
    };
    match progress.stage {
        DupStage::Listing => format!("Listing, {} found", files(progress.files_done)),
        DupStage::QuickCheck => of("Quick check,"),
        DupStage::Hashing => of("Hashing"),
        DupStage::Comparing => of("Comparing"),
        DupStage::Rechecking => of("Checking again,"),
    }
}

fn files(n: u64) -> String {
    match n {
        0 | 1 => count(n, "file", "files"),
        _ => format!("{} files", grouped(n)),
    }
}

/// The groups a search found and the copies marked in each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupView {
    /// The scope and options it ran with.
    pub setup: DupSetup,
    /// The groups.
    pub result: DupResult,
    /// Each group's copies, marked or not, in the group's order.
    pub marks: Vec<Vec<bool>>,
    /// Copies left out after the search as hidden.
    pub hidden_left_out: u64,
}

impl DupView {
    /// The groups, nothing marked.
    pub fn new(setup: DupSetup, result: DupResult, hidden_left_out: u64) -> Self {
        let marks = result
            .groups
            .iter()
            .map(|g| vec![false; g.files.len()])
            .collect();
        Self {
            setup,
            result,
            marks,
            hidden_left_out,
        }
    }

    /// The line said as the groups open: the count line first, then what
    /// was left out: "1 group, 2 files, 5 bytes to free. 1 empty file left
    /// out."
    pub fn start_line(&self) -> String {
        // The engine's notes say the hidden files left out (lane AS).
        let mut parts = vec![self.result.count_line()];
        parts.extend(self.result.notes());
        format!("{}.", parts.join(". "))
    }

    /// The groups list's title: "Duplicates, 3 groups".
    pub fn title(&self) -> String {
        format!(
            "Duplicates, {}",
            count(self.result.groups.len() as u64, "group", "groups")
        )
    }

    /// A group's row, the number first, and its marks: "Group 1 of 3, 3
    /// copies, 2.1 MB each, 4.2 MB to free", "2 marked".
    pub fn group_row(&self, group: usize) -> Option<(String, String)> {
        let found = self.result.groups.get(group)?;
        let row = found.row(group + 1, self.result.groups.len());
        let marked = self.marked_in(group);
        let marks = if marked == 0 {
            "none marked".to_owned()
        } else {
            format!("{marked} marked")
        };
        Some((row, marks))
    }

    /// How many copies of a group are marked.
    pub fn marked_in(&self, group: usize) -> usize {
        self.marks
            .get(group)
            .map(|m| m.iter().filter(|on| **on).count())
            .unwrap_or_default()
    }

    /// A copy's row, its mark first and its name next: "Kept: a.txt, in
    /// D:\x", "Marked: b.txt, in D:\y".
    pub fn member_row(&self, group: usize, file: usize) -> Option<String> {
        let member = self.member(group, file)?;
        let word = if self.is_marked(group, file) {
            "Marked"
        } else {
            "Kept"
        };
        Some(format!("{word}: {}", member.row()))
    }

    /// The members list's title: "Group 1 of 3, 3 copies".
    pub fn members_title(&self, group: usize) -> String {
        let files = self
            .result
            .groups
            .get(group)
            .map(|g| g.files.len())
            .unwrap_or_default();
        format!(
            "Group {} of {}, {}",
            group + 1,
            self.result.groups.len(),
            count(files as u64, "copy", "copies")
        )
    }

    /// One copy of a group.
    pub fn member(&self, group: usize, file: usize) -> Option<&DupFile> {
        self.result.groups.get(group)?.files.get(file)
    }

    /// Whether a copy is marked.
    pub fn is_marked(&self, group: usize, file: usize) -> bool {
        self.marks
            .get(group)
            .and_then(|m| m.get(file))
            .copied()
            .unwrap_or_default()
    }

    /// Changes a copy's mark; the line to say: "Marked b.txt", "Kept
    /// b.txt". Marking the last unmarked copy is refused with
    /// [`ONE_COPY_MUST_STAY`].
    pub fn toggle(&mut self, group: usize, file: usize) -> Result<String, &'static str> {
        let name = self
            .member(group, file)
            .and_then(|m| m.place.name().map(|n| display_name(n).into_owned()))
            .ok_or(ONE_COPY_MUST_STAY)?;
        let marks = self.marks.get_mut(group).ok_or(ONE_COPY_MUST_STAY)?;
        let was = marks.get(file).copied().unwrap_or_default();
        if !was && marks.iter().filter(|on| !**on).count() <= 1 {
            return Err(ONE_COPY_MUST_STAY);
        }
        if let Some(mark) = marks.get_mut(file) {
            *mark = !was;
        }
        Ok(if was {
            format!("Kept {name}")
        } else {
            format!("Marked {name}")
        })
    }

    /// Marks every copy but the ones `rule` keeps, in every group, as the
    /// engine's helper chooses; a group it leaves alone keeps its marks.
    /// Returns the helper's choice, for its words.
    pub fn apply(&mut self, rule: &KeepRule) -> Selection {
        let selection = select(&self.result, rule);
        for choice in &selection.choices {
            if choice.mark.is_empty() {
                continue;
            }
            if let Some(marks) = self.marks.get_mut(choice.group) {
                marks.iter_mut().for_each(|m| *m = false);
                for file in &choice.mark {
                    if let Some(mark) = marks.get_mut(*file) {
                        *mark = true;
                    }
                }
            }
        }
        selection
    }

    /// The marks as a selection for the engine's check: in each group, the
    /// unmarked copies kept and the marked ones marked.
    pub fn selection(&self) -> Selection {
        let mut choices = Vec::new();
        let mut marked = Vec::new();
        let mut bytes = 0u64;
        for (at, group) in self.result.groups.iter().enumerate() {
            let (mut keep, mut mark) = (Vec::new(), Vec::new());
            for (file, member) in group.files.iter().enumerate() {
                if self.is_marked(at, file) {
                    mark.push(file);
                    marked.push(member.place.clone());
                    if member.same_file_as.is_none() {
                        bytes = bytes.saturating_add(group.size);
                    }
                } else {
                    keep.push(file);
                }
            }
            choices.push(GroupChoice {
                group: at,
                keep,
                mark,
                left_alone: None,
            });
        }
        Selection {
            rule: KeepRule::default(),
            choices,
            marked,
            bytes,
        }
    }

    /// The marked copies and the bytes they free.
    pub fn marked_totals(&self) -> (usize, u64) {
        let selection = self.selection();
        (selection.marked.len(), selection.bytes)
    }

    /// The trash row's words: "Trash the marked: 4 files, 44 KB".
    pub fn trash_row(&self) -> String {
        let (files, bytes) = self.marked_totals();
        if files == 0 {
            return "Trash the marked: none marked yet".to_owned();
        }
        format!(
            "Trash the marked: {}, {}",
            count(files as u64, "file", "files"),
            human_size(bytes)
        )
    }

    /// What the search saw of a copy, for the plan's check before each
    /// step: its kind, size and time as hashed.
    pub fn seen(&self, place: &Place) -> Option<Entry> {
        let member = self
            .result
            .groups
            .iter()
            .flat_map(|g| g.files.iter())
            .find(|f| &f.place == place)?;
        let name = member.place.name().unwrap_or_default().to_owned();
        let mut entry = Entry::new(name, EntryKind::File);
        entry.size = Some(member.size);
        entry.modified = member.modified;
        Some(entry)
    }

    /// The bytes the given copies free.
    pub fn bytes_of(&self, places: &[Place]) -> u64 {
        self.result
            .groups
            .iter()
            .flat_map(|g| g.files.iter().map(move |f| (g.size, f)))
            .filter(|(_, f)| f.same_file_as.is_none() && places.contains(&f.place))
            .map(|(size, _)| size)
            .sum()
    }
}

/// The trash question, the count first: "Trash 4 copies, 44 KB? y or n".
pub fn trash_question(files: usize, bytes: u64) -> String {
    format!(
        "Trash {}, {}? y or n",
        count(files as u64, "copy", "copies"),
        human_size(bytes)
    )
}

/// "3 hidden files left out".
pub fn hidden_line(n: u64) -> String {
    if n == 1 {
        return "1 hidden file left out".to_owned();
    }
    format!("{} hidden files left out", grouped(n))
}

/// Drops from `result` the copies `hidden` says are hidden, and every
/// group left with fewer than two; the number dropped.
pub fn leave_out(result: &mut DupResult, hidden: impl Fn(&Place) -> bool) -> u64 {
    let mut dropped = 0;
    for group in &mut result.groups {
        let keep: Vec<bool> = group.files.iter().map(|f| !hidden(&f.place)).collect();
        if keep.iter().all(|k| *k) {
            continue;
        }
        let mut new_index = Vec::with_capacity(keep.len());
        let mut next = 0usize;
        for k in &keep {
            new_index.push(k.then_some(next));
            if *k {
                next += 1;
            }
        }
        let mut files = Vec::new();
        for (at, mut file) in std::mem::take(&mut group.files).into_iter().enumerate() {
            if !keep[at] {
                dropped += 1;
                continue;
            }
            file.same_file_as = file.same_file_as.and_then(|of| new_index[of]);
            files.push(file);
        }
        group.files = files;
    }
    result.groups.retain(|g| g.distinct() >= 2);
    result.groups.sort_by_key(|g| std::cmp::Reverse(g.waste()));
    dropped
}

#[cfg(test)]
mod tests {
    use diroxide_core::Timestamp;
    use diroxide_ops::dups::DupGroup;

    use super::*;

    /// A place written the Unix way, on Windows under `C:`; no test reads
    /// it.
    fn place(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("C:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    fn file(path: &str, found: usize, day: u8) -> DupFile {
        DupFile {
            place: place(path),
            size: 2_000,
            modified: Timestamp::from_civil(2026, 11, day, 9, 0, 0, diroxide_core::UtcOffset::UTC),
            found,
            same_file_as: None,
        }
    }

    pub(crate) fn result() -> DupResult {
        DupResult {
            groups: vec![DupGroup {
                size: 2_000,
                files: vec![
                    file("/a/new.txt", 0, 10),
                    file("/b/old.txt", 1, 2),
                    file("/c/mid.txt", 2, 5),
                ],
                digest: vec![1, 2, 3],
                bytes_compared: false,
            }],
            files_seen: 5,
            left_out: Vec::new(),
            empty_left_out: 1,
            small_left_out: 0,
            hidden_left_out: 0,
            hard_links_left_out: 0,
            unreadable: 0,
            reached_twice: 0,
            no_quick: 0,
            hasher: "BLAKE3",
        }
    }

    fn setup() -> DupSetup {
        DupSetup {
            side: Side::Left,
            scope: DupScope::ThisFolder,
            roots: vec![place("/a")],
            least: 0,
            byte_compare: false,
            hidden: false,
            ask_above: None,
            interval_ms: 500,
        }
    }

    fn cells(text: &str) -> usize {
        text.chars().count()
    }

    #[test]
    fn the_words_fit_a_line_of_the_display() {
        for (line, n) in [
            (ONE_COPY_MUST_STAY, 19),
            (NO_GROUPS_OPEN, 36),
            (NOTHING_MARKED, 38),
            (NOTHING_TRASHED, 27),
            (MARKS_UNCHANGED, 31),
            (NOTHING_SEARCHED, 32),
            (SCOPE_TITLE, 21),
            (OPTIONS_TITLE, 23),
            (RULES_TITLE, 21),
        ] {
            assert_eq!(cells(line), n, "{line}");
        }
        let mut hashing = DupProgress::start(DupStage::Hashing, 450, 0);
        hashing.files_done = 120;
        assert_eq!(progress_line(&hashing), "Hashing 120 of 450 files");
        assert_eq!(cells(&progress_line(&hashing)), 24);
        let mut listing = DupProgress::start(DupStage::Listing, 0, 0);
        listing.files_done = 1200;
        assert_eq!(progress_line(&listing), "Listing, 1,200 files found");
        let mut checking = DupProgress::start(DupStage::Rechecking, 6, 0);
        checking.files_done = 3;
        assert_eq!(progress_line(&checking), "Checking again, 3 of 6 files");
        assert_eq!(trash_question(4, 44_000), "Trash 4 copies, 44.0 KB? y or n");
        assert_eq!(hidden_line(1), "1 hidden file left out");
        assert_eq!(hidden_line(3), "3 hidden files left out");
        let rows = setup().option_rows();
        assert_eq!(rows[0].1, format!("Find now: {}", place("/a")));
        assert_eq!(rows[1].1, "Least size: any size");
        assert_eq!(rows[2].1, "Byte check: off, hashes only");
        assert_eq!(rows[3].1, "Hidden files: left out");
        for (_, row) in &rows[1..] {
            assert!(cells(row) <= 40, "{row}");
        }
    }

    #[test]
    fn the_groups_open_with_nothing_marked_and_their_count_line_first() {
        let found = DupResult {
            hidden_left_out: 2,
            ..result()
        };
        let view = DupView::new(setup(), found, 2);
        assert_eq!(
            view.start_line(),
            "1 group, 3 files, 4.0 KB to free. 1 empty file left out. 2 hidden files left out."
        );
        assert_eq!(view.title(), "Duplicates, 1 group");
        let (row, marks) = view.group_row(0).unwrap();
        assert_eq!(row, "Group 1 of 1, 3 copies, 2.0 KB each, 4.0 KB to free");
        assert_eq!(marks, "none marked");
        assert_eq!(view.trash_row(), "Trash the marked: none marked yet");
        assert_eq!(view.members_title(0), "Group 1 of 1, 3 copies");
        assert!(view.member_row(0, 0).unwrap().starts_with("Kept: new.txt"));
    }

    #[test]
    fn a_rule_marks_all_but_its_keeper_and_the_last_copy_always_stays() {
        let mut view = DupView::new(setup(), result(), 0);
        let chosen = view.apply(&KeepRule::Oldest);
        assert_eq!(
            chosen.summary(),
            "Marked 2 files, 4.0 KB. Kept the oldest in 1 group."
        );
        assert!(!view.is_marked(0, 1), "the oldest stays");
        assert!(view.is_marked(0, 0) && view.is_marked(0, 2));
        assert_eq!(view.group_row(0).unwrap().1, "2 marked");
        assert_eq!(view.trash_row(), "Trash the marked: 2 files, 4.0 KB");
        assert!(
            view.member_row(0, 0)
                .unwrap()
                .starts_with("Marked: new.txt")
        );
        // Marking the keeper too is refused: one copy must stay.
        assert_eq!(view.toggle(0, 1), Err(ONE_COPY_MUST_STAY));
        assert_eq!(view.toggle(0, 0).as_deref(), Ok("Kept new.txt"));
        assert_eq!(view.toggle(0, 1).as_deref(), Ok("Marked old.txt"));
        assert_eq!(view.toggle(0, 0), Err(ONE_COPY_MUST_STAY));
        let selection = view.selection();
        assert_eq!(selection.choices[0].keep, [0]);
        assert_eq!(selection.choices[0].mark, [1, 2]);
        assert_eq!(selection.bytes, 4_000);
        let seen = view.seen(&place("/b/old.txt")).unwrap();
        assert_eq!(seen.size, Some(2_000));
        assert_eq!(seen.kind, EntryKind::File);
        assert_eq!(view.bytes_of(&selection.marked), 4_000);
    }

    #[test]
    fn hidden_copies_are_left_out_and_a_lone_copy_is_no_group() {
        let mut found = result();
        let dropped = leave_out(&mut found, |p| p.is_within(&place("/c")));
        assert_eq!(dropped, 1);
        assert_eq!(found.groups[0].files.len(), 2);
        let dropped = leave_out(&mut found, |p| p.is_within(&place("/b")));
        assert_eq!(dropped, 1);
        assert!(found.groups.is_empty(), "one copy left is no group");
    }
}
