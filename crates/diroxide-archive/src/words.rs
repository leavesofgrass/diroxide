//! The archive provider's sentences, each with its meaning in its first 40
//! cells (research/alpha2-plan.md part three, 3.1 to 3.3): titles, the
//! status lines, the progress line of a sequential listing, and the
//! read-only refusal. The question shapes are in [`crate::guard`], the
//! member rows in [`crate::index::member_row`].

use diroxide_core::format::human_size;
use diroxide_core::{Message, Place};
use diroxide_vfs::VfsError;

use crate::sniff::Sniffed;

/// The refusal for any change inside an archive in this alpha.
pub const READ_ONLY: &str = "Archives are read only here. Use Pack to make a new one.";

/// The operation words of the provider's read-only refusals, so
/// [`read_only_refusal`] knows them.
pub(crate) const READ_ONLY_OPERATION: &str = "change an archive in place";

/// The error every change inside an archive gives.
pub(crate) fn read_only_error() -> VfsError {
    VfsError::unsupported("archive", READ_ONLY_OPERATION)
}

/// The sentence to say for an error, when it is the archive provider's
/// read-only refusal: "Archives are read only here. Use Pack to make a new
/// one." The app says it in place of the generic "Could not ..." line.
pub fn read_only_refusal(error: &VfsError) -> Option<Message> {
    match error {
        VfsError::Unsupported {
            provider,
            operation,
        } if provider == "archive" && operation == READ_ONLY_OPERATION => {
            Some(Message::error(READ_ONLY))
        }
        _ => None,
    }
}

/// The title line for a place inside an archive, the kind first: "course.zip,
/// archive, in D:\docs" at the root, "week1, in course.zip" inside, and the
/// format named when the bytes and the name disagree: "a.zip, 7z archive,
/// in D:\docs".
pub fn title(place: &Place, sniffed: Option<&Sniffed>) -> String {
    let plain = place.title();
    let Some((container, member)) = place.archive() else {
        return plain;
    };
    match sniffed {
        Some(sniffed) if member.is_empty() && sniffed.disagrees() => {
            let name = container.name().unwrap_or("archive");
            let kind = sniffed.format.spoken_name();
            match container.parent() {
                Some(folder) => format!("{name}, {kind}, in {folder}"),
                None => format!("{name}, {kind}"),
            }
        }
        _ => plain,
    }
}

/// The progress line of a sequential archive being read, at most every
/// ten seconds in screen-reader mode: "Reading backup.tar.xz: 3.2 of 14.0
/// GB", or the bytes alone when the size is not known.
pub fn reading_line(name: &str, done: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => {
            let total_words = human_size(total);
            let unit = total_words.rsplit(' ').next().unwrap_or("");
            let done_words = human_size(done);
            // "3.2 of 14.0 GB" when both sizes share a unit.
            match done_words.strip_suffix(unit) {
                Some(number) if !unit.is_empty() => {
                    format!("Reading {name}: {number}of {total_words}")
                }
                _ => format!("Reading {name}: {done_words} of {total_words}"),
            }
        }
        None => format!("Reading {name}: {}", human_size(done)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::ArchiveFormat;
    use crate::sniff::Signature;
    use diroxide_core::format::within_cells;
    use diroxide_core::place::PathStyle;

    #[test]
    fn titles_say_the_kind_first() {
        let docs = Place::parse_with(r"D:\docs\course.zip", PathStyle::Windows).unwrap();
        let root = docs.clone().into_archive();
        assert_eq!(title(&root, None), r"course.zip, archive, in D:\docs");
        let week1 = root.join("week1").unwrap();
        assert_eq!(title(&week1, None), "week1, in course.zip");
        let lying = Place::parse_with(r"D:\docs\a.zip", PathStyle::Windows)
            .unwrap()
            .into_archive();
        let sniffed = Sniffed {
            format: ArchiveFormat::SevenZ,
            by_name: Some(ArchiveFormat::Zip),
            signature: Some(Signature::SevenZ),
        };
        assert_eq!(
            title(&lying, Some(&sniffed)),
            r"a.zip, 7z archive, in D:\docs"
        );
        assert!(within_cells(&title(&lying, Some(&sniffed)), "7z archive"));
    }

    #[test]
    fn the_reading_line_and_the_refusal_fit_the_display() {
        let line = reading_line("backup.tar.xz", 3_200_000_000, Some(14_000_000_000));
        assert_eq!(line, "Reading backup.tar.xz: 3.2 of 14.0 GB");
        assert!(line.chars().count() <= 40);
        assert_eq!(
            reading_line("a.tar.gz", 999, Some(14_000_000_000)),
            "Reading a.tar.gz: 999 bytes of 14.0 GB"
        );
        assert_eq!(
            reading_line("a.tar.gz", 5_000, None),
            "Reading a.tar.gz: 5.0 KB"
        );
        assert!(within_cells(READ_ONLY, "Archives are read only here."));
        let said = read_only_refusal(&read_only_error()).unwrap();
        assert_eq!(said.text, READ_ONLY);
        assert!(read_only_refusal(&VfsError::NotFound).is_none());
    }
}
