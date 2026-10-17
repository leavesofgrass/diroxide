//! Entries from what the parallel walker read: the name and the file type
//! from the folder read, and the metadata when the filter needs it (free
//! on Windows, where the folder read carries it; one `lstat` elsewhere).
//! The same facts the local provider gives a panel row, so "hidden" and
//! the kinds mean the same in a search as in a panel: a leading dot on
//! Unix, the hidden attribute on Windows; a link is a link, never what it
//! points to; a file with an archive extension is an archive.
//!
//! Owner and group are said as numbers on Unix (the local provider reads
//! the names once per folder, which a parallel walk cannot share), and
//! macOS's hidden flag is not read: both are in the hand-back as a
//! request for the provider's own entry builder.

use std::ffi::OsStr;
use std::fs::{self, FileType, Metadata};
use std::path::Path;

use diroxide_core::place::{ARCHIVE_EXTENSIONS, strip_extended_prefix};
use diroxide_core::{Attributes, Entry, EntryKind, Link, Timestamp};

/// The Windows hidden attribute.
#[cfg(windows)]
const ATTRIBUTE_HIDDEN: u32 = 0x2;

/// Whether a name ends with an archive extension after a stem that is not
/// empty and not only dots: the files a panel lists as archives.
pub(crate) fn has_archive_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    ARCHIVE_EXTENSIONS.iter().any(|ext| {
        lower
            .strip_suffix(ext)
            .is_some_and(|stem| !stem.is_empty() && !stem.bytes().all(|b| b == b'.'))
    })
}

/// The kind from a file type: a link is a link.
pub(crate) fn kind_of(name: &str, file_type: FileType) -> EntryKind {
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

/// Whether an entry is hidden, as a panel row says: the hidden attribute
/// on Windows (from metadata the folder read already holds), a leading
/// dot elsewhere.
pub(crate) fn is_hidden(name: &OsStr, metadata: Option<&Metadata>) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let _ = name;
        metadata.is_some_and(|m| m.file_attributes() & ATTRIBUTE_HIDDEN != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        name.as_encoded_bytes().first() == Some(&b'.')
    }
}

/// An entry from its name and file type, with the metadata's facts when
/// it is given. `path` is the entry's full path, read only for a link's
/// target.
pub(crate) fn entry_from(
    name: &OsStr,
    path: &Path,
    file_type: Option<FileType>,
    metadata: Option<&Metadata>,
) -> Entry {
    let (text, not_unicode) = match name.to_str() {
        Some(text) => (text.to_owned(), false),
        None => (name.to_string_lossy().into_owned(), true),
    };
    let file_type = metadata.map(Metadata::file_type).or(file_type);
    let kind = file_type.map_or(EntryKind::Special, |t| kind_of(&text, t));
    let mut entry = Entry::new(text, kind);
    entry.name_not_unicode = not_unicode;
    entry.needs_care = not_unicode || diroxide_vfs::names::needs_care(&entry.name);
    entry.hidden = is_hidden(name, metadata);
    if let Some(metadata) = metadata {
        fill(&mut entry, metadata);
    }
    if kind == EntryKind::Link {
        entry.link = Some(link_of(path, &entry.name));
    }
    entry
}

/// Adds the metadata's facts to an entry made without them.
pub(crate) fn fill(entry: &mut Entry, metadata: &Metadata) {
    if metadata.file_type().is_file() {
        entry.size = Some(metadata.len());
    }
    entry.modified = metadata.modified().ok().map(Timestamp::from_system_time);
    entry.accessed = metadata.accessed().ok().map(Timestamp::from_system_time);
    entry.created = metadata.created().ok().map(Timestamp::from_system_time);
    platform(entry, metadata);
}

#[cfg(windows)]
fn platform(entry: &mut Entry, metadata: &Metadata) {
    use std::os::windows::fs::MetadataExt;
    let attributes = metadata.file_attributes();
    entry.attributes = Some(Attributes::Windows(attributes));
    entry.hidden = attributes & ATTRIBUTE_HIDDEN != 0;
}

#[cfg(unix)]
fn platform(entry: &mut Entry, metadata: &Metadata) {
    use std::os::unix::fs::MetadataExt;
    entry.attributes = Some(Attributes::Unix(metadata.mode() & 0o7777));
    entry.volume = Some(metadata.dev());
    entry.inode = Some(metadata.ino());
    entry.hard_links = Some(metadata.nlink());
    entry.blocks = Some(metadata.blocks());
    let nanos = u32::try_from(metadata.ctime_nsec()).unwrap_or(0);
    entry.changed = Some(Timestamp::from_unix(metadata.ctime(), nanos));
    entry.owner = Some(metadata.uid().to_string());
    entry.group = Some(metadata.gid().to_string());
}

#[cfg(not(any(windows, unix)))]
fn platform(_entry: &mut Entry, _metadata: &Metadata) {}

/// Where a link points: its target as stored, and what the target is, or
/// `None` when it is broken.
pub(crate) fn link_of(path: &Path, name: &str) -> Link {
    let target = fs::read_link(path)
        .map(|target| strip_extended_prefix(&target.to_string_lossy()).into_owned())
        .unwrap_or_default();
    let points_to = fs::metadata(path)
        .ok()
        .map(|metadata| kind_of(name, metadata.file_type()));
    Link { target, points_to }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_need_a_stem() {
        assert!(has_archive_extension("course.ZIP"));
        assert!(has_archive_extension("backup.tar.gz"));
        assert!(!has_archive_extension(".zip"));
        assert!(!has_archive_extension("..zip"));
        assert!(!has_archive_extension("report.pdf"));
    }

    #[test]
    fn this_file_reads_as_a_panel_reads_it() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let metadata = fs::symlink_metadata(&path).unwrap();
        let entry = entry_from(
            OsStr::new("Cargo.toml"),
            &path,
            Some(metadata.file_type()),
            Some(&metadata),
        );
        assert_eq!(entry.kind, EntryKind::File);
        assert_eq!(entry.size, Some(metadata.len()));
        assert!(entry.modified.is_some() && entry.attributes.is_some());
        assert!(!entry.hidden && !entry.needs_care);
        let bare = entry_from(
            OsStr::new("Cargo.toml"),
            &path,
            Some(metadata.file_type()),
            None,
        );
        assert_eq!(bare.size, None);
        assert_eq!(bare.kind, EntryKind::File);
    }
}
