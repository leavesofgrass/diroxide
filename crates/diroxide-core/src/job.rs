//! The vocabulary of jobs: what kind of operation, on what, what to do
//! when a name is taken, what state a job is in, and the answers a user
//! gives to its questions. The job engine (`diroxide-ops`) plans and runs
//! them; the app asks for them through [`Effect::StartJob`](crate::Effect).
//!
//! A [`JobRequest`] is serializable (scripts, saved tasks, the undo
//! journal) and so never holds a secret: options that do travel with a
//! secret go in a typed side structure of their own.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::access::{AttributeEdit, LinkEdit, Recursion};
use crate::ids::JobId;
use crate::place::Place;

/// What a job does.
///
/// A kind this version does not know, from a plugin or a newer journal,
/// reads as [`JobKind::Unknown`], which the engine refuses to plan.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobKind {
    /// Copy the sources into the destination folder.
    Copy,
    /// Move the sources into the destination folder.
    Move,
    /// Give one source a new name in the same folder, or several sources
    /// their new names ([`JobRequest::renames`]).
    Rename,
    /// Copy one source beside itself under a new name.
    Duplicate,
    /// Make a new folder.
    MakeFolder,
    /// Move the sources to the trash: the default delete.
    Trash,
    /// Delete the sources for good, after listing every path.
    DeletePermanently,
    /// Make a symbolic link.
    Link,
    /// Change permissions or attributes.
    SetAttributes,
    /// Change times.
    SetTimes,
    /// Pack the sources into a new archive.
    Pack,
    /// Unpack an archive into the destination folder.
    Unpack,
    /// Read every member of an archive and check its checksums; nothing
    /// is written ("Test archive").
    TestArchive,
    /// Split a file into numbered parts (`name.001`, `name.002`); the part
    /// size travels in [`JobRequest::parts`].
    Split,
    /// Join numbered parts back into one file.
    Join,
    /// Convert the sources with a recipe.
    Convert,
    /// Add up the sizes of folders.
    Dirsize,
    /// Download a file, such as an optional component, checked against its
    /// pin.
    Download,
    /// Run a plugin's command, with the plugin's own progress.
    PluginCommand,
    /// Undo the last job.
    Undo,
    /// A kind this version does not know. Never planned.
    #[serde(other)]
    Unknown,
}

impl JobKind {
    /// Every kind, [`JobKind::Unknown`] last: the word tests walk it, and
    /// lists of jobs (the job history's filter, `diro` help) take their
    /// order from it.
    pub const ALL: &'static [JobKind] = &[
        JobKind::Copy,
        JobKind::Move,
        JobKind::Rename,
        JobKind::Duplicate,
        JobKind::MakeFolder,
        JobKind::Trash,
        JobKind::DeletePermanently,
        JobKind::Link,
        JobKind::SetAttributes,
        JobKind::SetTimes,
        JobKind::Pack,
        JobKind::Unpack,
        JobKind::TestArchive,
        JobKind::Split,
        JobKind::Join,
        JobKind::Convert,
        JobKind::Dirsize,
        JobKind::Download,
        JobKind::PluginCommand,
        JobKind::Undo,
        JobKind::Unknown,
    ];

    /// The verb for "Could not VERB OBJECT" messages: "copy", "move to the
    /// trash".
    pub const fn verb(self) -> &'static str {
        match self {
            JobKind::Copy => "copy",
            JobKind::Move => "move",
            JobKind::Rename => "rename",
            JobKind::Duplicate => "duplicate",
            JobKind::MakeFolder => "make the folder",
            JobKind::Trash => "move to the trash",
            JobKind::DeletePermanently => "delete",
            JobKind::Link => "link",
            JobKind::SetAttributes => "change the attributes of",
            JobKind::SetTimes => "change the times of",
            JobKind::Pack => "pack",
            JobKind::Unpack => "unpack",
            JobKind::TestArchive => "test",
            JobKind::Split => "split",
            JobKind::Join => "join",
            JobKind::Convert => "convert",
            JobKind::Dirsize => "measure",
            JobKind::Download => "download",
            JobKind::PluginCommand => "run",
            JobKind::Undo => "undo",
            JobKind::Unknown => "run an unknown job on",
        }
    }

    /// The progress word, capitalized for the start of a line: "Copying",
    /// "Moving to the trash".
    pub const fn progress_word(self) -> &'static str {
        match self {
            JobKind::Copy => "Copying",
            JobKind::Move => "Moving",
            JobKind::Rename => "Renaming",
            JobKind::Duplicate => "Duplicating",
            JobKind::MakeFolder => "Making a folder",
            JobKind::Trash => "Moving to the trash",
            JobKind::DeletePermanently => "Deleting",
            JobKind::Link => "Linking",
            JobKind::SetAttributes => "Changing attributes",
            JobKind::SetTimes => "Changing times",
            JobKind::Pack => "Packing",
            JobKind::Unpack => "Unpacking",
            JobKind::TestArchive => "Testing",
            JobKind::Split => "Splitting",
            JobKind::Join => "Joining",
            JobKind::Convert => "Converting",
            JobKind::Dirsize => "Measuring",
            JobKind::Download => "Downloading",
            JobKind::PluginCommand => "Running",
            JobKind::Undo => "Undoing",
            JobKind::Unknown => "Working",
        }
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.verb())
    }
}

/// What to do when the destination name is taken: the job's starting
/// policy (the `--policy` flag, scripts). Each conflict is still answered
/// one at a time while the policy is `Ask` ([`ConflictAnswer`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPolicy {
    /// Ask the user, one conflict at a time.
    #[default]
    Ask,
    /// Leave the existing entry and skip the source.
    Skip,
    /// Replace the existing entry (its old copy goes to the trash first
    /// where the place has one).
    Overwrite,
    /// Keep both, giving the new one a numbered name.
    KeepBoth,
    /// Ask for a new name for each conflict.
    Rename,
    /// Replace the existing entry only when the source is newer; skip
    /// otherwise, ties included.
    Newer,
}

/// What kind of name clash a conflict is (research/file-operations.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictKind {
    /// A file where a file of that name exists.
    FileOverFile,
    /// A folder where a folder exists: always a merge, with questions per
    /// file inside.
    FolderOverFolder,
    /// A file where a folder of that name exists: skipped and reported,
    /// never resolved by deleting.
    FileOverFolder,
    /// A folder where a file of that name exists: skipped and reported.
    FolderOverFile,
    /// A file over a read-only or hidden file: replacing needs its own
    /// answer.
    ReadOnlyTarget,
    /// The source and the target are the same entry: nothing to do, never
    /// asked.
    SameFile,
}

impl ConflictKind {
    /// The situation, said first, before the name: "Name taken",
    /// "Folder exists", "Read-only file exists", "Same file", "Cannot
    /// merge".
    pub const fn situation(self) -> &'static str {
        match self {
            ConflictKind::FileOverFile => "Name taken",
            ConflictKind::FolderOverFolder => "Folder exists",
            ConflictKind::FileOverFolder | ConflictKind::FolderOverFile => "Cannot merge",
            ConflictKind::ReadOnlyTarget => "Read-only file exists",
            ConflictKind::SameFile => "Same file",
        }
    }
}

/// What to do about one conflict.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictAction {
    /// Leave the existing entry; skip this source. The answer Escape gives.
    Skip,
    /// Replace the existing entry.
    Replace,
    /// Keep both, the new one under a numbered name.
    KeepBoth,
    /// Write the new one under this name instead.
    Rename(String),
    /// Replace only when the source is newer.
    ReplaceIfNewer,
    /// Stop the job.
    Cancel,
}

/// The user's answer to one conflict: an action, and whether it applies to
/// the rest of the job ("Replace all that exist").
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConflictAnswer {
    /// What to do.
    pub action: ConflictAction,
    /// Whether the same answer settles the job's later conflicts of the
    /// same kind.
    pub apply_to_rest: bool,
}

impl ConflictAnswer {
    /// Skip this one: the answer a conflict starts on, and Escape's.
    pub const fn skip() -> Self {
        Self {
            action: ConflictAction::Skip,
            apply_to_rest: false,
        }
    }

    /// An answer for this conflict alone.
    pub const fn once(action: ConflictAction) -> Self {
        Self {
            action,
            apply_to_rest: false,
        }
    }

    /// An answer for this conflict and the rest of the job.
    pub const fn for_all(action: ConflictAction) -> Self {
        Self {
            action,
            apply_to_rest: true,
        }
    }
}

/// What a job is doing now, carried on every progress update so the app
/// can say a change of state at once and progress only at its interval.
///
/// The transfer queue (R25, part 1) is the jobs list, not a second thing:
/// a job waiting for a free slot is [`JobState::Queued`], and a job whose
/// host went away waits as [`JobState::WaitingForConnection`] behind the
/// host's one shared gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobState {
    /// Planned and confirmed, waiting for a free slot of its class (two
    /// running jobs per host, four remote jobs in all, one component
    /// download at a time).
    Queued,
    /// Walking the sources to count entries and bytes.
    #[default]
    Counting,
    /// Working.
    Running,
    /// Paused by the user.
    Paused,
    /// No progress for a while (a slow or lost network place).
    Stalled,
    /// Waiting for the user to answer a question (a conflict, an error, a
    /// password). A job that waits for an answer gives up its slot.
    WaitingForAnswer,
    /// Its host was lost: the job waits while the host's gate tries the
    /// connection again, and goes on by itself when it is back.
    WaitingForConnection,
    /// Checking what was written against its source.
    Verifying,
    /// Done, cancelled or failed; the report says which.
    Finished,
}

impl JobState {
    /// Every state, in the order a job meets them; the word tests walk it.
    pub const ALL: &'static [JobState] = &[
        JobState::Queued,
        JobState::Counting,
        JobState::Running,
        JobState::Paused,
        JobState::Stalled,
        JobState::WaitingForAnswer,
        JobState::WaitingForConnection,
        JobState::Verifying,
        JobState::Finished,
    ];

    /// The state in words, for the jobs line: "counting", "paused",
    /// "waiting for an answer", "waiting its turn", "waiting for the
    /// connection". Each fits 40 cells after a job's longest word.
    pub const fn word(self) -> &'static str {
        match self {
            JobState::Queued => "waiting its turn",
            JobState::Counting => "counting",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Stalled => "stalled",
            JobState::WaitingForAnswer => "waiting for an answer",
            JobState::WaitingForConnection => "waiting for the connection",
            JobState::Verifying => "verifying",
            JobState::Finished => "finished",
        }
    }

    /// Whether the job holds no slot of the queue in this state: it waits
    /// its turn, waits for an answer, or has finished. A job that waits for
    /// an answer releases its slot, so one forgotten question cannot hold
    /// the queue (R25, 2.2).
    pub const fn holds_no_slot(self) -> bool {
        matches!(
            self,
            JobState::Queued | JobState::WaitingForAnswer | JobState::Finished
        )
    }
}

/// What a job does when an item fails after the engine's own silent
/// retries (R25 N9): ask, as alpha 1 always did; continue and list the
/// failures at the end without a question, for unattended work; or stop
/// at the first failure. Chosen per job, and as the default for waiting
/// jobs in the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorPolicy {
    /// Ask about each failure: Retry, Skip, Skip all like this or Cancel.
    #[default]
    Ask,
    /// Skip what fails, never open a question, and list the failures in
    /// the result.
    ContinueAndList,
    /// Stop the job at its first failure.
    StopAtFirst,
}

impl ErrorPolicy {
    /// Every policy, the default first.
    pub const ALL: &'static [ErrorPolicy] = &[
        ErrorPolicy::Ask,
        ErrorPolicy::ContinueAndList,
        ErrorPolicy::StopAtFirst,
    ];

    /// The choice as listed, verb first and inside 40 cells: "Ask about
    /// each error", "Continue and list errors", "Stop at the first error".
    pub const fn label(self) -> &'static str {
        match self {
            ErrorPolicy::Ask => "Ask about each error",
            ErrorPolicy::ContinueAndList => "Continue and list errors",
            ErrorPolicy::StopAtFirst => "Stop at the first error",
        }
    }

    /// Whether this is the default, asking: a request leaves it out of its
    /// text then, so old readers and old requests agree.
    pub fn is_ask(&self) -> bool {
        *self == ErrorPolicy::Ask
    }
}

/// The answer to an error on one item. The same four everywhere; no
/// "Ignore".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorAction {
    /// Try the item again.
    Retry,
    /// Skip this item.
    Skip,
    /// Skip this item and every later error of the same class in this job.
    SkipAllLikeThis,
    /// Stop the job.
    Cancel,
}

impl ErrorAction {
    /// The choice as listed, verb first: "Retry", "Skip", "Skip all like
    /// this", "Cancel the job".
    pub const fn label(self) -> &'static str {
        match self {
            ErrorAction::Retry => "Retry",
            ErrorAction::Skip => "Skip",
            ErrorAction::SkipAllLikeThis => "Skip all like this",
            ErrorAction::Cancel => "Cancel the job",
        }
    }
}

/// Who started a job: kept in the undo journal and the job history, so
/// "Save as task" and the reports can say it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobOrigin {
    /// The user, from a key, a menu or the palette.
    #[default]
    User,
    /// The command line (`diro`).
    CommandLine,
    /// A plugin, by name.
    Plugin(String),
    /// A script command, by name.
    Script(String),
    /// A hot folder's rule, by name.
    HotFolder(String),
    /// Undo of an earlier job.
    Undo,
}

/// How a split sizes its parts (plan section 5.4): from a preset or a
/// typed size, or as a number of equal parts. The preset table itself is
/// configuration (`split_presets`); the request carries only the bytes,
/// so a script or a saved task replays the same split.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartSize {
    /// Parts of this many bytes each; the last part holds the rest, and
    /// an exact multiple makes no empty last part.
    Bytes(u64),
    /// This many parts, as equal as whole bytes allow; the last is the
    /// smallest.
    EqualParts(u32),
}

impl PartSize {
    /// The size of each part but the last, for a file of `total` bytes;
    /// `None` when no split is possible (zero bytes or zero parts).
    pub fn part_bytes(self, total: u64) -> Option<u64> {
        match self {
            PartSize::Bytes(0) | PartSize::EqualParts(0) => None,
            PartSize::Bytes(bytes) => Some(bytes),
            PartSize::EqualParts(parts) => Some(total.div_ceil(u64::from(parts)).max(1)),
        }
    }

    /// How many parts a file of `total` bytes makes; an empty file makes
    /// none.
    pub fn parts_for(self, total: u64) -> u64 {
        match self.part_bytes(total) {
            Some(bytes) if total > 0 => total.div_ceil(bytes),
            _ => 0,
        }
    }
}

/// A request for a job, as the app hands it to the job engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRequest {
    /// The job's id, chosen by the app.
    pub id: JobId,
    /// What the job does.
    pub kind: JobKind,
    /// What it acts on.
    pub sources: Vec<Place>,
    /// Where results go: the other panel's place for copy and move.
    pub destination: Option<Place>,
    /// A new name, for rename, duplicate, make folder and pack.
    pub new_name: Option<String>,
    /// Several renames at once: each source with its new name (bulk
    /// rename). Empty for every other job; a rename plans from these when
    /// there are any, and from `new_name` otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renames: Vec<(Place, String)>,
    /// What to do when a name is taken.
    pub conflicts: ConflictPolicy,
    /// Who asked.
    #[serde(default)]
    pub origin: JobOrigin,
    /// How a split sizes its parts; `None` for every other job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<PartSize>,
    /// What a `SetAttributes` or `SetTimes` job changes and where, its
    /// recursion included ([`AttributeEdit`]); `None` for every other job
    /// (R11 9.5; W2c's request 3). Boxed, so a request stays small in the
    /// effects that carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Box<AttributeEdit>>,
    /// How a `Link` job makes its links, their kind and form, or that it
    /// changes where its one source link points ([`LinkEdit`]); `None`
    /// reads as new symbolic links with absolute targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkEdit>,
    /// What the job does when an item fails after the engine's own
    /// retries ([`ErrorPolicy`]); asking unless chosen otherwise. A request
    /// written before the queue reads as asking.
    #[serde(default, skip_serializing_if = "ErrorPolicy::is_ask")]
    pub errors: ErrorPolicy,
}

impl JobRequest {
    /// A request with sources, no destination, no new name, the default
    /// conflict and error policies, and the user as its origin.
    pub fn new(id: JobId, kind: JobKind, sources: Vec<Place>) -> Self {
        Self {
            id,
            kind,
            sources,
            destination: None,
            new_name: None,
            renames: Vec::new(),
            conflicts: ConflictPolicy::default(),
            origin: JobOrigin::default(),
            parts: None,
            attributes: None,
            link: None,
            errors: ErrorPolicy::default(),
        }
    }

    /// The same request with an error policy.
    pub fn on_errors(mut self, policy: ErrorPolicy) -> Self {
        self.errors = policy;
        self
    }

    /// The same request changing permissions, attributes or times as the
    /// edit says, inside folders as its recursion says.
    pub fn changing(mut self, edit: AttributeEdit) -> Self {
        self.attributes = Some(Box::new(edit));
        self
    }

    /// The same request making links, or changing where one points, as
    /// the edit says.
    pub fn linking(mut self, edit: LinkEdit) -> Self {
        self.link = Some(edit);
        self
    }

    /// Which entries an attribute job changes: its edit's recursion, or
    /// this item only.
    pub fn recursion(&self) -> Recursion {
        self.attributes
            .as_ref()
            .map_or(Recursion::ThisItem, |edit| edit.recursion)
    }

    /// How a link job makes its links: the request's own, or new symbolic
    /// links with absolute targets.
    pub fn link_edit(&self) -> LinkEdit {
        self.link.unwrap_or_default()
    }

    /// The same request splitting into parts of this size.
    pub fn split_into(mut self, parts: PartSize) -> Self {
        self.parts = Some(parts);
        self
    }

    /// The same request with a destination.
    pub fn to(mut self, destination: Place) -> Self {
        self.destination = Some(destination);
        self
    }

    /// The same request with a new name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.new_name = Some(name.into());
        self
    }

    /// The same request renaming each source to its new name.
    pub fn renaming(mut self, renames: Vec<(Place, String)>) -> Self {
        self.renames = renames;
        self
    }

    /// The same request with an origin.
    pub fn from_origin(mut self, origin: JobOrigin) -> Self {
        self.origin = origin;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_for_messages() {
        assert_eq!(JobKind::Trash.verb(), "move to the trash");
        assert_eq!(JobKind::Copy.progress_word(), "Copying");
        assert_eq!(JobKind::Download.progress_word(), "Downloading");
        assert_eq!(ConflictPolicy::default(), ConflictPolicy::Ask);
        let request = JobRequest::new(JobId::new(1), JobKind::Rename, Vec::new()).named("b");
        assert_eq!(request.new_name.as_deref(), Some("b"));
        assert_eq!(JobState::WaitingForAnswer.word(), "waiting for an answer");
        assert_eq!(ErrorAction::SkipAllLikeThis.label(), "Skip all like this");
        assert_eq!(ConflictKind::FileOverFile.situation(), "Name taken");
    }

    #[test]
    fn unknown_kinds_fall_back_and_old_requests_still_read() {
        let kind: JobKind = serde_json::from_str("\"teleport\"").unwrap();
        assert_eq!(kind, JobKind::Unknown);
        let old = r#"{"id":3,"kind":"copy","sources":[],"destination":null,"new_name":null,"conflicts":"ask"}"#;
        let request: JobRequest = serde_json::from_str(old).unwrap();
        assert_eq!(request.origin, JobOrigin::User);
        assert!(request.renames.is_empty());
        assert_eq!(
            serde_json::to_string(&ConflictPolicy::Newer).unwrap(),
            "\"newer\""
        );
    }

    /// W2c's request 3 (P2b): an attribute job and a link job carry their
    /// edits, so the request alone travels in a script or the journal, and
    /// a request written before them still reads.
    #[test]
    fn requests_carry_their_attribute_and_link_edits() {
        use crate::access::{LinkForm, LinkKind, ModeChange};
        let edit = AttributeEdit::new(Recursion::FilesAndFolders)
            .with_mode(ModeChange::Bits { on: 0o44, off: 0 });
        let request = JobRequest::new(JobId::new(5), JobKind::SetAttributes, Vec::new())
            .changing(edit.clone());
        assert_eq!(request.recursion(), Recursion::FilesAndFolders);
        let json = serde_json::to_string(&request).unwrap();
        assert!(
            json.contains(r#""attributes":{"recursion":"files_and_folders""#),
            "{json}"
        );
        let read: JobRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(read, request);
        assert_eq!(read.attributes.as_deref(), Some(&edit));
        let link = JobRequest::new(JobId::new(6), JobKind::Link, Vec::new())
            .linking(LinkEdit::new(LinkKind::Hard, LinkForm::Absolute));
        let read: JobRequest =
            serde_json::from_str(&serde_json::to_string(&link).unwrap()).unwrap();
        assert_eq!(read.link_edit().kind, LinkKind::Hard);
        let old = r#"{"id":3,"kind":"link","sources":[],"destination":null,"new_name":null,"conflicts":"ask"}"#;
        let old: JobRequest = serde_json::from_str(old).unwrap();
        assert_eq!(old.attributes, None);
        assert_eq!(old.link_edit(), LinkEdit::default());
        assert_eq!(old.recursion(), Recursion::ThisItem);
        let written = serde_json::to_string(&old).unwrap();
        assert!(!written.contains(r#""link":"#), "{written}");
        assert!(!written.contains(r#""attributes":"#), "{written}");
    }

    /// The position of a kind in [`JobKind::ALL`]. The match has no
    /// fallback arm, so a kind added without a place in the list (and so
    /// without the word tests below) does not compile.
    const fn position(kind: JobKind) -> usize {
        match kind {
            JobKind::Copy => 0,
            JobKind::Move => 1,
            JobKind::Rename => 2,
            JobKind::Duplicate => 3,
            JobKind::MakeFolder => 4,
            JobKind::Trash => 5,
            JobKind::DeletePermanently => 6,
            JobKind::Link => 7,
            JobKind::SetAttributes => 8,
            JobKind::SetTimes => 9,
            JobKind::Pack => 10,
            JobKind::Unpack => 11,
            JobKind::TestArchive => 12,
            JobKind::Split => 13,
            JobKind::Join => 14,
            JobKind::Convert => 15,
            JobKind::Dirsize => 16,
            JobKind::Download => 17,
            JobKind::PluginCommand => 18,
            JobKind::Undo => 19,
            JobKind::Unknown => 20,
        }
    }

    #[test]
    fn every_kind_has_its_words() {
        assert_eq!(JobKind::ALL.len(), 21);
        let mut words = std::collections::HashSet::new();
        for (at, kind) in JobKind::ALL.iter().enumerate() {
            assert_eq!(position(*kind), at, "{kind:?}");
            let verb = kind.verb();
            assert!(!verb.is_empty() && verb == verb.to_lowercase(), "{kind:?}");
            assert_eq!(kind.to_string(), verb);
            let word = kind.progress_word();
            assert!(
                word.starts_with(|c: char| c.is_ascii_uppercase()),
                "{kind:?}: {word}"
            );
            // "WORD paused at 100 percent" says its state in 40 cells for
            // every kind; the jobs line with an estimate is checked in the
            // engine's crate.
            let paused = format!("{word} paused at 100 percent");
            assert!(
                crate::format::within_cells(&paused, &format!("{word} paused")),
                "{paused}"
            );
            assert!(words.insert(word), "two kinds say {word}");
            let json = serde_json::to_string(kind).unwrap();
            let back: JobKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, *kind, "{json}");
        }
        assert_eq!(JobKind::Split.progress_word(), "Splitting");
        assert_eq!(JobKind::Join.progress_word(), "Joining");
        assert_eq!(JobKind::TestArchive.progress_word(), "Testing");
        assert_eq!(JobKind::TestArchive.verb(), "test");
        assert_eq!(
            serde_json::to_string(&JobKind::TestArchive).unwrap(),
            "\"test-archive\""
        );
    }

    #[test]
    fn a_split_carries_its_part_size_and_older_requests_have_none() {
        let request = JobRequest::new(JobId::new(4), JobKind::Split, Vec::new())
            .split_into(PartSize::Bytes(20));
        assert_eq!(request.parts, Some(PartSize::Bytes(20)));
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""parts":{"bytes":20}"#), "{json}");
        let back: JobRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, request);
        let copy = JobRequest::new(JobId::new(5), JobKind::Copy, Vec::new());
        assert!(!serde_json::to_string(&copy).unwrap().contains("parts"));
        assert_eq!(PartSize::Bytes(20).parts_for(223), 12);
        assert_eq!(PartSize::Bytes(20).parts_for(240), 12, "no empty last part");
        assert_eq!(PartSize::Bytes(20).parts_for(0), 0);
        assert_eq!(PartSize::EqualParts(3).part_bytes(10), Some(4));
        assert_eq!(PartSize::EqualParts(3).parts_for(10), 3);
        assert_eq!(
            PartSize::EqualParts(5).parts_for(2),
            2,
            "never more parts than bytes"
        );
        assert_eq!(PartSize::EqualParts(0).part_bytes(10), None);
        assert_eq!(PartSize::Bytes(0).parts_for(10), 0);
    }

    /// The queue's vocabulary (R25 commit 65): every state has its word,
    /// each fits 40 cells after the longest progress word, and the states
    /// that hold no slot are the waiting and finished ones.
    #[test]
    fn every_state_has_a_word_that_fits_40_cells() {
        assert_eq!(JobState::ALL.len(), 9);
        let mut words = std::collections::HashSet::new();
        for state in JobState::ALL {
            let word = state.word();
            assert!(words.insert(word), "two states say {word}");
            // A list row says the state first, then the job: the longest
            // word leaves a third of the line for the verb and the target.
            assert!(word.chars().count() <= 26, "{word}");
            assert!(
                crate::format::within_cells(&format!("{word}: copy 12 items"), word),
                "{word}"
            );
            let json = serde_json::to_string(state).unwrap();
            assert_eq!(serde_json::from_str::<JobState>(&json).unwrap(), *state);
        }
        assert_eq!(JobState::Queued.word(), "waiting its turn");
        assert_eq!(
            JobState::WaitingForConnection.word(),
            "waiting for the connection"
        );
        assert_eq!(
            serde_json::to_string(&JobState::WaitingForConnection).unwrap(),
            "\"waiting-for-connection\""
        );
        assert!(JobState::Queued.holds_no_slot());
        assert!(JobState::WaitingForAnswer.holds_no_slot());
        assert!(!JobState::WaitingForConnection.holds_no_slot());
        assert!(!JobState::Running.holds_no_slot());
        assert_eq!(JobState::default(), JobState::Counting);
    }

    /// The error policy travels in the request, asking by default; a
    /// request written before it reads as asking and writes nothing new.
    #[test]
    fn the_error_policy_travels_and_old_requests_ask() {
        let old = r#"{"id":3,"kind":"copy","sources":[],"destination":null,"new_name":null,"conflicts":"ask"}"#;
        let read: JobRequest = serde_json::from_str(old).unwrap();
        assert_eq!(read.errors, ErrorPolicy::Ask);
        let written = serde_json::to_string(&read).unwrap();
        assert!(!written.contains("errors"), "{written}");
        let unattended = JobRequest::new(JobId::new(4), JobKind::Copy, Vec::new())
            .on_errors(ErrorPolicy::ContinueAndList);
        let json = serde_json::to_string(&unattended).unwrap();
        assert!(json.contains(r#""errors":"continue-and-list""#), "{json}");
        assert_eq!(
            serde_json::from_str::<JobRequest>(&json).unwrap(),
            unattended
        );
        for policy in ErrorPolicy::ALL {
            assert!(policy.label().chars().count() <= 40, "{policy:?}");
            let json = serde_json::to_string(policy).unwrap();
            assert_eq!(serde_json::from_str::<ErrorPolicy>(&json).unwrap(), *policy);
        }
        assert_eq!(ErrorPolicy::StopAtFirst.label(), "Stop at the first error");
    }

    #[test]
    fn answers_say_whether_they_apply_to_the_rest() {
        assert_eq!(ConflictAnswer::skip().action, ConflictAction::Skip);
        assert!(ConflictAnswer::for_all(ConflictAction::Replace).apply_to_rest);
        assert!(!ConflictAnswer::once(ConflictAction::KeepBoth).apply_to_rest);
    }
}
