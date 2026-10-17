//! Facts about each file as a whole: its encoding, its byte-order mark,
//! its line endings and whether its last line ends with a line break. A
//! file with another line-ending style is one fact ("Line endings differ:
//! CRLF here, LF there. Compared ignoring them."), never 340 changes
//! (R18, "Say what differs when it sounds the same").
//!
//! The encoding is found the way the viewer finds it
//! (`diroxide_preview::encoding`): a byte-order mark, then UTF-16 without
//! one, then UTF-8 when the bytes are valid, else Windows-1252, where
//! every byte decodes, so Latin-1 bytes are a note, never a crash.

use diroxide_core::names::display_name;
use diroxide_preview::encoding;

use crate::lines::{LineEnding, split_lines};

/// The line endings a file uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Endings {
    /// No line break at all: an empty file or a single line.
    None,
    /// Every break is LF.
    Lf,
    /// Every break is CRLF.
    CrLf,
    /// Every break is a lone CR.
    Cr,
    /// More than one kind.
    Mixed,
}

impl Endings {
    /// The style as said: "LF", "CRLF", "CR", "mixed", "none".
    pub const fn name(self) -> &'static str {
        match self {
            Endings::None => "none",
            Endings::Lf => "LF",
            Endings::CrLf => "CRLF",
            Endings::Cr => "CR",
            Endings::Mixed => "mixed",
        }
    }
}

/// The facts of one file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileFacts {
    /// The encoding's name as said: "UTF-8", "UTF-16", "Windows-1252".
    pub encoding: &'static str,
    /// Whether the file starts with a byte-order mark.
    pub byte_order_mark: bool,
    /// The line endings it uses.
    pub endings: Endings,
    /// Whether its last line ends with a line break; true for an empty
    /// file, which has no last line.
    pub final_line_break: bool,
    /// Whether some bytes could not be decoded and were replaced.
    pub decoding_errors: bool,
}

impl FileFacts {
    /// Reads a file's facts from its bytes, and its text, decoded, without
    /// the byte-order mark.
    pub fn read(bytes: &[u8]) -> (FileFacts, String) {
        let choice = encoding::choose(bytes, None);
        let decoded = encoding::decode(bytes, None);
        let lines = split_lines(&decoded.text);
        let mut endings = Endings::None;
        for line in &lines {
            let this = match line.ending {
                LineEnding::None => continue,
                LineEnding::Lf => Endings::Lf,
                LineEnding::CrLf => Endings::CrLf,
                LineEnding::Cr => Endings::Cr,
            };
            endings = match endings {
                Endings::None => this,
                same if same == this => same,
                _ => Endings::Mixed,
            };
        }
        let facts = FileFacts {
            encoding: choice.name(),
            byte_order_mark: choice.bom_len > 0,
            endings,
            final_line_break: lines.last().is_none_or(|l| l.ending != LineEnding::None),
            decoding_errors: decoded.had_errors,
        };
        (facts, decoded.text)
    }
}

/// A fact that differs between the two files.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FactDifference {
    /// The encodings differ.
    Encoding {
        /// The before file's encoding.
        before: &'static str,
        /// The after file's encoding.
        after: &'static str,
    },
    /// One file has a byte-order mark and the other does not.
    ByteOrderMark {
        /// Whether the before file has one.
        before: bool,
    },
    /// The line-ending styles differ (both files have line breaks).
    LineEndings {
        /// The before file's style.
        before: Endings,
        /// The after file's style.
        after: Endings,
        /// Whether the compare ignored them.
        ignored: bool,
    },
    /// One file's last line has no line break.
    FinalLineBreak {
        /// Whether the before file's last line has one.
        before: bool,
    },
    /// Some bytes of a file could not be decoded.
    DecodingErrors {
        /// Whether the before file had them.
        before: bool,
        /// Whether the after file had them.
        after: bool,
    },
}

/// The facts that differ, in the order a report says them.
pub fn fact_differences(
    before: &FileFacts,
    after: &FileFacts,
    line_endings_ignored: bool,
) -> Vec<FactDifference> {
    let mut out = Vec::new();
    if before.encoding != after.encoding {
        out.push(FactDifference::Encoding {
            before: before.encoding,
            after: after.encoding,
        });
    }
    if before.byte_order_mark != after.byte_order_mark {
        out.push(FactDifference::ByteOrderMark {
            before: before.byte_order_mark,
        });
    }
    if before.endings != after.endings
        && before.endings != Endings::None
        && after.endings != Endings::None
    {
        out.push(FactDifference::LineEndings {
            before: before.endings,
            after: after.endings,
            ignored: line_endings_ignored,
        });
    }
    if before.final_line_break != after.final_line_break {
        out.push(FactDifference::FinalLineBreak {
            before: before.final_line_break,
        });
    }
    if before.decoding_errors || after.decoding_errors {
        out.push(FactDifference::DecodingErrors {
            before: before.decoding_errors,
            after: after.decoding_errors,
        });
    }
    out
}

/// How a report names the two sides. In the panels the after file is
/// "here" (this panel) and the before file "there" (the other panel),
/// and Swap exchanges them; on the command line each file is named, the
/// first one first.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sides {
    before: String,
    after: String,
    before_first: bool,
}

impl Sides {
    /// The panels, read from the other panel's file to this panel's: the
    /// default direction (plan question 40).
    pub fn panels() -> Sides {
        Sides {
            before: "there".to_owned(),
            after: "here".to_owned(),
            before_first: false,
        }
    }

    /// The panels after Swap: read from this panel's file to the other's.
    pub fn panels_swapped() -> Sides {
        Sides {
            before: "here".to_owned(),
            after: "there".to_owned(),
            before_first: true,
        }
    }

    /// Two named files, read from the first to the second: "in old.txt",
    /// "in new.txt". Hidden characters in the names show as question
    /// marks.
    pub fn files(before: &str, after: &str) -> Sides {
        Sides {
            before: format!("in {}", display_name(before)),
            after: format!("in {}", display_name(after)),
            before_first: true,
        }
    }

    /// The same sides with the direction turned around.
    pub fn swapped(&self) -> Sides {
        Sides {
            before: self.after.clone(),
            after: self.before.clone(),
            before_first: !self.before_first,
        }
    }

    /// The before side's words: "there", or "in old.txt".
    pub fn before(&self) -> &str {
        &self.before
    }

    /// The after side's words: "here", or "in new.txt".
    pub fn after(&self) -> &str {
        &self.after
    }

    /// Two values with their sides, in the reading order: "CRLF here, LF
    /// there", or "LF in old.txt, CRLF in new.txt".
    pub fn pair(&self, before_value: &str, after_value: &str) -> String {
        if self.before_first {
            format!(
                "{before_value} {}, {after_value} {}",
                self.before, self.after
            )
        } else {
            format!(
                "{after_value} {}, {before_value} {}",
                self.after, self.before
            )
        }
    }
}

impl FactDifference {
    /// The fact as one sentence, meaning first: "Line endings differ:
    /// CRLF here, LF there. Compared ignoring them."
    pub fn sentence(&self, sides: &Sides) -> String {
        match self {
            FactDifference::Encoding { before, after } => {
                format!("Encodings differ: {}.", sides.pair(before, after))
            }
            FactDifference::ByteOrderMark { before } => {
                let (with, without) = if *before {
                    (sides.before(), sides.after())
                } else {
                    (sides.after(), sides.before())
                };
                format!("Byte-order mark {with}, none {without}.")
            }
            FactDifference::LineEndings {
                before,
                after,
                ignored,
            } => {
                let mut text = format!(
                    "Line endings differ: {}.",
                    sides.pair(before.name(), after.name())
                );
                if *ignored {
                    text.push_str(" Compared ignoring them.");
                }
                text
            }
            FactDifference::FinalLineBreak { before } => {
                let without = if *before {
                    sides.after()
                } else {
                    sides.before()
                };
                format!("Last line has no line break {without}.")
            }
            FactDifference::DecodingErrors { before, after } => {
                let place = match (before, after) {
                    (true, true) => "in both files".to_owned(),
                    (true, false) => sides.before().to_owned(),
                    _ => sides.after().to_owned(),
                };
                format!("Some bytes {place} are not text; each shows as a question mark.")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_are_read_from_the_bytes() {
        let (facts, text) = FileFacts::read(b"a\r\nb\r\n");
        assert_eq!(facts.encoding, "UTF-8");
        assert!(!facts.byte_order_mark && facts.final_line_break);
        assert_eq!(facts.endings, Endings::CrLf);
        assert_eq!(text, "a\r\nb\r\n");
        let (bom, text) = FileFacts::read(b"\xef\xbb\xbfa\nb");
        assert!(bom.byte_order_mark && !bom.final_line_break);
        assert_eq!(text, "a\nb");
        let (mixed, _) = FileFacts::read(b"a\nb\r\nc\rd");
        assert_eq!(mixed.endings, Endings::Mixed);
        let (empty, _) = FileFacts::read(b"");
        assert!(empty.final_line_break);
        assert_eq!(empty.endings, Endings::None);
        let (latin, text) = FileFacts::read(b"caf\xe9\n");
        assert_eq!(
            (latin.encoding, text.as_str()),
            ("Windows-1252", "caf\u{e9}\n")
        );
    }

    #[test]
    fn differing_facts_read_as_sentences_from_either_side() {
        let (crlf, _) = FileFacts::read(b"a\r\n");
        let (lf, _) = FileFacts::read(b"a\n");
        let found = fact_differences(&lf, &crlf, true);
        assert_eq!(
            found[0].sentence(&Sides::panels()),
            "Line endings differ: CRLF here, LF there. Compared ignoring them."
        );
        assert_eq!(
            found[0].sentence(&Sides::files("old.txt", "new.txt")),
            "Line endings differ: LF in old.txt, CRLF in new.txt. Compared ignoring them."
        );
        let (bom, _) = FileFacts::read(b"\xef\xbb\xbfa");
        let (plain, _) = FileFacts::read(b"a\n");
        let found = fact_differences(&plain, &bom, true);
        let said: Vec<String> = found.iter().map(|f| f.sentence(&Sides::panels())).collect();
        assert_eq!(
            said,
            [
                "Byte-order mark here, none there.",
                "Last line has no line break here."
            ]
        );
        // After Swap the files trade places as before and after, and the
        // sentence still says which panel holds the mark.
        let swapped: Vec<String> = fact_differences(&bom, &plain, true)
            .iter()
            .map(|f| f.sentence(&Sides::panels_swapped()))
            .collect();
        assert_eq!(swapped, said);
        assert_eq!(Sides::panels().swapped(), Sides::panels_swapped());
    }

    #[test]
    fn a_single_line_has_no_line_ending_style_to_compare() {
        let (one, _) = FileFacts::read(b"a");
        let (two, _) = FileFacts::read(b"a\nb\n");
        let found = fact_differences(&one, &two, true);
        assert_eq!(found, [FactDifference::FinalLineBreak { before: false }]);
    }
}
