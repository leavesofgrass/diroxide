//! Entries from metadata: one stat per entry (free on Windows, where the
//! directory read carries it), the kind, the size, the times, the
//! platform's attributes and owners, links with their targets, and the
//! name checks. Nothing here sniffs contents, draws icons or counts a
//! folder's items (research/file-operations.md, "Large directories").

use std::fs::{self, Metadata};
use std::path::Path;
use std::sync::Arc;

use diroxide_core::place::{ARCHIVE_EXTENSIONS, strip_extended_prefix};
use diroxide_core::{Entry, EntryKind, Link, Timestamp};

use super::owners::Owners;
use crate::names;
use crate::sys;

/// What every entry of one listing shares, read once per folder.
#[derive(Clone, Debug, Default)]
pub(crate) struct EntryContext {
    /// Owner and group names (Unix).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) owners: Option<Arc<Owners>>,
    /// The folder's volume, which its entries share (Windows, where the
    /// directory read has no volume per entry).
    pub(crate) volume: Option<u64>,
}

/// Whether a name ends with an archive extension after a non-empty stem,
/// ignoring case: the files listed as archives.
pub(crate) fn has_archive_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    ARCHIVE_EXTENSIONS
        .iter()
        .any(|ext| lower.len() > ext.len() && lower.ends_with(ext))
}

/// The kind of what metadata describes, by the metadata's own type: a
/// link (a symbolic link or a junction) is a link, never what it points
/// to.
pub(crate) fn kind_of(name: &str, metadata: &Metadata) -> EntryKind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        EntryKind::Link
    } else if file_type.is_dir() {
        EntryKind::Folder
    } else if file_type.is_file() {
        if has_archive_extension(name) {
            EntryKind::Archive
        } else {
            EntryKind::File
        }
    } else {
        EntryKind::Special
    }
}

/// Where a link points: its target as stored (with any extended-length
/// prefix removed, since it is shown), and what the target is, or `None`
/// when it is broken.
pub(crate) fn link_of(path: &Path, name: &str) -> Link {
    let target = fs::read_link(path)
        .map(|target| strip_extended_prefix(&target.to_string_lossy()).into_owned())
        .unwrap_or_default();
    let points_to = fs::metadata(path)
        .ok()
        .map(|metadata| kind_of(name, &metadata));
    Link { target, points_to }
}

/// An entry from its name and its metadata (read without following a
/// final link). `path` is the entry's full path, used only to read a
/// link's target.
pub(crate) fn entry_from(
    name: String,
    metadata: &Metadata,
    path: &Path,
    context: &EntryContext,
) -> Entry {
    let kind = kind_of(&name, metadata);
    let mut entry = Entry::new(name, kind);
    if metadata.file_type().is_file() {
        entry.size = Some(metadata.len());
    }
    entry.modified = metadata.modified().ok().map(Timestamp::from_system_time);
    entry.accessed = metadata.accessed().ok().map(Timestamp::from_system_time);
    entry.created = metadata.created().ok().map(Timestamp::from_system_time);
    entry.needs_care = names::needs_care(&entry.name);
    sys::fill_platform(&mut entry, metadata, context);
    if kind == EntryKind::Link {
        entry.link = Some(link_of(path, &entry.name));
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_need_a_stem() {
        assert!(has_archive_extension("course.ZIP"));
        assert!(has_archive_extension("backup.tar.gz"));
        assert!(!has_archive_extension(".zip"));
        assert!(!has_archive_extension("report.pdf"));
    }
}
