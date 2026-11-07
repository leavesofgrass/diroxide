//! Readers: each takes some files and reads their fields in one cost
//! class. The joint between this crate and the preview crate: the preview
//! crate's image words and ffprobe readers implement [`Reader`] (R30
//! commits 33 and 35), and the pipeline's body adds the guard and the
//! cost classes without changing the trait.

use std::sync::Arc;
use std::time::Duration;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{CancelToken, ReadSeek, ToolRunner, VfsError};

use crate::field::Field;

/// How costly a reader is: the three loading modes of the custom columns
/// (R28 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cost {
    /// From the listing or one small read: shown with the row.
    Fast,
    /// Filled after the list appears, visible rows first.
    Background,
    /// Only when asked: a key, Say Details, a column sorted by it.
    OnDemand,
}

impl Cost {
    /// The class in words, for Settings and the help: "fast",
    /// "background", "on demand".
    pub const fn words(self) -> &'static str {
        match self {
            Cost::Fast => "fast",
            Cost::Background => "background",
            Cost::OnDemand => "on demand",
        }
    }
}

/// What a reader is asked about: the file, its entry, what its type
/// check found, and what a reader that starts a program needs: the
/// asking read's cancel token and the runner of helper programs (MA's
/// request 1, VL's 1).
#[derive(Clone, Copy)]
pub struct ReadRequest<'a> {
    /// The file's place.
    pub place: &'a Place,
    /// Its entry, as listed.
    pub entry: &'a Entry,
    /// Its type's id, when checked: `jpeg`.
    pub type_id: Option<&'a str>,
    /// Its group, when known: `images`.
    pub group: Option<&'a str>,
    /// The token that cancels the read that asks, when it has one: a
    /// reader that runs a program stops it when the token is set. The
    /// pipeline gives every reader its read's token.
    pub cancel: Option<&'a CancelToken>,
    /// The runner of helper programs the asking read goes through (the
    /// app's tools registry, a test's double), when it gives one: a
    /// reader that runs a program uses it before any runner of its own.
    /// An `Arc`, so the pipeline can hand it to the reader's thread.
    pub tools: Option<&'a Arc<dyn ToolRunner>>,
}

impl std::fmt::Debug for ReadRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadRequest")
            .field("place", &self.place)
            .field("entry", &self.entry)
            .field("type_id", &self.type_id)
            .field("group", &self.group)
            .field("cancel", &self.cancel)
            .field("tools", &self.tools.map(|_| "a runner"))
            .finish()
    }
}

impl<'a> ReadRequest<'a> {
    /// A request for a file with nothing else known: no type, no group,
    /// no cancel token and no runner.
    pub fn new(place: &'a Place, entry: &'a Entry) -> Self {
        Self {
            place,
            entry,
            type_id: None,
            group: None,
            cancel: None,
            tools: None,
        }
    }
}

/// The limits a read is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    /// The most bytes a reader may read of one file.
    pub max_bytes: u64,
    /// How long one read may take.
    pub deadline: Duration,
}

impl Default for ReadLimits {
    /// 64 MiB and two seconds, the quick view's container cap and its
    /// deadline (R28 2.3).
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            deadline: Duration::from_secs(2),
        }
    }
}

/// Why a reader read nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetaError {
    /// The file is larger than the read may take.
    #[error("it is too large to read here")]
    TooLarge,
    /// The file is not what the reader reads, or is damaged, with why.
    #[error("{0}")]
    Unreadable(String),
    /// The read passed its deadline.
    #[error("it took too long to read")]
    TooSlow,
    /// The read could not reach the file.
    #[error(transparent)]
    Place(#[from] VfsError),
    /// The reader panicked or its thread could not start; the guard kept
    /// the program running.
    #[error("the reader failed")]
    ReaderFailed,
    /// The guard already runs its most readers, some of them left past
    /// their deadline: refused at once rather than starting another.
    #[error("busy with an earlier file")]
    Busy,
}

impl MetaError {
    /// A file the reader cannot read, with why in words.
    pub fn unreadable(why: impl Into<String>) -> Self {
        MetaError::Unreadable(why.into())
    }

    /// An I/O error from a reader's source, in words: a short read is a
    /// damaged or cut-off file.
    pub fn from_io(error: &std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::UnexpectedEof => MetaError::unreadable("the file ends too soon"),
            std::io::ErrorKind::InvalidData => MetaError::unreadable("the file is damaged"),
            _ => MetaError::unreadable(error.to_string().to_lowercase()),
        }
    }
}

/// A reader of one kind of file's fields.
pub trait Reader: Send + Sync {
    /// A stable id: `image`, `audio`, `pdf`.
    fn id(&self) -> &'static str;

    /// Its cost class.
    fn cost(&self) -> Cost;

    /// The keys of the fields it can give, for the column chooser and for
    /// a cell's words when it fails ("unreadable: REASON" for its keys,
    /// "not available" for the rest). Empty by default: such a reader's
    /// failure names no column.
    fn keys(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether it reads this file, by its type, group or name, without
    /// reading the file.
    fn accepts(&self, request: &ReadRequest<'_>) -> bool;

    /// Reads the file's fields from `source`, within `limits`.
    fn read(
        &self,
        source: &mut dyn ReadSeek,
        request: &ReadRequest<'_>,
        limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError>;
}
