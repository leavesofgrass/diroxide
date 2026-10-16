// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Errors from reading and resolving theme files. Every message names the
//! key or line at fault and reads well aloud.

use std::path::PathBuf;

/// Why a theme could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    /// The file could not be read.
    #[error("could not read {}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid TOML.
    #[error("not valid TOML{}: {message}", at_line(line))]
    Syntax {
        /// 1-based line of the error, when known.
        line: Option<usize>,
        /// What the TOML reader said.
        message: String,
    },
    /// A key has a value of the wrong kind or out of range.
    #[error("{key}: {message}")]
    Invalid {
        /// Dotted key, for example `styles.find_hit.background`.
        key: String,
        /// What is wrong, and how to fix it.
        message: String,
    },
    /// A required key is missing.
    #[error("{0} is missing; add it, or set theme.inherits to a built-in theme")]
    Missing(String),
    /// `inherits` names a theme that does not exist.
    #[error("theme.inherits names {0}, which is not a built-in theme")]
    UnknownBase(String),
    /// The file is too large to be a theme (over 256 KiB).
    #[error("{} is too large to be a theme file", .0.display())]
    TooLarge(PathBuf),
}

fn at_line(line: &Option<usize>) -> String {
    line.map(|l| format!(" at line {l}")).unwrap_or_default()
}

impl ThemeError {
    pub(crate) fn invalid(key: impl Into<String>, message: impl Into<String>) -> Self {
        ThemeError::Invalid {
            key: key.into(),
            message: message.into(),
        }
    }
}
