//! SFTP failures as the provider's errors (R24 section 2.6): a status
//! code, a lost session, a time limit or a cancel becomes one of the
//! protocol's error kinds, with a sentence that puts the cause first and
//! fits one 40-cell Braille line.
//!
//! The server's own words are never the sentence: they are cleaned, cut
//! and placed last, in the hint, after "Server said:".

use diroxide_plugins::protocol::{ErrorKind, RpcError};

use super::codec::status;
use super::mux::SftpError;
use crate::lines;

/// The most of a server's words a hint carries.
pub const SERVER_WORDS: usize = 120;

/// What the plugin was doing when the server answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// Opening the session.
    Connect,
    /// Listing a folder.
    List,
    /// Reading an entry's details.
    Stat,
    /// Reading a file.
    Read,
    /// Making a file (OPEN with CREAT).
    Create,
    /// Writing a file.
    Write,
    /// Closing a written file.
    Close,
    /// Making a folder.
    MakeFolder,
    /// Removing a folder.
    RemoveFolder,
    /// Removing a file or a link.
    Remove,
    /// Renaming.
    Rename,
    /// Changing attributes.
    SetAttributes,
    /// Making a link.
    Link,
    /// Asking the free space.
    Space,
    /// Copying on the server.
    Copy,
    /// Flushing a written file.
    Sync,
}

impl Op {
    /// What failed, in words, for "Could not ... on the server.".
    pub const fn words(self) -> &'static str {
        match self {
            Op::Connect => "start SFTP",
            Op::List => "list the folder",
            Op::Stat => "read its details",
            Op::Read => "read the file",
            Op::Create => "make the file",
            Op::Write => "write the file",
            Op::Close => "finish the file",
            Op::MakeFolder => "make the folder",
            Op::RemoveFolder => "remove the folder",
            Op::Remove => "remove it",
            Op::Rename => "rename it",
            Op::SetAttributes => "change it",
            Op::Link => "make the link",
            Op::Space => "read the free space",
            Op::Copy => "copy it",
            Op::Sync => "flush the file",
        }
    }
}

/// The server's words, cleaned and cut, for a hint: "Server said: ...";
/// `None` when it said nothing.
pub fn server_said(message: &[u8]) -> Option<String> {
    let words = lines::cleaned(&String::from_utf8_lossy(message), SERVER_WORDS);
    (!words.is_empty()).then(|| format!("Server said: {words}"))
}

fn with_server_words(kind: ErrorKind, sentence: String, message: &[u8]) -> RpcError {
    match server_said(message) {
        Some(hint) => RpcError::with_hint(kind, sentence, hint),
        None => RpcError::new(kind, sentence),
    }
}

/// The error for a status the server answered.
pub fn from_status(op: Op, code: u32, message: &[u8]) -> RpcError {
    match code {
        status::NO_SUCH_FILE => RpcError::new(ErrorKind::NotFound, lines::NOT_THERE),
        status::PERMISSION_DENIED => {
            with_server_words(ErrorKind::PermissionDenied, lines::denied(), message)
        }
        status::OP_UNSUPPORTED => RpcError::new(ErrorKind::Unsupported, lines::server_cannot()),
        status::BAD_MESSAGE => RpcError::new(ErrorKind::Failed, lines::not_understood()),
        _ => with_server_words(ErrorKind::Failed, lines::could_not(op.words()), message),
    }
}

/// The error for any SFTP failure on a connection to `host`.
pub fn map(op: Op, error: &SftpError, host: &str) -> RpcError {
    match error {
        SftpError::Status { code, message } => from_status(op, *code, message),
        SftpError::Lost(_) => RpcError::with_hint(
            ErrorKind::Unavailable,
            lines::connection_lost(host),
            lines::OPEN_TO_RECONNECT,
        ),
        SftpError::TimedOut => RpcError::new(ErrorKind::TimedOut, lines::no_answer(host)),
        SftpError::Cancelled => RpcError::new(ErrorKind::RequestCancelled, lines::CANCELLED),
        SftpError::ShellChatter(_) => RpcError::with_hint(
            ErrorKind::Unavailable,
            lines::SHELL_PRINTED,
            lines::SHELL_PRINTED_HINT,
        ),
        SftpError::Protocol(_) => RpcError::new(ErrorKind::Unavailable, lines::NOT_SFTP),
    }
}

/// Whether a failure means the session is gone.
pub fn is_lost(error: &SftpError) -> bool {
    matches!(error, SftpError::Lost(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_kinds() {
        assert_eq!(
            from_status(Op::Stat, status::NO_SUCH_FILE, b"No such file").kind(),
            Some(ErrorKind::NotFound)
        );
        assert_eq!(
            from_status(Op::Read, status::PERMISSION_DENIED, b"").kind(),
            Some(ErrorKind::PermissionDenied)
        );
        assert_eq!(
            from_status(Op::Link, status::OP_UNSUPPORTED, b"").kind(),
            Some(ErrorKind::Unsupported)
        );
        let failed = from_status(Op::Write, status::FAILURE, b"Odd \x1b[2Jthing");
        assert_eq!(failed.kind(), Some(ErrorKind::Failed));
        assert_eq!(failed.message, "Could not write the file.");
        assert_eq!(failed.hint(), Some("Server said: Odd thing"));
    }

    #[test]
    fn a_lost_session_says_the_host_first_line() {
        let lost = map(Op::List, &SftpError::Lost("gone".into()), "build.example");
        assert_eq!(lost.kind(), Some(ErrorKind::Unavailable));
        assert_eq!(lost.message, "Connection lost: build.example.");
        assert!(is_lost(&SftpError::Lost(String::new())));
        let late = map(Op::Read, &SftpError::TimedOut, "build.example");
        assert_eq!(late.kind(), Some(ErrorKind::TimedOut));
        assert_eq!(late.message, "No answer from build.example.");
    }

    #[test]
    fn every_sentence_fits_one_braille_line() {
        for op in [
            Op::Connect,
            Op::List,
            Op::Stat,
            Op::Read,
            Op::Create,
            Op::Write,
            Op::Close,
            Op::MakeFolder,
            Op::RemoveFolder,
            Op::Remove,
            Op::Rename,
            Op::SetAttributes,
            Op::Link,
            Op::Space,
            Op::Copy,
            Op::Sync,
        ] {
            for code in [0, 1, 2, 3, 4, 5, 8, 99] {
                let error = from_status(op, code, b"");
                assert!(error.message.chars().count() <= 40, "{}", error.message);
            }
        }
    }
}
