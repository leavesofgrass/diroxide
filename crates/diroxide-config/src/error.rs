//! Errors of the configuration.

use std::io;

use diroxide_core::Explain;
use thiserror::Error;

/// Why a configuration file could not be read or written. The text is the
/// cause, naming the file first.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The file is not valid TOML, or a value has the wrong type.
    #[error("{file} could not be read: {message}")]
    Parse {
        /// The file's name, such as `keymap.toml`.
        file: String,
        /// What is wrong, with the line when known.
        message: String,
    },
    /// A key, command id or layer name is not valid.
    #[error("{file} has a bad entry: {message}")]
    BadEntry {
        /// The file's name.
        file: String,
        /// The entry and what is wrong with it.
        message: String,
    },
    /// The file could not be read or written.
    #[error("{file} could not be opened or saved: {message}")]
    Io {
        /// The file's name.
        file: String,
        /// The system's words.
        message: String,
    },
}

impl ConfigError {
    pub(crate) fn io(file: &str, error: &io::Error) -> Self {
        Self::Io {
            file: file.to_owned(),
            message: error.to_string(),
        }
    }

    pub(crate) fn parse(file: &str, message: impl ToString) -> Self {
        Self::Parse {
            file: file.to_owned(),
            message: message.to_string().trim().to_owned(),
        }
    }

    pub(crate) fn bad_entry(file: &str, message: impl Into<String>) -> Self {
        Self::BadEntry {
            file: file.to_owned(),
            message: message.into(),
        }
    }

    /// The same error with the file's name filled in.
    pub fn in_file(self, name: &str) -> Self {
        match self {
            Self::Parse { message, .. } => Self::Parse {
                file: name.to_owned(),
                message,
            },
            Self::BadEntry { message, .. } => Self::BadEntry {
                file: name.to_owned(),
                message,
            },
            Self::Io { message, .. } => Self::Io {
                file: name.to_owned(),
                message,
            },
        }
    }
}

impl Explain for ConfigError {
    fn next_step(&self) -> String {
        match self {
            ConfigError::Parse { .. } | ConfigError::BadEntry { .. } => {
                "Fix the file, or rename it to start from the defaults.".to_owned()
            }
            ConfigError::Io { .. } => "Check that the config folder can be written.".to_owned(),
        }
    }
}
