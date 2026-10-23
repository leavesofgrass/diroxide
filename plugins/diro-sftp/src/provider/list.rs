//! Streamed listings and one entry's details (R25 commit 58; R24
//! sections 2.3 and 5.4).
//!
//! - **Streamed:** READDIR's batches are gathered into rows and sent to
//!   the host as `$/progress` partials of [`BATCH`] entries while the
//!   folder is still being read; the answer carries the rest.
//! - **Names are bytes** (answer 88). A name that is not valid UTF-8
//!   travels as `name_raw`, its bytes in base64, with a lossy `name` for
//!   show; it is never rewritten.
//! - **Hostile names are dropped and counted:** a name with a slash or a
//!   NUL, an empty one, and any `.` or `..` past the one of each every
//!   server sends. The count is in the answer, and said once on Say
//!   Status: "2 names skipped: unsafe."
//! - **Unfinished files** (the engine's `.diro-part-` names) are left out
//!   as every listing of the program leaves them out.
//! - **Links** are shown with their targets, read in one pipelined run
//!   per batch, and never followed.

use std::collections::BTreeMap;
use std::sync::Arc;

use diroxide_core::{EntryKind, Timestamp};
use diroxide_plugins::base64;
use diroxide_plugins::protocol::{Params, RpcError, methods};
use diroxide_plugins::shapes::{WireEntry, WirePlace, format_time};
use diroxide_vfs::names::is_unfinished;
use serde_json::{Value, json};

use super::{At, Call, Connection};
use crate::lines;
use crate::sftp::codec::{Attrs, NameEntry};
use crate::sftp::errors::Op;
use crate::sftp::ops;

/// Entries in one partial batch (R3 section 7).
pub const BATCH: usize = 1000;

/// The type bits of a mode.
const TYPE_BITS: u32 = 0o170_000;
const FOLDER_BITS: u32 = 0o040_000;
const FILE_BITS: u32 = 0o100_000;
const LINK_BITS: u32 = 0o120_000;

/// What a name from the server is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameCheck {
    /// A name to show.
    Keep,
    /// `.` or `..`.
    Dot,
    /// A slash, a NUL, or nothing at all: never shown, counted.
    Unsafe,
    /// An unfinished file of the engine's: left out.
    Unfinished,
}

/// Tells what a name from the server is.
pub fn check_name(name: &[u8]) -> NameCheck {
    if name == b"." || name == b".." {
        NameCheck::Dot
    } else if name.is_empty() || name.contains(&b'/') || name.contains(&0) {
        NameCheck::Unsafe
    } else if std::str::from_utf8(name).is_ok_and(is_unfinished) {
        NameCheck::Unfinished
    } else {
        NameCheck::Keep
    }
}

/// The kind of an entry, from its mode's type bits.
pub fn kind_of(attrs: &Attrs) -> EntryKind {
    match attrs.permissions.map(|p| p & TYPE_BITS) {
        Some(FOLDER_BITS) => EntryKind::Folder,
        Some(LINK_BITS) => EntryKind::Link,
        Some(FILE_BITS) | None => EntryKind::File,
        Some(_) => EntryKind::Special,
    }
}

/// An entry as the host reads it, from a name's bytes and its attributes.
pub fn entry(name: &[u8], attrs: &Attrs, place: Option<&str>) -> WireEntry {
    let (text, raw) = match std::str::from_utf8(name) {
        Ok(text) => (text.to_owned(), None),
        Err(_) => (
            String::from_utf8_lossy(name).into_owned(),
            Some(base64::encode(name)),
        ),
    };
    let kind = kind_of(attrs);
    WireEntry {
        name: text,
        name_raw: raw,
        place: place.map(|p| WirePlace(p.to_owned())),
        kind,
        size: attrs.size.filter(|_| kind != EntryKind::Folder),
        modified: attrs
            .times
            .map(|(_, mtime)| format_time(Timestamp::from_unix(i64::from(mtime), 0))),
        mode: attrs.permissions.map(|p| p & 0o7777),
        attributes: Vec::new(),
        owner: None,
        link_target: None,
        mime: None,
        revision: None,
        extra: BTreeMap::new(),
    }
}

/// A link's target as text for show: cleaned, never followed.
fn target_words(target: &[u8]) -> String {
    lines::cleaned(&String::from_utf8_lossy(target), 1024)
}

/// The names of one listing, sorted as they come.
#[derive(Debug, Default)]
struct Gathered {
    rows: Vec<WireEntry>,
    skipped: usize,
    dots: [bool; 2],
}

impl Gathered {
    fn take(&mut self, folder: &[u8], names: Vec<NameEntry>) -> Vec<(usize, Vec<u8>)> {
        let mut links = Vec::new();
        for name in names {
            match check_name(&name.filename) {
                NameCheck::Keep => {}
                NameCheck::Unfinished => continue,
                NameCheck::Unsafe => {
                    self.skipped += 1;
                    continue;
                }
                NameCheck::Dot => {
                    // Every server sends one of each; more is not a
                    // listing a file manager can trust.
                    let at = usize::from(name.filename.len() == 2);
                    if self.dots[at] {
                        self.skipped += 1;
                    }
                    self.dots[at] = true;
                    continue;
                }
            }
            let row = entry(&name.filename, &name.attrs, None);
            if row.kind == EntryKind::Link {
                links.push((self.rows.len(), ops::join(folder, &name.filename)));
            }
            self.rows.push(row);
        }
        links
    }
}

fn partial(call: &Call<'_>, token: &str, rows: &[WireEntry]) {
    let mut params = Params::new();
    params.insert("token".into(), json!(token));
    params.insert("value".into(), json!({"kind": "partial", "entries": rows}));
    call.peer.notify(methods::PROGRESS, params);
}

/// `provider/list`: the folder's entries, streamed in partial batches
/// when the host gave a `partial_token`.
pub fn list(connection: &Arc<Connection>, at: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    let link = connection.link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let handle = ops::opendir(session, &at.path, cancel)
        .map_err(|e| connection.fail_at(Op::List, &e, session, &at.path))?;
    let token = call.text("partial_token");
    let mut gathered = Gathered::default();
    let mut sent = 0usize;
    let result = loop {
        if call.cancel.is_cancelled() {
            break Err(crate::sftp::mux::SftpError::Cancelled);
        }
        let names = match ops::readdir(session, &handle, cancel) {
            Ok(Some(names)) => names,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        };
        let links = gathered.take(&at.path, names);
        if !links.is_empty() {
            let paths: Vec<Vec<u8>> = links.iter().map(|(_, p)| p.clone()).collect();
            match ops::readlinks(session, &paths, cancel) {
                Ok(targets) => {
                    for ((row, _), target) in links.iter().zip(targets) {
                        gathered.rows[*row].link_target = target.as_deref().map(target_words);
                    }
                }
                Err(error) => break Err(error),
            }
        }
        if let Some(token) = token {
            while gathered.rows.len() - sent >= BATCH {
                partial(call, token, &gathered.rows[sent..sent + BATCH]);
                sent += BATCH;
            }
        }
    };
    // The handle is closed whatever happened; a lost link closes it too.
    if !matches!(result, Err(ref e) if crate::sftp::errors::is_lost(e)) {
        let _ = session.close(&handle);
    }
    result.map_err(|e| connection.fail(Op::List, &e))?;
    if gathered.skipped > 0 {
        let mut params = Params::new();
        params.insert("text".into(), json!(lines::names_skipped(gathered.skipped)));
        params.insert("importance".into(), json!("status"));
        call.peer.notify(methods::UI_ANNOUNCE, params);
    }
    Ok(json!({
        "entries": &gathered.rows[sent..],
        "complete": true,
        "skipped": gathered.skipped,
    }))
}

/// `provider/stat`: one entry's details, a link itself unless
/// `follow_links` is set.
pub fn stat(connection: &Arc<Connection>, at: &At, call: &Call<'_>) -> Result<Value, RpcError> {
    let link = connection.link()?;
    let session = link.session();
    let cancel = Some(call.cancel);
    let attrs = ops::attributes(session, &at.path, call.flag("follow_links"), cancel)
        .map_err(|e| connection.fail(Op::Stat, &e))?;
    let name: &[u8] = if at.path == b"/" { b"/" } else { at.name() };
    let mut row = entry(name, &attrs, Some(&at.place.to_string()));
    if row.kind == EntryKind::Link
        && let Ok(target) = ops::readlink(session, &at.path, cancel)
    {
        row.link_target = Some(target_words(&target));
    }
    Ok(json!(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_kept_dropped_or_counted() {
        assert_eq!(check_name(b"report.txt"), NameCheck::Keep);
        assert_eq!(check_name(b"\xff\xfe"), NameCheck::Keep);
        assert_eq!(check_name(b"back\\slash"), NameCheck::Keep);
        assert_eq!(check_name(b"."), NameCheck::Dot);
        assert_eq!(check_name(b".."), NameCheck::Dot);
        assert_eq!(check_name(b"..."), NameCheck::Keep);
        assert_eq!(check_name(b"a/b"), NameCheck::Unsafe);
        assert_eq!(check_name(b"a\0b"), NameCheck::Unsafe);
        assert_eq!(check_name(b""), NameCheck::Unsafe);
        assert_eq!(check_name(b".diro-part-0a1b2c3d"), NameCheck::Unfinished);
        assert_eq!(check_name(b".diro-part-x"), NameCheck::Keep);
    }

    #[test]
    fn entries_read_their_kind_size_time_and_bytes() {
        let attrs = Attrs {
            size: Some(4096),
            uid_gid: None,
            permissions: Some(0o040_755),
            times: Some((0, 1_700_000_000)),
            extended: Vec::new(),
        };
        let folder = entry(b"docs", &attrs, None);
        assert_eq!(folder.kind, EntryKind::Folder);
        assert_eq!(folder.size, None);
        assert_eq!(folder.mode, Some(0o755));
        assert_eq!(folder.modified.as_deref(), Some("2023-11-14T22:13:20Z"));
        let raw = entry(
            b"caf\xe9",
            &Attrs {
                permissions: Some(0o100_644),
                size: Some(3),
                ..Attrs::default()
            },
            None,
        );
        assert_eq!(raw.kind, EntryKind::File);
        assert_eq!(raw.size, Some(3));
        assert_eq!(raw.name, "caf\u{FFFD}");
        assert_eq!(
            raw.name_raw.as_deref().and_then(base64::decode).as_deref(),
            Some(&b"caf\xe9"[..])
        );
        let link = entry(
            b"l",
            &Attrs {
                permissions: Some(0o120_777),
                ..Attrs::default()
            },
            None,
        );
        assert_eq!(link.kind, EntryKind::Link);
        let fifo = entry(
            b"p",
            &Attrs {
                permissions: Some(0o010_644),
                ..Attrs::default()
            },
            None,
        );
        assert_eq!(fifo.kind, EntryKind::Special);
    }

    #[test]
    fn only_a_second_dot_name_is_counted() {
        let name = |n: &[u8]| NameEntry {
            filename: n.to_vec(),
            longname: Vec::new(),
            attrs: Attrs::default(),
        };
        let mut gathered = Gathered::default();
        gathered.take(
            b"/d",
            vec![
                name(b"."),
                name(b".."),
                name(b"a"),
                name(b"a/b"),
                name(b".."),
            ],
        );
        assert_eq!(gathered.rows.len(), 1);
        assert_eq!(gathered.skipped, 2);
    }
}
