//! What can go wrong, as the host hears it: each error is one short cause,
//! meaning first, for the host's "Could not VERB OBJECT: CAUSE." line, with
//! the next step as the error's hint where there is one.

use std::fmt;

use diroxide_plugins::protocol::{ErrorKind, RpcError};

use crate::git::version::Version;
use crate::lines;

/// A failure of the plugin's work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitError {
    /// The host found no git program.
    NotFound,
    /// The git found is older than the plugin needs.
    TooOld(Version),
    /// The folder is in no git repository.
    NotARepository,
    /// The repository's configuration runs a program and the owner has not
    /// trusted it.
    NotTrusted,
    /// The owner cancelled, or the host cancelled the request.
    Cancelled,
    /// git ran and refused: its reason in words, and the next step when
    /// the plugin knows one.
    Refused {
        /// The cause, meaning first.
        cause: String,
        /// The next step, a sentence of its own.
        hint: Option<String>,
    },
    /// A call to the host failed.
    Host(RpcError),
    /// Something the plugin needed was missing or could not be read.
    Bad(String),
}

impl GitError {
    /// git refused, in its own words and the plugin's next step.
    pub fn refused(cause: impl Into<String>, hint: Option<&str>) -> Self {
        GitError::Refused {
            cause: cause.into(),
            hint: hint.map(str::to_owned),
        }
    }

    /// The error on the wire: its kind, its cause and its hint.
    pub fn to_rpc(&self) -> RpcError {
        match self {
            GitError::NotFound => {
                RpcError::with_hint(ErrorKind::NotFound, lines::GIT_NOT_FOUND, lines::GET_GIT)
            }
            GitError::TooOld(version) => RpcError::with_hint(
                ErrorKind::Unsupported,
                lines::too_old(*version),
                lines::GET_GIT,
            ),
            GitError::NotARepository => RpcError::new(ErrorKind::NotFound, lines::NOT_A_REPOSITORY),
            GitError::NotTrusted => {
                RpcError::with_hint(ErrorKind::Untrusted, lines::NOT_TRUSTED, lines::TRUST_HINT)
            }
            GitError::Cancelled => {
                RpcError::new(ErrorKind::RequestCancelled, "the request was cancelled")
            }
            GitError::Refused { cause, hint } => match hint {
                Some(hint) => RpcError::with_hint(ErrorKind::Failed, cause.clone(), hint.clone()),
                None => RpcError::new(ErrorKind::Failed, cause.clone()),
            },
            GitError::Host(error) => error.clone(),
            GitError::Bad(why) => RpcError::new(ErrorKind::InvalidParams, why.clone()),
        }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rpc().message)
    }
}

impl std::error::Error for GitError {}

impl From<RpcError> for GitError {
    fn from(error: RpcError) -> Self {
        if error.code == ErrorKind::RequestCancelled.code() {
            GitError::Cancelled
        } else {
            GitError::Host(error)
        }
    }
}
