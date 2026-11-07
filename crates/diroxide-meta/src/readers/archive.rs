//! Archive facts from a zip's central directory (R12, "Archive
//! metadata"): the files and folders, the unpacked and packed sizes,
//! whether entries are encrypted, and the archive's comment. The central
//! directory sits at the end of the file, so this is one seek and one
//! read of the directory; no member is unpacked. It reads through the
//! `zip` crate directly: this crate may not depend on the archive crate
//! (the layering in `docs/dev/architecture.md`).
//!
//! Tar, 7z and the rest need a full pass or a tool, so they have no
//! facts here; the archive listing of the quick view counts them.

use diroxide_core::details::DetailGroup;
use diroxide_vfs::ReadSeek;

use crate::field::Field;
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
use crate::words::{count, grouped, human_size};

/// The most entries counted; past it the counts say "at least".
pub const MOST_ENTRIES: usize = 1_000_000;

/// The zip-based archives (not the documents, whose facts are their
/// properties).
const TYPES: [&str; 4] = ["zip", "jar", "apk", "cbz"];

/// Names read as zip archives when the type is not checked yet.
const NAMES: [&str; 6] = [".zip", ".jar", ".apk", ".cbz", ".xpi", ".war"];

/// The zip facts reader: `archive.entries`, `archive.unpacked`,
/// `archive.encrypted`, `archive.comment`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ZipFacts;

impl Reader for ZipFacts {
    fn id(&self) -> &'static str {
        "archive"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "archive.entries",
            "archive.unpacked",
            "archive.encrypted",
            "archive.comment",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        super::takes(request, &TYPES, &NAMES)
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let archive = zip::ZipArchive::new(source).map_err(zip_error)?;
        let mut files = 0u64;
        let mut folders = 0u64;
        let mut encrypted = 0u64;
        let mut unpacked = 0u64;
        let mut packed = 0u64;
        let total = archive.len();
        for index in 0..total.min(MOST_ENTRIES) {
            let Ok(entry) = archive.by_index_data(index) else {
                continue;
            };
            if entry.is_dir() {
                folders += 1;
            } else {
                files += 1;
                unpacked = unpacked.saturating_add(entry.size());
                packed = packed.saturating_add(entry.compressed_size());
            }
            if entry.encrypted() {
                encrypted += 1;
            }
        }
        let mut entries = format!(
            "{}, {}",
            count_grouped(files, "file", "files"),
            count_grouped(folders, "folder", "folders")
        );
        if total > MOST_ENTRIES {
            entries = format!("at least {entries}");
        }
        let mut fields = vec![
            Field::new("archive.entries", "Entries", entries, DetailGroup::Content),
            Field::new(
                "archive.unpacked",
                "Unpacked",
                format!("{}, packed {}", human_size(unpacked), human_size(packed)),
                DetailGroup::Size,
            ),
            Field::new(
                "archive.encrypted",
                "Encrypted",
                match encrypted {
                    0 => "no".to_owned(),
                    n => format!("yes, {}", count_grouped(n, "entry", "entries")),
                },
                DetailGroup::Flags,
            ),
        ];
        let comment = String::from_utf8_lossy(archive.comment()).trim().to_owned();
        if !comment.is_empty() {
            fields.push(Field::new(
                "archive.comment",
                "Comment",
                comment,
                DetailGroup::Content,
            ));
        }
        Ok(fields)
    }
}

/// A count with its noun, the number grouped: "1,204 files", "no folders".
fn count_grouped(n: u64, singular: &str, plural: &str) -> String {
    if n > 999 {
        format!("{} {plural}", grouped(n))
    } else {
        count(n, singular, plural)
    }
}

fn zip_error(error: zip::result::ZipError) -> MetaError {
    match error {
        zip::result::ZipError::Io(io) => MetaError::from_io(&io),
        zip::result::ZipError::UnsupportedArchive(_) => {
            MetaError::unreadable("it uses a zip feature not read here")
        }
        _ => MetaError::unreadable("not a zip archive, or a damaged one"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Cursor, Write};

    use diroxide_core::{Entry, EntryKind, Place};
    use zip::write::SimpleFileOptions;

    use super::*;

    /// A zip made in memory: `(name, contents)` members, names ending in
    /// a slash made as folders, and a comment.
    pub(crate) fn zip_of(members: &[(&str, &[u8])], comment: &str) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in members {
            if let Some(folder) = name.strip_suffix('/') {
                writer.add_directory(folder, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(contents).unwrap();
            }
        }
        writer.set_comment(comment.to_owned()).unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn lines(bytes: Vec<u8>) -> Result<Vec<String>, MetaError> {
        let place = Place::parse(if cfg!(windows) { "C:\\b.zip" } else { "/b.zip" }).unwrap();
        let entry = Entry::new("bundle.zip", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: None,
            group: None,
        };
        assert!(ZipFacts.accepts(&request));
        ZipFacts
            .read(&mut Cursor::new(bytes), &request, &ReadLimits::default())
            .map(|fields| fields.into_iter().map(|f| f.cleaned().line()).collect())
    }

    #[test]
    fn counts_sizes_and_the_comment_from_the_directory() {
        let bytes = zip_of(
            &[
                ("docs/", b""),
                ("docs/a.txt", &[b'a'; 1500]),
                ("b.bin", &[0u8; 2500]),
            ],
            "Built for Ada Example\r\nversion 2",
        );
        let got = lines(bytes).unwrap();
        assert_eq!(
            got,
            [
                "Entries: 2 files, 1 folder",
                "Unpacked: 4.0 KB, packed 4.0 KB",
                "Encrypted: no",
                "Comment: Built for Ada Example version 2",
            ]
        );
        for line in &got {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert_eq!(
            lines(zip_of(&[], "")).unwrap()[0],
            "Entries: no files, no folders"
        );
    }

    #[test]
    fn not_a_zip_is_said_in_words() {
        assert_eq!(
            lines(b"PK\x03\x04 but nothing after".to_vec()),
            Err(MetaError::unreadable("not a zip archive, or a damaged one"))
        );
        assert!(lines(Vec::new()).is_err());
    }
}
