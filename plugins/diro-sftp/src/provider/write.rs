//! The write session (R25 commit 60; R24 section 2.3; R25 section 2.5):
//! `write_begin`, chunks, then `write_end`, which commits or leaves
//! nothing behind.
//!
//! - **The engine's own unfinished name.** The bytes go to a partial file
//!   beside the target named as every part of the program names one
//!   (`.diro-part-` and eight hexadecimal digits,
//!   `diroxide_vfs::names::unfinished_name`), so listings leave it out and
//!   the engine's journal knows it. Its digits come from the target's path,
//!   so a resume finds the same file.
//! - **Every WRITE reply is checked** (a later write can succeed after an
//!   earlier one failed), chunks must come in order, and before a commit:
//!   `fsync@openssh.com` when the server offers it, the size the server
//!   holds compared with the bytes sent, the CLOSE status checked (a quota
//!   error can arrive there).
//! - **The commit** renames the partial file over the target with
//!   `posix-rename@openssh.com` where offered, in one step. Without it,
//!   the old target is renamed aside to an unfinished name, the new file
//!   renamed in, and only then the old one removed (create, verify, then
//!   destroy); a failure in the middle puts the old one back. A write that
//!   must not replace uses plain RENAME, which never replaces.
//! - **Resume:** `resume` reopens the partial file and answers the offset
//!   it goes on from: its length, or the host's offset when it holds more
//!   (the rest is cut). No partial file, and it starts over at 0. The
//!   ledger and the checks of whether to resume are the engine's.
//! - **Abandon:** `commit` false closes the file and removes the partial
//!   one. A plugin killed in the middle, or a lost connection, leaves only
//!   the unfinished file, never a half-written target.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_plugins::base64;
use diroxide_plugins::protocol::{ErrorKind, RpcError};
use diroxide_vfs::names::unfinished_name;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::list::{NameCheck, check_name, entry};
use super::read::spool_file;
use super::{At, Call, Connection, Link};
use crate::lines;
use crate::rpc::Cancel;
use crate::sftp::codec::{Attrs, Request, Response, open, status, strings};
use crate::sftp::errors::Op;
use crate::sftp::mux::SftpError;
use crate::sftp::ops;

/// The most one chunk may carry.
pub const MAX_CHUNK: usize = 16 * 1024 * 1024;

/// The eight hexadecimal digits of a target's unfinished file: from its
/// path, so every write session to one target, and its resume, names the
/// same file. `salt` gives a second name for the old target set aside.
pub fn unfinished_token(target: &[u8], salt: &str) -> u32 {
    let mut hash = Sha256::new();
    hash.update(b"diro-sftp unfinished\0");
    hash.update(salt.as_bytes());
    hash.update(b"\0");
    hash.update(target);
    let digest = hash.finalize();
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
}

/// The unfinished file beside a target.
pub fn unfinished_path(folder: &[u8], target: &[u8], salt: &str) -> Vec<u8> {
    ops::join(
        folder,
        unfinished_name(unfinished_token(target, salt)).as_bytes(),
    )
}

/// One open write session.
struct Open {
    link: Arc<Link>,
    place: String,
    target: Vec<u8>,
    folder: Vec<u8>,
    partial: Vec<u8>,
    handle: Vec<u8>,
    next: u64,
    overwrite: bool,
}

/// The next write session's number, across every connection, so an id
/// names one session.
static NEXT_WRITE: AtomicU64 = AtomicU64::new(1);

/// A connection's open write sessions.
#[derive(Default)]
pub struct Writes {
    open: Mutex<HashMap<String, Arc<Mutex<Open>>>>,
}

impl std::fmt::Debug for Writes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Writes").field("open", &self.len()).finish()
    }
}

impl Writes {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Mutex<Open>>>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many sessions are open.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether none is open.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Whether a session with this id is open here.
    pub fn has(&self, id: &str) -> bool {
        self.lock().contains_key(id)
    }

    fn get(&self, id: &str) -> Result<Arc<Mutex<Open>>, RpcError> {
        self.lock().get(id).cloned().ok_or_else(no_session)
    }

    fn take(&self, id: &str) -> Result<Arc<Mutex<Open>>, RpcError> {
        self.lock().remove(id).ok_or_else(no_session)
    }

    /// Closes every session's handle, as the connection goes; the partial
    /// files stay for a resume.
    pub fn close_all(&self) {
        let all: Vec<Arc<Mutex<Open>>> = self.lock().drain().map(|(_, s)| s).collect();
        for session in all {
            let session = session.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = session.link.session().close(&session.handle);
        }
    }
}

/// A write session that is not open: its connection was lost or closed.
pub fn no_session() -> RpcError {
    RpcError::with_hint(
        ErrorKind::Unavailable,
        lines::WRITE_ENDED,
        lines::OPEN_TO_RECONNECT,
    )
}

/// Opens the partial file for a resume: its length, cut to the host's
/// offset when it holds more; `None` when there is none.
fn reopen(
    connection: &Arc<Connection>,
    link: &Link,
    partial: &[u8],
    asked: u64,
    call: &Call<'_>,
) -> Result<Option<(Vec<u8>, u64)>, RpcError> {
    let session = link.session();
    let cancel = Some(call.cancel);
    let handle = match session.open(partial, open::WRITE, Attrs::default(), cancel) {
        Ok(handle) => handle,
        Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) => return Ok(None),
        Err(error) => return Err(connection.fail(Op::Create, &error)),
    };
    let size = fstat_size(session, &handle, cancel).map_err(|e| connection.fail(Op::Write, &e))?;
    if size <= asked {
        return Ok(Some((handle, size)));
    }
    let cut = Attrs {
        size: Some(asked),
        ..Attrs::default()
    };
    let h = handle.clone();
    let answer = session
        .call(
            |id| Request::Fsetstat {
                id,
                handle: h,
                attrs: cut,
            },
            cancel,
        )
        .and_then(ops::expect_ok);
    match answer {
        Ok(()) => Ok(Some((handle, asked))),
        Err(error) => {
            let _ = session.close(&handle);
            Err(connection.fail(Op::Write, &error))
        }
    }
}

/// The size the server holds for an open file.
fn fstat_size(
    session: &crate::sftp::Session,
    handle: &[u8],
    cancel: Option<&Cancel>,
) -> Result<u64, SftpError> {
    let handle = handle.to_vec();
    match session.call(|id| Request::Fstat { id, handle }, cancel)? {
        Response::Attrs { attrs, .. } => attrs
            .size
            .ok_or_else(|| SftpError::Protocol("the server gave no size".to_owned())),
        other => Err(ops::refused(other)),
    }
}

/// `provider/write_begin`: checks the target and its folder, opens the
/// partial file (or reopens it for a resume), and answers the session's
/// id and, for a resume, the offset the next chunk goes to.
pub fn begin(connection: &Arc<Connection>, at: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    if at.path == b"/" || check_name(at.name()) != NameCheck::Keep {
        return Err(RpcError::new(
            ErrorKind::InvalidName,
            "The name cannot be written here.",
        ));
    }
    let overwrite = match call.text("conflict").unwrap_or("fail") {
        "fail" => false,
        "overwrite" => true,
        other => {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!("no conflict rule {other}: fail or overwrite"),
            ));
        }
    };
    let resume = call
        .params
        .get("resume")
        .and_then(|r| r.get("offset"))
        .and_then(Value::as_u64);
    let link = connection.transfer_link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    match ops::lstat(session, &at.path, cancel) {
        Ok(attrs) if super::list::kind_of(&attrs) == diroxide_core::EntryKind::Folder => {
            return Err(RpcError::new(ErrorKind::IsADirectory, lines::IS_A_FOLDER));
        }
        Ok(_) if !overwrite => {
            return Err(RpcError::new(ErrorKind::AlreadyExists, lines::NAME_TAKEN));
        }
        Ok(_) => {}
        Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) => {}
        Err(error) => return Err(connection.fail(Op::Stat, &error)),
    }
    let folder = at.parent_path();
    match session.stat(&folder, cancel) {
        Ok(attrs) if super::list::kind_of(&attrs) == diroxide_core::EntryKind::Folder => {}
        Ok(_) => return Err(RpcError::new(ErrorKind::NotADirectory, lines::NOT_A_FOLDER)),
        Err(error) => return Err(connection.fail(Op::Stat, &error)),
    }
    let partial = unfinished_path(&folder, &at.path, "");
    let writes = connection.writes();
    let busy = writes
        .lock()
        .values()
        .any(|s| s.lock().unwrap_or_else(PoisonError::into_inner).partial == partial);
    if busy {
        return Err(RpcError::new(ErrorKind::Conflict, lines::WRITE_RUNNING));
    }
    let reopened = match resume {
        Some(asked) => reopen(connection, &link, &partial, asked, call)?,
        None => None,
    };
    let (handle, next) = match reopened {
        Some(found) => found,
        None => {
            let handle = session
                .open(
                    &partial,
                    open::WRITE | open::CREAT | open::TRUNC,
                    Attrs::default(),
                    cancel,
                )
                .map_err(|e| connection.fail(Op::Create, &e))?;
            (handle, 0)
        }
    };
    let id = format!("w{}", NEXT_WRITE.fetch_add(1, Ordering::SeqCst));
    writes.lock().insert(
        id.clone(),
        Arc::new(Mutex::new(Open {
            link: Arc::clone(&link),
            place: at.place.to_string(),
            target: at.path.clone(),
            folder,
            partial: partial.clone(),
            handle,
            next,
            overwrite,
        })),
    );
    let mut answer = json!({"write_id": id});
    if resume.is_some() {
        answer["offset"] = json!(next);
    }
    if let Ok(name) = String::from_utf8(partial)
        && let Ok(unfinished) =
            At::parse(&format!("{}{name}", super::authority(connection.target())))
    {
        answer["unfinished"] = json!(unfinished.place.to_string());
    }
    Ok(answer)
}

/// `provider/write_chunk`: the next bytes, inline or from the spool, at
/// the offset the session expects; every WRITE answer is checked.
pub fn chunk(connection: &Arc<Connection>, call: &Call<'_>) -> Result<Value, RpcError> {
    let id = call.text("write_id").unwrap_or_default();
    let session = connection.writes().get(id)?;
    let mut session = session.lock().unwrap_or_else(PoisonError::into_inner);
    let offset = call.number("offset").unwrap_or(session.next);
    if offset != session.next {
        return Err(RpcError::with_hint(
            ErrorKind::Conflict,
            lines::OUT_OF_ORDER,
            format!("Expected byte {}, got {offset}.", session.next),
        ));
    }
    let data = match (call.text("data"), call.text("spool_path")) {
        (Some(text), _) => base64::decode(text)
            .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "the data is not base64"))?,
        (None, Some(spool_path)) => {
            let path = spool_file(call.state.spool_dir.as_deref(), spool_path)?;
            std::fs::read(&path).map_err(|e| {
                RpcError::new(
                    ErrorKind::InvalidParams,
                    format!("the spool file cannot be read: {e}"),
                )
            })?
        }
        (None, None) => Vec::new(),
    };
    if data.len() > MAX_CHUNK {
        return Err(RpcError::new(
            ErrorKind::TooLarge,
            "The chunk is too large.",
        ));
    }
    session
        .link
        .session()
        .write_pipelined(&session.handle, offset, &data, Some(call.cancel))
        .map_err(|e| connection.fail(Op::Write, &e))?;
    session.next += data.len() as u64;
    Ok(json!({"length": session.next}))
}

/// Removes the partial file and closes its handle, for an abandon or a
/// commit that failed; a lost link cannot, and the file stays for a
/// resume.
fn abandon(session: &Open) {
    let sftp = session.link.session();
    let _ = sftp.close(&session.handle);
    let path = session.partial.clone();
    let _ = sftp
        .call(|id| Request::Remove { id, path }, None)
        .and_then(ops::expect_ok);
}

/// Checks the written file before it may replace anything: flushed when
/// the server offers it, the size the server holds equal to the bytes
/// sent, the CLOSE status OK.
fn verify(connection: &Arc<Connection>, session: &Open, call: &Call<'_>) -> Result<(), RpcError> {
    let sftp = session.link.session();
    let cancel = Some(call.cancel);
    if sftp.extensions().has("fsync@openssh.com") {
        sftp.extended("fsync@openssh.com", strings(&[&session.handle]), cancel)
            .and_then(ops::expect_ok)
            .map_err(|e| connection.fail(Op::Sync, &e))?;
    }
    let held =
        fstat_size(sftp, &session.handle, cancel).map_err(|e| connection.fail(Op::Write, &e))?;
    if held != session.next {
        return Err(RpcError::with_hint(
            ErrorKind::Failed,
            lines::SIZE_DIFFERS,
            lines::size_held(held, session.next),
        ));
    }
    sftp.close(&session.handle)
        .map_err(|e| connection.fail(Op::Close, &e))
}

/// Puts the verified partial file in place.
fn commit(connection: &Arc<Connection>, session: &Open, call: &Call<'_>) -> Result<(), RpcError> {
    let sftp = session.link.session();
    let cancel = Some(call.cancel);
    let rename = |from: &[u8], to: &[u8]| {
        let (from, to) = (from.to_vec(), to.to_vec());
        sftp.call(|id| Request::Rename { id, from, to }, cancel)
            .and_then(ops::expect_ok)
    };
    let exists = match ops::lstat(sftp, &session.target, cancel) {
        Ok(_) => true,
        Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) => false,
        Err(error) => return Err(connection.fail(Op::Stat, &error)),
    };
    if !exists {
        return rename(&session.partial, &session.target).map_err(|e| {
            if ops::lstat(sftp, &session.target, cancel).is_ok() {
                RpcError::new(ErrorKind::AlreadyExists, lines::NAME_TAKEN)
            } else {
                connection.fail(Op::Rename, &e)
            }
        });
    }
    if !session.overwrite {
        return Err(RpcError::new(ErrorKind::AlreadyExists, lines::NAME_TAKEN));
    }
    if sftp.extensions().has("posix-rename@openssh.com") {
        return sftp
            .extended(
                "posix-rename@openssh.com",
                strings(&[&session.partial, &session.target]),
                cancel,
            )
            .and_then(ops::expect_ok)
            .map_err(|e| connection.fail(Op::Rename, &e));
    }
    // No atomic replace: the old target aside, the new one in, then the
    // old one removed; a failure in the middle puts the old one back.
    let aside = unfinished_path(&session.folder, &session.target, "aside");
    rename(&session.target, &aside).map_err(|e| connection.fail(Op::Rename, &e))?;
    if let Err(error) = rename(&session.partial, &session.target) {
        let _ = rename(&aside, &session.target);
        return Err(connection.fail(Op::Rename, &error));
    }
    let path = aside.clone();
    let _ = sftp
        .call(|id| Request::Remove { id, path }, cancel)
        .and_then(ops::expect_ok);
    Ok(())
}

/// `provider/write_end`: commits (verify, then rename into place) and
/// answers the new entry, or abandons and leaves nothing.
pub fn end(connection: &Arc<Connection>, call: &Call<'_>) -> Result<Value, RpcError> {
    let id = call.text("write_id").unwrap_or_default();
    let session = connection.writes().take(id)?;
    let session = session.lock().unwrap_or_else(PoisonError::into_inner);
    if !call.flag("commit") {
        abandon(&session);
        return Ok(Value::Null);
    }
    let done = verify(connection, &session, call).and_then(|()| commit(connection, &session, call));
    connection.forget(&session.target);
    if let Err(error) = done {
        if !connection.is_gone() {
            abandon(&session);
        }
        return Err(error);
    }
    let sftp = session.link.session();
    let attrs = ops::lstat(sftp, &session.target, Some(call.cancel))
        .map_err(|e| connection.fail(Op::Stat, &e))?;
    let name = session
        .target
        .rsplit(|b| *b == b'/')
        .next()
        .unwrap_or_default()
        .to_vec();
    Ok(json!(entry(&name, &attrs, Some(&session.place))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unfinished_name_is_the_engines_and_follows_the_target() {
        let one = unfinished_path(b"/srv", b"/srv/a.txt", "");
        let again = unfinished_path(b"/srv", b"/srv/a.txt", "");
        let other = unfinished_path(b"/srv", b"/srv/b.txt", "");
        let aside = unfinished_path(b"/srv", b"/srv/a.txt", "aside");
        assert_eq!(one, again);
        assert_ne!(one, other);
        assert_ne!(one, aside);
        let name = std::str::from_utf8(&one[5..]).unwrap();
        assert!(diroxide_vfs::names::is_unfinished(name), "{name}");
        assert!(one.starts_with(b"/srv/.diro-part-"));
    }
}
