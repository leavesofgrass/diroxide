//! Free space and volumes (R25 commit 62; R25 section 3.4): one row from
//! `statvfs@openssh.com`, for the volumes list and for the copy plan's
//! room check.
//!
//! - **Free is what the owner may write:** `f_bavail` times `f_frsize`
//!   (the block size when a server gives no fragment size); the total is
//!   `f_blocks` times the same.
//! - **Not known is never zero:** a server without the extension answers
//!   the row with no numbers, and says so in words, "This server does not
//!   report free space."
//! - **With a place,** the volume that holds it: a place not made yet is
//!   asked of its nearest folder that is. **With an authority,** the home
//!   folder's volume, named "Home on build.example".

use std::sync::Arc;

use diroxide_plugins::protocol::RpcError;
use serde_json::{Value, json};

use super::{At, Call, Connection, authority};
use crate::lines;
use crate::sftp::codec::{StatVfs, status};
use crate::sftp::errors::Op;
use crate::sftp::ops;

/// The bytes the owner may still write, and the size, from a `statvfs`
/// answer.
pub fn space(vfs: &StatVfs) -> (u64, u64) {
    let unit = if vfs.frsize > 0 {
        vfs.frsize
    } else {
        vfs.bsize
    };
    (
        vfs.bavail.saturating_mul(unit),
        vfs.blocks.saturating_mul(unit),
    )
}

/// The `statvfs` answer for a path, or for its nearest folder that
/// exists; `None` when the server does not offer the extension.
fn statvfs(
    connection: &Arc<Connection>,
    path: &[u8],
    call: &Call<'_>,
) -> Result<Option<StatVfs>, RpcError> {
    let link = connection.link()?;
    let session = link.session();
    let mut at = path.to_vec();
    loop {
        match session.statvfs(&at, Some(call.cancel)) {
            Ok(answer) => return Ok(answer),
            Err(error) if ops::is_status(&error, status::NO_SUCH_FILE) && at != b"/" => {
                at = match at.iter().rposition(|b| *b == b'/') {
                    Some(0) | None => b"/".to_vec(),
                    Some(cut) => at[..cut].to_vec(),
                };
            }
            Err(error) => return Err(connection.fail(Op::Space, &error)),
        }
    }
}

fn row(name: String, place: String, vfs: Option<&StatVfs>) -> Value {
    let mut row = json!({"name": name, "place": place});
    if let Some(vfs) = vfs {
        let (free, total) = space(vfs);
        row["free"] = json!(free);
        row["total"] = json!(total);
    }
    row
}

/// `provider/volumes`: with a `place`, the one volume that holds it;
/// with an `authority`, the home folder's volume.
pub fn volumes(connection: &Arc<Connection>, call: &Call<'_>) -> Result<Value, RpcError> {
    let host = connection.host().to_owned();
    let root = format!("{}/", authority(connection.target()));
    let (name, place, path) = match At::from_params(call.params, "place") {
        Ok(at) => (host, root, at.path),
        Err(_) => {
            let home = connection.link()?.session().home().to_vec();
            let place = String::from_utf8(home.clone())
                .ok()
                .filter(|h| h.starts_with('/'))
                .map_or(root, |h| format!("{}{h}", authority(connection.target())));
            (lines::home_on(&host), place, home)
        }
    };
    let vfs = statvfs(connection, &path, call)?;
    let mut answer = json!({"volumes": [row(name, place, vfs.as_ref())]});
    if vfs.is_none() {
        answer["note"] = json!(lines::NO_FREE_SPACE);
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vfs(bsize: u64, frsize: u64, blocks: u64, bavail: u64) -> StatVfs {
        StatVfs {
            bsize,
            frsize,
            blocks,
            bfree: bavail.saturating_add(10),
            bavail,
            files: 0,
            ffree: 0,
            favail: 0,
            fsid: 0,
            flag: 0,
            namemax: 255,
        }
    }

    #[test]
    fn free_is_bavail_times_the_fragment_size() {
        assert_eq!(
            space(&vfs(1_048_576, 4096, 1_000_000, 250_000)),
            (1_024_000_000, 4_096_000_000)
        );
        // No fragment size: the block size stands in.
        assert_eq!(space(&vfs(512, 0, 100, 40)), (20_480, 51_200));
        // Numbers too large to multiply stay at the most a u64 holds.
        assert_eq!(space(&vfs(4096, 4096, u64::MAX, u64::MAX)).0, u64::MAX);
    }
}
