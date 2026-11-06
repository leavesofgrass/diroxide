//! Readers: each takes some files and reads their fields in one cost
//! class. The joint between this crate and the preview crate: the preview
//! crate's image words and ffprobe readers implement [`Reader`] (R30
//! commits 33 and 35), and the pipeline's body adds the guard and the
//! cost classes without changing the trait.

use std::time::Duration;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{ReadSeek, VfsError};

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

/// What a reader is asked about: the file, its entry, and what its type
/// check found.
#[derive(Clone, Copy, Debug)]
pub struct ReadRequest<'a> {
    /// The file's place.
    pub place: &'a Place,
    /// Its entry, as listed.
    pub entry: &'a Entry,
    /// Its type's id, when checked: `jpeg`.
    pub type_id: Option<&'a str>,
    /// Its group, when known: `images`.
    pub group: Option<&'a str>,
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
}

/// A reader of one kind of file's fields.
pub trait Reader: Send + Sync {
    /// A stable id: `image`, `audio`, `pdf`.
    fn id(&self) -> &'static str;

    /// Its cost class.
    fn cost(&self) -> Cost;

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
