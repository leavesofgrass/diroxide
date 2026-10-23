//! The SFTP operations the provider is built from (R24 section 2.5,
//! `ops`), each one request or a short pipelined run of them on a
//! [`Session`]: paths and names are bytes throughout (answer 88).

use std::time::Duration;

use super::codec::{Attrs, NameEntry, Request, Response, status};
use super::mux::{SftpError, Ticket};
use super::session::Session;
use crate::rpc::Cancel;

/// How long one request of a pipelined run may wait.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// The status code and words of an answer, when it is a status.
pub fn status_of(response: &Response) -> Option<(u32, &[u8])> {
    match response {
        Response::Status { code, message, .. } => Some((*code, message.as_slice())),
        _ => None,
    }
}

/// An answer that was not the one wanted, as an error.
pub fn refused(response: Response) -> SftpError {
    match response {
        Response::Status { code, message, .. } => SftpError::Status { code, message },
        other => SftpError::Protocol(format!("an unexpected {} answer", other.name())),
    }
}

/// OK, or the status as an error.
pub fn expect_ok(response: Response) -> Result<(), SftpError> {
    match status_of(&response) {
        Some((status::OK, _)) => Ok(()),
        _ => Err(refused(response)),
    }
}

/// Whether an error is a status with this code.
pub fn is_status(error: &SftpError, code: u32) -> bool {
    matches!(error, SftpError::Status { code: c, .. } if *c == code)
}

/// A path's attributes, a link itself rather than what it points to.
pub fn lstat(session: &Session, path: &[u8], cancel: Option<&Cancel>) -> Result<Attrs, SftpError> {
    let path = path.to_vec();
    match session.call(|id| Request::Lstat { id, path }, cancel)? {
        Response::Attrs { attrs, .. } => Ok(attrs),
        other => Err(refused(other)),
    }
}

/// A path's attributes, following links when `follow` is set.
pub fn attributes(
    session: &Session,
    path: &[u8],
    follow: bool,
    cancel: Option<&Cancel>,
) -> Result<Attrs, SftpError> {
    if follow {
        session.stat(path, cancel)
    } else {
        lstat(session, path, cancel)
    }
}

/// Opens a folder for reading.
pub fn opendir(
    session: &Session,
    path: &[u8],
    cancel: Option<&Cancel>,
) -> Result<Vec<u8>, SftpError> {
    let path = path.to_vec();
    match session.call(|id| Request::Opendir { id, path }, cancel)? {
        Response::Handle { handle, .. } => Ok(handle),
        other => Err(refused(other)),
    }
}

/// The next names of an open folder; `None` at its end.
pub fn readdir(
    session: &Session,
    handle: &[u8],
    cancel: Option<&Cancel>,
) -> Result<Option<Vec<NameEntry>>, SftpError> {
    let handle = handle.to_vec();
    match session.call(|id| Request::Readdir { id, handle }, cancel)? {
        Response::Name { entries, .. } => Ok(Some(entries)),
        Response::Status {
            code: status::EOF, ..
        } => Ok(None),
        other => Err(refused(other)),
    }
}

/// A link's target, as the server stores it.
pub fn readlink(
    session: &Session,
    path: &[u8],
    cancel: Option<&Cancel>,
) -> Result<Vec<u8>, SftpError> {
    let path = path.to_vec();
    match session.call(|id| Request::Readlink { id, path }, cancel)? {
        Response::Name { mut entries, .. } if !entries.is_empty() => {
            Ok(entries.swap_remove(0).filename)
        }
        other => Err(refused(other)),
    }
}

/// Many links' targets at once, the requests pipelined; a link whose
/// target cannot be read gives `None`.
pub fn readlinks(
    session: &Session,
    paths: &[Vec<u8>],
    cancel: Option<&Cancel>,
) -> Result<Vec<Option<Vec<u8>>>, SftpError> {
    let tickets: Vec<Ticket> = paths
        .iter()
        .map(|path| {
            let path = path.clone();
            session.mux().send(|id| Request::Readlink { id, path })
        })
        .collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(tickets.len());
    for ticket in tickets {
        match ticket.wait(cancel, RUN_TIMEOUT)? {
            Response::Name { mut entries, .. } if !entries.is_empty() => {
                out.push(Some(entries.swap_remove(0).filename));
            }
            _ => out.push(None),
        }
    }
    Ok(out)
}

/// A child's path: the folder's path, a slash, the name.
pub fn join(folder: &[u8], name: &[u8]) -> Vec<u8> {
    let mut path = folder.to_vec();
    if path.last() != Some(&b'/') {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_join_with_one_slash() {
        assert_eq!(join(b"/", b"a"), b"/a");
        assert_eq!(join(b"/srv", b"a"), b"/srv/a");
        assert_eq!(join(b"/srv/", b"\xff"), b"/srv/\xff");
    }

    #[test]
    fn statuses_are_told_apart() {
        let eof = Response::Status {
            id: 1,
            code: status::EOF,
            message: Vec::new(),
            language: Vec::new(),
        };
        assert_eq!(status_of(&eof), Some((status::EOF, &b""[..])));
        let error = refused(eof);
        assert!(is_status(&error, status::EOF));
        assert!(!is_status(&error, status::OK));
    }
}
