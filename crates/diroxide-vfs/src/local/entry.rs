//! Entries from metadata: one stat per entry (free on Windows, where the
//! directory read carries it), the kind, the size, the times, the
//! platform's attributes and owners, links with their targets, and the
//! name checks. Nothing here sniffs contents, draws icons or counts a
//! folder's items (research/file-operations.md, "Large directories").
//!
//! [`LocalEntryBuilder`] gives the same entries to a caller that reads
//! folders itself, such as the search crate's parallel walker (P3b, W3s's
//! request 3), so a search and a panel say the same facts the same way.

use std::ffi::OsStr;
use std::fs::{self, FileType, Metadata};
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
    #[cfg_attr(not(windows), allow(dead_code, reason = "Unix reads it per entry"))]
    pub(crate) volume: Option<u64>,
}

impl EntryContext {
    /// What any entry shares, wherever it is: the owner names on Unix,
    /// read at most every half minute; no volume.
    pub(crate) fn shared() -> Self {
        #[cfg(unix)]
        {
            Self {
                owners: Some(Owners::shared()),
                ..Self::default()
            }
        }
        #[cfg(not(unix))]
        {
            Self::default()
        }
    }
}

/// Whether a name ends with an archive extension after a stem that is not
/// empty and not only dots, ignoring case: the files listed as archives,
/// the same stem rule as the place parser's (`..zip` is no archive).
pub(crate) fn has_archive_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    ARCHIVE_EXTENSIONS.iter().any(|ext| {
        lower
            .strip_suffix(ext)
            .is_some_and(|stem| !stem.is_empty() && !stem.bytes().all(|b| b == b'.'))
    })
}

/// The kind of what metadata describes, by the metadata's own type: a
/// link (a symbolic link or a junction) is a link, never what it points
/// to.
pub(crate) fn kind_of(name: &str, metadata: &Metadata) -> EntryKind {
    kind_of_type(name, metadata.file_type())
}

/// The kind a file type gives: a link is a link, a file with an archive
/// extension an archive, anything that is no file, folder or link
/// special.
pub(crate) fn kind_of_type(name: &str, file_type: FileType) -> EntryKind {
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
    entry.needs_care = names::needs_care(&entry.name);
    fill_facts(&mut entry, metadata, context);
    if kind == EntryKind::Link {
        entry.link = Some(link_of(path, &entry.name));
    }
    entry
}

/// The facts metadata holds, added to an entry: the size of a file, the
/// times, and the platform's attributes, owners and hidden rule.
fn fill_facts(entry: &mut Entry, metadata: &Metadata, context: &EntryContext) {
    if metadata.file_type().is_file() {
        entry.size = Some(metadata.len());
    }
    entry.modified = metadata.modified().ok().map(Timestamp::from_system_time);
    entry.accessed = metadata.accessed().ok().map(Timestamp::from_system_time);
    entry.created = metadata.created().ok().map(Timestamp::from_system_time);
    sys::fill_platform(entry, metadata, context);
}

/// A name as text, and whether it was not Unicode (then the text is a
/// lossy copy, as a listing keeps it).
fn name_text(name: &OsStr) -> (String, bool) {
    match name.to_str() {
        Some(text) => (text.to_owned(), false),
        None => (name.to_string_lossy().into_owned(), true),
    }
}

/// Builds entries the way the local provider lists them, for a caller
/// that reads folders itself (the search crate's parallel walker): the
/// kind by the file type, a file's size, the times, the platform's
/// attributes, owner and group by name on Unix (the number when no name
/// is known), hidden by the platform's rule (the hidden attribute on
/// Windows, a leading dot elsewhere, and the Finder's flag on macOS), the
/// online-only fact, links with their targets, and the name checks.
///
/// One builder serves a whole walk on every thread: it holds the owner
/// names read once, kept for as long as the builder lives, and a clone
/// shares them.
#[derive(Clone, Debug)]
pub struct LocalEntryBuilder {
    context: EntryContext,
}

impl Default for LocalEntryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalEntryBuilder {
    /// A builder for entries anywhere on this machine, as a stat gives
    /// them: owner names on Unix; on Windows no volume, which only
    /// [`LocalEntryBuilder::in_folder`] knows.
    pub fn new() -> Self {
        Self {
            context: EntryContext::shared(),
        }
    }

    /// A builder for the entries of one folder, as its listing gives
    /// them: on Windows also the folder's volume, which its entries share
    /// (one look at the folder).
    pub fn in_folder(folder: &Path) -> Self {
        Self {
            context: sys::list_context(folder),
        }
    }

    /// An entry from its name, its full path and its metadata, read
    /// without following a final link (`symlink_metadata`, or what a
    /// folder read gives). `path` is read again only for a link's target.
    /// A name that is not Unicode is kept as a lossy copy and marked
    /// [`Entry::name_not_unicode`], as a listing marks it.
    pub fn entry(&self, name: &OsStr, path: &Path, metadata: &Metadata) -> Entry {
        let (text, lossy) = name_text(name);
        let mut entry = entry_from(text, metadata, path, &self.context);
        entry.name_not_unicode = lossy;
        entry
    }

    /// An entry from what a folder read gives before any stat: its name
    /// and, when known, its file type (special when it is not). Hidden
    /// by the name alone, so never hidden on Windows, where only the
    /// metadata holds the attribute. [`LocalEntryBuilder::fill`] adds the
    /// rest once the metadata is read.
    pub fn bare(&self, name: &OsStr, path: &Path, file_type: Option<FileType>) -> Entry {
        let (text, lossy) = name_text(name);
        let kind = file_type.map_or(EntryKind::Special, |t| kind_of_type(&text, t));
        let mut entry = Entry::new(text, kind);
        entry.name_not_unicode = lossy;
        entry.needs_care = names::needs_care(&entry.name);
        entry.hidden = sys::is_hidden(name, None);
        if kind == EntryKind::Link {
            entry.link = Some(link_of(path, &entry.name));
        }
        entry
    }

    /// Adds metadata's facts to an entry [`LocalEntryBuilder::bare`]
    /// made: a file's size, the times, the attributes, the owners, and
    /// the hidden rule with the metadata.
    pub fn fill(&self, entry: &mut Entry, metadata: &Metadata) {
        fill_facts(entry, metadata, &self.context);
    }

    /// Whether an entry is hidden by the platform's rule, from its name
    /// and its metadata when there is any: the hidden attribute on
    /// Windows (no without the metadata), a leading dot elsewhere, and
    /// the Finder's flag on macOS when the metadata is given. The rule a
    /// walk prunes by, the same as a listed entry's [`Entry::hidden`].
    pub fn is_hidden(name: &OsStr, metadata: Option<&Metadata>) -> bool {
        sys::is_hidden(name, metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_gives_what_a_listing_gives() {
        use crate::{LocalProvider, OpContext, Provider};
        let scratch = crate::test_support::scratch();
        let root = scratch.path();
        fs::write(root.join("report.pdf"), b"pdf").unwrap();
        fs::write(root.join("course.zip"), b"PK").unwrap();
        fs::write(root.join(".profile"), b"p").unwrap();
        fs::create_dir(root.join("photos")).unwrap();
        let place = diroxide_core::Place::parse(root.to_str().unwrap()).unwrap();
        let listed = LocalProvider::new()
            .list(&place, &OpContext::new())
            .unwrap();
        let builder = LocalEntryBuilder::in_folder(root);
        for item in fs::read_dir(root).unwrap() {
            let item = item.unwrap();
            // The folder read's own metadata, as the listing reads it.
            let metadata = item.metadata().unwrap();
            let built = builder.entry(&item.file_name(), &item.path(), &metadata);
            let mut from_list = listed
                .iter()
                .find(|e| e.name == built.name)
                .unwrap()
                .clone();
            // Reading a folder can touch its access time; nothing else may
            // differ.
            from_list.accessed = built.accessed;
            assert_eq!(built, from_list, "{}", built.name);
            assert_eq!(
                LocalEntryBuilder::is_hidden(&item.file_name(), Some(&metadata)),
                built.hidden
            );
            let mut bare =
                builder.bare(&item.file_name(), &item.path(), Some(metadata.file_type()));
            assert_eq!(bare.kind, built.kind);
            assert_eq!(bare.size, None);
            builder.fill(&mut bare, &metadata);
            assert_eq!(bare.size, built.size);
            assert_eq!(bare.attributes, built.attributes);
            assert_eq!((&bare.owner, bare.hidden), (&built.owner, built.hidden));
        }
        let pdf = listed.iter().find(|e| e.name == "report.pdf").unwrap();
        assert_eq!(pdf.kind, EntryKind::File);
        let zip = listed.iter().find(|e| e.name == "course.zip").unwrap();
        assert_eq!(zip.kind, EntryKind::Archive);
        let dot = builder.bare(OsStr::new(".profile"), &root.join(".profile"), None);
        assert_eq!(dot.kind, EntryKind::Special, "no file type, no kind");
        assert_eq!(dot.hidden, cfg!(unix), "a dot hides only on Unix");
    }

    #[test]
    fn archives_need_a_stem() {
        assert!(has_archive_extension("course.ZIP"));
        assert!(has_archive_extension("backup.tar.gz"));
        assert!(!has_archive_extension(".zip"));
        assert!(!has_archive_extension("..zip"));
        assert!(has_archive_extension("notes.txt.GZ"));
        assert!(!has_archive_extension("report.pdf"));
    }
}
