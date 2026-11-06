//! What can go wrong reading an image, and how each becomes the host's
//! error: a kind the host knows, and a cause in lowercase words that the
//! host puts after "Could not open fedora.iso:".
//!
//! The host keeps the plugin's words only for its catch-all kinds
//! (`failed`), so every worded cause travels as `failed`, with its next
//! step in `data.hint`; the kinds whose meaning the host already words
//! (not found, not a folder, changed, cancelled) go as themselves.

use std::fmt;
use std::io;

use diroxide_plugins::protocol::{ErrorKind, RpcError};

use crate::lines::cause;

/// Why a request about an image failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskError {
    /// The name is not in the image.
    NotFound,
    /// A folder was needed.
    NotAFolder,
    /// A file was needed.
    IsAFolder,
    /// The image or a part of it needs something not here, or uses
    /// something not read: the cause in words, and the next step.
    Needs {
        /// The cause, lowercase: "it needs 7-Zip to read exFAT."
        cause: String,
        /// The next step, a sentence, or empty.
        hint: String,
    },
    /// A header or table does not add up: the cause in words.
    Damaged(String),
    /// A guard refused: the cause in words.
    Refused(String),
    /// The image changed since it was opened.
    Changed,
    /// The request was cancelled.
    Cancelled,
    /// Reading the image failed: the cause in words.
    Io(String),
    /// The host's own answer to a call the plugin made, kept whole.
    Host(RpcError),
}

impl DiskError {
    /// A damaged structure: "its GPT is damaged: WHY".
    pub fn damaged(what: &str, why: impl fmt::Display) -> Self {
        DiskError::Damaged(cause::damaged(what, &why.to_string()))
    }

    /// Something the plugin does not read, with no next step.
    pub fn uses(what: &str) -> Self {
        DiskError::Needs {
            cause: cause::uses(what),
            hint: String::new(),
        }
    }

    /// Something only 7-Zip reads.
    pub fn needs_7zip(what: &str) -> Self {
        DiskError::Needs {
            cause: cause::needs_7zip(what),
            hint: cause::GET_7ZIP.to_owned(),
        }
    }

    /// The error as the plugin's answer to the host.
    pub fn to_rpc(&self) -> RpcError {
        match self {
            DiskError::NotFound => RpcError::new(ErrorKind::NotFound, "it is not in the image"),
            DiskError::NotAFolder => RpcError::new(ErrorKind::NotADirectory, "it is not a folder"),
            DiskError::IsAFolder => RpcError::new(ErrorKind::IsADirectory, "it is a folder"),
            DiskError::Needs { cause, hint } if !hint.is_empty() => {
                RpcError::with_hint(ErrorKind::Failed, cause.clone(), hint.clone())
            }
            DiskError::Needs { cause, .. } => RpcError::new(ErrorKind::Failed, cause.clone()),
            DiskError::Damaged(why) | DiskError::Refused(why) => {
                RpcError::new(ErrorKind::Failed, why.clone())
            }
            DiskError::Changed => RpcError::new(ErrorKind::Conflict, cause::CHANGED),
            DiskError::Cancelled => {
                RpcError::new(ErrorKind::RequestCancelled, "the request was cancelled")
            }
            DiskError::Io(why) => RpcError::new(ErrorKind::Failed, cause::read_failed(why)),
            DiskError::Host(error) => error.clone(),
        }
    }

    /// An I/O error from a reader crate: a cancel stays a cancel, a host
    /// error inside it stays the host's, anything else is "the image could
    /// not be read".
    pub fn from_io(error: &io::Error) -> Self {
        if let Some(inner) = error.get_ref().and_then(|e| e.downcast_ref::<HostIo>()) {
            return inner.0.clone();
        }
        match error.kind() {
            io::ErrorKind::Interrupted => DiskError::Cancelled,
            io::ErrorKind::UnexpectedEof => {
                DiskError::Damaged(cause::damaged("data", "it ends before its end"))
            }
            _ => DiskError::Io(lowercase_first(&error.to_string())),
        }
    }
}

impl fmt::Display for DiskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rpc().message)
    }
}

impl std::error::Error for DiskError {}

impl From<RpcError> for DiskError {
    fn from(error: RpcError) -> Self {
        match error.kind() {
            Some(ErrorKind::RequestCancelled) => DiskError::Cancelled,
            _ => DiskError::Host(error),
        }
    }
}

/// A plugin error carried through a reader crate's `std::io::Error`, so a
/// cancel or the host's refusal comes out of the crate as itself.
#[derive(Debug)]
pub struct HostIo(pub DiskError);

impl fmt::Display for HostIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for HostIo {}

impl HostIo {
    /// The error wrapped for a reader crate.
    pub fn into_io(error: DiskError) -> io::Error {
        let kind = match error {
            DiskError::Cancelled => io::ErrorKind::Interrupted,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, HostIo(error))
    }
}

/// Text with its first letter made lowercase, for a cause after a colon.
pub fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first)
            if first.is_uppercase() && !chars.clone().next().is_some_and(char::is_uppercase) =>
        {
            first.to_lowercase().chain(chars).collect()
        }
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worded_causes_travel_as_failed_with_their_next_step() {
        let error = DiskError::needs_7zip("exFAT").to_rpc();
        assert_eq!(error.kind(), Some(ErrorKind::Failed));
        assert_eq!(error.message, "it needs 7-Zip to read exFAT.");
        assert_eq!(error.hint(), Some(cause::GET_7ZIP));
        assert_eq!(
            DiskError::NotFound.to_rpc().kind(),
            Some(ErrorKind::NotFound)
        );
        assert_eq!(
            DiskError::Changed.to_rpc().kind(),
            Some(ErrorKind::Conflict)
        );
        assert_eq!(
            DiskError::Io("Timed out".into()).to_rpc().message,
            "reading failed: Timed out"
        );
    }

    #[test]
    fn an_error_through_a_crate_comes_out_as_itself() {
        let io = HostIo::into_io(DiskError::Cancelled);
        assert_eq!(DiskError::from_io(&io), DiskError::Cancelled);
        let io = HostIo::into_io(DiskError::Changed);
        assert_eq!(DiskError::from_io(&io), DiskError::Changed);
        let plain = io::Error::other("Bad sector");
        assert_eq!(
            DiskError::from_io(&plain),
            DiskError::Io("bad sector".into())
        );
        assert_eq!(lowercase_first("NTFS index"), "NTFS index");
    }
}
