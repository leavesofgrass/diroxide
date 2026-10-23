//! Copying on the server (R25 commit 62): `copy_hint` and `copy` with
//! the `copy-data` extension (OpenSSH 9.0 and later).
//!
//! - **`copy_hint` has no side effects:** it never connects, and says
//!   server side only when both places are on one connection (the same
//!   host, user and port) and that server names `copy-data`. A copy
//!   between two sites, or to another account on one host, streams
//!   through the engine.
//! - **`copy` keeps the write session's order:** the bytes go to the
//!   engine's own unfinished name beside the target, the size is
//!   compared, both files are closed with their status checked, the
//!   source's permissions and times are kept when asked, and only then
//!   is the copy renamed into place, never over a name unless asked.
//! - **A long copy is one request:** it runs on the transfer session, so
//!   listings never wait behind it, and the host hears a progress line
//!   every few seconds while the server works.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::EntryKind;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use serde_json::{Value, json};

use super::list::{NameCheck, check_name, entry, kind_of};
use super::write::{Placing, fstat_size, put_in_place, unfinished_path};
use super::{At, Call, Connection, Provider};
use crate::lines;
use crate::sftp::codec::{Attrs, Request, Response, open, status};
use crate::sftp::errors::Op;
use crate::sftp::mux::SftpError;
use crate::sftp::ops;

/// The longest one server-side copy may run.
pub const COPY_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);

/// How often a running copy tells the host so.
pub const COPYING_EVERY: Duration = Duration::from_secs(5);

/// `provider/copy_hint`: `server_side` or `stream`, with why in words.
pub fn copy_hint(provider: &Provider, call: &Call<'_>) -> Result<Value, RpcError> {
    let from = At::from_params(call.params, "from")?;
    let to = At::from_params(call.params, "to")?;
    let (mode, why) = if from.target != to.target {
        ("stream", lines::COPY_OTHER_SERVER)
    } else {
        match provider.connection(&from.target) {
            Ok(connection) if connection.can_copy_on_server() => {
                ("server_side", lines::COPY_ON_SERVER)
            }
            Ok(_) => ("stream", lines::NO_SERVER_COPY),
            Err(_) => ("stream", lines::COPY_NOT_CONNECTED),
        }
    };
    Ok(json!({"mode": mode, "why": why}))
}

/// Waits for one long request, telling the host every few seconds.
fn wait_long(
    ticket: crate::sftp::mux::Ticket,
    call: &Call<'_>,
    host: &str,
) -> Result<Response, SftpError> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("sftp-copy".to_owned())
        .spawn(move || {
            let _ = tx.send(ticket.wait(None, COPY_LIMIT));
        })
        .map_err(|e| SftpError::Lost(e.to_string()))?;
    let mut said = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(answer) => return answer,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(SftpError::Lost("the copy stopped".to_owned()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if call.cancel.is_cancelled() {
            return Err(SftpError::Cancelled);
        }
        if said.elapsed() >= COPYING_EVERY {
            said = Instant::now();
            if let Some(token) = call.text("progress_token") {
                let mut params = Params::new();
                params.insert("token".into(), json!(token));
                params.insert(
                    "value".into(),
                    json!({"kind": "report", "message": lines::copying_on(host)}),
                );
                call.peer.notify(methods::PROGRESS, params);
            }
        }
    }
}

/// `provider/copy`: a file copied on the server with `copy-data`.
pub fn copy(connection: &Arc<Connection>, from: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    let to = At::from_params(call.params, "to")?;
    if to.target != from.target {
        return Err(RpcError::new(
            ErrorKind::Unsupported,
            lines::COPY_OTHER_SERVER,
        ));
    }
    if !connection.can_copy_on_server() {
        return Err(RpcError::new(ErrorKind::Unsupported, lines::NO_SERVER_COPY));
    }
    if to.path == b"/" || check_name(to.name()) != NameCheck::Keep {
        return Err(RpcError::new(
            ErrorKind::InvalidName,
            "The name cannot be written here.",
        ));
    }
    let overwrite = call.flag("overwrite");
    let link = connection.transfer_link()?;
    let sftp = link.session();
    let cancel = Some(call.cancel);
    let source = sftp
        .stat(&from.path, cancel)
        .map_err(|e| connection.fail(Op::Stat, &e))?;
    if kind_of(&source) != EntryKind::File {
        return Err(RpcError::new(
            ErrorKind::Unsupported,
            lines::COPY_FILES_ONLY,
        ));
    }
    match ops::lstat(sftp, &to.path, cancel) {
        Ok(attrs) if kind_of(&attrs) == EntryKind::Folder => {
            return Err(RpcError::new(ErrorKind::IsADirectory, lines::IS_A_FOLDER));
        }
        Ok(_) if !overwrite => {
            return Err(RpcError::new(ErrorKind::AlreadyExists, lines::NAME_TAKEN));
        }
        Ok(_) => {}
        Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) => {}
        Err(error) => return Err(connection.fail(Op::Stat, &error)),
    }
    let folder = to.parent_path();
    let partial = unfinished_path(&folder, &to.path, "");
    let read = sftp
        .open(&from.path, open::READ, Attrs::default(), cancel)
        .map_err(|e| connection.fail_at(Op::Read, &e, sftp, &from.path))?;
    let write = match sftp.open(
        &partial,
        open::WRITE | open::CREAT | open::TRUNC,
        Attrs::default(),
        cancel,
    ) {
        Ok(handle) => handle,
        Err(error) => {
            let _ = sftp.close(&read);
            return Err(connection.fail(Op::Create, &error));
        }
    };
    let remove_partial = || {
        let path = partial.clone();
        let _ = sftp
            .call(|id| Request::Remove { id, path }, None)
            .and_then(ops::expect_ok);
    };
    let data = ops::copy_data_request(&read, 0, 0, &write, 0);
    let copied = sftp
        .mux()
        .send(|id| Request::Extended {
            id,
            name: b"copy-data".to_vec(),
            data,
        })
        .and_then(|ticket| wait_long(ticket, call, connection.host()))
        .and_then(ops::expect_ok);
    let checked = copied
        .and_then(|()| fstat_size(sftp, &write, cancel))
        .map_err(|e| connection.fail_at(Op::Copy, &e, sftp, &partial))
        .and_then(|held| {
            let wanted = source.size.unwrap_or(held);
            if held == wanted {
                Ok(())
            } else {
                Err(RpcError::with_hint(
                    ErrorKind::Failed,
                    lines::SIZE_DIFFERS,
                    lines::size_held(held, wanted),
                ))
            }
        });
    let _ = sftp.close(&read);
    let closed = sftp
        .close(&write)
        .map_err(|e| connection.fail_at(Op::Close, &e, sftp, &partial));
    if let Err(error) = checked.and(closed) {
        if !connection.is_gone() {
            remove_partial();
        }
        return Err(error);
    }
    if call.flag("preserve") {
        let keep = Attrs {
            permissions: source.permissions.map(|p| p & 0o7777),
            times: source.times,
            ..Attrs::default()
        };
        let path = partial.clone();
        let _ = sftp
            .call(
                |id| Request::Setstat {
                    id,
                    path,
                    attrs: keep,
                },
                cancel,
            )
            .and_then(ops::expect_ok);
    }
    let placed = put_in_place(
        connection,
        sftp,
        &Placing {
            partial: &partial,
            target: &to.path,
            folder: &folder,
            overwrite,
        },
        cancel,
    );
    connection.forget(&to.path);
    if let Err(error) = placed {
        if !connection.is_gone() {
            remove_partial();
        }
        return Err(error);
    }
    let attrs = ops::lstat(sftp, &to.path, cancel).map_err(|e| connection.fail(Op::Stat, &e))?;
    Ok(json!(entry(to.name(), &attrs, Some(&to.place.to_string()))))
}
