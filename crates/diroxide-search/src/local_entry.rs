//! Entries from what the parallel walker read: the name and the file type
//! from the folder read, and the metadata when the filter needs it (free
//! on Windows, where the folder read carries it; one `lstat` elsewhere).
//!
//! The local provider's own entry builder makes them
//! ([`LocalEntryBuilder`], P3b), so a search says the same facts as a
//! panel row: hidden by the platform's rule (the hidden attribute on
//! Windows, a leading dot elsewhere, and the Finder's flag on macOS when
//! the metadata is read); a link is a link, never what it points to; a
//! file with an archive extension is an archive; owner and group by name
//! on Unix, from the names read once for the whole walk.

use std::ffi::OsStr;
use std::fs::{FileType, Metadata};
use std::path::Path;

use diroxide_core::Entry;
use diroxide_vfs::LocalEntryBuilder;

/// Whether an entry is hidden, as a panel row says: the hidden attribute
/// on Windows (from metadata the folder read already holds), a leading
/// dot elsewhere, and on macOS the Finder's flag when the metadata is
/// given.
pub(crate) fn is_hidden(name: &OsStr, metadata: Option<&Metadata>) -> bool {
    LocalEntryBuilder::is_hidden(name, metadata)
}

/// An entry from its name and file type, with the metadata's facts when
/// it is given. `path` is the entry's full path, read only for a link's
/// target.
pub(crate) fn entry_from(
    builder: &LocalEntryBuilder,
    name: &OsStr,
    path: &Path,
    file_type: Option<FileType>,
    metadata: Option<&Metadata>,
) -> Entry {
    match metadata {
        Some(metadata) => builder.entry(name, path, metadata),
        None => builder.bare(name, path, file_type),
    }
}

/// Adds the metadata's facts to an entry made without them.
pub(crate) fn fill(builder: &LocalEntryBuilder, entry: &mut Entry, metadata: &Metadata) {
    builder.fill(entry, metadata);
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::EntryKind;

    #[test]
    fn archives_need_a_stem() {
        let builder = LocalEntryBuilder::new();
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .metadata()
            .unwrap()
            .file_type();
        let kind = |name: &str| {
            builder
                .bare(OsStr::new(name), &here.join(name), Some(file))
                .kind
        };
        assert_eq!(kind("course.ZIP"), EntryKind::Archive);
        assert_eq!(kind("backup.tar.gz"), EntryKind::Archive);
        assert_eq!(kind(".zip"), EntryKind::File);
        assert_eq!(kind("..zip"), EntryKind::File);
        assert_eq!(kind("report.pdf"), EntryKind::File);
    }

    #[test]
    fn this_file_reads_as_a_panel_reads_it() {
        let builder = LocalEntryBuilder::new();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let entry = entry_from(
            &builder,
            OsStr::new("Cargo.toml"),
            &path,
            Some(metadata.file_type()),
            Some(&metadata),
        );
        assert_eq!(entry.kind, EntryKind::File);
        assert_eq!(entry.size, Some(metadata.len()));
        assert!(entry.modified.is_some() && entry.attributes.is_some());
        assert!(!entry.hidden && !entry.needs_care);
        if cfg!(unix) {
            // An owner by name where the machine knows one; never empty.
            assert!(entry.owner.as_deref().is_some_and(|o| !o.is_empty()));
        }
        let mut bare = entry_from(
            &builder,
            OsStr::new("Cargo.toml"),
            &path,
            Some(metadata.file_type()),
            None,
        );
        assert_eq!(bare.size, None);
        assert_eq!(bare.kind, EntryKind::File);
        fill(&builder, &mut bare, &metadata);
        assert_eq!(bare.size, entry.size);
        assert_eq!(bare.owner, entry.owner);
    }
}
