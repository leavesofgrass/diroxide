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
    /// Split a file into numbered parts (`name.001`, `name.002`).
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobState {
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
    /// password).
    WaitingForAnswer,
    /// Checking what was written against its source.
    Verifying,
    /// Done, cancelled or failed; the report says which.
    Finished,
}

impl JobState {
    /// The state in words, for the jobs line: "counting", "paused",
    /// "waiting for an answer".
    pub const fn word(self) -> &'static str {
        match self {
            JobState::Counting => "counting",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Stalled => "stalled",
            JobState::WaitingForAnswer => "waiting for an answer",
            JobState::Verifying => "verifying",
            JobState::Finished => "finished",
        }
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
}

impl JobRequest {
    /// A request with sources, no destination, no new name, the default
    /// conflict policy, and the user as its origin.
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
        }
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

    #[test]
    fn answers_say_whether_they_apply_to_the_rest() {
        assert_eq!(ConflictAnswer::skip().action, ConflictAction::Skip);
        assert!(ConflictAnswer::for_all(ConflictAction::Replace).apply_to_rest);
        assert!(!ConflictAnswer::once(ConflictAction::KeepBoth).apply_to_rest);
    }
}
