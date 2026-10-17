//! Part names and sibling discovery (plan section 5.4; research/alpha2-plan.md
//! 5.4 and commit 22): how split parts are named, and how a join finds a
//! whole set from any one of its parts.
//!
//! The rules, as research/alpha2-plan.md 5.4 writes them:
//!
//! - **Names.** A part is the whole file's name, a dot and its number in
//!   at least three digits: `report.pdf.001`. Past 999 the number simply
//!   grows (`report.pdf.1000`) and is never wrapped. Order is numeric,
//!   never by text, so `.1000` comes after `.999`.
//! - **From any part.** The set is the name without its number, and every
//!   sibling with a number written the same way: three digits at least
//!   (the plain way), or zero-padded to one longer width (`.0001`, a way
//!   some splitters write). A part whose name fits both ways (`.1000`
//!   beside `.001` and `.0001` sets) asks which.
//! - **Where a set starts.** At `.001`, as 7-Zip, Total Commander and
//!   diroxide write; or at `.000`, as Krusader writes, which is accepted and
//!   said. A folder holding both `.000` and `.001` for one name asks,
//!   unless the checksum file's size settles it.
//! - **What is wrong with a set,** each named before anything is written:
//!   a gap, an empty part, a middle part of another size, a last part
//!   longer than the first, or parts that do not add up to the checksum
//!   file's size.
//! - **Not byte splits.** Spanned zip sets (`.z01`, `.z02`, `.zip`) and RAR
//!   volumes (`.r00`, `.part1.rar`) are not joined by putting their bytes
//!   together, and the refusal says so.

use std::fmt;

use diroxide_core::format::{count, human_size};
use diroxide_core::names::{spell_out, split_part};
use diroxide_core::{Entry, EntryKind, Place, Timestamp};

/// The name of part `number` of `base`: `report.pdf.001`, three digits at
/// least, growing past 999 (`report.pdf.1000`).
pub fn part_name(base: &str, number: u64) -> String {
    format!("{base}.{number:03}")
}

/// A size as the split and join lines say it: decimal units, and no ".0"
/// on a round number ("20 MB", "3.1 MB", "512 bytes").
pub fn size_words(bytes: u64) -> String {
    let words = human_size(bytes);
    match words.split_once(".0 ") {
        Some((number, unit)) => format!("{number} {unit}"),
        None => words,
    }
}

/// How a set writes its numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    /// Three digits at least, growing past 999: `.001`, `.1000`.
    Plain,
    /// Zero-padded to this many digits, more than three: `.0001`.
    Padded(usize),
}

impl Width {
    /// The digits of a number written this way.
    pub fn digits(self, number: u32) -> String {
        match self {
            Width::Plain => format!("{number:03}"),
            Width::Padded(width) => format!("{number:0width$}"),
        }
    }

    /// Whether a number written with `width` digits is written this way.
    fn holds(self, number: u32, width: usize) -> bool {
        match self {
            Width::Plain => width == plain_width(number),
            Width::Padded(padded) => width == padded,
        }
    }
}

/// How many digits the plain way writes a number with.
fn plain_width(number: u32) -> usize {
    number.to_string().len().max(3)
}

/// One part of a set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundPart {
    /// Its number: 1 for `.001`.
    pub number: u32,
    /// Where it is.
    pub place: Place,
    /// Its size in bytes, as listed.
    pub size: u64,
    /// Its modified time, as listed.
    pub modified: Option<Timestamp>,
}

/// Why a name cannot be joined as a set of byte parts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotJoinable {
    /// The name has no part number.
    NotAPart(String),
    /// A spanned zip set: parts of a zip, not bytes split.
    SpannedZip,
    /// RAR volumes.
    RarVolumes,
}

impl fmt::Display for NotJoinable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotJoinable::NotAPart(name) => {
                write!(
                    f,
                    "Not a numbered part: {}. Nothing was joined.",
                    spell_out(name)
                )
            }
            NotJoinable::SpannedZip => {
                f.write_str("Spanned zip sets need the 7z tool. Nothing was joined.")
            }
            NotJoinable::RarVolumes => {
                f.write_str("RAR volumes need the 7z tool. Nothing was joined.")
            }
        }
    }
}

/// What is wrong with a set, each said with its part's number.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SetProblem {
    /// A number between the first and the last has no part.
    Missing(u32),
    /// A part has no bytes.
    Empty(u32),
    /// A part before the last is not the first part's size.
    WrongSize {
        /// The part.
        number: u32,
        /// Its size.
        size: u64,
        /// The size expected: the first part's.
        expected: u64,
    },
    /// The last part is longer than the first.
    LastLonger {
        /// The last part.
        number: u32,
        /// Its size.
        size: u64,
        /// The first part's size.
        first: u64,
    },
    /// The parts do not add up to the checksum file's size.
    Total {
        /// What the parts add up to.
        total: u64,
        /// What the checksum file says.
        expected: u64,
    },
}

impl SetProblem {
    /// Whether the user may join anyway: only a middle part of another
    /// size, when no checksum file says the size.
    pub fn can_continue(&self) -> bool {
        matches!(self, SetProblem::WrongSize { .. })
    }
}

/// A set of parts found from one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartSet {
    /// The folder the parts are in.
    pub folder: Place,
    /// The name without the part number: the whole file's name.
    pub base: String,
    /// How the set writes its numbers.
    pub width: Width,
    /// The parts, in numeric order.
    pub parts: Vec<FoundPart>,
    /// Where numbering starts: 1, or 0 for a Krusader set.
    pub first: u32,
    /// What discovery found worth saying: "Parts start at 000", the parts
    /// written another way and left out.
    pub notes: Vec<String>,
}

impl PartSet {
    /// The bytes of all the parts, as listed.
    pub fn total(&self) -> u64 {
        self.parts.iter().map(|part| part.size).sum()
    }

    /// The places of the parts, in order.
    pub fn places(&self) -> Vec<Place> {
        self.parts.iter().map(|part| part.place.clone()).collect()
    }

    /// "Part 004", in the set's own digits.
    pub fn label(&self, number: u32) -> String {
        format!("Part {}", self.width.digits(number))
    }

    /// A part's row, the number first: "Part 004, 20 MB".
    pub fn row(&self, part: &FoundPart) -> String {
        format!("{}, {}", self.label(part.number), size_words(part.size))
    }

    /// What is wrong with the set: the gaps, then the empty parts, then the
    /// sizes. With the checksum file's size, the sum decides; without it,
    /// every part but the last must be the first part's size, and the last
    /// no longer.
    pub fn problems(&self, sidecar_size: Option<u64>) -> Vec<SetProblem> {
        let mut problems = Vec::new();
        let mut expected = self.first;
        for part in &self.parts {
            while expected < part.number {
                problems.push(SetProblem::Missing(expected));
                expected += 1;
            }
            expected = part.number.saturating_add(1);
        }
        problems.extend(
            self.parts
                .iter()
                .filter(|part| part.size == 0)
                .map(|part| SetProblem::Empty(part.number)),
        );
        let Some(first) = self.parts.first().map(|part| part.size) else {
            return problems;
        };
        let last = self.parts.len() - 1;
        let middle = self.parts[..last]
            .iter()
            .find(|part| part.size != first && part.size > 0);
        match sidecar_size {
            Some(size) if self.total() == size => {}
            Some(size) => {
                if let Some(part) = middle {
                    problems.push(SetProblem::WrongSize {
                        number: part.number,
                        size: part.size,
                        expected: first,
                    });
                } else {
                    problems.push(SetProblem::Total {
                        total: self.total(),
                        expected: size,
                    });
                }
            }
            None => {
                if let Some(part) = middle {
                    problems.push(SetProblem::WrongSize {
                        number: part.number,
                        size: part.size,
                        expected: first,
                    });
                }
                let end = &self.parts[last];
                if last > 0 && end.size > first {
                    problems.push(SetProblem::LastLonger {
                        number: end.number,
                        size: end.size,
                        first,
                    });
                }
            }
        }
        problems
    }

    /// A problem in words, the part first, ending with what was done:
    /// "Part 004 is missing. Nothing was joined.", "Part 007 is 18 KB,
    /// expected 20 MB."
    pub fn sentence(&self, problem: &SetProblem, all: &[SetProblem]) -> String {
        match problem {
            SetProblem::Missing(number) => {
                let others = all
                    .iter()
                    .filter(|p| matches!(p, SetProblem::Missing(_)))
                    .count()
                    .saturating_sub(1);
                if others == 0 {
                    format!("{} is missing. Nothing was joined.", self.label(*number))
                } else {
                    format!(
                        "{} is missing, and {} more. Nothing was joined.",
                        self.label(*number),
                        others
                    )
                }
            }
            SetProblem::Empty(number) => {
                format!("{} is empty. Nothing was joined.", self.label(*number))
            }
            SetProblem::WrongSize {
                number,
                size,
                expected,
            } => format!(
                "{} is {}, expected {}.",
                self.label(*number),
                size_words(*size),
                size_words(*expected)
            ),
            SetProblem::LastLonger {
                number,
                size,
                first,
            } => format!(
                "{} is {}, longer than the first, {}.",
                self.label(*number),
                size_words(*size),
                size_words(*first)
            ),
            SetProblem::Total { total, expected } => format!(
                "Parts add up to {}, the checksum file says {}.",
                size_words(*total),
                size_words(*expected)
            ),
        }
    }
}

/// What discovery found from one part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovery {
    /// One set.
    Found(PartSet),
    /// Two sets could be meant; the question says which is Yes. The
    /// answer starts on No.
    Ask {
        /// The question, ending "y or n": "Part 000 is there too. Join
        /// from 000? y or n".
        question: String,
        /// The set a Yes joins.
        yes: Box<PartSet>,
        /// The set a No joins.
        no: Box<PartSet>,
    },
}

/// Whether a name is a spanned zip's or a RAR set's.
fn not_bytes(name: &str, siblings: &[&str]) -> Option<NotJoinable> {
    let lower = name.to_lowercase();
    let numbered = |prefix: char| {
        let (stem, extension) = lower.rsplit_once('.')?;
        let digits = extension.strip_prefix(prefix)?;
        (digits.len() == 2 && digits.bytes().all(|b| b.is_ascii_digit())).then_some(stem)
    };
    if numbered('z').is_some() {
        return Some(NotJoinable::SpannedZip);
    }
    if numbered('r').is_some() {
        return Some(NotJoinable::RarVolumes);
    }
    if let Some(stem) = lower.strip_suffix(".rar") {
        let volume = stem
            .rsplit_once(".part")
            .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        let sibling = format!("{stem}.r00");
        if volume || siblings.iter().any(|s| s.to_lowercase() == sibling) {
            return Some(NotJoinable::RarVolumes);
        }
    }
    if let Some(stem) = lower.strip_suffix(".zip") {
        let sibling = format!("{stem}.z01");
        if siblings.iter().any(|s| s.to_lowercase() == sibling) {
            return Some(NotJoinable::SpannedZip);
        }
    }
    None
}

/// A sibling that is a numbered part of the same base.
struct Sibling<'a> {
    entry: &'a Entry,
    number: u32,
    width: usize,
}

/// Finds the set a part belongs to, from the listing of its folder. `from`
/// is the chosen part's name; `case_sensitive` is the folder's rule for
/// names. Discovery reads nothing but the listing.
pub fn find_set(
    folder: &Place,
    entries: &[Entry],
    from: &str,
    case_sensitive: bool,
) -> Result<Discovery, NotJoinable> {
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    if let Some(refusal) = not_bytes(from, &names) {
        return Err(refusal);
    }
    let chosen = split_part(from).ok_or_else(|| NotJoinable::NotAPart(from.to_owned()))?;
    let fold = |text: &str| {
        if case_sensitive {
            text.to_owned()
        } else {
            text.to_lowercase()
        }
    };
    let base_key = fold(chosen.base);
    let siblings: Vec<Sibling<'_>> = entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::File | EntryKind::Archive))
        .filter_map(|entry| {
            let part = split_part(&entry.name)?;
            (fold(part.base) == base_key).then_some(Sibling {
                entry,
                number: part.number,
                width: part.width,
            })
        })
        .collect();
    let padded_width = (chosen.width > 3).then_some(chosen.width);
    let leading_zero = chosen.width > chosen.number.to_string().len();
    let any_plain = siblings.iter().any(|s| s.width == 3);
    let any_padded = padded_width.is_some_and(|width| {
        siblings
            .iter()
            .any(|s| s.width == width && s.width > s.number.to_string().len())
    });
    let base = chosen.base.to_owned();
    let build = |width: Width| set_of(folder, &base, width, &siblings);
    match padded_width {
        Some(width) if leading_zero => Ok(start_of(build(Width::Padded(width)))),
        Some(width) if any_plain && any_padded => {
            let yes = settle_start(build(Width::Plain));
            let no = settle_start(build(Width::Padded(width)));
            Ok(Discovery::Ask {
                question: format!(
                    "Parts are numbered two ways, 001 and {}. Join the 001 set? y or n",
                    Width::Padded(width).digits(1)
                ),
                yes: Box::new(yes),
                no: Box::new(no),
            })
        }
        Some(width) if any_padded => Ok(start_of(build(Width::Padded(width)))),
        _ => Ok(start_of(build(Width::Plain))),
    }
}

/// The set of one width, every sibling written that way, in numeric order,
/// with a note for the parts written another way.
fn set_of(folder: &Place, base: &str, width: Width, siblings: &[Sibling<'_>]) -> PartSet {
    let mut parts: Vec<FoundPart> = Vec::new();
    let mut other = 0u64;
    for sibling in siblings {
        if width.holds(sibling.number, sibling.width) {
            let Ok(place) = folder.join(&sibling.entry.name) else {
                continue;
            };
            parts.push(FoundPart {
                number: sibling.number,
                place,
                size: sibling.entry.size.unwrap_or(0),
                modified: sibling.entry.modified,
            });
        } else {
            other += 1;
        }
    }
    parts.sort_by_key(|part| part.number);
    let mut notes = Vec::new();
    if other > 0 {
        notes.push(format!(
            "Left out {} numbered another way.",
            count(other, "part", "parts")
        ));
    }
    PartSet {
        folder: folder.clone(),
        base: base.to_owned(),
        width,
        parts,
        first: 1,
        notes,
    }
}

/// Where a set starts: at 001, or at 000 when 000 is there and 001 is
/// not; when both are there, Discovery asks.
fn start_of(set: PartSet) -> Discovery {
    let zero = set.parts.first().is_some_and(|part| part.number == 0);
    let one = set.parts.iter().any(|part| part.number == 1);
    match (zero, one) {
        (true, true) => {
            let mut yes = set.clone();
            yes.first = 0;
            yes.notes.push("Parts start at 000.".to_owned());
            let mut no = set;
            no.parts.retain(|part| part.number != 0);
            no.notes.push("Part 000 left out.".to_owned());
            Discovery::Ask {
                question: "Part 000 is there too. Join from 000? y or n".to_owned(),
                yes: Box::new(yes),
                no: Box::new(no),
            }
        }
        (true, false) => {
            let mut set = set;
            set.first = 0;
            set.notes.push("Parts start at 000.".to_owned());
            Discovery::Found(set)
        }
        _ => Discovery::Found(set),
    }
}

/// A set inside another question: its start settled the default way, at
/// 001 when both 000 and 001 are there.
fn settle_start(set: PartSet) -> PartSet {
    match start_of(set) {
        Discovery::Found(set) => set,
        Discovery::Ask { no, .. } => *no,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn folder() -> Place {
        Place::parse(if cfg!(windows) { "D:\\out" } else { "/out" }).unwrap()
    }

    fn file(name: &str, size: u64) -> Entry {
        Entry::new(name, EntryKind::File).with_size(size)
    }

    fn parts(names: &[(&str, u64)]) -> Vec<Entry> {
        names.iter().map(|(name, size)| file(name, *size)).collect()
    }

    fn found(entries: &[Entry], from: &str) -> PartSet {
        match find_set(&folder(), entries, from, true).unwrap() {
            Discovery::Found(set) => set,
            other => panic!("asked: {other:?}"),
        }
    }

    fn numbers(set: &PartSet) -> Vec<u32> {
        set.parts.iter().map(|part| part.number).collect()
    }

    #[test]
    fn names_have_three_digits_and_grow_past_999() {
        assert_eq!(part_name("report.pdf", 1), "report.pdf.001");
        assert_eq!(part_name("report.pdf", 999), "report.pdf.999");
        assert_eq!(part_name("report.pdf", 1000), "report.pdf.1000");
        assert_eq!(part_name("report.pdf", 123_456), "report.pdf.123456");
        assert_eq!(size_words(20_000_000), "20 MB");
        assert_eq!(size_words(3_100_000), "3.1 MB");
        assert_eq!(size_words(18_000), "18 KB");
        assert_eq!(size_words(512), "512 bytes");
    }

    /// The edge-case table of research/alpha2-plan.md 5.4.
    #[test]
    fn the_edge_cases_of_the_001_rules() {
        let three = parts(&[("r.pdf.001", 10), ("r.pdf.002", 10), ("r.pdf.003", 4)]);
        // From any part, the same set, in numeric order.
        for from in ["r.pdf.001", "r.pdf.002", "r.pdf.003"] {
            let set = found(&three, from);
            assert_eq!(numbers(&set), vec![1, 2, 3], "{from}");
            assert_eq!(set.base, "r.pdf");
            assert!(set.problems(None).is_empty());
            assert_eq!(set.total(), 24);
        }
        // A gap, named before anything is written.
        let gap = parts(&[("r.pdf.001", 10), ("r.pdf.002", 10), ("r.pdf.005", 3)]);
        let set = found(&gap, "r.pdf.005");
        let problems = set.problems(None);
        assert_eq!(
            problems[..2],
            [SetProblem::Missing(3), SetProblem::Missing(4)]
        );
        assert_eq!(
            set.sentence(&problems[0], &problems),
            "Part 003 is missing, and 1 more. Nothing was joined."
        );
        let one_gap = parts(&[("r.pdf.001", 10), ("r.pdf.002", 10), ("r.pdf.004", 3)]);
        let set = found(&one_gap, "r.pdf.001");
        let problems = set.problems(None);
        assert_eq!(
            set.sentence(&problems[0], &problems),
            "Part 003 is missing. Nothing was joined."
        );
        // A first part missing.
        let late = parts(&[("r.pdf.002", 10), ("r.pdf.003", 3)]);
        assert_eq!(
            found(&late, "r.pdf.002").problems(None)[0],
            SetProblem::Missing(1)
        );
        // Past 999, in numeric order, never by text.
        let mut many: Vec<Entry> = (1..=1001).map(|n| file(&part_name("big", n), 8)).collect();
        many.reverse();
        let set = found(&many, "big.1000");
        assert_eq!(set.parts.len(), 1001);
        assert_eq!(set.parts[999].number, 1000);
        assert_eq!(set.parts[1000].number, 1001);
        assert!(set.problems(None).is_empty());
        assert_eq!(set.label(1000), "Part 1000");
        // A Krusader set, from 000 alone: accepted and said.
        let krusader = parts(&[("k.zip.000", 5), ("k.zip.002", 5)]);
        let set = found(&krusader, "k.zip.002");
        assert_eq!(set.first, 0);
        assert!(set.notes.contains(&"Parts start at 000.".to_owned()));
        assert_eq!(set.problems(None), vec![SetProblem::Missing(1)]);
        // An empty part, a middle part of another size, a long last part.
        let odd = parts(&[
            ("o.001", 20),
            ("o.002", 0),
            ("o.003", 20),
            ("o.004", 18),
            ("o.005", 30),
        ]);
        let set = found(&odd, "o.001");
        let problems = set.problems(None);
        assert_eq!(
            problems,
            vec![
                SetProblem::Empty(2),
                SetProblem::WrongSize {
                    number: 4,
                    size: 18,
                    expected: 20
                },
                SetProblem::LastLonger {
                    number: 5,
                    size: 30,
                    first: 20
                },
            ]
        );
        assert_eq!(
            set.sentence(&problems[0], &problems),
            "Part 002 is empty. Nothing was joined."
        );
        assert_eq!(
            set.sentence(&problems[1], &problems),
            "Part 004 is 18 bytes, expected 20 bytes."
        );
        assert!(problems[1].can_continue());
        assert!(!problems[2].can_continue());
        // With the checksum file's size, the sum decides.
        let uneven = parts(&[("u.001", 20), ("u.002", 18), ("u.003", 5)]);
        let set = found(&uneven, "u.001");
        assert!(set.problems(Some(43)).is_empty(), "the sum matches");
        assert_eq!(
            set.problems(Some(50)),
            vec![SetProblem::WrongSize {
                number: 2,
                size: 18,
                expected: 20
            }]
        );
        let short = parts(&[("s.001", 20), ("s.002", 20)]);
        let set = found(&short, "s.001");
        let problems = set.problems(Some(60));
        assert_eq!(
            problems,
            vec![SetProblem::Total {
                total: 40,
                expected: 60
            }]
        );
        assert_eq!(
            set.sentence(&problems[0], &problems),
            "Parts add up to 40 bytes, the checksum file says 60 bytes."
        );
        // A one-part set is a set.
        assert_eq!(
            numbers(&found(&parts(&[("one.001", 3)]), "one.001")),
            vec![1]
        );
    }

    #[test]
    fn both_000_and_001_ask_and_the_answer_picks_the_set() {
        let both = parts(&[("k.bin.000", 5), ("k.bin.001", 5), ("k.bin.002", 2)]);
        let Discovery::Ask { question, yes, no } =
            find_set(&folder(), &both, "k.bin.001", true).unwrap()
        else {
            panic!("expected a question");
        };
        assert_eq!(question, "Part 000 is there too. Join from 000? y or n");
        assert!(within_cells(&question, "Part 000 is there too"));
        assert_eq!(numbers(&yes), vec![0, 1, 2]);
        assert_eq!(yes.first, 0);
        assert_eq!(numbers(&no), vec![1, 2]);
        assert!(no.problems(None).is_empty());
    }

    #[test]
    fn a_mixed_width_keeps_to_the_chosen_parts_way_or_asks() {
        let mixed = parts(&[
            ("m.001", 4),
            ("m.002", 4),
            ("m.0001", 4),
            ("m.0002", 4),
            ("m.1000", 4),
        ]);
        let plain = found(&mixed, "m.001");
        assert_eq!(numbers(&plain), vec![1, 2, 1000]);
        assert_eq!(plain.width, Width::Plain);
        assert_eq!(plain.notes, vec!["Left out 2 parts numbered another way."]);
        let padded = found(&mixed, "m.0002");
        assert_eq!(numbers(&padded), vec![1, 2, 1000]);
        assert_eq!(padded.width, Width::Padded(4));
        assert_eq!(padded.label(2), "Part 0002");
        // .1000 fits both ways: asked.
        let Discovery::Ask { question, yes, no } =
            find_set(&folder(), &mixed, "m.1000", true).unwrap()
        else {
            panic!("expected a question");
        };
        assert!(question.starts_with("Parts are numbered two ways, 001 and 0001."));
        assert!(question.ends_with("y or n"));
        assert_eq!(yes.width, Width::Plain);
        assert_eq!(no.width, Width::Padded(4));
        // .1000 with only a padded set beside it belongs to it.
        let only_padded = parts(&[("p.0999", 4), ("p.1000", 1)]);
        assert_eq!(found(&only_padded, "p.1000").width, Width::Padded(4));
    }

    #[test]
    fn spanned_zip_and_rar_sets_are_refused_with_the_sentence() {
        let names = parts(&[
            ("a.z01", 1),
            ("a.z02", 1),
            ("a.zip", 1),
            ("b.r00", 1),
            ("b.rar", 1),
        ]);
        for (from, refusal) in [
            ("a.z01", NotJoinable::SpannedZip),
            ("A.Z02", NotJoinable::SpannedZip),
            ("a.zip", NotJoinable::SpannedZip),
            ("b.r00", NotJoinable::RarVolumes),
            ("b.rar", NotJoinable::RarVolumes),
            ("c.part1.rar", NotJoinable::RarVolumes),
        ] {
            assert_eq!(
                find_set(&folder(), &names, from, true),
                Err(refusal),
                "{from}"
            );
        }
        assert_eq!(
            NotJoinable::SpannedZip.to_string(),
            "Spanned zip sets need the 7z tool. Nothing was joined."
        );
        assert_eq!(
            find_set(&folder(), &names, "report.pdf", true),
            Err(NotJoinable::NotAPart("report.pdf".into()))
        );
        assert!(
            NotJoinable::NotAPart("report.pdf".into())
                .to_string()
                .starts_with("Not a numbered part: report.pdf.")
        );
    }

    #[test]
    fn names_that_ignore_case_join_their_case_twins() {
        let mixed = parts(&[
            ("Report.PDF.001", 4),
            ("report.pdf.002", 2),
            ("other.001", 9),
        ]);
        let set = match find_set(&folder(), &mixed, "report.pdf.002", false).unwrap() {
            Discovery::Found(set) => set,
            other => panic!("{other:?}"),
        };
        assert_eq!(numbers(&set), vec![1, 2]);
        let strict = found(&mixed, "report.pdf.002");
        assert_eq!(
            numbers(&strict),
            vec![2],
            "a case-sensitive place keeps them apart"
        );
        // Folders and links are never parts.
        let mut with_folder = parts(&[("f.001", 3)]);
        with_folder.push(Entry::new("f.002", EntryKind::Folder));
        assert_eq!(numbers(&found(&with_folder, "f.001")), vec![1]);
    }

    #[test]
    fn rows_say_the_part_first_with_its_size() {
        let set = found(
            &parts(&[("r.pdf.001", 20_000_000), ("r.pdf.002", 3_100_000)]),
            "r.pdf.001",
        );
        assert_eq!(set.row(&set.parts[0]), "Part 001, 20 MB");
        assert_eq!(set.row(&set.parts[1]), "Part 002, 3.1 MB");
        assert_eq!(
            set.sentence(
                &SetProblem::WrongSize {
                    number: 7,
                    size: 18_000,
                    expected: 20_000_000
                },
                &[]
            ),
            "Part 007 is 18 KB, expected 20 MB."
        );
    }
}
