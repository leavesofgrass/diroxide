//! `provider/read`: a range of a file in an image, inline (base64) or
//! into the host's spool folder (`transfer = file`). Stateless and
//! ranged, so the host's reader can seek; a range at or past the end
//! answers `eof` with no bytes. A link reads as its target's text.
//!
//! A `spool_path` must name a plain new file directly inside the spool
//! folder the host named at `initialize`; anything else is refused before
//! a byte is read, and the file is made new, never opened over one that
//! is there.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use diroxide_plugins::base64;
use diroxide_plugins::protocol::{ErrorKind, RpcError};
use serde_json::{Value, json};

use super::{Call, inner_of};
use crate::error::DiskError;

/// The most one inline read answers: 4 MiB, under the host's 8 MiB line
/// once in base64.
pub const MAX_INLINE: u64 = 4 * 1024 * 1024;

/// The most one read through the spool answers.
pub const MAX_SPOOLED: u64 = 256 * 1024 * 1024;

/// The length read when the host names none: the host's own chunk.
pub const DEFAULT_LENGTH: u64 = 1024 * 1024;

/// One piece of a spooled read.
const PIECE: usize = 4 * 1024 * 1024;

fn refused(why: &str) -> DiskError {
    DiskError::Host(RpcError::new(
        ErrorKind::InvalidParams,
        format!("the spool path is refused: {why}"),
    ))
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
/// path whose folder is the spool folder written the same way.
pub fn spool_file(spool: Option<&Path>, spool_path: &str) -> Result<PathBuf, DiskError> {
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

/// Answers `provider/read`.
pub fn read(call: &Call<'_>) -> Result<Value, DiskError> {
    let at = call.at("place")?;
    let offset = call.number("offset").unwrap_or(0);
    let to_spool = match call.text("transfer") {
        None | Some("inline") => false,
        Some("file") => true,
        Some(other) => {
            return Err(DiskError::Host(RpcError::new(
                ErrorKind::InvalidParams,
                format!("transfer {other} is not inline or file"),
            )));
        }
    };
    let most = if to_spool { MAX_SPOOLED } else { MAX_INLINE };
    let length = call.number("length").unwrap_or(DEFAULT_LENGTH).min(most);
    let length = usize::try_from(length).unwrap_or(usize::MAX);
    let open = call.images.get(call.peer, &at.container)?;
    let inner = inner_of(&open, &at)?;
    if inner.names.is_empty() {
        return Err(DiskError::IsAFolder);
    }
    let volume = open.image.volume(inner.partition)?;
    if !to_spool {
        let bytes = volume.with_tree(|tree| tree.read(&inner.names, offset, length))?;
        let eof = bytes.len() < length;
        return Ok(json!({"data": base64::encode(&bytes), "length": bytes.len(), "eof": eof}));
    }
    let spool = call
        .images
        .spool
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let path = spool_file(
        spool.as_deref(),
        call.text("spool_path")
            .ok_or_else(|| refused("a file transfer needs a spool_path"))?,
    )?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| refused(&format!("it cannot be made new: {e}")))?;
    let mut done = 0usize;
    let mut eof = false;
    while done < length {
        call.cancel.check().map_err(DiskError::from)?;
        let want = (length - done).min(PIECE);
        let bytes = volume.with_tree(|tree| tree.read(&inner.names, offset + done as u64, want))?;
        file.write_all(&bytes)
            .map_err(|e| DiskError::Io(e.to_string()))?;
        done += bytes.len();
        if bytes.len() < want {
            eof = true;
            break;
        }
    }
    Ok(json!({"length": done, "eof": eof}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spool_path_must_be_a_plain_name_inside_the_spool() {
        let spool = Path::new("D:/state/spool");
        assert!(spool_file(Some(spool), "part-1.bin").is_ok());
        assert!(spool_file(Some(spool), "D:/state/spool/part-1.bin").is_ok());
        assert!(spool_file(Some(spool), "../x").is_err());
        assert!(spool_file(Some(spool), "D:/elsewhere/x.bin").is_err());
        assert!(spool_file(Some(spool), "x:stream").is_err());
        assert!(spool_file(None, "x.bin").is_err());
    }
}
