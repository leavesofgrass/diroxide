//! The vocabulary of jobs: what kind of operation, on what, and what to do
//! when a name is taken. The job engine (`diroxide-ops`) plans and runs
//! them; the app asks for them through [`Effect::StartJob`](crate::Effect).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::JobId;
use crate::place::Place;

/// What a job does.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobKind {
    /// Copy the sources into the destination folder.
    Copy,
    /// Move the sources into the destination folder.
    Move,
    /// Give one source a new name in the same folder.
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
    /// Convert the sources with a recipe.
    Convert,
    /// Add up the sizes of folders.
    Dirsize,
    /// Undo the last job's renames and moves.
    Undo,
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
            JobKind::Convert => "convert",
            JobKind::Dirsize => "measure",
            JobKind::Undo => "undo",
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
            JobKind::Convert => "Converting",
            JobKind::Dirsize => "Measuring",
            JobKind::Undo => "Undoing",
        }
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.verb())
    }
}

/// What to do when the destination name is taken. Asked once per job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPolicy {
    /// Ask the user, once, and apply the answer to the rest of the job.
    #[default]
    Ask,
    /// Leave the existing entry and skip the source.
    Skip,
    /// Replace the existing entry.
    Overwrite,
    /// Keep both, giving the new one a numbered name.
    KeepBoth,
    /// Ask for a new name for each conflict.
    Rename,
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
    /// What to do when a name is taken.
    pub conflicts: ConflictPolicy,
}

impl JobRequest {
    /// A request with sources, no destination, no new name, and the
    /// default conflict policy.
    pub fn new(id: JobId, kind: JobKind, sources: Vec<Place>) -> Self {
        Self {
            id,
            kind,
            sources,
            destination: None,
            new_name: None,
            conflicts: ConflictPolicy::default(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_for_messages() {
        assert_eq!(JobKind::Trash.verb(), "move to the trash");
        assert_eq!(JobKind::Copy.progress_word(), "Copying");
        assert_eq!(ConflictPolicy::default(), ConflictPolicy::Ask);
        let request = JobRequest::new(JobId::new(1), JobKind::Rename, Vec::new()).named("b");
        assert_eq!(request.new_name.as_deref(), Some("b"));
    }
}
