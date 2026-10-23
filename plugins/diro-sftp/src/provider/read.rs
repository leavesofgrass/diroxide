//! Ranged reads (R25 commit 59; R22 N8): `provider/read` reads `length`
//! bytes from `offset`, with 64 requests of 32 KiB in flight, from a read
//! handle the session caches, and hands them back inline (base64) or
//! through the host's spool folder (`transfer = file`).
//!
//! - **Stateless and ranged:** each call stands alone, so the host's
//!   reader can seek and a remote move can read the destination back
//!   (answer 96). A range at or past the end answers `eof` with no bytes.
//! - **The handle cache:** a file read in many chunks is opened once; a
//!   handle the server no longer knows is opened again, once.
//! - **The spool:** a `spool_path` must name a plain new file directly
//!   inside the spool folder the host named at `initialize`; anything
//!   else is refused before a byte is read, and the file is made new,
//!   never opened over something already there (no planted link is
//!   written through).
//! - **The transfer session** (answer 87): reads go to the connection's
//!   second session once it is up, so listings never wait behind them.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_plugins::base64;
use diroxide_plugins::protocol::{ErrorKind, RpcError};
use serde_json::{Value, json};

use super::{At, Call, Connection};
use crate::sftp::codec::status;
use crate::sftp::errors::Op;
use crate::sftp::mux::SftpError;
use crate::sftp::ops;

/// The most one inline read answers: 4 MiB, under the host's 8 MiB line
/// once in base64.
pub const MAX_INLINE: u64 = 4 * 1024 * 1024;

/// The most one read through the spool answers.
pub const MAX_SPOOLED: u64 = 256 * 1024 * 1024;

/// The length read when the host names none: the host's own chunk.
pub const DEFAULT_LENGTH: u64 = 1024 * 1024;

fn refused(why: &str) -> RpcError {
    RpcError::new(
        ErrorKind::InvalidParams,
        format!("the spool path is refused: {why}"),
    )
}

/// A spool name: letters, digits, `-`, `_` and `.`, not a dot name, not
/// ending in a dot.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Where a `spool_path` points, by its text alone: a plain name, or a full
/// path whose folder is the spool folder written the same way. Anything
/// else is refused, never resolved.
pub fn spool_file(spool: Option<&Path>, spool_path: &str) -> Result<PathBuf, RpcError> {
    let spool = spool.ok_or_else(|| refused("the host named no spool folder"))?;
    let unified = spool_path.replace('\\', "/");
    let (folder, name) = match unified.rsplit_once('/') {
        Some((folder, name)) => (Some(folder), name),
        None => (None, unified.as_str()),
    };
    if !plain_name(name) {
        return Err(refused("the name is not a plain file name"));
    }
    if let Some(folder) = folder {
        let ours = spool.to_string_lossy().replace('\\', "/");
        if folder != ours.trim_end_matches('/') {
            return Err(refused("it is outside the spool folder"));
        }
    }
    Ok(spool.join(name))
}

/// Makes the spool file new: never opened over one that exists.
fn create(path: &Path) -> Result<File, RpcError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| refused(&format!("it cannot be made new: {e}")))
}

/// Reads the range on `link`'s session into `sink`; a cached handle the
/// server has forgotten is opened again, once, when nothing was read yet.
fn read_range(
    connection: &Arc<Connection>,
    at: &At,
    offset: u64,
    length: u64,
    call: &Call<'_>,
    sink: &mut dyn FnMut(&[u8]) -> bool,
) -> Result<u64, RpcError> {
    let link = connection.transfer_link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let mut tried = false;
    loop {
        let handle = session
            .read_handle(&at.path, cancel)
            .map_err(|e| connection.fail_at(Op::Read, &e, session, &at.path))?;
        let mut got = 0u64;
        let mut counted = |bytes: &[u8]| {
            got += bytes.len() as u64;
            sink(bytes)
        };
        match session.read_pipelined(&handle, offset, length, cancel, &mut counted) {
            Ok(n) => return Ok(n),
            Err(error) if !tried && got == 0 && ops::is_status(&error, status::FAILURE) => {
                tried = true;
                session.forget_handle(&at.path);
            }
            Err(error) => {
                if !matches!(error, SftpError::Lost(_)) {
                    session.forget_handle(&at.path);
                }
                return Err(connection.fail(Op::Read, &error));
            }
        }
    }
}

/// `provider/read`: a range of a file, inline or through the spool.
pub fn read(connection: &Arc<Connection>, at: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    let offset = call.number("offset").unwrap_or(0);
    let asked = call.number("length").unwrap_or(DEFAULT_LENGTH);
    match call.text("transfer").unwrap_or("inline") {
        "inline" => {
            let length = asked.min(MAX_INLINE);
            let mut data = Vec::new();
            let n = read_range(connection, at, offset, length, call, &mut |bytes| {
                data.extend_from_slice(bytes);
                true
            })?;
            Ok(json!({
                "data": base64::encode(&data),
                "length": n,
                "eof": n < asked,
            }))
        }
        "file" => {
            let spool_path = call
                .text("spool_path")
                .ok_or_else(|| refused("a file transfer needs a spool_path"))?;
            let path = spool_file(call.state.spool_dir.as_deref(), spool_path)?;
            let mut file = create(&path)?;
            let length = asked.min(MAX_SPOOLED);
            let mut failed = None;
            let n = read_range(
                connection,
                at,
                offset,
                length,
                call,
                &mut |bytes| match file.write_all(bytes) {
                    Ok(()) => true,
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                },
            )?;
            if let Some(e) = failed.or_else(|| file.flush().err()) {
                return Err(RpcError::new(
                    ErrorKind::Failed,
                    format!("the spool file could not be written: {e}"),
                ));
            }
            Ok(json!({
                "spool_path": spool_path,
                "length": n,
                "eof": n < asked,
            }))
        }
        other => Err(RpcError::new(
            ErrorKind::InvalidParams,
            format!("no transfer {other}: inline or file"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spool_paths_stay_in_the_spool() {
        let spool = Path::new("D:/state/spool");
        assert_eq!(
            spool_file(Some(spool), "a.part").unwrap(),
            spool.join("a.part")
        );
        assert_eq!(
            spool_file(Some(spool), "D:/state/spool/b.part").unwrap(),
            spool.join("b.part")
        );
        assert_eq!(
            spool_file(Some(spool), "D:\\state\\spool\\c.part").unwrap(),
            spool.join("c.part")
        );
        for bad in [
            "../escape.part",
            "D:/state/elsewhere/a.part",
            "D:/state/spool/../a.part",
            "sub/a.part",
            "..",
            "a.part:stream",
            "a.part.",
            "",
            "a part",
            "D:/STATE/spool2/a.part",
        ] {
            let error = spool_file(Some(spool), bad).unwrap_err();
            assert_eq!(error.code, ErrorKind::InvalidParams.code(), "{bad}");
        }
        assert!(spool_file(None, "a.part").is_err());
    }
}
