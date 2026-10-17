//! Split file, as a plan (plan section 5.4; research/alpha2-plan.md 3.7, 5.3
//! and commits 23 and 24).
//!
//! A split writes the focused file, or each marked file in turn, as
//! numbered parts in the other panel (`report.pdf.001`, `report.pdf.002`)
//! and then the `.crc` sidecar ([`crate::sidecar`]). The part size comes
//! from the request's [`PartSize`]: a preset's bytes or a custom size
//! ([`PartSize::Bytes`]), or a number of equal parts
//! ([`PartSize::EqualParts`]). The plan says the outcome before anything is
//! written, use first: "12 parts of 20 MB, the last 3.1 MB", then "223 MB
//! in all", then "Enough space: 12 GB free on D:". An exact multiple makes
//! no empty last part. More than 999 parts asks first; more than 99,999 is
//! refused with the size to use instead.
//!
//! The presets themselves live in the configuration (`[[split_presets]]`);
//! the app passes a preset's part bytes, and shows the preset's own
//! details line (its capacity and headroom) beside the plan's, which says
//! the exact part bytes.

use diroxide_core::format::{count, grouped};
use diroxide_core::job::PartSize;
use diroxide_core::names::spell_out;
use diroxide_core::{Place, Timestamp};

use crate::part_names::{part_name, size_words};

/// The most parts a split makes without asking first: 999, the most three
/// digits hold, which every joiner reads.
pub const PARTS_WITHOUT_ASKING: u64 = 999;

/// The most parts a split makes at all.
pub const MOST_PARTS: u64 = 99_999;

/// A size rounded up to two figures, for "use parts of at least" advice,
/// so the size said is never below the size needed.
pub fn size_words_up(bytes: u64) -> String {
    let mut unit: u64 = 1;
    while unit <= u64::MAX / 1000 && bytes / unit >= 1000 {
        unit *= 1000;
    }
    if unit == 1 {
        return size_words(bytes);
    }
    let step = if bytes / unit < 100 { unit / 10 } else { unit };
    size_words(bytes.div_ceil(step).saturating_mul(step))
}

/// A drive or folder's root in words: "D:" for `D:\`, "/" for the root.
pub fn drive_words(root: &Place) -> String {
    let text = spell_out(&root.to_string()).into_owned();
    match text.strip_suffix('\\') {
        Some(drive) if drive.len() == 2 && drive.ends_with(':') => drive.to_owned(),
        _ => text,
    }
}

/// One file's split, as planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitFile {
    /// The file split.
    pub source: Place,
    /// Its size when planned.
    pub size: u64,
    /// Its modified time when planned: a change by the end fails the split.
    pub modified: Option<Timestamp>,
    /// The name the parts are numbered after: the file's own, or a free
    /// one ("report (2).pdf") when parts of that name are there already.
    pub base: String,
    /// The folder the parts go to.
    pub folder: Place,
    /// The size of each part but the last.
    pub part_bytes: u64,
    /// How many parts.
    pub parts: u64,
    /// The size of the last part: the part size for an exact multiple.
    pub last_bytes: u64,
    /// The sidecar.
    pub sidecar: Place,
}

impl SplitFile {
    /// The parts of a file of `size` bytes, by `rule`: `None` when the
    /// rule makes no parts (a part size or a count of zero, an empty file).
    pub fn sizes(size: u64, rule: PartSize) -> Option<(u64, u64, u64)> {
        let part_bytes = rule.part_bytes(size)?;
        let parts = rule.parts_for(size);
        if parts == 0 {
            return None;
        }
        let last = size - (parts - 1) * part_bytes;
        Some((part_bytes, parts, last))
    }

    /// The outcome, use first: "12 parts of 20 MB, the last 3.1 MB"; "12
    /// parts of 20 MB each" for an exact multiple; "1 part of 3.1 MB".
    pub fn outcome(&self) -> String {
        if self.parts == 1 {
            return format!("1 part of {}", size_words(self.last_bytes));
        }
        if self.last_bytes == self.part_bytes {
            return format!(
                "{} of {} each",
                count(self.parts, "part", "parts"),
                size_words(self.part_bytes)
            );
        }
        format!(
            "{} of {}, the last {}",
            count(self.parts, "part", "parts"),
            size_words(self.part_bytes),
            size_words(self.last_bytes)
        )
    }

    /// The exact bytes, for the details line: "Parts of 20,000,000 bytes,
    /// the last 3,145,728; 223,145,728 bytes in all".
    pub fn details_line(&self) -> String {
        if self.parts == 1 || self.last_bytes == self.part_bytes {
            format!(
                "Parts of {} bytes; {} bytes in all",
                grouped(self.part_bytes),
                grouped(self.size)
            )
        } else {
            format!(
                "Parts of {} bytes, the last {}; {} bytes in all",
                grouped(self.part_bytes),
                grouped(self.last_bytes),
                grouped(self.size)
            )
        }
    }

    /// The name of part `number`, from 1.
    pub fn part_name(&self, number: u64) -> String {
        part_name(&self.base, number)
    }

    /// The row said when the parts get another name than the file's, as
    /// parts of the file's name are there already: "Name taken: parts are
    /// report (2).pdf.001 and on".
    pub fn renamed_row(&self) -> Option<String> {
        let own = self.source.name()?;
        (own != self.base).then(|| {
            format!(
                "Name taken: parts are {} and on",
                spell_out(&self.part_name(1))
            )
        })
    }

    /// The source's name, spelled out.
    pub fn name(&self) -> String {
        self.source.name().map_or_else(
            || spell_out(&self.source.to_string()).into_owned(),
            |name| spell_out(name).into_owned(),
        )
    }
}

/// What a split plan knows beyond its steps: each file's parts, and the
/// room at the destination.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SplitFacts {
    /// The files, in the order they are split.
    pub files: Vec<SplitFile>,
    /// The destination's drive and its free bytes, when the provider says.
    pub space: Option<(Place, u64)>,
}

impl SplitFacts {
    /// The parts in all.
    pub fn parts(&self) -> u64 {
        self.files.iter().map(|file| file.parts).sum()
    }

    /// The bytes in all.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// The plan's rows, before anything is written: the outcome (per file
    /// when there are several, the name first), the total, and the room:
    /// "12 parts of 20 MB, the last 3.1 MB", "223 MB in all", "Enough
    /// space: 12 GB free on D:".
    pub fn rows(&self) -> Vec<String> {
        let mut rows: Vec<String> = match self.files.as_slice() {
            [one] => vec![one.outcome()],
            many => many
                .iter()
                .map(|file| format!("{}: {}", file.name(), file.outcome()))
                .collect(),
        };
        if self.files.is_empty() {
            return rows;
        }
        rows.extend(self.files.iter().filter_map(SplitFile::renamed_row));
        rows.push(format!("{} in all", size_words(self.bytes())));
        if let Some((drive, free)) = &self.space {
            let room = if *free >= self.bytes() {
                "Enough space"
            } else {
                "Not enough space"
            };
            rows.push(format!(
                "{room}: {} free on {}",
                size_words(*free),
                drive_words(drive)
            ));
        }
        rows
    }

    /// The refusal past [`MOST_PARTS`], with the size to use instead:
    /// "Too many parts: 150,000. Use parts of 6.1 MB or more."
    pub fn too_many(size: u64, parts: u64) -> String {
        format!(
            "Too many parts: {}. Use parts of {} or more.",
            grouped(parts),
            size_words_up(size.div_ceil(MOST_PARTS))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn place(text: &str) -> Place {
        Place::parse(&if cfg!(windows) {
            format!("D:{}", text.replace('/', "\\"))
        } else {
            text.to_owned()
        })
        .unwrap()
    }

    fn file(size: u64, rule: PartSize) -> SplitFile {
        let (part_bytes, parts, last_bytes) = SplitFile::sizes(size, rule).unwrap();
        SplitFile {
            source: place("/a/report.pdf"),
            size,
            modified: None,
            base: "report.pdf".into(),
            folder: place("/b"),
            part_bytes,
            parts,
            last_bytes,
            sidecar: place("/b/report.pdf.crc"),
        }
    }

    #[test]
    fn the_outcome_is_said_before_anything_is_written() {
        let split = file(223_100_000, PartSize::Bytes(20_000_000));
        assert_eq!(split.parts, 12);
        assert_eq!(split.outcome(), "12 parts of 20 MB, the last 3.1 MB");
        assert!(within_cells(
            &split.outcome(),
            "12 parts of 20 MB, the last 3.1 MB"
        ));
        assert_eq!(
            split.details_line(),
            "Parts of 20,000,000 bytes, the last 3,100,000; 223,100,000 bytes in all"
        );
        let facts = SplitFacts {
            files: vec![split],
            space: Some((place("/"), 12_000_000_000)),
        };
        let rows = facts.rows();
        assert_eq!(rows[1], "223 MB in all");
        let drive = if cfg!(windows) { "D:" } else { "/" };
        assert_eq!(rows[2], format!("Enough space: 12 GB free on {drive}"));
        for row in &rows {
            assert!(row.chars().count() <= 40, "{row}");
        }
    }

    #[test]
    fn an_exact_multiple_makes_no_empty_last_part() {
        let exact = file(240_000_000, PartSize::Bytes(20_000_000));
        assert_eq!((exact.parts, exact.last_bytes), (12, 20_000_000));
        assert_eq!(exact.outcome(), "12 parts of 20 MB each");
        let one = file(3_100_000, PartSize::Bytes(20_000_000));
        assert_eq!(one.outcome(), "1 part of 3.1 MB");
        assert_eq!(
            SplitFile::sizes(0, PartSize::Bytes(10)),
            None,
            "nothing to split"
        );
        assert_eq!(SplitFile::sizes(10, PartSize::Bytes(0)), None);
        assert_eq!(SplitFile::sizes(10, PartSize::EqualParts(0)), None);
    }

    #[test]
    fn equal_parts_are_as_equal_as_whole_bytes_allow() {
        let equal = file(10, PartSize::EqualParts(3));
        assert_eq!((equal.part_bytes, equal.parts, equal.last_bytes), (4, 3, 2));
        let few = file(2, PartSize::EqualParts(5));
        assert_eq!(few.parts, 2, "never more parts than bytes");
        let big = file(1_000_000_000, PartSize::EqualParts(4));
        assert_eq!(big.outcome(), "4 parts of 250 MB each");
    }

    /// The preset sizes of research/alpha2-plan.md 5.3, as the app passes
    /// them (the configuration's part bytes after headroom), each with the
    /// split it gives. The figures themselves are tied to their sources by
    /// the configuration's own test. The floppy row is the formatted disk's
    /// data area, 1,457,664 bytes (2,847 clusters of 512 bytes), the change
    /// this brief asks of the configuration: one percent off the raw
    /// 1,474,560 would give parts that do not fit.
    #[test]
    fn the_preset_sizes_split_as_their_rows_say() {
        let presets: [(&str, u64, u64, &str); 7] = [
            (
                "Email attachment",
                18_000_000,
                60_000_000,
                "4 parts of 18 MB, the last 6 MB",
            ),
            (
                "GitHub file limit",
                104_857_600,
                300_000_000,
                "3 parts of 105 MB, the last 90.3 MB",
            ),
            (
                "CD",
                729_907_200,
                1_500_000_000,
                "3 parts of 730 MB, the last 40.2 MB",
            ),
            (
                "FAT32's largest file",
                4_294_967_295,
                8_589_934_590,
                "2 parts of 4.3 GB each",
            ),
            (
                "DVD",
                4_653_369_262,
                9_000_000_000,
                "2 parts of 4.7 GB, the last 4.3 GB",
            ),
            (
                "Blu-ray",
                24_775_061_667,
                30_000_000_000,
                "2 parts of 24.8 GB, the last 5.2 GB",
            ),
            (
                "Floppy",
                1_457_664,
                3_000_000,
                "3 parts of 1.5 MB, the last 84.7 KB",
            ),
        ];
        for (name, part_bytes, size, outcome) in presets {
            let split = file(size, PartSize::Bytes(part_bytes));
            assert_eq!(split.outcome(), outcome, "{name}");
            assert!(
                split.details_line().contains(&grouped(part_bytes)),
                "{name}"
            );
        }
    }

    #[test]
    fn too_many_parts_names_the_size_to_use() {
        let line = SplitFacts::too_many(600_000_000_000, 600_000);
        assert_eq!(
            line,
            "Too many parts: 600,000. Use parts of 6.1 MB or more."
        );
        assert!(within_cells(&line, "Too many parts: 600,000."));
        assert_eq!(size_words_up(6_000_060), "6.1 MB");
        assert_eq!(size_words_up(99_950_001), "100 MB");
        assert_eq!(size_words_up(512), "512 bytes");
        let needed = 600_000_000_000u64.div_ceil(MOST_PARTS);
        assert!(6_100_000 >= needed, "never below the size needed");
    }
}
