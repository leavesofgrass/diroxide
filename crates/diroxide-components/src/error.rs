//! What can go wrong with a component, in words.
//!
//! The `Display` text is the cause, lowercase, for the house form "Could
//! not download diro-examples: CAUSE. NEXT." ([`diroxide_core::Message`]);
//! [`crate::messages`] turns the common ones into the short lines of the
//! screen-reader path (R22 section 9), meaning first.

use std::path::Path;

use diroxide_core::Explain;
use diroxide_core::format::human_size;
use thiserror::Error;

/// Why a component could not be listed, fetched, installed or removed.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ComponentError {
    /// No component has this id.
    #[error("no component is called {0}")]
    NotKnown(String),
    /// A name in a list or a request is not a plain name.
    #[error("the name {0} is not a plain name")]
    BadName(String),
    /// The list itself is wrong; the words say where.
    #[error("{0}")]
    Manifest(String),
    /// The owner was not asked, or said yes to another component.
    #[error("the download of {0} was not agreed to")]
    NotAsked(String),
    /// Another download or install of the same component is running.
    #[error("{0} is already being downloaded")]
    Busy(String),
    /// The owner stopped it; what arrived is kept for the next try.
    #[error("it was stopped")]
    Cancelled,
    /// The component has no files for this computer.
    #[error("there is no build of {0} for this computer")]
    NoBuild(String),
    /// Nowhere to fetch a file from.
    #[error("there is no address for {file}")]
    NoSource {
        /// The file's name.
        file: String,
    },
    /// No program can fetch over the network in this build.
    #[error("curl was not found")]
    NoTransport,
    /// A fetch failed; the reason is in words.
    #[error("{file}: {reason}")]
    Fetch {
        /// The file's name.
        file: String,
        /// Why, in words.
        reason: String,
    },
    /// A file arrived with the wrong size.
    #[error("{file} is {} where {} were expected", human_size(*got), human_size(*expected))]
    Size {
        /// The file's name.
        file: String,
        /// The bytes that arrived.
        got: u64,
        /// The pinned size.
        expected: u64,
    },
    /// A file arrived with the wrong hash.
    #[error("{file} did not match its pin")]
    Hash {
        /// The file's name.
        file: String,
    },
    /// Not enough room on the volume for the download and its unpacking.
    #[error("it needs {} and {} is free", human_size(*needed), human_size(*free))]
    NoRoom {
        /// Bytes needed.
        needed: u64,
        /// Bytes free.
        free: u64,
    },
    /// An archive could not be unpacked, or went past its limits.
    #[error("{0}")]
    Unpack(String),
    /// A file or folder could not be read or written.
    #[error("{path}: {reason}")]
    Io {
        /// The full path.
        path: String,
        /// The system's words.
        reason: String,
    },
}

impl ComponentError {
    /// An I/O error at `path`.
    pub fn io(path: &Path, error: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        }
    }
}

impl Explain for ComponentError {
    fn next_step(&self) -> String {
        match self {
            ComponentError::NotKnown(_) => "Run diro components list for the ids.".to_owned(),
            ComponentError::NotAsked(_) => "Answer the question first.".to_owned(),
            ComponentError::Busy(_) => "Wait for it to finish.".to_owned(),
            ComponentError::Cancelled => "It resumes from where it stopped.".to_owned(),
            ComponentError::NoTransport => "Install curl, or install from a file.".to_owned(),
            ComponentError::Hash { .. } | ComponentError::Size { .. } => {
                "Get it again, or check the source.".to_owned()
            }
            ComponentError::NoRoom { .. } => "Make room, then try again.".to_owned(),
            ComponentError::Fetch { .. } | ComponentError::NoSource { .. } => {
                "Try again later, or install from a file.".to_owned()
            }
            ComponentError::NoBuild(_) => "Install it from your system's packages.".to_owned(),
            ComponentError::BadName(_)
            | ComponentError::Manifest(_)
            | ComponentError::Unpack(_)
            | ComponentError::Io { .. } => String::new(),
        }
    }
}
