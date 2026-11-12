//! The preview, which is the plan: one row per entry with its old name,
//! its new name and its state in words, problems first, and a count line
//! that leads with the worst news.
//!
//! Planning is pure: the caller hands over the entries, the recipe and
//! the facts of each folder (the names already there, whether case
//! matters, the place's name rules). [`super::read_context`] reads those
//! facts through the engine's providers; tests give them by hand.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use diroxide_core::format::{count, grouped};
use diroxide_core::job::ConflictPolicy;
use diroxide_core::names::spell_out;
use diroxide_core::{Entry, EntryKind, JobId, JobKind, JobRequest, Place, UtcOffset};
use diroxide_vfs::Capabilities;
use diroxide_vfs::names::{NameIssue, NameRules, check_name_with};

use super::compile::{CompiledRecipe, RecipeError, Subject, ValueProblem};
use super::rule::Recipe;

/// One entry to rename: its place and what its provider said of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameSource {
    /// The full place.
    pub place: Place,
    /// The entry, for its name, kind, dates and size.
    pub entry: Entry,
}

impl RenameSource {
    /// A source from its place and entry.
    pub fn new(place: Place, entry: Entry) -> Self {
        Self { place, entry }
    }

    /// Whether it is a folder, whose whole name is its stem.
    pub fn is_folder(&self) -> bool {
        self.entry.kind == EntryKind::Folder
    }
}

/// Where a template's `{meta:FIELD}` values come from: the app feeds the
/// details fields of `diroxide-meta` here, so the names of the fields are
/// the details view's. A field with no value is that row's problem, never
/// an empty string. Slow fields are the caller's to read before planning.
pub trait FieldLookup: Send + Sync {
    /// The field's value for one source, or `None` when it has none.
    fn field(&self, source: &RenameSource, key: &str) -> Option<String>;
}

/// What planning needs to know of one folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderFacts {
    /// Every name in the folder now, the sources' own included.
    pub names: Vec<String>,
    /// Whether `a.txt` and `A.txt` are two names there.
    pub case_sensitive: bool,
    /// The place's name rules.
    pub rules: NameRules,
}

impl FolderFacts {
    /// A folder's facts from its names and its provider's capabilities.
    pub fn from_capabilities(names: Vec<String>, capabilities: &Capabilities) -> Self {
        Self {
            names,
            case_sensitive: capabilities.case_sensitive,
            rules: NameRules::from_capabilities(capabilities),
        }
    }

    /// Facts for a folder nothing could be read of: no other names known,
    /// case ignored (the stricter guess, since the other guess can lose a
    /// file), and this platform's own name rules.
    pub fn unknown() -> Self {
        Self {
            names: Vec::new(),
            case_sensitive: false,
            rules: NameRules::native(),
        }
    }
}

/// Everything planning reads besides the entries and the recipe.
#[derive(Clone)]
pub struct RenameContext<'a> {
    folders: HashMap<Place, FolderFacts>,
    lookup: Option<&'a dyn FieldLookup>,
    offset: UtcOffset,
    portable: bool,
}

impl std::fmt::Debug for RenameContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenameContext")
            .field("folders", &self.folders)
            .field("lookup", &self.lookup.is_some())
            .field("offset", &self.offset)
            .field("portable", &self.portable)
            .finish()
    }
}

impl Default for RenameContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> RenameContext<'a> {
    /// A context with no folders known, dates in UTC, and portable names
    /// on.
    pub fn new() -> Self {
        Self {
            folders: HashMap::new(),
            lookup: None,
            offset: UtcOffset::UTC,
            portable: true,
        }
    }

    /// The same context, knowing one folder's facts.
    #[must_use]
    pub fn with_folder(mut self, folder: Place, facts: FolderFacts) -> Self {
        self.folders.insert(folder, facts);
        self
    }

    /// The same context, reading `{meta:FIELD}` values from a lookup.
    #[must_use]
    pub fn with_lookup(mut self, lookup: &'a dyn FieldLookup) -> Self {
        self.lookup = Some(lookup);
        self
    }

    /// The same context, writing dates at this offset from UTC (the
    /// clock's local offset, usually).
    #[must_use]
    pub fn with_offset(mut self, offset: UtcOffset) -> Self {
        self.offset = offset;
        self
    }

    /// The same context with portable names on or off. On (the default),
    /// a name Windows refuses (a reserved character, a device name, a
    /// trailing dot or space) is a problem on every system, so it can be
    /// copied anywhere later.
    #[must_use]
    pub fn portable(mut self, on: bool) -> Self {
        self.portable = on;
        self
    }

    /// The facts of a folder, or [`FolderFacts::unknown`].
    pub(crate) fn facts(&self, folder: &Place) -> FolderFacts {
        self.folders
            .get(folder)
            .cloned()
            .unwrap_or_else(FolderFacts::unknown)
    }

    pub(crate) fn lookup(&self) -> Option<&'a dyn FieldLookup> {
        self.lookup
    }

    pub(crate) fn offset(&self) -> UtcOffset {
        self.offset
    }

    pub(crate) fn is_portable(&self) -> bool {
        self.portable
    }
}

/// What the owner chose row by row, kept by the source's position in the
/// list given to [`plan_rename_with`], so a new recipe keeps them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    /// The rows left out ("Skipped by you").
    pub skipped: BTreeSet<usize>,
    /// The rows given a name by hand ("Edited by you"); later rule
    /// changes do not overwrite them.
    pub edited: BTreeMap<usize, String>,
}

impl Choices {
    /// No choices.
    pub fn new() -> Self {
        Self::default()
    }

    /// Skips a row, or takes it back when it was skipped (Space in the
    /// preview). Returns whether the row is now skipped.
    pub fn toggle_skip(&mut self, index: usize) -> bool {
        if self.skipped.remove(&index) {
            false
        } else {
            self.skipped.insert(index);
            true
        }
    }

    /// Gives a row a name by hand.
    pub fn edit(&mut self, index: usize, name: impl Into<String>) {
        self.edited.insert(index, name.into());
    }

    /// Returns a row to the rules: neither skipped nor edited.
    pub fn reset(&mut self, index: usize) {
        self.skipped.remove(&index);
        self.edited.remove(&index);
    }
}

/// Why a row cannot be renamed, other than a clash.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Blocker {
    /// The place refuses the new name, or portable names do.
    Name(NameIssue),
    /// A placeholder has no value for this entry, written as in the
    /// template: `{meta:exif.date}`.
    MissingValue(String),
    /// The counter went below zero.
    CounterBelowZero,
    /// The old name is not valid Unicode, so no rule can edit it.
    NotUnicode,
    /// The place is a drive or a root, which has no name to change.
    NoFolder,
}

impl Blocker {
    /// The blocker in words, after "Problem, ": "no value for
    /// {meta:exif.date}".
    pub fn words(&self) -> String {
        match self {
            Blocker::Name(NameIssue::ReservedCharacter(c)) => {
                format!("has the reserved character {c}")
            }
            Blocker::Name(NameIssue::Empty) => "empty name".to_owned(),
            Blocker::Name(NameIssue::TooLong { max }) => format!("too long, over {max}"),
            Blocker::Name(issue) => format!("name {}", issue.describe()),
            Blocker::MissingValue(placeholder) => format!("no value for {placeholder}"),
            Blocker::CounterBelowZero => "counter below zero".to_owned(),
            Blocker::NotUnicode => "name is not valid Unicode".to_owned(),
            Blocker::NoFolder => "a drive has no name to change".to_owned(),
        }
    }

    /// Whether the row has a new name worth showing after the words.
    fn shows_new_name(&self) -> bool {
        matches!(self, Blocker::Name(issue) if *issue != NameIssue::Empty)
    }
}

/// A row's state, in the brief's six words and the owner's two choices.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowState {
    /// The name changes, nothing to say.
    Changes,
    /// The name stays as it is.
    Unchanged,
    /// The owner left the row out.
    Skipped,
    /// Another row would get the same new name.
    ConflictWithRow {
        /// The other row's old name.
        other: String,
    },
    /// A name already in the folder, which stays, is the new name.
    ConflictOnDisk {
        /// The name in the folder.
        existing: String,
    },
    /// The name is not allowed here, or could not be made.
    NotAllowed(Blocker),
    /// Allowed, but said in the confirmation: hidden characters, a
    /// leading space, a shell character, a case twin.
    NeedsCare(NameIssue),
}

impl RowState {
    /// Whether the row blocks the batch until fixed or skipped.
    pub fn is_problem(&self) -> bool {
        matches!(
            self,
            RowState::ConflictWithRow { .. }
                | RowState::ConflictOnDisk { .. }
                | RowState::NotAllowed(_)
        )
    }

    /// Whether the row is a clash of names.
    pub fn is_conflict(&self) -> bool {
        matches!(
            self,
            RowState::ConflictWithRow { .. } | RowState::ConflictOnDisk { .. }
        )
    }

    /// Whether the row will be renamed.
    pub fn renames(&self) -> bool {
        matches!(self, RowState::Changes | RowState::NeedsCare(_))
    }

    /// The order rows are listed in: problems, care, changes, the rest.
    fn rank(&self) -> u8 {
        match self {
            RowState::ConflictWithRow { .. }
            | RowState::ConflictOnDisk { .. }
            | RowState::NotAllowed(_) => 0,
            RowState::NeedsCare(_) => 1,
            RowState::Changes => 2,
            RowState::Unchanged => 3,
            RowState::Skipped => 4,
        }
    }
}

/// Something a changing row is worth knowing, said before its names: the
/// engine takes a temporary name for these.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Note {
    /// Two rows trade names.
    Swap {
        /// The other row's old name.
        with: String,
    },
    /// Three or more rows pass names around in a ring.
    Cycle {
        /// How many rows are in the ring.
        length: usize,
    },
    /// Only the case changes, on a folder where case does not matter.
    CaseOnly,
}

/// One row of the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameRow {
    /// The source's position in the list given to the planner (the
    /// counter's order, and the key of [`Choices`]).
    pub index: usize,
    /// The source's place.
    pub place: Place,
    /// The name on disk.
    pub old: String,
    /// The name it would get; the old name when it would not change.
    pub new: String,
    /// The state.
    pub state: RowState,
    /// How many more problems the row has beyond the one its state says.
    pub more: usize,
    /// Whether the owner typed the new name.
    pub edited: bool,
    /// A swap, a ring or a case-only change.
    pub note: Option<Note>,
}

impl RenameRow {
    /// The row in words, its state first so the first 40 cells hold the
    /// state and the name that identifies the row. An ordinary row has no
    /// state word: "IMG_0001.jpg becomes trip-0001.jpg". Hidden
    /// characters are spelled out (`[U+200B]`).
    pub fn line(&self) -> String {
        let old = spell_out(&self.old);
        let new = spell_out(&self.new);
        let becomes = format!("{old} becomes {new}");
        let more = match self.more {
            0 => String::new(),
            n => format!(", and {n} more"),
        };
        match &self.state {
            RowState::Changes => match (&self.note, self.edited) {
                (_, true) => format!("Edited by you: {becomes}"),
                (Some(Note::Swap { with }), _) => {
                    format!("Swap with {}: {becomes}", spell_out(with))
                }
                (Some(Note::Cycle { length }), _) => format!("Ring of {length}: {becomes}"),
                (Some(Note::CaseOnly), _) => format!("Case only: {becomes}"),
                (None, false) => becomes,
            },
            RowState::Unchanged => format!("Same name: {old}"),
            RowState::Skipped => format!("Skipped by you: {old}"),
            RowState::ConflictWithRow { other } => {
                format!(
                    "Problem, same new name as {}{more}: {becomes}",
                    spell_out(other)
                )
            }
            RowState::ConflictOnDisk { .. } => format!("Problem, name taken{more}: {becomes}"),
            RowState::NotAllowed(blocker) if blocker.shows_new_name() => {
                format!("Problem, {}{more}: {becomes}", blocker.words())
            }
            RowState::NotAllowed(blocker) => format!("Problem, {}{more}: {old}", blocker.words()),
            RowState::NeedsCare(issue) => format!("Care, {}: {becomes}", issue.describe()),
        }
    }
}

/// A plan the owner reads: the rows, problems first, and what made them,
/// so [`super::run_rename`] can plan again against the folders as they
/// are when the owner says yes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenamePlan {
    rows: Vec<RenameRow>,
    sources: Vec<RenameSource>,
    recipe: Recipe,
    choices: Choices,
    portable: bool,
}

/// Why a plan cannot become a job.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RenameRefused {
    /// Problems remain; the line says how many and the first.
    #[error("{0}")]
    Problems(String),
    /// No row would change.
    #[error("Nothing to rename: no name would change")]
    NothingToRename,
    /// The folders changed since the preview; the line says how.
    #[error("{0}")]
    Changed(String),
    /// The recipe could not run when planned again.
    #[error("{0}")]
    Recipe(String),
    /// The engine refused the plan; the line is its reason.
    #[error("Cannot rename: {0}")]
    Engine(String),
}

impl RenameRefused {
    /// The refusal in one line, meaning first.
    pub fn line(&self) -> String {
        self.to_string()
    }
}

impl RenamePlan {
    /// The rows, problems first, then rows that need care, then changes,
    /// then unchanged and skipped rows; the given order inside each.
    pub fn rows(&self) -> &[RenameRow] {
        &self.rows
    }

    /// The rows in the given order (the counter's order), for a view
    /// that keeps the panel's order.
    pub fn rows_in_order(&self) -> Vec<&RenameRow> {
        let mut rows: Vec<&RenameRow> = self.rows.iter().collect();
        rows.sort_by_key(|row| row.index);
        rows
    }

    /// The row for a source position.
    pub fn row(&self, index: usize) -> Option<&RenameRow> {
        self.rows.iter().find(|row| row.index == index)
    }

    /// The sources the plan was made from, in their given order.
    pub fn sources(&self) -> &[RenameSource] {
        &self.sources
    }

    /// The recipe the plan was made from.
    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    /// The owner's choices the plan was made with.
    pub fn choices(&self) -> &Choices {
        &self.choices
    }

    /// Whether portable names were on.
    pub fn is_portable(&self) -> bool {
        self.portable
    }

    /// How many rows block the batch.
    pub fn problems(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.state.is_problem())
            .count()
    }

    /// How many rows will be renamed but need care.
    pub fn needs_care(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row.state, RowState::NeedsCare(_)))
            .count()
    }

    /// How many rows will be renamed.
    pub fn renaming(&self) -> usize {
        self.rows.iter().filter(|row| row.state.renames()).count()
    }

    /// Whether the plan can run: something changes and nothing blocks.
    pub fn is_ready(&self) -> bool {
        self.problems() == 0 && self.renaming() > 0
    }

    /// The count line, first in the dialog and first after every
    /// recompute, the worst news inside 40 cells: "Rename 12 of 14: 2
    /// conflicts"; "Rename 12 of 14: 2 problems" when not every problem
    /// is a clash; "Rename 3 of 3: 1 needs care"; "Rename 47 of 50: no
    /// problems"; "No name would change".
    pub fn count_line(&self) -> String {
        let total = self.rows.len() as u64;
        let renaming = self.renaming() as u64;
        let problems = self.problems() as u64;
        let conflicts = self
            .rows
            .iter()
            .filter(|row| row.state.is_conflict())
            .count() as u64;
        let care = self.needs_care() as u64;
        let head = format!("Rename {} of {}", grouped(renaming), grouped(total));
        if problems > 0 {
            let words = if conflicts == problems {
                count(problems, "conflict", "conflicts")
            } else {
                count(problems, "problem", "problems")
            };
            format!("{head}: {}", grouped_words(&words, problems))
        } else if renaming == 0 {
            "No name would change".to_owned()
        } else if care > 0 {
            let verb = if care == 1 { "needs" } else { "need" };
            format!("{head}: {} {verb} care", grouped(care))
        } else {
            format!("{head}: no problems")
        }
    }

    /// Why Apply cannot run, with the first problem, or `None` when it
    /// can: "Cannot rename: 2 problems. First: name taken: a.txt becomes
    /// b.txt".
    pub fn refusal_line(&self) -> Option<String> {
        let problems = self.problems() as u64;
        if problems == 0 {
            return (self.renaming() == 0).then(|| RenameRefused::NothingToRename.line());
        }
        let first = self.rows.iter().find(|row| row.state.is_problem())?;
        let line = first.line();
        let detail = line.strip_prefix("Problem, ").unwrap_or(&line);
        Some(format!(
            "Cannot rename: {}. First: {detail}",
            count(problems, "problem", "problems")
        ))
    }

    /// The confirmation's question, which starts on No: "Rename 12
    /// items?"; "Rename 12 items, 1 needs care?".
    pub fn question(&self) -> String {
        let items = count(self.renaming() as u64, "item", "items");
        match self.needs_care() {
            0 => format!("Rename {items}?"),
            1 => format!("Rename {items}, 1 needs care?"),
            n => format!("Rename {items}, {n} need care?"),
        }
    }

    /// The position in [`RenamePlan::rows`] of the next problem row after
    /// `after` (from the top when `None`), wrapping round; `None` when
    /// there is none.
    pub fn next_problem(&self, after: Option<usize>) -> Option<usize> {
        let n = self.rows.len();
        let start = after.map_or(0, |at| at + 1);
        (0..n)
            .map(|i| (start + i) % n)
            .find(|&i| self.rows[i].state.is_problem())
    }

    /// The position of the problem row before `before`, wrapping round.
    pub fn previous_problem(&self, before: Option<usize>) -> Option<usize> {
        let n = self.rows.len();
        if n == 0 {
            return None;
        }
        let start = before.unwrap_or(0) + n;
        (1..=n)
            .map(|i| (start - i) % n)
            .find(|&i| self.rows[i].state.is_problem())
    }

    /// The renames the job will make, each source with its new name, in
    /// the given order.
    pub fn pairs(&self) -> Vec<(Place, String)> {
        self.rows_in_order()
            .into_iter()
            .filter(|row| row.state.renames())
            .map(|row| (row.place.clone(), row.new.clone()))
            .collect()
    }

    /// The job request: every renaming row as a pair, and never an
    /// overwrite (a clash the run meets is skipped and said, never
    /// replaced). The engine orders the steps, takes temporary names for
    /// swaps, rings and case-only changes, and keeps one undo record.
    pub fn request(&self, job: JobId) -> Result<JobRequest, RenameRefused> {
        if let Some(line) = self.refusal_line() {
            return Err(if self.problems() > 0 {
                RenameRefused::Problems(line)
            } else {
                RenameRefused::NothingToRename
            });
        }
        let pairs = self.pairs();
        let sources = pairs.iter().map(|(place, _)| place.clone()).collect();
        let mut request = JobRequest::new(job, JobKind::Rename, sources).renaming(pairs);
        request.conflicts = ConflictPolicy::Skip;
        Ok(request)
    }
}

/// A count in words with its number grouped: "1,204 conflicts".
fn grouped_words(words: &str, n: u64) -> String {
    match words.split_once(' ') {
        Some((number, rest)) if number == n.to_string() => format!("{} {rest}", grouped(n)),
        _ => words.to_owned(),
    }
}

/// Plans a batch rename with no row choices: [`plan_rename_with`].
pub fn plan_rename(
    sources: &[RenameSource],
    recipe: &Recipe,
    context: &RenameContext<'_>,
) -> Result<RenamePlan, RecipeError> {
    plan_rename_with(sources, recipe, context, &Choices::new())
}

/// A row while it is planned.
struct Draft {
    folder: Option<Place>,
    old: String,
    new: String,
    blockers: Vec<Blocker>,
    care: Vec<NameIssue>,
    edited: bool,
    skipped: bool,
}

/// Plans a batch rename: every source's new name from the recipe (or the
/// owner's own), then every name checked under its folder's rules, then
/// every clash found, in a loop, since a row that cannot change keeps its
/// name and may clash in turn. A recipe that cannot compile is refused
/// as a whole; everything else is a row's state.
pub fn plan_rename_with(
    sources: &[RenameSource],
    recipe: &Recipe,
    context: &RenameContext<'_>,
    choices: &Choices,
) -> Result<RenamePlan, RecipeError> {
    let compiled = CompiledRecipe::new(recipe)?;
    let mut drafts = Vec::with_capacity(sources.len());
    let mut position: HashMap<(usize, Option<Place>), i64> = HashMap::new();
    for (index, source) in sources.iter().enumerate() {
        let folder = source.place.parent();
        let parent_name = folder
            .as_ref()
            .and_then(|f| f.name().map(str::to_owned))
            .unwrap_or_default();
        let old = source.entry.name.clone();
        let mut draft = Draft {
            folder: folder.clone(),
            old: old.clone(),
            new: old.clone(),
            blockers: Vec::new(),
            care: Vec::new(),
            edited: false,
            skipped: choices.skipped.contains(&index),
        };
        // Every pass's counter moves for every source, skipped or not, so
        // skipping a row never renumbers the others.
        let mut name = old.clone();
        let mut made = Ok(());
        for (p, pass) in compiled.passes.iter().enumerate() {
            let key = (p, pass.counter.per_folder.then(|| folder.clone()).flatten());
            let k = position.entry(key).or_insert(0);
            let counter = pass
                .counter
                .start
                .saturating_add(pass.counter.step.saturating_mul(*k));
            *k += 1;
            if made.is_err() {
                continue;
            }
            let subject = Subject {
                source,
                parent: &parent_name,
                counter,
                offset: context.offset(),
                lookup: context.lookup(),
            };
            match pass.apply(&compiled, &name, &subject) {
                Ok(next) => name = next,
                Err(problem) => made = Err(problem),
            }
        }
        if folder.is_none() {
            draft.blockers.push(Blocker::NoFolder);
        } else if source.entry.name_not_unicode {
            draft.blockers.push(Blocker::NotUnicode);
        } else if let Some(typed) = choices.edited.get(&index) {
            draft.new = typed.clone();
            draft.edited = true;
        } else {
            match made {
                Ok(()) => draft.new = name,
                Err(ValueProblem::Missing(placeholder)) => {
                    draft.blockers.push(Blocker::MissingValue(placeholder));
                }
                Err(ValueProblem::CounterBelowZero) => {
                    draft.blockers.push(Blocker::CounterBelowZero);
                }
            }
        }
        drafts.push(draft);
    }

    // Names checked under each folder's rules.
    let mut facts: HashMap<Place, FolderFacts> = HashMap::new();
    for draft in &drafts {
        if let Some(folder) = &draft.folder {
            facts
                .entry(folder.clone())
                .or_insert_with(|| context.facts(folder));
        }
    }
    for draft in &mut drafts {
        if draft.skipped || !draft.blockers.is_empty() || draft.new == draft.old {
            continue;
        }
        let Some(folder) = &draft.folder else {
            continue;
        };
        let mut rules = facts[folder].rules;
        if context.is_portable() {
            rules.windows = true;
            rules.length_in_utf16 |= cfg!(windows);
        }
        let before = check_name_with(&draft.old, &[], &rules);
        for issue in check_name_with(&draft.new, &[], &rules) {
            // A refused issue the old name already had is kept as care:
            // the rename did not bring it, and the owner may be fixing
            // something else.
            if issue.refused(&rules) && !before.contains(&issue) {
                draft.blockers.push(Blocker::Name(issue));
            } else if !matches!(issue, NameIssue::CaseTwin(_)) {
                draft.care.push(issue);
            }
        }
    }

    // Clashes, folder by folder, until nothing more changes.
    let fold = |folder: &Option<Place>, name: &str| -> String {
        match folder.as_ref().and_then(|f| facts.get(f)) {
            Some(facts) if facts.case_sensitive => name.to_owned(),
            _ => name.to_lowercase(),
        }
    };
    let renames =
        |draft: &Draft| !draft.skipped && draft.blockers.is_empty() && draft.new != draft.old;
    // The names that stay in each folder whatever the batch does: every
    // name there that is not a source's, by its folded key.
    let mut source_keys: HashSet<(Place, String)> = HashSet::new();
    for draft in &drafts {
        if let Some(folder) = &draft.folder {
            source_keys.insert((folder.clone(), fold(&draft.folder, &draft.old)));
        }
    }
    let mut staying: HashMap<(Place, String), String> = HashMap::new();
    for (folder, folder_facts) in &facts {
        let place = Some(folder.clone());
        for name in &folder_facts.names {
            let key = fold(&place, name);
            if !source_keys.contains(&(folder.clone(), key.clone())) {
                staying
                    .entry((folder.clone(), key))
                    .or_insert_with(|| name.clone());
            }
        }
    }
    let mut clash: Vec<Option<RowState>> = vec![None; drafts.len()];
    loop {
        let moving =
            |i: usize, clash: &[Option<RowState>]| renames(&drafts[i]) && clash[i].is_none();
        let finals: Vec<String> = (0..drafts.len())
            .map(|i| {
                let name = if moving(i, &clash) {
                    &drafts[i].new
                } else {
                    &drafts[i].old
                };
                fold(&drafts[i].folder, name)
            })
            .collect();
        let mut by_final: HashMap<(Option<Place>, &str), Vec<usize>> = HashMap::new();
        for (i, key) in finals.iter().enumerate() {
            by_final
                .entry((drafts[i].folder.clone(), key.as_str()))
                .or_default()
                .push(i);
        }
        let mut found: Vec<(usize, RowState)> = Vec::new();
        for i in 0..drafts.len() {
            if !moving(i, &clash) {
                continue;
            }
            let draft = &drafts[i];
            let key = &finals[i];
            let others = &by_final[&(draft.folder.clone(), key.as_str())];
            if let Some(&j) = others.iter().find(|&&j| j != i) {
                let state = if moving(j, &clash) {
                    RowState::ConflictWithRow {
                        other: drafts[j].old.clone(),
                    }
                } else {
                    RowState::ConflictOnDisk {
                        existing: drafts[j].old.clone(),
                    }
                };
                found.push((i, state));
            } else if let Some(existing) = draft
                .folder
                .as_ref()
                .and_then(|folder| staying.get(&(folder.clone(), key.clone())))
            {
                found.push((
                    i,
                    RowState::ConflictOnDisk {
                        existing: existing.clone(),
                    },
                ));
            }
        }
        if found.is_empty() {
            break;
        }
        for (i, state) in found {
            clash[i] = Some(state);
        }
    }

    // Case twins on folders where case matters: care, not a problem.
    let mut lowered: HashMap<(Place, String), Vec<String>> = HashMap::new();
    for ((folder, _), name) in &staying {
        if facts[folder].case_sensitive {
            lowered
                .entry((folder.clone(), name.to_lowercase()))
                .or_default()
                .push(name.clone());
        }
    }
    for (i, draft) in drafts.iter().enumerate() {
        let Some(folder) = &draft.folder else {
            continue;
        };
        if facts[folder].case_sensitive {
            let name = if renames(draft) && clash[i].is_none() {
                &draft.new
            } else {
                &draft.old
            };
            lowered
                .entry((folder.clone(), name.to_lowercase()))
                .or_default()
                .push(name.clone());
        }
    }
    for i in 0..drafts.len() {
        if !renames(&drafts[i]) || clash[i].is_some() {
            continue;
        }
        let Some(folder) = drafts[i].folder.clone() else {
            continue;
        };
        if !facts[&folder].case_sensitive {
            continue;
        }
        let new = drafts[i].new.clone();
        if let Some(twin) = lowered
            .get(&(folder, new.to_lowercase()))
            .and_then(|names| names.iter().find(|name| **name != new))
        {
            drafts[i].care.push(NameIssue::CaseTwin(twin.clone()));
        }
    }
    // States and notes.
    let mut rows: Vec<RenameRow> = Vec::with_capacity(drafts.len());
    for (i, draft) in drafts.iter().enumerate() {
        let mut more = 0;
        let state = if draft.skipped {
            RowState::Skipped
        } else if let Some(first) = draft.blockers.first() {
            more = draft.blockers.len() - 1;
            RowState::NotAllowed(first.clone())
        } else if draft.new == draft.old {
            RowState::Unchanged
        } else if let Some(state) = &clash[i] {
            state.clone()
        } else if let Some(issue) = draft.care.first() {
            RowState::NeedsCare(issue.clone())
        } else {
            RowState::Changes
        };
        rows.push(RenameRow {
            index: i,
            place: sources[i].place.clone(),
            old: draft.old.clone(),
            new: if draft.skipped {
                draft.old.clone()
            } else {
                draft.new.clone()
            },
            state,
            more,
            edited: draft.edited,
            note: None,
        });
    }
    let key_of = |row: &RenameRow, name: &str| fold(&drafts[row.index].folder, name);
    let moving: Vec<bool> = rows.iter().map(|row| row.state.renames()).collect();
    let mut by_old: HashMap<(Option<Place>, String), usize> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        if moving[i] {
            by_old.insert((drafts[row.index].folder.clone(), key_of(row, &row.old)), i);
        }
    }
    let mut notes: Vec<Option<Note>> = vec![None; rows.len()];
    for (i, row) in rows.iter().enumerate() {
        if !moving[i] {
            continue;
        }
        let folder = drafts[row.index].folder.clone();
        if key_of(row, &row.new) == key_of(row, &row.old) {
            notes[i] = Some(Note::CaseOnly);
            continue;
        }
        // Follow the names round: who holds my new name, who holds theirs.
        let mut length = 1;
        let mut at = i;
        let ring = loop {
            let Some(&next) = by_old.get(&(folder.clone(), key_of(&rows[at], &rows[at].new)))
            else {
                break false;
            };
            if next == i {
                break true;
            }
            length += 1;
            at = next;
            if length > rows.len() {
                break false;
            }
        };
        if ring {
            notes[i] = Some(if length == 2 {
                Note::Swap {
                    with: rows[at].old.clone(),
                }
            } else {
                Note::Cycle { length }
            });
        }
    }
    for (row, note) in rows.iter_mut().zip(notes) {
        row.note = note;
    }
    rows.sort_by_key(|row| (row.state.rank(), row.index));
    Ok(RenamePlan {
        rows,
        sources: sources.to_vec(),
        recipe: recipe.clone(),
        choices: choices.clone(),
        portable: context.is_portable(),
    })
}
