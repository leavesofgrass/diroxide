//! Every line the plugin says or hands the host, in one place, so each is
//! tested at 40 cells with its meaning first (the owner reads one line of
//! a 40-cell Braille display at a time). Words before symbols; sizes in
//! the app's decimal units ([`human_size`]); a cause is lowercase words
//! for the host's "Could not VERB OBJECT: CAUSE." line, and a next step is
//! a sentence of its own.

use diroxide_core::format::{count, human_size};

/// The extra key a row's words travel under: "Partition 2, home, ext4,
/// 40.0 GB" for a partition, a note such as "compressed by NTFS, read it
/// with 7-Zip" for a file.
pub const ABOUT: &str = "disk.about";

/// What a partition holds, for its row: a label, the table's type words,
/// and the file system found in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartitionWords<'a> {
    /// Its number, from 1, in table order (logical MBR partitions from 5).
    pub number: u32,
    /// The partition's name in the table, or the file system's label.
    pub label: Option<&'a str>,
    /// The table's type in words: "EFI System", "Linux filesystem".
    pub kind: Option<&'a str>,
    /// The file system found in it: "ext4", "FAT32", "NTFS".
    pub file_system: Option<&'a str>,
    /// Its size in bytes.
    pub size: u64,
    /// Locked by encryption (BitLocker, LUKS, FileVault): shown, never
    /// opened.
    pub locked: bool,
}

/// A partition's row, number first: "Partition 2, home, ext4, 40.0 GB",
/// "Partition 1, EFI System, FAT32, 537 MB", "Partition 3, BitLocker,
/// 40.0 GB, locked". The label wins over the type words; the type words
/// are left out when the file system says the same.
pub fn partition_row(words: &PartitionWords<'_>) -> String {
    let mut parts = vec![format!("Partition {}", words.number)];
    if words.locked {
        // What locks it and that it is locked, inside 40 cells; the
        // table's words would push "locked" past the line.
        parts.push(words.file_system.unwrap_or("encryption").to_owned());
        parts.push(human_size(words.size));
        parts.push("locked".to_owned());
        return parts.join(", ");
    }
    let label = words.label.map(str::trim).filter(|l| !l.is_empty());
    match (label, words.kind) {
        (Some(label), _) => parts.push(label.to_owned()),
        (None, Some(kind)) if Some(kind) != words.file_system => parts.push(kind.to_owned()),
        _ => {}
    }
    if let Some(fs) = words.file_system
        && !parts.iter().any(|p| p == fs)
    {
        parts.push(fs.to_owned());
    }
    parts.push(human_size(words.size));
    parts.join(", ")
}

/// A volume's name for the volumes list, without the size the host adds:
/// "Partition 2, home, ext4".
pub fn partition_volume(words: &PartitionWords<'_>) -> String {
    let row = partition_row(&PartitionWords {
        locked: false,
        ..words.clone()
    });
    match row.rsplit_once(", ") {
        Some((name, _size)) => name.to_owned(),
        None => row,
    }
}

/// The title line inside a partition: "Partition 2, in disk.img".
pub fn partition_title(number: u32, image: &str) -> String {
    format!("Partition {number}, in {image}")
}

/// The title line of a folder in an image: "EFI, in fedora.iso".
pub fn folder_title(folder: &str, image: &str) -> String {
    format!("{folder}, in {image}")
}

/// The details' first row: "Disk image: GPT, 2 partitions, 8.0 GB", or
/// for one file system with no table "Disk image: ISO 9660, 4.7 GB".
pub fn details_first(layout: &str, partitions: Option<usize>, size: u64) -> String {
    match partitions {
        Some(n) => format!(
            "Disk image: {layout}, {}, {}",
            count(n as u64, "partition", "partitions"),
            human_size(size)
        ),
        None => format!("Disk image: {layout}, {}", human_size(size)),
    }
}

/// Said when an image opens: "Opened disk.vhdx: 3 partitions, 120.0 GB",
/// or "Opened fedora.iso: ISO 9660, 2.1 GB".
pub fn opened(image: &str, layout: &str, partitions: Option<usize>, size: u64) -> String {
    match partitions {
        Some(n) => format!(
            "Opened {image}: {}, {}",
            count(n as u64, "partition", "partitions"),
            human_size(size)
        ),
        None => format!("Opened {image}: {layout}, {}", human_size(size)),
    }
}

/// The sparse guard's line, both figures: "Sparse: claims 2.2 TB, holds
/// 1.1 GB". The longest two sizes still fit 40 cells.
pub fn sparse(claimed: u64, held: u64) -> String {
    format!(
        "Sparse: claims {}, holds {}",
        human_size(claimed),
        human_size(held)
    )
}

/// Images inside images past the limit: "Nested too deep: 4 levels
/// allowed".
pub fn nested_too_deep(most: usize) -> String {
    format!(
        "Nested too deep: {} allowed",
        count(most as u64, "level", "levels")
    )
}

/// The image's size or time changed under an open place.
pub const IMAGE_CHANGED: &str = "Image changed since it was opened";

/// A listing that came from 7-Zip, said once an image.
pub const READ_WITH_7ZIP: &str = "Read with 7-Zip: no built-in reader";

/// 7-Zip is needed and is not there.
pub const NEEDS_7ZIP: &str = "Needs 7-Zip for this image: not found";

/// A folder that leads back to one it is inside was left out.
pub const LOOP_SKIPPED: &str = "Skipped a loop in the image";

/// A file NTFS compressed: its row's note.
pub const NTFS_COMPRESSED: &str = "compressed by NTFS, read it with 7-Zip";

/// A file NTFS encrypted: its row's note.
pub const NTFS_ENCRYPTED: &str = "encrypted by NTFS, not readable here";

/// Names a listing left out because the rules refused them: "3 names
/// skipped", "1 name skipped".
pub fn names_skipped(n: usize) -> String {
    format!("{} skipped", count(n as u64, "name", "names"))
}

/// Loops and names left out of one listing, said once after it: "Skipped
/// a loop in the image", "3 names skipped", or both in one short line, "3
/// names and a loop skipped".
pub fn listing_notes(loops: usize, names: usize) -> Option<String> {
    match (loops, names) {
        (0, 0) => None,
        (_, 0) => Some(LOOP_SKIPPED.to_owned()),
        (0, n) => Some(names_skipped(n)),
        (_, n) => Some(format!(
            "{} and a loop skipped",
            count(n as u64, "name", "names")
        )),
    }
}

/// An NTFS file's other streams, in its row's note: "Has alternate
/// streams: 2".
pub fn alternate_streams(n: usize) -> String {
    format!("Has alternate streams: {n}")
}

/// A link's row note, its target spelled out: "Link to: ../lib/x".
pub fn link_to(target: &str) -> String {
    format!("Link to: {}", diroxide_core::names::display_name(target))
}

/// The causes, lowercase, for the host's "Could not VERB OBJECT: CAUSE."
/// line, each ending with a period when a next step follows it.
pub mod cause {
    use diroxide_core::format::human_size;

    /// No reader in the plugin and no 7-Zip either.
    pub fn needs_7zip(what: &str) -> String {
        format!("it needs 7-Zip to read {what}.")
    }

    /// The next step after [`needs_7zip`].
    pub const GET_7ZIP: &str = "Get 7-Zip from Manage optional components.";

    /// Nothing the plugin knows is in the image.
    pub const NOT_AN_IMAGE: &str = "no reader knows this image";

    /// A partition that holds no files (swap, a BIOS boot area).
    pub fn no_files(what: &str) -> String {
        format!("it holds {what}, which has no files")
    }

    /// A locked partition, never opened.
    pub fn locked(by: &str) -> String {
        format!("it is locked by {by}, and diroxide does not unlock it")
    }

    /// A header or table that does not add up, in words: "its GPT is
    /// damaged: the header's checksum is wrong".
    pub fn damaged(what: &str, why: &str) -> String {
        format!("its {what} is damaged: {why}")
    }

    /// A claim larger than the guard allows: "its block map claims 3.0
    /// GB, over the 268.4 MB limit".
    pub fn claims_too_much(what: &str, claimed: u64, limit: u64) -> String {
        format!(
            "its {what} claims {}, over the {} limit",
            human_size(claimed),
            human_size(limit)
        )
    }

    /// The 7-Zip fallback or a convert would write more than the spool
    /// budget allows.
    pub fn too_large_to_unpack(claimed: u64, held: u64) -> String {
        format!(
            "it is too large to unpack here: claims {}, holds {}",
            human_size(claimed),
            human_size(held)
        )
    }

    /// A feature the reader refuses, in its words.
    pub fn uses(what: &str) -> String {
        format!("it uses {what}, which this plugin does not read")
    }

    /// The image is nested past the limit.
    pub fn nested(most: usize) -> String {
        format!("images are nested more than {most} levels deep")
    }

    /// The image changed under an open place.
    pub const CHANGED: &str = "the image changed since it was opened";

    /// A path goes deeper than the cap.
    pub fn too_deep(most: usize) -> String {
        format!("it is more than {most} folders deep in the image")
    }

    /// A folder lists more entries than the limit.
    pub fn too_many_entries(most: usize) -> String {
        format!(
            "the folder has more than {} entries",
            diroxide_core::format::grouped(most as u64)
        )
    }

    /// Reading the image itself failed, with the cause in words.
    pub fn read_failed(why: &str) -> String {
        format!("reading failed: {why}")
    }

    /// A file NTFS compressed, which the built-in reader cannot read.
    pub const NTFS_COMPRESSED: &str = "NTFS compressed it, and only 7-Zip reads that.";

    /// A password-protected image through 7-Zip: 7-Zip reads a password
    /// from a pipe (spike DS4), and this plugin asks none.
    pub const PASSWORD: &str = "it is encrypted, and this plugin asks no password";

    /// 7-Zip is older than extraction allows.
    pub fn old_7zip(found: &str, needs: &str) -> String {
        format!("7-Zip {found} is too old: needs {needs}")
    }
}

#[cfg(test)]
mod tests {
    use diroxide_core::format::{BRAILLE_CELLS, within_cells};

    use super::*;

    fn fits(line: &str) {
        assert!(
            line.chars().count() <= BRAILLE_CELLS,
            "{line:?} is {} cells",
            line.chars().count()
        );
    }

    #[test]
    fn partition_rows_say_the_number_first_and_fit_40_cells() {
        let home = PartitionWords {
            number: 2,
            label: Some("home"),
            kind: Some("Linux filesystem"),
            file_system: Some("ext4"),
            size: 40_000_000_000,
            locked: false,
        };
        assert_eq!(partition_row(&home), "Partition 2, home, ext4, 40.0 GB");
        assert_eq!(partition_volume(&home), "Partition 2, home, ext4");
        let efi = PartitionWords {
            number: 1,
            label: None,
            kind: Some("EFI System"),
            file_system: Some("FAT32"),
            size: 536_870_912,
            locked: false,
        };
        assert_eq!(
            partition_row(&efi),
            "Partition 1, EFI System, FAT32, 537 MB"
        );
        let locked = PartitionWords {
            number: 3,
            label: None,
            kind: Some("Microsoft basic data"),
            file_system: Some("BitLocker"),
            size: 40_000_000_000,
            locked: true,
        };
        assert_eq!(
            partition_row(&locked),
            "Partition 3, BitLocker, 40.0 GB, locked"
        );
        assert!(within_cells(&partition_row(&locked), "locked"));
        for line in [
            partition_row(&home),
            partition_row(&efi),
            partition_row(&locked),
        ] {
            fits(&line);
        }
        let bare = PartitionWords {
            number: 4,
            size: 1_000_000,
            ..PartitionWords::default()
        };
        assert_eq!(partition_row(&bare), "Partition 4, 1.0 MB");
        let same = PartitionWords {
            number: 1,
            kind: Some("NTFS"),
            file_system: Some("NTFS"),
            size: 1_000_000,
            ..PartitionWords::default()
        };
        assert_eq!(partition_row(&same), "Partition 1, NTFS, 1.0 MB");
    }

    #[test]
    fn titles_details_and_the_opened_line_fit_40_cells() {
        let lines = [
            (folder_title("EFI", "fedora.iso"), "EFI, in fedora.iso"),
            (partition_title(2, "disk.img"), "Partition 2, in disk.img"),
            (
                details_first("GPT", Some(2), 8_000_000_000),
                "Disk image: GPT, 2 partitions, 8.0 GB",
            ),
            (
                details_first("ISO 9660", None, 4_700_000_000),
                "Disk image: ISO 9660, 4.7 GB",
            ),
            (
                opened("disk.vhdx", "GPT", Some(3), 120_000_000_000),
                "Opened disk.vhdx: 3 partitions, 120 GB",
            ),
            (
                sparse(2_199_023_255_552, 1_073_741_824),
                "Sparse: claims 2.2 TB, holds 1.1 GB",
            ),
            (nested_too_deep(4), "Nested too deep: 4 levels allowed"),
            (names_skipped(3), "3 names skipped"),
            (names_skipped(1), "1 name skipped"),
            (alternate_streams(2), "Has alternate streams: 2"),
            (link_to("../lib/x"), "Link to: ../lib/x"),
        ];
        for (line, want) in lines {
            assert_eq!(line, want);
            fits(&line);
        }
        for line in [
            IMAGE_CHANGED,
            READ_WITH_7ZIP,
            NEEDS_7ZIP,
            LOOP_SKIPPED,
            NTFS_COMPRESSED,
            NTFS_ENCRYPTED,
        ] {
            fits(line);
        }
        // The longest sizes still fit.
        fits(&sparse(999_900_000_000_000, 999_900_000));
        assert_eq!(listing_notes(0, 0), None);
        assert_eq!(listing_notes(1, 0).as_deref(), Some(LOOP_SKIPPED));
        assert_eq!(
            listing_notes(1, 3).as_deref(),
            Some("3 names and a loop skipped")
        );
        fits(&listing_notes(2, 999_999).unwrap_or_default());
    }

    #[test]
    fn causes_read_after_could_not_open_with_the_meaning_first() {
        // The shortest object leaves the cause 18 cells; a longer name
        // pushes the cause along, as every "Could not" line does.
        let could = |cause: &str| format!("Could not open a.vhd: {}", cause);
        for (cause, needle) in [
            (cause::needs_7zip("exFAT"), "it needs 7-Zip"),
            (cause::NOT_AN_IMAGE.to_owned(), "no reader knows"),
            (cause::no_files("Linux swap"), "it holds Linux"),
            (cause::locked("BitLocker"), "it is locked by"),
            (
                cause::damaged("GPT", "the header's checksum is wrong"),
                "its GPT is damaged",
            ),
            (
                cause::claims_too_much("block map", 3_000_000_000, 268_435_456),
                "its block map",
            ),
            (
                cause::too_large_to_unpack(2_199_023_255_552, 1_000_000),
                "it is too large",
            ),
            (cause::uses("inline data"), "it uses inline"),
            (cause::nested(4), "images are nested"),
            (cause::CHANGED.to_owned(), "the image changed"),
            (cause::too_many_entries(1_000_000), "the folder has"),
            (cause::read_failed("timed out"), "reading failed"),
            (cause::NTFS_COMPRESSED.to_owned(), "NTFS compressed it"),
            (cause::PASSWORD.to_owned(), "it is encrypted"),
            (cause::old_7zip("24.08", "25.01"), "7-Zip 24.08 is too"),
        ] {
            let line = could(&cause);
            assert!(within_cells(&line, needle), "{line}");
        }
        assert!(within_cells(cause::GET_7ZIP, "Get 7-Zip from"));
        assert_eq!(
            cause::too_many_entries(1_000_000),
            "the folder has more than 1,000,000 entries"
        );
    }
}
