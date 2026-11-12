//! Compare two folders: one row per relative path, its status in R18's
//! words (`research/diffing.md`, "The status vocabulary").
//!
//! The library's sides are neutral: [`Side::Left`] is the place the
//! caller passes first, and every word says it as "here" (this panel),
//! [`Side::Right`] as "there" (the other panel). The app passes the active
//! panel first. Rows never say left or right (R2).
//!
//! The walk goes down both trees in lockstep, folder by folder, merging
//! the names by the pair's case rule, through each side's own provider,
//! so any two places compare: two local folders, a local folder and an
//! archive, a folder and an SFTP place. It keeps the boundary rule of the
//! engine's walk: a link is a row of its own, compared by its target text
//! and never followed; a folder on another drive than its root's, or a
//! folder already walked, is left out and said; an unfinished copy is left
//! alone. A folder that cannot be listed is an Unreadable row and never
//! stops the compare. A cancel keeps what was compared and marks the rest
//! "Not compared", so a partial result never reads as a whole one.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::names::spell_out;
use diroxide_core::{Entry, EntryKind, Place, PlaceKind, Side, Timestamp};
use diroxide_vfs::equal::{EqualStop, Stream, equal_with};
use diroxide_vfs::names::is_unfinished;
use diroxide_vfs::{OpContext, Provider, Registry, VfsError};

use crate::engine::JobControl;
use crate::error::OpsError;

/// The deepest the compare goes by default, the job engine's own limit.
pub const DEFAULT_MAX_DEPTH: usize = 256;

/// Two modified times this close count as the same: two seconds always
/// (R18), since FAT and ZIP keep two-second times and no capability says
/// a place's time resolution.
pub const TOLERANCE: Duration = Duration::from_secs(2);

/// Where an entry is, as a row says it: "here" for the left side (this
/// panel), "there" for the right (the other panel).
pub const fn at(side: Side) -> &'static str {
    match side {
        Side::Left => "here",
        Side::Right => "there",
    }
}

/// The side in a sentence: "this panel", "the other panel".
pub const fn panel(side: Side) -> &'static str {
    match side {
        Side::Left => "this panel",
        Side::Right => "the other panel",
    }
}

/// What decides whether two files are the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CompareBy {
    /// Size and modified time, within [`CompareCriteria::tolerance`]: the
    /// default, the quick check.
    #[default]
    SizeAndTime,
    /// Size only; times are not looked at (a server whose clock is wrong).
    Size,
    /// Size and time first; files of the same size are then read on both
    /// sides, block by block, stopping at the first difference, so "same"
    /// means the same contents. Over SFTP this reads both files in full.
    Content,
}

impl CompareBy {
    /// How the rows were verified, as a line said with the result:
    /// "Compared by size and time; times within 2 seconds count as the
    /// same."
    pub fn line(self, tolerance: Duration) -> String {
        match self {
            CompareBy::SizeAndTime => format!(
                "Compared by size and time; times within {} count as the same.",
                seconds(tolerance)
            ),
            CompareBy::Size => "Compared by size only; times not looked at.".to_owned(),
            CompareBy::Content => "Compared by contents where the sizes match.".to_owned(),
        }
    }
}

/// "2 seconds", "1.5 seconds".
fn seconds(window: Duration) -> String {
    if window.subsec_nanos() == 0 {
        count(window.as_secs(), "second", "seconds")
    } else {
        format!("{:.1} seconds", window.as_secs_f64())
    }
}

/// A filter from the app's one filter language, given to the compare: an
/// entry it does not keep is a Skipped row, on both sides, so a hidden
/// rule never looks like a missing file.
pub trait EntryFilter: Send + Sync + fmt::Debug {
    /// Whether the entry at `names` (from the roots) takes part.
    fn keep(&self, names: &[String], entry: &Entry) -> bool;

    /// Why a row was skipped, in words: "matches the filter *.tmp".
    fn reason(&self) -> String;
}

/// What a compare looks at.
#[derive(Clone, Debug)]
pub struct CompareCriteria {
    /// What decides "the same".
    pub by: CompareBy,
    /// Whether folders on both sides are compared inside too (on by
    /// default). Off, folders on both sides are not compared inside; a
    /// folder on one side only is still listed whole, so a copy or a
    /// removal of it says everything it holds.
    pub subfolders: bool,
    /// Whether hidden entries take part (on by default). Off, a name
    /// hidden on either side is a Skipped row.
    pub hidden: bool,
    /// Whether two times one hour apart, within the tolerance, count as
    /// the same: a FAT, exFAT, FTP or archive place's times move by an
    /// hour across a daylight saving change. Off by default, a choice per
    /// pair; said in the rows it decided.
    pub ignore_one_hour: bool,
    /// Two times this close count as the same: [`TOLERANCE`].
    pub tolerance: Duration,
    /// The filter, when one is set.
    pub filter: Option<Arc<dyn EntryFilter>>,
    /// The deepest the compare goes below the two roots.
    pub max_depth: usize,
}

impl Default for CompareCriteria {
    fn default() -> Self {
        Self {
            by: CompareBy::default(),
            subfolders: true,
            hidden: true,
            ignore_one_hour: false,
            tolerance: TOLERANCE,
            filter: None,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

/// What a compare learned about one side's place before the walk: what
/// the synchronize plan needs to know about it without asking again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideFacts {
    /// The provider's scheme: `file`, `archive`, `sftp`, a plugin's.
    pub scheme: String,
    /// Whether the place is inside an archive.
    pub archive: bool,
    /// Whether the place has a trash: a removal or an overwrite there goes
    /// to it first, and can be undone.
    pub trash: bool,
    /// Whether the place can be written to.
    pub writable: bool,
    /// Whether names that differ only by case are different entries.
    pub case_sensitive: bool,
}

impl SideFacts {
    /// Reads the facts of a place through its provider.
    pub fn read(provider: &dyn Provider, place: &Place) -> Self {
        let capabilities = provider.capabilities(place);
        let scheme = match place.remote() {
            Some(remote) => remote.scheme().to_owned(),
            None => provider.scheme().to_owned(),
        };
        Self {
            scheme,
            archive: place.kind() == PlaceKind::Archive,
            trash: capabilities.trash,
            writable: capabilities.writable,
            case_sensitive: capabilities.case_sensitive,
        }
    }
}

/// The status of one relative path across the two sides (R18).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowState {
    /// The same by the criteria; for two folders, both there (what is
    /// inside has rows of its own). "Same, times differ" when the
    /// contents were proven equal and the times are not.
    Same,
    /// On both sides and not the same; the reason is in the row's facts
    /// ("newer here", "same size, content differs").
    Different,
    /// Only here (on the left).
    OnlyLeft,
    /// Only there (on the right).
    OnlyRight,
    /// A file on one side, a folder or a link on the other; or one of two
    /// names that differ only by case on one side. Never resolved.
    TypeDiffers,
    /// A side could not be read or listed.
    Unreadable,
    /// Left out by the filter or the hidden rule.
    Skipped,
    /// Not compared, because the compare was cancelled first.
    NotCompared,
}

impl RowState {
    /// Only on this side.
    pub const fn only(side: Side) -> Self {
        match side {
            Side::Left => RowState::OnlyLeft,
            Side::Right => RowState::OnlyRight,
        }
    }

    /// Whether the row is a problem, said first and never acted on.
    pub const fn is_problem(self) -> bool {
        matches!(
            self,
            RowState::TypeDiffers | RowState::Unreadable | RowState::NotCompared
        )
    }

    /// The status as a row's first words: "Same", "Different", "Only
    /// here", "Only there", "Type differs", "Unreadable", "Skipped", "Not
    /// compared".
    pub const fn words(self) -> &'static str {
        match self {
            RowState::Same => "Same",
            RowState::Different => "Different",
            RowState::OnlyLeft => "Only here",
            RowState::OnlyRight => "Only there",
            RowState::TypeDiffers => "Type differs",
            RowState::Unreadable => "Unreadable",
            RowState::Skipped => "Skipped",
            RowState::NotCompared => "Not compared",
        }
    }
}

/// One fact that says how two entries differ, or why a row has no answer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowFact {
    /// This side's file is newer.
    Newer(Side),
    /// This side's file is larger.
    Larger(Side),
    /// The times count as the same although the sizes differ.
    SameTime,
    /// The sizes match and the contents differ.
    ContentDiffers,
    /// The contents are the same and the times differ.
    TimesDiffer,
    /// The times are not equal and within the tolerance: said, since the
    /// tolerance decided the row.
    TimesWithin(Duration),
    /// The times are an hour apart, which the pair's setting forgives.
    HourApart,
    /// A side has no modified time to go by.
    NoTime,
    /// The two links point to different targets.
    PointsElsewhere,
    /// What each side is, for a type that differs: "file here, folder
    /// there".
    Kinds(EntryKind, EntryKind),
    /// This side has two names that differ only by case, which the pair
    /// cannot tell apart.
    CaseTwins(Side),
    /// The names match only by case: "name differs in case there".
    NameCase,
    /// Why a side could not be read, in words; `None` for both sides.
    Unreadable(Option<Side>, String),
    /// Why the row was skipped: "hidden", the filter's reason.
    Filtered(String),
}

impl RowFact {
    /// The fact in words: "newer here", "larger there", "same size,
    /// content differs", "file here, folder there".
    pub fn words(&self) -> String {
        match self {
            RowFact::Newer(side) => format!("newer {}", at(*side)),
            RowFact::Larger(side) => format!("larger {}", at(*side)),
            RowFact::SameTime => "same time".to_owned(),
            RowFact::ContentDiffers => "same size, content differs".to_owned(),
            RowFact::TimesDiffer => "times differ".to_owned(),
            RowFact::TimesWithin(window) => format!("times within {}", seconds(*window)),
            RowFact::HourApart => "times 1 hour apart".to_owned(),
            RowFact::NoTime => "no time to go by".to_owned(),
            RowFact::PointsElsewhere => "the links point elsewhere".to_owned(),
            RowFact::Kinds(left, right) => format!(
                "{} {}, {} {}",
                left.word(),
                at(Side::Left),
                right.word(),
                at(Side::Right)
            ),
            RowFact::CaseTwins(side) => format!("two names differ only by case {}", at(*side)),
            RowFact::NameCase => "name differs in case".to_owned(),
            RowFact::Unreadable(_, why) | RowFact::Filtered(why) => why.clone(),
        }
    }
}

/// One relative path in a compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareRow {
    /// The names from the roots down to the entry.
    pub names: Vec<String>,
    /// The entry here (on the left), when there is one.
    pub left: Option<Entry>,
    /// The entry there (on the right), when there is one.
    pub right: Option<Entry>,
    /// The status across the two sides.
    pub state: RowState,
    /// How the two differ, or why there is no answer.
    pub facts: Vec<RowFact>,
}

impl CompareRow {
    /// The entry on one side.
    pub fn entry(&self, side: Side) -> Option<&Entry> {
        match side {
            Side::Left => self.left.as_ref(),
            Side::Right => self.right.as_ref(),
        }
    }

    /// What the row is: the left entry's kind, else the right's.
    pub fn kind(&self) -> EntryKind {
        self.left
            .as_ref()
            .or(self.right.as_ref())
            .map_or(EntryKind::File, |entry| entry.kind)
    }

    /// Whether the row is a folder on every side it is on.
    pub fn is_folder(&self) -> bool {
        [&self.left, &self.right]
            .iter()
            .filter_map(|entry| entry.as_ref())
            .all(|entry| entry.kind == EntryKind::Folder)
    }

    /// The side whose file is newer, when one is.
    pub fn newer(&self) -> Option<Side> {
        self.facts.iter().find_map(|fact| match fact {
            RowFact::Newer(side) => Some(*side),
            _ => None,
        })
    }

    /// The side that could not be read, for an Unreadable row: `None` for
    /// both or not known.
    pub fn unreadable_side(&self) -> Option<Side> {
        self.facts.iter().find_map(|fact| match fact {
            RowFact::Unreadable(side, _) => *side,
            _ => None,
        })
    }

    /// The entry's name, its hidden characters spelled out.
    pub fn name(&self) -> String {
        spell_out(self.names.last().map_or("", String::as_str)).into_owned()
    }

    /// The folder it is in, relative to the roots, with `separator`
    /// between names; empty at the top.
    pub fn folder(&self, separator: char) -> String {
        let folder = &self.names[..self.names.len().saturating_sub(1)];
        spell_out(&folder.join(&separator.to_string())).into_owned()
    }

    /// The name, then the folder: "report.pdf, in photos\2024"; the folder
    /// comes after the name, as in the flat view.
    pub fn where_words(&self, separator: char) -> String {
        let folder = self.folder(separator);
        if folder.is_empty() {
            self.name()
        } else {
            format!("{}, in {folder}", self.name())
        }
    }

    /// Whether this row is inside the row whose names are `names`.
    pub fn is_under(&self, names: &[String]) -> bool {
        self.names.len() > names.len() && self.names.starts_with(names)
    }

    /// The facts in words, joined: "newer here, larger here".
    pub fn reasons(&self) -> Vec<String> {
        self.facts.iter().map(RowFact::words).collect()
    }

    /// The row in words, status first, then the name, the folder and the
    /// reason (R18): "Different, intro.md, in docs, newer here"; "Type
    /// differs, photos, folder here, file there"; "Unreadable there,
    /// locked, permission denied"; "Same, b.txt, times differ".
    pub fn line(&self, separator: char) -> String {
        let status = match (self.state, self.unreadable_side()) {
            (RowState::Unreadable, Some(side)) => format!("Unreadable {}", at(side)),
            (state, _) => state.words().to_owned(),
        };
        let mut line = format!("{status}, {}", self.where_words(separator));
        for reason in self.reasons() {
            line.push_str(", ");
            line.push_str(&reason);
        }
        line
    }
}

/// How many rows have each status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompareCounts {
    /// Every row.
    pub all: u64,
    /// The same.
    pub same: u64,
    /// Different.
    pub different: u64,
    /// Only here.
    pub only_left: u64,
    /// Only there.
    pub only_right: u64,
    /// Type differs.
    pub type_differs: u64,
    /// Unreadable.
    pub unreadable: u64,
    /// Skipped.
    pub skipped: u64,
    /// Not compared.
    pub not_compared: u64,
}

impl CompareCounts {
    /// The rows that are not the same.
    pub const fn differences(&self) -> u64 {
        self.all - self.same
    }

    /// Every category of the count line that is not the same, in its
    /// order: problems first. The counts add up to
    /// [`CompareCounts::differences`].
    pub fn parts(&self) -> Vec<(u64, &'static str)> {
        [
            (self.unreadable, "unreadable"),
            (self.type_differs, "type differs"),
            (self.not_compared, "not compared"),
            (self.different, "different"),
            (self.only_left, "only here"),
            (self.only_right, "only there"),
            (self.skipped, "skipped"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .collect()
    }
}

/// What a compare found: one row per relative path, in the order a walk
/// of the two trees gives (each folder before what is inside it).
#[derive(Clone, Debug)]
pub struct Comparison {
    /// The place here (the left root).
    pub left: Place,
    /// The place there (the right root).
    pub right: Place,
    /// What was looked at.
    pub criteria: CompareCriteria,
    /// The facts here.
    pub left_facts: SideFacts,
    /// The facts there.
    pub right_facts: SideFacts,
    /// The rows.
    pub rows: Vec<CompareRow>,
    /// What was not compared, with the reason in words: another drive
    /// mounted inside, a loop, an unfinished copy, a name that is not
    /// valid Unicode.
    pub left_out: Vec<(Place, String)>,
    /// Whether the compare was cancelled before its end: the rows not
    /// reached are "Not compared", and the count line says so first.
    pub cancelled: bool,
}

impl Comparison {
    /// The facts of one side.
    pub fn facts(&self, side: Side) -> &SideFacts {
        match side {
            Side::Left => &self.left_facts,
            Side::Right => &self.right_facts,
        }
    }

    /// The root of one side.
    pub fn root(&self, side: Side) -> &Place {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    /// The separator rows put between names: the left root's own.
    pub fn separator(&self) -> char {
        separator_of(&self.left)
    }

    /// How many rows have each status.
    pub fn counts(&self) -> CompareCounts {
        let mut counts = CompareCounts::default();
        for row in &self.rows {
            counts.all += 1;
            match row.state {
                RowState::Same => counts.same += 1,
                RowState::Different => counts.different += 1,
                RowState::OnlyLeft => counts.only_left += 1,
                RowState::OnlyRight => counts.only_right += 1,
                RowState::TypeDiffers => counts.type_differs += 1,
                RowState::Unreadable => counts.unreadable += 1,
                RowState::Skipped => counts.skipped += 1,
                RowState::NotCompared => counts.not_compared += 1,
            }
        }
        counts
    }

    /// The count line, said first (R18): the outcome, every category
    /// that is not zero, the same count last, adding up to the total:
    /// "17 differences in 1,202 items: 12 only here, 3 only there, 2
    /// different. 1,185 same." A cancelled compare starts "Cancelled, ".
    pub fn count_line(&self) -> String {
        let counts = self.counts();
        let items = if counts.all == 1 { "item" } else { "items" };
        let all = grouped(counts.all);
        let head = match counts.differences() {
            _ if counts.all == 0 && self.cancelled => {
                return "Cancelled before anything was compared.".to_owned();
            }
            _ if counts.all == 0 => "Both folders are empty.".to_owned(),
            0 => format!("No differences in {all} {items}."),
            1 => format!("1 difference in {all} {items}"),
            n => format!("{} differences in {all} {items}", grouped(n)),
        };
        let mut line = if self.cancelled {
            format!("Cancelled, {}{}", head[..1].to_lowercase(), &head[1..])
        } else {
            head
        };
        let parts = counts.parts();
        if !parts.is_empty() {
            let parts: Vec<String> = parts
                .iter()
                .map(|(n, what)| format!("{} {what}", grouped(*n)))
                .collect();
            line.push_str(": ");
            line.push_str(&parts.join(", "));
            line.push('.');
        }
        if counts.same > 0 && counts.same < counts.all {
            line.push_str(&format!(" {} same.", grouped(counts.same)));
        }
        line
    }

    /// The lines said after the count line: how the rows were verified,
    /// and the hour allowance when on.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = vec![self.criteria.by.line(self.criteria.tolerance)];
        if self.criteria.ignore_one_hour {
            notes.push("Times 1 hour apart count as the same: daylight saving.".to_owned());
        }
        notes
    }

    /// The whole result as plain lines, for a script or a Braille display:
    /// the count line, its notes, then the problems, the differences, the
    /// skipped rows, what was left out, and the same rows only when `same`
    /// is set.
    pub fn lines(&self, same: bool) -> Vec<String> {
        let separator = self.separator();
        let mut lines = vec![self.count_line()];
        lines.extend(self.notes());
        let rows = |keep: &dyn Fn(&CompareRow) -> bool| -> Vec<String> {
            self.rows
                .iter()
                .filter(|row| keep(row))
                .map(|row| row.line(separator))
                .collect()
        };
        lines.extend(rows(&|row| row.state.is_problem()));
        lines.extend(rows(&|row| {
            !row.state.is_problem() && !matches!(row.state, RowState::Same | RowState::Skipped)
        }));
        lines.extend(rows(&|row| row.state == RowState::Skipped));
        lines.extend(
            self.left_out.iter().map(|(place, why)| {
                format!("Not compared: {}, {why}", spell_out(&place.to_string()))
            }),
        );
        if same {
            lines.extend(rows(&|row| row.state == RowState::Same));
        }
        lines
    }

    /// The rows that differ on a side, for "select the differences"
    /// between the two panels: each row on `side` that is not the same
    /// and not skipped.
    pub fn differing(&self, side: Side) -> Vec<&CompareRow> {
        self.rows
            .iter()
            .filter(|row| {
                !matches!(row.state, RowState::Same | RowState::Skipped)
                    && row.entry(side).is_some()
            })
            .collect()
    }

    /// The place of a row on one side.
    pub fn place_of(&self, row: &CompareRow, side: Side) -> Result<Place, OpsError> {
        place_at(self.root(side), &row.names)
    }
}

/// The separator under a place: `\` in a Windows path, `/` elsewhere.
pub(crate) fn separator_of(place: &Place) -> char {
    match place.local_path() {
        Some(path) if !path.starts_with('/') => '\\',
        _ => '/',
    }
}

/// The place `names` below `root`.
pub(crate) fn place_at(root: &Place, names: &[String]) -> Result<Place, OpsError> {
    let mut place = root.clone();
    for name in names {
        place = place.join(name).map_err(VfsError::from)?;
    }
    Ok(place)
}

/// Where a running compare is, said at the app's progress interval.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareProgress {
    /// Rows found so far.
    pub entries: u64,
    /// Folders listed so far, on both sides.
    pub folders: u64,
    /// Bytes read for a content compare, on both sides.
    pub bytes_read: u64,
    /// The folder being compared, relative to the roots; empty at the top.
    pub folder: String,
    /// Whether the compare is waiting because it was paused.
    pub paused: bool,
    /// Whether the compare is over.
    pub done: bool,
}

impl CompareProgress {
    /// The progress in words, meaning first: "Comparing: 1,200 entries, in
    /// photos"; "Comparing contents: 1,200 entries, 3.1 MB read";
    /// "Compare paused: 1,200 entries".
    pub fn describe(&self) -> String {
        let entries = if self.entries == 1 {
            "1 entry".to_owned()
        } else {
            format!("{} entries", grouped(self.entries))
        };
        let mut line = if self.paused {
            format!("Compare paused: {entries}")
        } else if self.done {
            format!("Compare finished: {entries}")
        } else if self.bytes_read > 0 {
            format!(
                "Comparing contents: {entries}, {} read",
                human_size(self.bytes_read)
            )
        } else {
            format!("Comparing: {entries}")
        };
        if !self.done && !self.folder.is_empty() {
            line.push_str(", in ");
            line.push_str(&self.folder);
        }
        line
    }
}

/// Takes a running compare's progress. Called on the compare's thread;
/// the app says a change of state at once and the rest at its interval.
pub trait CompareSink: Send + Sync {
    /// Takes one update.
    fn progress(&self, progress: &CompareProgress);
}

impl<F> CompareSink for F
where
    F: Fn(&CompareProgress) + Send + Sync,
{
    fn progress(&self, progress: &CompareProgress) {
        self(progress);
    }
}

/// How often the compare tells its sink at most, besides its start and
/// its end.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// The size of each block in a content compare, on each side.
const CHUNK: usize = 64 * 1024;

/// Compares two folders, on any two providers the registry serves: the
/// place here (this panel) first, the place there second.
///
/// Cancellable through `control` (Esc on a running compare), between
/// folders and between the blocks of a content read: a cancelled compare
/// returns what it compared, the rest "Not compared", and
/// [`Comparison::cancelled`] set; cancelled before the roots are read, it
/// returns [`VfsError::Cancelled`]. Paused, it waits. Refused when a root is
/// not a folder, cannot be listed, or when one root is the other or inside
/// it.
pub fn compare_dirs(
    registry: &Registry,
    left: &Place,
    right: &Place,
    criteria: &CompareCriteria,
    control: &JobControl,
    sink: &dyn CompareSink,
) -> Result<Comparison, OpsError> {
    if left.is_within(right) || right.is_within(left) {
        return Err(OpsError::Refused(format!(
            "Cannot compare: one folder is inside the other, {} and {}",
            spell_out(&left.to_string()),
            spell_out(&right.to_string())
        )));
    }
    let ctx = OpContext::new().with_cancel(control.cancel.clone());
    let left_provider = registry.resolve(left)?;
    let right_provider = registry.resolve(right)?;
    let left_root = root_entry(&*left_provider, left, &ctx)?;
    let right_root = root_entry(&*right_provider, right, &ctx)?;
    let left_facts = SideFacts::read(&*left_provider, left);
    let right_facts = SideFacts::read(&*right_provider, right);
    let mut walk = PairWalk {
        sides: [
            WalkSide::new(&*left_provider, left, &left_root),
            WalkSide::new(&*right_provider, right, &right_root),
        ],
        criteria,
        fold_case: !(left_facts.case_sensitive && right_facts.case_sensitive),
        ctx: &ctx,
        control,
        sink,
        rows: Vec::new(),
        left_out: Vec::new(),
        progress: CompareProgress::default(),
        told: None,
        separator: separator_of(left),
        queue: Vec::new(),
    };
    let cancelled = match walk.run() {
        Ok(()) => false,
        Err(OpsError::Vfs(VfsError::Cancelled)) => {
            walk.not_compared();
            true
        }
        Err(error) => return Err(error),
    };
    let PairWalk {
        mut rows,
        left_out,
        mut progress,
        ..
    } = walk;
    order_preorder(&mut rows);
    progress.done = true;
    progress.folder.clear();
    sink.progress(&progress);
    Ok(Comparison {
        left: left.clone(),
        right: right.clone(),
        criteria: criteria.clone(),
        left_facts,
        right_facts,
        rows,
        left_out,
        cancelled,
    })
}

/// A root's entry, refused unless it is a folder.
fn root_entry(provider: &dyn Provider, place: &Place, ctx: &OpContext) -> Result<Entry, OpsError> {
    let entry = provider.stat(place, ctx)?;
    let folder = entry.kind == EntryKind::Folder
        || (entry.kind == EntryKind::Archive && place.kind() == PlaceKind::Archive);
    if folder {
        Ok(entry)
    } else {
        Err(OpsError::Refused(format!(
            "Cannot compare: {} is not a folder",
            spell_out(&place.to_string())
        )))
    }
}

/// One side of the walk: its provider, root, the root's volume and the
/// folders already walked.
struct WalkSide<'a> {
    provider: &'a dyn Provider,
    root: Place,
    volume: Option<u64>,
    seen: HashSet<(u64, u64)>,
}

impl<'a> WalkSide<'a> {
    fn new(provider: &'a dyn Provider, root: &Place, entry: &Entry) -> Self {
        let mut seen = HashSet::new();
        if let (Some(volume), Some(inode)) = (entry.volume, entry.inode) {
            seen.insert((volume, inode));
        }
        Self {
            provider,
            root: root.clone(),
            volume: entry.volume,
            seen,
        }
    }

    /// Whether a folder inside may be walked: on the root's drive and not
    /// walked before. The reason in words when not.
    fn may_enter(&mut self, entry: &Entry) -> Result<(), &'static str> {
        if let (Some(root), Some(here)) = (self.volume, entry.volume)
            && root != here
        {
            return Err(crate::walk::OTHER_DRIVE);
        }
        if let (Some(volume), Some(inode)) = (entry.volume, entry.inode)
            && !self.seen.insert((volume, inode))
        {
            return Err(crate::walk::LOOP);
        }
        Ok(())
    }
}

/// A folder to compare: its names from the roots, whether each side has
/// it, and its own row (none for the roots).
struct Pending {
    names: Vec<String>,
    on: [bool; 2],
    row: Option<usize>,
}

/// One name in a folder, paired across the two sides.
struct Pair {
    left: Option<Entry>,
    right: Option<Entry>,
    /// The side with two names that differ only by case, when this entry
    /// is one of them: never paired, a type that differs.
    twins: Option<Side>,
}

struct PairWalk<'a> {
    sides: [WalkSide<'a>; 2],
    criteria: &'a CompareCriteria,
    fold_case: bool,
    ctx: &'a OpContext,
    control: &'a JobControl,
    sink: &'a dyn CompareSink,
    rows: Vec<CompareRow>,
    left_out: Vec<(Place, String)>,
    progress: CompareProgress,
    told: Option<Instant>,
    separator: char,
    /// The folders still to compare; the current one is put back when a
    /// cancel stops it.
    queue: Vec<Pending>,
}

const SIDES: [Side; 2] = [Side::Left, Side::Right];

impl PairWalk<'_> {
    fn run(&mut self) -> Result<(), OpsError> {
        self.tell(true);
        self.queue.push(Pending {
            names: Vec::new(),
            on: [true, true],
            row: None,
        });
        while let Some(pending) = self.queue.pop() {
            if let Err(error) = self.folder(&pending) {
                self.queue.push(pending);
                return Err(error);
            }
            self.tell(false);
        }
        Ok(())
    }

    /// Compares one folder pair: lists each side, pairs the names, adds a
    /// row for each pair and queues the folders inside.
    fn folder(&mut self, pending: &Pending) -> Result<(), OpsError> {
        self.hold()?;
        self.progress.folder = pending.names.join(&self.separator.to_string());
        let mut listed: [Option<Vec<Entry>>; 2] = [None, None];
        for (at, side) in SIDES.iter().enumerate() {
            if !pending.on[at] {
                continue;
            }
            let place = place_at(&self.sides[at].root, &pending.names)?;
            match self.sides[at].provider.list(&place, self.ctx) {
                Ok(entries) => listed[at] = Some(entries),
                Err(VfsError::Cancelled) => return Err(VfsError::Cancelled.into()),
                Err(error) => match pending.row {
                    // A root that cannot be listed ends the compare.
                    None => return Err(error.into()),
                    Some(row) => {
                        let row = &mut self.rows[row];
                        row.state = RowState::Unreadable;
                        row.facts = vec![RowFact::Unreadable(Some(*side), error.to_string())];
                    }
                },
            }
            self.progress.folders += 1;
        }
        if pending
            .on
            .iter()
            .zip(&listed)
            .any(|(on, list)| *on && list.is_none())
        {
            // A side could not be listed: the folder's row says so, and
            // nothing inside is compared.
            return Ok(());
        }
        let [left, right] = listed;
        let pairs = self.pair(
            &pending.names,
            left.unwrap_or_default(),
            right.unwrap_or_default(),
        );
        for pair in pairs {
            let names = {
                let mut names = pending.names.clone();
                let name = pair.left.as_ref().or(pair.right.as_ref());
                names.push(name.map(|e| e.name.clone()).unwrap_or_default());
                names
            };
            self.add(names, pair)?;
        }
        Ok(())
    }

    /// After a cancel: the folders not reached, and the one stopped in,
    /// are "Not compared".
    fn not_compared(&mut self) {
        for pending in std::mem::take(&mut self.queue) {
            if let Some(row) = pending.row {
                let row = &mut self.rows[row];
                row.state = RowState::NotCompared;
                row.facts.clear();
            }
        }
    }

    /// Waits while paused; fails when cancelled.
    fn hold(&mut self) -> Result<(), OpsError> {
        let mut said = false;
        while self.control.is_paused() && !self.control.cancel.is_cancelled() {
            if !said {
                self.progress.paused = true;
                self.sink.progress(&self.progress);
                said = true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if said {
            self.progress.paused = false;
            self.tell(true);
        }
        self.ctx.check().map_err(OpsError::from)
    }

    /// Tells the sink, at most every [`PROGRESS_EVERY`] unless `now`.
    fn tell(&mut self, now: bool) {
        let due = self
            .told
            .is_none_or(|told| told.elapsed() >= PROGRESS_EVERY);
        if now || due {
            self.told = Some(Instant::now());
            self.sink.progress(&self.progress);
        }
    }

    /// Pairs one folder's two listings by name, folded by the pair's case
    /// rule, leaving out what the walk leaves out; names that only one
    /// side can tell apart become rows of their own.
    fn pair(&mut self, names: &[String], left: Vec<Entry>, right: Vec<Entry>) -> Vec<Pair> {
        let mut by_key: BTreeMap<String, [Vec<Entry>; 2]> = BTreeMap::new();
        for (at, entries) in [left, right].into_iter().enumerate() {
            for entry in entries {
                if !self.keep(at, names, &entry) {
                    continue;
                }
                let key = if self.fold_case {
                    entry.name.to_lowercase()
                } else {
                    entry.name.clone()
                };
                by_key.entry(key).or_default()[at].push(entry);
            }
        }
        let mut pairs = Vec::new();
        for (_, [mut left, mut right]) in by_key {
            if left.len() > 1 || right.len() > 1 {
                let side = if left.len() > 1 {
                    Side::Left
                } else {
                    Side::Right
                };
                for entry in left.drain(..) {
                    pairs.push(Pair {
                        left: Some(entry),
                        right: None,
                        twins: Some(side),
                    });
                }
                for entry in right.drain(..) {
                    pairs.push(Pair {
                        left: None,
                        right: Some(entry),
                        twins: Some(side),
                    });
                }
                continue;
            }
            pairs.push(Pair {
                left: left.pop(),
                right: right.pop(),
                twins: None,
            });
        }
        pairs
    }

    /// Whether the walk keeps an entry, saying why when it does not.
    fn keep(&mut self, at: usize, names: &[String], entry: &Entry) -> bool {
        if !entry.kind.is_file_system() {
            return false;
        }
        let why = if is_unfinished(&entry.name) {
            crate::walk::UNFINISHED
        } else if entry.name_not_unicode {
            "the name is not valid Unicode"
        } else {
            return true;
        };
        let place = place_at(&self.sides[at].root, names).and_then(|folder| {
            folder
                .join(&entry.name)
                .map_err(|e| OpsError::from(VfsError::from(e)))
        });
        if let Ok(place) = place {
            self.left_out.push((place, why.to_owned()));
        }
        false
    }

    /// Why a pair is skipped by the hidden rule or the filter, when it is.
    fn skipped(
        &self,
        names: &[String],
        left: &Option<Entry>,
        right: &Option<Entry>,
    ) -> Option<String> {
        let entries = || left.iter().chain(right.iter());
        if !self.criteria.hidden && entries().any(|entry| entry.hidden) {
            return Some("hidden".to_owned());
        }
        let filter = self.criteria.filter.as_ref()?;
        entries()
            .any(|entry| !filter.keep(names, entry))
            .then(|| filter.reason())
    }

    /// Adds the row for one pair, and queues its folder when it is one.
    fn add(&mut self, names: Vec<String>, pair: Pair) -> Result<(), OpsError> {
        let Pair { left, right, twins } = pair;
        let mut row = CompareRow {
            names,
            left,
            right,
            state: RowState::Same,
            facts: Vec::new(),
        };
        if let Some(why) = self.skipped(&row.names, &row.left, &row.right) {
            row.state = RowState::Skipped;
            row.facts.push(RowFact::Filtered(why));
            self.push(row);
            return Ok(());
        }
        if let Some(side) = twins {
            row.state = RowState::TypeDiffers;
            row.facts.push(RowFact::CaseTwins(side));
            self.push(row);
            return Ok(());
        }
        let folder_on = |entry: &Option<Entry>| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.kind == EntryKind::Folder)
        };
        let descend = [folder_on(&row.left), folder_on(&row.right)];
        match (&row.left, &row.right) {
            (Some(_), None) => row.state = RowState::OnlyLeft,
            (None, Some(_)) => row.state = RowState::OnlyRight,
            (Some(left), Some(right)) => {
                let case = left.name != right.name;
                match self.judge(&row.names, left, right) {
                    Ok((state, facts)) => {
                        row.state = state;
                        row.facts = facts;
                    }
                    Err(error) => {
                        // Cancelled inside a content read.
                        row.state = RowState::NotCompared;
                        self.push(row);
                        return Err(error);
                    }
                }
                if case {
                    row.facts.push(RowFact::NameCase);
                }
            }
            (None, None) => return Ok(()),
        }
        // Folders are walked inside when both sides are folders (unless
        // subfolders are off), and always on the one side that has the
        // folder, so a copy or a removal of it lists everything inside;
        // never through a type that differs.
        let one_side = row.left.is_none() || row.right.is_none();
        let enter = row.state != RowState::TypeDiffers
            && (descend[0] || descend[1])
            && (self.criteria.subfolders || one_side);
        if !enter {
            self.push(row);
            return Ok(());
        }
        if row.names.len() >= self.criteria.max_depth {
            // Its insides are not known, so nothing is done to it.
            row.state = RowState::Unreadable;
            row.facts = vec![RowFact::Unreadable(
                None,
                format!("too deep, more than {} levels", self.criteria.max_depth),
            )];
            self.push(row);
            return Ok(());
        }
        let mut on = [false, false];
        for (at, entry) in [&row.left, &row.right].into_iter().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            if entry.kind != EntryKind::Folder {
                continue;
            }
            if let Err(why) = self.sides[at].may_enter(entry) {
                let place = place_at(&self.sides[at].root, &row.names)?;
                self.left_out.push((place, why.to_owned()));
                return Ok(());
            }
            on[at] = true;
        }
        let names = row.names.clone();
        let index = self.push(row);
        self.queue.push(Pending {
            names,
            on,
            row: Some(index),
        });
        Ok(())
    }

    fn push(&mut self, row: CompareRow) -> usize {
        self.rows.push(row);
        self.progress.entries += 1;
        self.rows.len() - 1
    }

    /// How two times stand: which is newer, or the same with what made
    /// them the same.
    fn times(&self, left: Timestamp, right: Timestamp) -> (Ordering, Option<RowFact>) {
        let nanos = |t: Timestamp| {
            i128::from(t.unix_seconds()) * 1_000_000_000 + i128::from(t.subsec_nanos())
        };
        let gap = nanos(left) - nanos(right);
        let window = i128::try_from(self.criteria.tolerance.as_nanos()).unwrap_or(i128::MAX / 4);
        let hour = 3_600 * 1_000_000_000_i128;
        if gap == 0 {
            (Ordering::Equal, None)
        } else if gap.abs() <= window {
            (
                Ordering::Equal,
                Some(RowFact::TimesWithin(self.criteria.tolerance)),
            )
        } else if self.criteria.ignore_one_hour && (gap.abs() - hour).abs() <= window {
            (Ordering::Equal, Some(RowFact::HourApart))
        } else {
            (gap.cmp(&0), None)
        }
    }

    /// The status of a path on both sides.
    fn judge(
        &mut self,
        names: &[String],
        left: &Entry,
        right: &Entry,
    ) -> Result<(RowState, Vec<RowFact>), OpsError> {
        let file = |entry: &Entry| matches!(entry.kind, EntryKind::File | EntryKind::Archive);
        if left.kind == EntryKind::Folder && right.kind == EntryKind::Folder {
            return Ok((RowState::Same, Vec::new()));
        }
        if left.kind == EntryKind::Link && right.kind == EntryKind::Link {
            let target = |entry: &Entry| entry.link.as_ref().map(|link| link.target.clone());
            return Ok(if target(left) == target(right) {
                (RowState::Same, Vec::new())
            } else {
                (RowState::Different, vec![RowFact::PointsElsewhere])
            });
        }
        if !(file(left) && file(right)) {
            if left.kind == EntryKind::Special || right.kind == EntryKind::Special {
                return Ok((
                    RowState::Unreadable,
                    vec![RowFact::Unreadable(None, "a special file".to_owned())],
                ));
            }
            return Ok((
                RowState::TypeDiffers,
                vec![RowFact::Kinds(left.kind, right.kind)],
            ));
        }
        let larger = match (left.size, right.size) {
            (Some(a), Some(b)) if a > b => Some(Side::Left),
            (Some(a), Some(b)) if b > a => Some(Side::Right),
            _ => None,
        };
        let same_size = left.size.is_some() && left.size == right.size;
        let time = match (left.modified, right.modified) {
            (Some(a), Some(b)) => Some(self.times(a, b)),
            _ => None,
        };
        let newer = match time {
            Some((Ordering::Greater, _)) => Some(Side::Left),
            Some((Ordering::Less, _)) => Some(Side::Right),
            _ => None,
        };
        let mut facts: Vec<RowFact> = newer
            .map(RowFact::Newer)
            .into_iter()
            .chain(larger.map(RowFact::Larger))
            .collect();
        match self.criteria.by {
            CompareBy::Size => {
                return Ok(if same_size {
                    (RowState::Same, Vec::new())
                } else {
                    (
                        RowState::Different,
                        larger.map(RowFact::Larger).into_iter().collect(),
                    )
                });
            }
            CompareBy::Content if same_size => {
                return Ok(match self.contents_match(names)? {
                    Err((side, why)) => {
                        (RowState::Unreadable, vec![RowFact::Unreadable(side, why)])
                    }
                    Ok(true) => match time {
                        Some((Ordering::Equal, within)) => {
                            (RowState::Same, within.into_iter().collect())
                        }
                        _ => (RowState::Same, vec![RowFact::TimesDiffer]),
                    },
                    Ok(false) => {
                        facts.push(RowFact::ContentDiffers);
                        (RowState::Different, facts)
                    }
                });
            }
            CompareBy::Content | CompareBy::SizeAndTime => {}
        }
        Ok(match time {
            Some((Ordering::Equal, within)) if same_size => {
                (RowState::Same, within.into_iter().collect())
            }
            Some((Ordering::Equal, _)) => {
                facts.push(RowFact::SameTime);
                (RowState::Different, facts)
            }
            Some(_) => (RowState::Different, facts),
            None => {
                facts.push(RowFact::NoTime);
                (RowState::Different, facts)
            }
        })
    }

    /// Reads both files block by block and stops at the first difference.
    /// `Err` with the side and the reason when a side cannot be read.
    fn contents_match(
        &mut self,
        names: &[String],
    ) -> Result<Result<bool, (Option<Side>, String)>, OpsError> {
        let mut readers = Vec::with_capacity(2);
        for (at, side) in SIDES.iter().enumerate() {
            let place = place_at(&self.sides[at].root, names)?;
            match self.sides[at].provider.read(&place, self.ctx) {
                Ok(reader) => readers.push(reader),
                Err(VfsError::Cancelled) => return Err(VfsError::Cancelled.into()),
                Err(error) => return Ok(Err((Some(*side), error.to_string()))),
            }
        }
        let mut right = readers.pop().ok_or(VfsError::NotFound)?;
        let mut left = readers.pop().ok_or(VfsError::NotFound)?;
        self.hold()?;
        // The one block compare (`diroxide_vfs::equal`); a pause waits and
        // a cancel stops between blocks, as before.
        let cancel = self.ctx.cancel.clone();
        let start = self.progress.bytes_read;
        let mut held = None;
        let answer = equal_with(&mut left, &mut right, CHUNK, &cancel, &mut |both| {
            self.progress.bytes_read = start + both;
            self.tell(false);
            self.hold().map_err(|error| {
                held = Some(error);
                VfsError::Cancelled
            })
        });
        if let Some(error) = held {
            return Err(error);
        }
        match answer {
            Ok(equality) => Ok(Ok(equality.is_same())),
            Err(EqualStop::Stopped(error)) => Err(error.into()),
            Err(EqualStop::Read(stream, error)) => {
                let side = match stream {
                    Stream::First => Side::Left,
                    Stream::Second => Side::Right,
                };
                Ok(Err((Some(side), error.to_string())))
            }
        }
    }
}

/// Puts rows in pre-order: each folder right before what is inside it,
/// names in their sorted order, as the engine's walk does.
fn order_preorder(rows: &mut [CompareRow]) {
    rows.sort_by(|a, b| {
        let fold = |names: &[String]| -> Vec<(String, String)> {
            names
                .iter()
                .map(|name| (name.to_lowercase(), name.clone()))
                .collect()
        };
        fold(&a.names).cmp(&fold(&b.names))
    });
}
