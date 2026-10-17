//! Join files, as a plan and its verdict (plan section 5.4;
//! research/alpha2-plan.md 3.7, 5.4 and commit 25).
//!
//! A join starts from any part: it finds the set by name and number
//! ([`crate::part_names`]), lists the parts in order with their sizes,
//! and names any gap or odd size before it starts ("Part 004 is missing.
//! Nothing was joined."). It writes the whole to the other panel through
//! the committing writer, reading the parts through the parts reader
//! ([`crate::parts`]), so a part that changes while it is read stops the
//! join and leaves nothing. The CRC-32 is streamed through the whole and
//! checked against the `.crc` sidecar when there is one, and the verdict is
//! said in words: "Joined report.pdf: checksum matches." After a verified
//! join the app offers to move the parts to the trash ("Move the 12 parts
//! to the trash? y or n", starting on No); Undo of a join trashes the
//! whole.

use diroxide_core::format::count;
use diroxide_core::names::spell_out;
use diroxide_core::{Message, Place};

use crate::part_names::{PartSet, size_words};
use crate::sidecar::SidecarRead;
use crate::split::drive_words;

/// What a join plan knows beyond its steps: the set, where the whole goes,
/// the sidecar, and the room.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct JoinFacts {
    /// The parts, in order.
    pub set: PartSet,
    /// The whole, in the destination.
    pub output: Place,
    /// The sidecar used, and what it said.
    pub sidecar: Option<(Place, SidecarRead)>,
    /// What discovery and the sidecar's reader found worth saying.
    pub notes: Vec<String>,
    /// The destination's drive and its free bytes, when the provider says.
    pub space: Option<(Place, u64)>,
}

impl JoinFacts {
    /// The whole's name, spelled out.
    pub fn name(&self) -> String {
        self.output.name().map_or_else(
            || spell_out(&self.output.to_string()).into_owned(),
            |name| spell_out(name).into_owned(),
        )
    }

    /// The plan's first line: "Join 12 parts into report.pdf, 223 MB".
    pub fn headline(&self) -> String {
        format!(
            "Join {} into {}, {}",
            count(self.set.parts.len() as u64, "part", "parts"),
            self.name(),
            size_words(self.set.total())
        )
    }

    /// The plan's rows, before anything is written: the headline, the
    /// check the join will make, what was found worth saying, the room,
    /// then each part in order with its size ("Part 001, 20 MB").
    pub fn rows(&self) -> Vec<String> {
        let mut rows = vec![self.headline()];
        rows.push(match &self.sidecar {
            Some((place, _)) => format!(
                "Checked against {}",
                place
                    .name()
                    .map_or_else(String::new, |name| spell_out(name).into_owned())
            ),
            None => "No checksum file: the join is not checked.".to_owned(),
        });
        rows.extend(self.notes.iter().cloned());
        if let Some((drive, free)) = &self.space {
            let room = if *free >= self.set.total() {
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
        rows.extend(self.set.parts.iter().map(|part| self.set.row(part)));
        rows
    }
}

/// What a join's check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Verdict {
    /// The size and the CRC-32 match the sidecar.
    Matches,
    /// The size or the CRC-32 differs from the sidecar.
    Differs,
    /// The size matches; the sidecar holds no CRC-32 to check.
    SizeOnly,
    /// There was no sidecar.
    NoSidecar,
}

/// A finished join: the whole, the parts, and the verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinOutcome {
    /// The whole.
    pub output: Place,
    /// The parts it was joined from, in order.
    pub parts: Vec<Place>,
    /// What the check found.
    pub verdict: Verdict,
    /// The whole's size.
    pub size: u64,
    /// The whole's CRC-32.
    pub crc32: u32,
}

impl JoinOutcome {
    /// The verdict in words, the fact first: "Joined report.pdf: checksum
    /// matches.", "Checksum differs: report.pdf may be damaged.", "Joined
    /// report.pdf. No checksum file found."
    pub fn sentence(&self) -> String {
        let name = self.output.name().map_or_else(
            || spell_out(&self.output.to_string()).into_owned(),
            |name| spell_out(name).into_owned(),
        );
        match self.verdict {
            Verdict::Matches => format!("Joined {name}: checksum matches."),
            Verdict::Differs => format!("Checksum differs: {name} may be damaged."),
            Verdict::SizeOnly => format!("Joined {name}: size matches, no checksum to check."),
            Verdict::NoSidecar => format!("Joined {name}. No checksum file found."),
        }
    }

    /// After a verified join, the offer to trash the parts, starting on No:
    /// "Move the 12 parts to the trash? y or n". `None` when the join was
    /// not verified: the parts stay.
    pub fn trash_offer(&self) -> Option<Message> {
        (self.verdict == Verdict::Matches).then(|| {
            let parts = match self.parts.len() {
                1 => "the part".to_owned(),
                n => format!("the {n} parts"),
            };
            Message::question(&format!("Move {parts} to the trash"))
        })
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

    fn outcome(verdict: Verdict, parts: usize) -> JoinOutcome {
        JoinOutcome {
            output: place("/c/report.pdf"),
            parts: (1..=parts)
                .map(|n| place(&format!("/b/report.pdf.{n:03}")))
                .collect(),
            verdict,
            size: 3,
            crc32: 1,
        }
    }

    #[test]
    fn the_verdict_is_said_in_words_and_only_a_match_offers_the_trash() {
        let matches = outcome(Verdict::Matches, 12);
        assert_eq!(matches.sentence(), "Joined report.pdf: checksum matches.");
        assert_eq!(
            matches.trash_offer().unwrap().text,
            "Move the 12 parts to the trash? y or n"
        );
        assert_eq!(
            outcome(Verdict::Matches, 1).trash_offer().unwrap().text,
            "Move the part to the trash? y or n"
        );
        let differs = outcome(Verdict::Differs, 12);
        assert_eq!(
            differs.sentence(),
            "Checksum differs: report.pdf may be damaged."
        );
        assert!(differs.trash_offer().is_none());
        assert_eq!(
            outcome(Verdict::NoSidecar, 2).sentence(),
            "Joined report.pdf. No checksum file found."
        );
        assert!(outcome(Verdict::SizeOnly, 2).trash_offer().is_none());
        for verdict in [
            Verdict::Matches,
            Verdict::Differs,
            Verdict::SizeOnly,
            Verdict::NoSidecar,
        ] {
            let line = outcome(verdict, 2).sentence();
            assert!(within_cells(&line, "report.pdf"), "{line}");
        }
    }
}
