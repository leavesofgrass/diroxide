//! An archive's listing in words (alpha 4, R30 commit 36; R12's rows,
//! R28 part 2.2): the members counted through the provider that opens the
//! archive (the archive provider, or a plugin that claims a disk image),
//! never extracted. The sentence: "zip, 128 files, 12 folders, 48 MB",
//! the size being what the members unpack to. The rows: "Entries: 128
//! files, 12 folders", "Unpacked: 48 MB, packed 12 MB", "Locked: 3
//! entries". At most [`MAX_ROWS`] member rows, then "and 1,203 more"; the
//! walk stops at [`MAX_MEMBERS`] members and says so.

use diroxide_core::format::human_size;
use diroxide_core::names::display_name;
use diroxide_core::{DetailGroup, DetailRow, Entry, EntryKind, Place};
use diroxide_filetype::FileType;
use diroxide_vfs::{OpContext, Registry};

use crate::words::count_words;
use crate::{Preview, PreviewError, PreviewKind, PreviewRequest, Previewer};

/// The most member rows a listing shows.
pub const MAX_ROWS: usize = 500;

/// The most members a listing walks.
pub const MAX_MEMBERS: usize = 50_000;

/// The key the archive provider puts its notes under, "locked" among
/// them (`diroxide-archive`'s `EXTRA_NOTE`, which the preview crate may
/// not depend on).
pub const NOTE_KEY: &str = "archive.note";

/// What a walk over an archive's members found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Listing {
    /// Files (and links and other members that are not folders).
    pub files: u64,
    /// Folders.
    pub folders: u64,
    /// What the members unpack to, when every size is known.
    pub unpacked: Option<u64>,
    /// What the members are packed to, when the format says.
    pub packed: Option<u64>,
    /// Members that need a password.
    pub locked: u64,
    /// The first [`MAX_ROWS`] members as rows: "docs/readme.txt, 2.1 KB".
    pub rows: Vec<String>,
    /// Whether the walk stopped at [`MAX_MEMBERS`].
    pub stopped: bool,
}

impl Listing {
    /// How many members were walked.
    pub fn members(&self) -> u64 {
        self.files + self.folders
    }

    /// The sentence: "zip, 128 files, 12 folders, 48 MB".
    pub fn sentence(&self, word: &str) -> String {
        let mut parts = vec![word.to_owned(), count_words(self.files, "file", "files")];
        if self.folders > 0 {
            parts.push(count_words(self.folders, "folder", "folders"));
        }
        if let Some(size) = self.unpacked {
            parts.push(human_size(size));
        }
        let mut line = parts.join(", ");
        if self.stopped {
            line.push_str(", or more");
        }
        line
    }

    /// The rows: entries, sizes and locked members.
    pub fn detail_rows(&self) -> Vec<DetailRow> {
        let mut entries = count_words(self.files, "file", "files");
        if self.folders > 0 {
            entries.push_str(", ");
            entries.push_str(&count_words(self.folders, "folder", "folders"));
        }
        if self.stopped {
            entries.push_str(", or more");
        }
        let mut rows = vec![DetailRow::new(
            "archive.entries",
            "Entries",
            entries,
            DetailGroup::Content,
        )];
        if let Some(unpacked) = self.unpacked {
            let value = match self.packed {
                Some(packed) => format!("{}, packed {}", human_size(unpacked), human_size(packed)),
                None => human_size(unpacked),
            };
            rows.push(DetailRow::new(
                "archive.unpacked",
                "Unpacked",
                value,
                DetailGroup::Size,
            ));
        }
        if self.locked > 0 {
            rows.push(DetailRow::new(
                "archive.locked",
                "Locked",
                count_words(self.locked, "entry", "entries"),
                DetailGroup::Flags,
            ));
        }
        rows
    }

    /// The listing's lines: the member rows, then "and 1,203 more" when
    /// there were more than [`MAX_ROWS`].
    pub fn lines(&self) -> Vec<String> {
        let mut lines = self.rows.clone();
        let shown = self.rows.len() as u64;
        if self.members() > shown {
            let more = format!(
                "and {} more",
                diroxide_core::format::grouped(self.members() - shown)
            );
            lines.push(if self.stopped {
                format!("{more}, or more")
            } else {
                more
            });
        }
        lines
    }
}

/// A member's row: its path in the archive, hidden characters shown as
/// question marks, then "folder" or its size.
fn member_row(path: &str, entry: &Entry) -> String {
    let shown = display_name(path);
    match (entry.kind, entry.size) {
        (EntryKind::Folder, _) => format!("{shown}, folder"),
        (_, Some(size)) => format!("{shown}, {}", human_size(size)),
        (kind, None) => format!("{shown}, {}", kind.word()),
    }
}

/// Walks an archive's members from its root place through `registry`,
/// folder by folder, counting and taking the first rows, until
/// [`MAX_MEMBERS`].
pub fn walk(root: &Place, registry: &Registry, ctx: &OpContext) -> Result<Listing, PreviewError> {
    let provider = registry.resolve(root)?;
    let mut listing = Listing {
        unpacked: Some(0),
        packed: Some(0),
        ..Listing::default()
    };
    let mut folders = vec![(root.clone(), String::new())];
    while let Some((place, prefix)) = folders.pop() {
        ctx.check()?;
        let mut entries = provider.list(&place, ctx)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            if listing.members() as usize >= MAX_MEMBERS {
                listing.stopped = true;
                return Ok(listing);
            }
            let path = format!("{prefix}{}", entry.name);
            if listing.rows.len() < MAX_ROWS {
                listing.rows.push(member_row(&path, &entry));
            }
            if entry
                .extra
                .get(NOTE_KEY)
                .is_some_and(|n| n.contains("locked"))
            {
                listing.locked += 1;
            }
            if entry.kind == EntryKind::Folder {
                listing.folders += 1;
                if let Ok(inner) = place.join(&entry.name) {
                    folders.push((inner, format!("{path}/")));
                }
                continue;
            }
            listing.files += 1;
            listing.unpacked = listing.unpacked.zip(entry.size).map(|(a, b)| a + b);
            listing.packed = listing.packed.zip(entry.packed_size).map(|(a, b)| a + b);
        }
    }
    if listing.files == 0 {
        listing.packed = None;
    }
    Ok(listing)
}

/// Whether an entry is an archive the listing reads.
pub fn takes(request: &PreviewRequest, file_type: Option<&FileType>) -> bool {
    match request.entry.kind {
        EntryKind::Archive => true,
        EntryKind::File => file_type.is_some_and(|t| t.group.as_deref() == Some("archives")),
        _ => false,
    }
}

/// The archive listing as a previewer: a listing preview whose summary is
/// the sentence and whose lines are the members.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArchivePreviewer;

impl Previewer for ArchivePreviewer {
    fn name(&self) -> &str {
        "archive"
    }

    fn version(&self) -> u32 {
        1
    }

    fn accepts(&self, request: &PreviewRequest, file_type: Option<&FileType>) -> bool {
        takes(request, file_type)
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let root = request.place.clone().into_archive();
        let base = crate::detail_rows(&request.entry, file_type);
        let word = file_type.map_or_else(|| "archive".to_owned(), |t| t.short.clone());
        let listing = match walk(&root, registry, ctx) {
            Ok(listing) => listing,
            Err(PreviewError::Vfs(diroxide_vfs::VfsError::Cancelled)) => {
                return Err(PreviewError::Superseded);
            }
            Err(error) => {
                return Ok(crate::details_preview(
                    PreviewKind::Listing,
                    format!("{word}, unreadable: {error}"),
                    base,
                    std::iter::empty(),
                ));
            }
        };
        let mut preview = crate::details_preview(
            PreviewKind::Listing,
            listing.sentence(&word),
            base,
            listing.detail_rows(),
        );
        preview.lines = listing.lines();
        preview.lines.truncate(request.max_lines.max(1));
        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diroxide_core::Place;
    use diroxide_vfs::{Capabilities, FileWriter, Provider, VfsError, WriteMode};

    use super::*;

    /// An archive of folders `a` to `l` with ten files each, and eight
    /// files at its root, one of them locked.
    #[derive(Debug)]
    struct Fake;

    impl Provider for Fake {
        fn scheme(&self) -> &str {
            "archive"
        }
        fn capabilities(&self, _: &Place) -> Capabilities {
            Capabilities::default()
        }
        fn list(&self, place: &Place, _: &OpContext) -> Result<Vec<Entry>, VfsError> {
            let (_, member) = place.archive().ok_or(VfsError::NotFound)?;
            if member.is_empty() {
                let mut out: Vec<Entry> = ('a'..='l')
                    .map(|c| Entry::new(c.to_string(), EntryKind::Folder))
                    .collect();
                for n in 0..8 {
                    let mut e =
                        Entry::new(format!("root{n}.txt"), EntryKind::File).with_size(1_000_000);
                    e.packed_size = Some(250_000);
                    if n == 0 {
                        e.extra.insert(NOTE_KEY.to_owned(), "locked".to_owned());
                    }
                    out.push(e);
                }
                return Ok(out);
            }
            Ok((0..10)
                .map(|n| {
                    let mut e =
                        Entry::new(format!("f{n}\u{202E}.txt"), EntryKind::File).with_size(325_000);
                    e.packed_size = Some(80_000);
                    e
                })
                .collect())
        }
        fn stat(&self, _: &Place, _: &OpContext) -> Result<Entry, VfsError> {
            Err(VfsError::NotFound)
        }
        fn read(
            &self,
            _: &Place,
            _: &OpContext,
        ) -> Result<Box<dyn std::io::Read + Send>, VfsError> {
            Err(VfsError::NotFound)
        }
        fn write(
            &self,
            _: &Place,
            _: WriteMode,
            _: &OpContext,
        ) -> Result<Box<dyn FileWriter>, VfsError> {
            Err(VfsError::unsupported("archive", "write"))
        }
        fn create_dir(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(VfsError::unsupported("archive", "make folders"))
        }
        fn rename(&self, _: &Place, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(VfsError::unsupported("archive", "rename"))
        }
        fn remove(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(VfsError::unsupported("archive", "remove"))
        }
        fn set_attributes(
            &self,
            _: &Place,
            _: &diroxide_vfs::AttributeChange,
            _: &OpContext,
        ) -> Result<diroxide_vfs::AttributeChange, VfsError> {
            Err(VfsError::unsupported("archive", "set attributes"))
        }
    }

    fn request() -> PreviewRequest {
        PreviewRequest {
            place: Place::parse("D:/docs/course.zip").unwrap(),
            entry: Entry::new("course.zip", EntryKind::Archive).with_size(12_000_000),
            generation: 1,
            max_lines: 1_000,
        }
    }

    #[test]
    fn a_listing_counts_sizes_and_says_the_rest() {
        let mut registry = Registry::with_local();
        registry.set_archive(Arc::new(Fake));
        let listing = walk(
            &request().place.into_archive(),
            &registry,
            &OpContext::new(),
        )
        .unwrap();
        assert_eq!(
            (listing.files, listing.folders, listing.locked),
            (128, 12, 1)
        );
        let line = listing.sentence("zip");
        assert_eq!(line, "zip, 128 files, 12 folders, 47.0 MB");
        assert!(line.chars().count() <= 40);
        let rows: Vec<String> = listing
            .detail_rows()
            .iter()
            .map(DetailRow::spoken)
            .collect();
        assert_eq!(
            rows,
            [
                "Entries: 128 files, 12 folders",
                "Unpacked: 47.0 MB, packed 11.6 MB",
                "Locked: 1 entry"
            ]
        );
        assert!(listing.rows.contains(&"a/f0?.txt, 325 KB".to_owned()));
        assert!(listing.rows.contains(&"b, folder".to_owned()));
    }

    #[test]
    fn past_five_hundred_rows_the_rest_is_counted() {
        let listing = Listing {
            files: 1_703,
            rows: vec!["x".to_owned(); MAX_ROWS],
            ..Listing::default()
        };
        let lines = listing.lines();
        assert_eq!(lines.len(), MAX_ROWS + 1);
        assert_eq!(lines[MAX_ROWS], "and 1,203 more");
    }

    #[test]
    fn the_previewer_takes_archives_and_says_an_unreadable_one() {
        let types = diroxide_filetype::TypeRegistry::builtin();
        assert!(takes(&request(), None));
        let mut file = request();
        file.entry.kind = EntryKind::File;
        assert!(takes(&file, types.get("zip")));
        assert!(!takes(&file, types.get("png")));
        let preview = ArchivePreviewer
            .render(
                &request(),
                types.get("zip"),
                &Registry::with_local(),
                &OpContext::new(),
            )
            .unwrap();
        assert_eq!(preview.kind, PreviewKind::Listing);
        assert!(
            preview.summary.starts_with("zip, unreadable: "),
            "{}",
            preview.summary
        );
        let mut registry = Registry::with_local();
        registry.set_archive(Arc::new(Fake));
        let preview = ArchivePreviewer
            .render(&request(), types.get("zip"), &registry, &OpContext::new())
            .unwrap();
        assert_eq!(preview.summary, "zip, 128 files, 12 folders, 47.0 MB");
        assert_eq!(preview.lines.len(), 140);
    }
}
