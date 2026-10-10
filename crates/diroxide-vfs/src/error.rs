//! Errors of providers.

use std::io;

use diroxide_core::{Explain, PlaceError};
use thiserror::Error;

/// Why a provider operation failed. The text is the cause, in lowercase
/// words, for "Could not VERB OBJECT: CAUSE." messages.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VfsError {
    /// The operation was cancelled.
    #[error("it was cancelled")]
    Cancelled,
    /// The place does not exist.
    #[error("it does not exist")]
    NotFound,
    /// The system refused access.
    #[error("permission was denied")]
    PermissionDenied,
    /// The name is already taken.
    #[error("the name is already taken")]
    AlreadyExists,
    /// A folder that must be empty is not.
    #[error("the folder is not empty")]
    NotEmpty,
    /// A folder was expected and this is not one.
    #[error("it is not a folder")]
    NotAFolder,
    /// The provider cannot do this.
    #[error("the {provider} provider cannot {operation}")]
    Unsupported {
        /// The provider's scheme.
        provider: String,
        /// What was asked, as a verb phrase: "move to the trash".
        operation: String,
    },
    /// No provider is registered for the place.
    #[error("nothing is set up to open {0} places")]
    NoProvider(String),
    /// The place cannot be used on this system.
    #[error(transparent)]
    Place(#[from] PlaceError),
    /// The name is not valid Unicode, so it cannot be used safely yet.
    #[error("the name is not valid Unicode")]
    NameNotUnicode,
    /// Any other I/O failure, with the system's words.
    #[error("{message}")]
    Io {
        /// The kind of failure.
        kind: io::ErrorKind,
        /// The system's description, in lowercase.
        message: String,
    },
}

impl VfsError {
    /// An "unsupported" error for a provider and an operation.
    pub fn unsupported(provider: &str, operation: &str) -> Self {
        Self::Unsupported {
            provider: provider.to_owned(),
            operation: operation.to_owned(),
        }
    }
}

impl From<io::Error> for VfsError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            io::ErrorKind::DirectoryNotEmpty => Self::NotEmpty,
            io::ErrorKind::NotADirectory => Self::NotAFolder,
            kind => {
                let text = error.to_string();
                let mut chars = text.chars();
                let message = match chars.next() {
                    Some(first) => first.to_lowercase().chain(chars).collect(),
                    None => "an input or output error happened".to_owned(),
                };
                Self::Io { kind, message }
            }
        }
    }
}

impl Explain for VfsError {
    fn next_step(&self) -> String {
        match self {
            VfsError::Cancelled => String::new(),
            VfsError::NotFound => "Press F5 to reload the folder.".to_owned(),
            VfsError::PermissionDenied => {
                "Check the permissions, or use another folder.".to_owned()
            }
            VfsError::AlreadyExists => "Choose another name.".to_owned(),
            VfsError::NotEmpty => "Empty it first, or move it to the trash.".to_owned(),
            VfsError::NotAFolder => "Choose a folder.".to_owned(),
            VfsError::Unsupported { .. } => "Copy it to a local folder first.".to_owned(),
            VfsError::NoProvider(_) => "Check the address, or enable the plugin for it.".to_owned(),
            VfsError::Place(inner) => inner.next_step(),
            VfsError::NameNotUnicode => "Rename it with another program first.".to_owned(),
            VfsError::Io { .. } => "Try again; if it keeps failing, check the disk.".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::Message;

    #[test]
    fn io_errors_map_to_words() {
        let missing = VfsError::from(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(missing, VfsError::NotFound);
        let m = Message::from_error("open", "notes.txt", &missing);
        assert_eq!(
            m.text,
            "Could not open notes.txt: it does not exist. Press F5 to reload the folder."
        );
        let other = VfsError::from(io::Error::other("Disk on fire"));
        assert_eq!(other.to_string(), "disk on fire");
    }
}
