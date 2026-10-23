//! The operations on one entry (R25 commit 61): make a folder, remove an
//! entry, rename, change attributes, make a link. The job engine walks,
//! lists and asks; each call here acts on one entry.
//!
//! - **Remove is permanent** (answer 86): SFTP has no trash, so
//!   `provider/trash` is refused in words, and the engine's own gate gives
//!   its full listing and a question that starts on No before any
//!   `provider/remove` comes.
//! - **Rename never replaces** unless asked; a replacing rename uses
//!   `posix-rename@openssh.com`, never replaces a folder, and is refused
//!   in words where the server lacks it.
//! - **Change permissions says the result in words:** the server may keep
//!   other bits than asked (a umask, a file system without them), so the
//!   entry is read again and the answer says what it holds now.
//! - **Links:** SYMLINK sends the target first, as OpenSSH reads it (its
//!   PROTOCOL file, 4.1); links are made only on a server that names an
//!   `@openssh.com` extension, hard links only with `hardlink@openssh.com`.

use std::sync::Arc;

use diroxide_core::EntryKind;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use diroxide_plugins::shapes::parse_time;
use serde_json::{Value, json};

use super::list::{NameCheck, check_name, kind_of};
use super::{At, Call, Connection};
use crate::lines;
use crate::sftp::codec::{Attrs, Request, status, strings};
use crate::sftp::errors::Op;
use crate::sftp::ops;

fn plain_name(at: &At) -> Result<(), RpcError> {
    if at.path == b"/" || check_name(at.name()) != NameCheck::Keep {
        return Err(RpcError::new(
            ErrorKind::InvalidName,
            "The name cannot be used here.",
        ));
    }
    Ok(())
}

fn mkdir(connection: &Arc<Connection>, path: &[u8], call: &Call<'_>) -> Result<(), RpcError> {
    let link = connection.link()?;
    let path = path.to_vec();
    link.session()
        .call(
            |id| Request::Mkdir {
                id,
                path,
                attrs: Attrs::default(),
            },
            Some(call.cancel),
        )
        .and_then(ops::expect_ok)
        .map_err(|e| connection.fail(Op::MakeFolder, &e))
}

/// `provider/create_dir`: one folder, or with `parents` every missing
/// folder on the way.
pub fn create_dir(
    connection: &Arc<Connection>,
    at: &At,
    call: &Call<'_>,
) -> Result<Value, RpcError> {
    plain_name(at)?;
    if call.flag("parents") {
        let link = connection.link()?;
        let session = link.session();
        let parent = at.parent_path();
        let mut sofar = Vec::new();
        for part in parent.split(|b| *b == b'/').filter(|p| !p.is_empty()) {
            sofar.push(b'/');
            sofar.extend_from_slice(part);
            match ops::lstat(session, &sofar, Some(call.cancel)) {
                Ok(attrs) if kind_of(&attrs) == EntryKind::Folder => {}
                Ok(_) => {
                    return Err(RpcError::new(ErrorKind::NotADirectory, lines::NOT_A_FOLDER));
                }
                Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) => {
                    mkdir(connection, &sofar, call)?;
                }
                Err(error) => return Err(connection.fail(Op::Stat, &error)),
            }
        }
    }
    mkdir(connection, &at.path, call)?;
    Ok(Value::Null)
}

/// `provider/remove`: one file, link or empty folder, for good. The
/// engine walks a folder's contents and lists them first.
pub fn remove(connection: &Arc<Connection>, at: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    if call.flag("recursive") {
        return Err(RpcError::new(
            ErrorKind::Unsupported,
            "The engine removes a folder's contents first.",
        ));
    }
    plain_name(at)?;
    let link = connection.link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let attrs = ops::lstat(session, &at.path, cancel).map_err(|e| connection.fail(Op::Stat, &e))?;
    let path = at.path.clone();
    let (op, done) = if kind_of(&attrs) == EntryKind::Folder {
        (
            Op::RemoveFolder,
            session.call(|id| Request::Rmdir { id, path }, cancel),
        )
    } else {
        (
            Op::Remove,
            session.call(|id| Request::Remove { id, path }, cancel),
        )
    };
    connection.forget(&at.path);
    done.and_then(ops::expect_ok)
        .map_err(|e| connection.fail(op, &e))?;
    Ok(Value::Null)
}

/// `provider/trash`: SFTP has none; a delete there is permanent, listed
/// and asked first by the engine (answer 86).
pub fn trash() -> Result<Value, RpcError> {
    Err(RpcError::with_hint(
        ErrorKind::Unsupported,
        lines::NO_TRASH,
        lines::NO_TRASH_HINT,
    ))
}

/// `provider/rename`: a rename or a move on one server. Without
/// `overwrite` it never replaces; with it, `posix-rename` replaces a file
/// or a link in one step and never a folder.
pub fn rename(connection: &Arc<Connection>, from: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    let to = At::from_params(call.params, "to")?;
    if to.target != from.target {
        return Err(RpcError::new(
            ErrorKind::Unsupported,
            "A move to another server is a copy.",
        ));
    }
    plain_name(from)?;
    plain_name(&to)?;
    let link = connection.link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let source =
        ops::lstat(session, &from.path, cancel).map_err(|e| connection.fail(Op::Stat, &e))?;
    let taken = match ops::lstat(session, &to.path, cancel) {
        Ok(attrs) => Some(attrs),
        Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) => None,
        Err(error) => return Err(connection.fail(Op::Stat, &error)),
    };
    connection.forget(&from.path);
    connection.forget(&to.path);
    let result = match (taken, call.flag("overwrite")) {
        (None, _) => {
            let (a, b) = (from.path.clone(), to.path.clone());
            session.call(|id| Request::Rename { id, from: a, to: b }, cancel)
        }
        (Some(_), false) => {
            return Err(RpcError::new(ErrorKind::AlreadyExists, lines::NAME_TAKEN));
        }
        (Some(target), true) => {
            if kind_of(&target) == EntryKind::Folder || kind_of(&source) == EntryKind::Folder {
                return Err(RpcError::new(ErrorKind::AlreadyExists, lines::NAME_TAKEN));
            }
            if !session.extensions().has("posix-rename@openssh.com") {
                return Err(RpcError::new(
                    ErrorKind::Unsupported,
                    lines::NO_ATOMIC_RENAME,
                ));
            }
            session.extended(
                "posix-rename@openssh.com",
                strings(&[&from.path, &to.path]),
                cancel,
            )
        }
    };
    result
        .and_then(ops::expect_ok)
        .map_err(|e| connection.fail(Op::Rename, &e))?;
    Ok(Value::Null)
}

/// Seconds since 1970 as SFTP version 3 keeps them: whole seconds that
/// end in 2106.
fn seconds(text: &str) -> Result<u32, RpcError> {
    parse_time(text)
        .and_then(|t| u32::try_from(t.unix_seconds()).ok())
        .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, lines::DATES_KEPT))
}

/// `provider/set_attributes`: permission bits and times. The entry is read
/// again, and the answer says in words what it holds now; when the
/// server kept other bits than asked, that is said as a warning.
pub fn set_attributes(
    connection: &Arc<Connection>,
    at: &At,
    call: &Call<'_>,
) -> Result<Value, RpcError> {
    let link = connection.link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let mode = call.number("mode").map(|m| (m & 0o7777) as u32);
    let modified = call.text("modified").map(seconds).transpose()?;
    let accessed = call.text("accessed").map(seconds).transpose()?;
    let mut attrs = Attrs {
        permissions: mode,
        ..Attrs::default()
    };
    if modified.is_some() || accessed.is_some() {
        let now =
            ops::lstat(session, &at.path, cancel).map_err(|e| connection.fail(Op::Stat, &e))?;
        let (was_a, was_m) = now.times.unwrap_or((0, 0));
        attrs.times = Some((accessed.unwrap_or(was_a), modified.unwrap_or(was_m)));
    }
    let path = at.path.clone();
    session
        .call(|id| Request::Setstat { id, path, attrs }, cancel)
        .and_then(ops::expect_ok)
        .map_err(|e| connection.fail(Op::SetAttributes, &e))?;
    let after = ops::lstat(session, &at.path, cancel).map_err(|e| connection.fail(Op::Stat, &e))?;
    let held = after.permissions.map(|p| p & 0o7777);
    let mut answer = json!({});
    if let Some(asked) = mode {
        let words = match held {
            Some(held) if held == asked => lines::permissions_now(held),
            Some(held) => lines::permissions_kept(held, asked),
            None => lines::permissions_unknown(asked),
        };
        if held != Some(asked) {
            let mut params = Params::new();
            params.insert("text".into(), json!(words));
            params.insert("importance".into(), json!("warning"));
            call.peer.notify(methods::UI_ANNOUNCE, params);
        }
        answer["words"] = json!(words);
        answer["mode"] = json!(held);
    }
    if let Some((_, mtime)) = after.times {
        answer["modified_seconds"] = json!(mtime);
    }
    Ok(answer)
}

/// `provider/create_link`: a symbolic link (OpenSSH's order: the target
/// first, then the link's path) or, with `kind = "hard"`, a hard link.
pub fn create_link(
    connection: &Arc<Connection>,
    at: &At,
    call: &Call<'_>,
) -> Result<Value, RpcError> {
    plain_name(at)?;
    let target = call
        .text("target")
        .filter(|t| !t.is_empty() && !t.contains('\0'))
        .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "a link needs a target"))?
        .as_bytes()
        .to_vec();
    let link = connection.link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let ext = session.extensions();
    let done = match call.text("kind").unwrap_or("symbolic") {
        "symbolic" => {
            if !ext.names().iter().any(|n| n.ends_with("@openssh.com")) {
                return Err(RpcError::new(ErrorKind::Unsupported, lines::NO_LINKS));
            }
            let second = at.path.clone();
            session.call(
                |id| Request::Symlink {
                    id,
                    first: target,
                    second,
                },
                cancel,
            )
        }
        "hard" => {
            if !ext.has("hardlink@openssh.com") {
                return Err(RpcError::new(ErrorKind::Unsupported, lines::NO_LINKS));
            }
            session.extended(
                "hardlink@openssh.com",
                strings(&[&target, &at.path]),
                cancel,
            )
        }
        other => {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!("no link kind {other}: symbolic or hard"),
            ));
        }
    };
    done.and_then(ops::expect_ok)
        .map_err(|e| connection.fail(Op::Link, &e))?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_whole_seconds_from_1970_to_2106() {
        assert_eq!(seconds("2023-11-14T22:13:20Z").unwrap(), 1_700_000_000);
        assert!(seconds("1969-12-31T23:59:59Z").is_err());
        assert!(seconds("2200-01-01T00:00:00Z").is_err());
        assert!(seconds("yesterday").is_err());
    }
}
