//! Unix (Linux and macOS): the twelve mode bits, owners and groups by
//! name, identities, the no-replace rename, times, free space and the
//! mounts. Everything here goes through the standard library or
//! `rustix`'s safe wrappers; there is no `unsafe` on this side.

use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use diroxide_core::{Attributes, Entry, Timestamp};

use super::Identity;
use crate::local::entry::EntryContext;
use crate::local::owners::Owners;

/// The macOS flag that hides a file from the Finder (`chflags hidden`).
#[cfg(target_os = "macos")]
const UF_HIDDEN: u32 = 0x8000;

/// The macOS flag of a dataless file: its contents are in the cloud and
/// reading it downloads them.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

/// The identity of the entry at `path`, without following a final link.
pub(crate) fn identity(path: &Path) -> io::Result<Identity> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(Identity {
        volume: metadata.dev(),
        id: u128::from(metadata.ino()),
    })
}

/// What a listing knows once per folder: the owner and group names, read
/// once (never a per-entry lookup, and no libc call that is not safe
/// across threads).
pub(crate) fn list_context(_folder: &Path) -> EntryContext {
    EntryContext {
        owners: Some(Owners::shared()),
        ..EntryContext::default()
    }
}

/// Fills an entry with what `lstat` holds: the twelve mode bits, the
/// owner and group by name (the number when no name is known), the
/// identity, the link and block counts, the change time, and hidden by
/// the leading dot (and the Finder's flag on macOS).
pub(crate) fn fill_platform(entry: &mut Entry, metadata: &Metadata, context: &EntryContext) {
    entry.attributes = Some(Attributes::Unix(metadata.mode() & 0o7777));
    entry.volume = Some(metadata.dev());
    entry.inode = Some(metadata.ino());
    entry.hard_links = Some(metadata.nlink());
    entry.blocks = Some(metadata.blocks());
    entry.changed = timestamp(metadata.ctime(), metadata.ctime_nsec());
    let (owner, group) = match &context.owners {
        Some(owners) => (owners.user(metadata.uid()), owners.group(metadata.gid())),
        None => (metadata.uid().to_string(), metadata.gid().to_string()),
    };
    entry.owner = Some(owner);
    entry.group = Some(group);
    entry.hidden = entry.name.starts_with('.');
    #[cfg(target_os = "macos")]
    {
        let flags = std::os::macos::fs::MetadataExt::st_flags(metadata);
        if flags & UF_HIDDEN != 0 {
            entry.hidden = true;
        }
        if flags & SF_DATALESS != 0 {
            entry.extra.insert(
                crate::facts::ONLINE_ONLY_KEY.to_owned(),
                crate::facts::ONLINE_ONLY_WORDS.to_owned(),
            );
        }
    }
}

/// Nothing more to read: `lstat` gave every fact a stat needs.
pub(crate) fn fill_from_handle(_entry: &mut Entry, _path: &Path) {}

/// Renames without ever replacing. Phase 0's body, replaced with the
/// platform's no-replace rename below.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// The top of the local file system: `/`.
pub(crate) fn roots() -> Vec<diroxide_core::Place> {
    diroxide_core::Place::parse("/").into_iter().collect()
}

fn timestamp(secs: i64, nanos: i64) -> Option<Timestamp> {
    u32::try_from(nanos)
        .ok()
        .map(|nanos| Timestamp::from_unix(secs, nanos))
}
