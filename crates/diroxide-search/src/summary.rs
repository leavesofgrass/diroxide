//! How a search went, in words: the status line ("Done: 214 found in
//! 3,120 folders", "Stopped at 214 found."), then one sentence for each
//! thing it skipped, so nothing is filtered silently ("214 found. Hidden
//! files skipped.").

use diroxide_core::filter::Unknown;
use diroxide_core::format::{grouped, human_size};

/// A count in words with the number grouped by three: "no folders", "1
/// folder", "3,120 folders".
pub(crate) fn counted(n: u64, one: &str, many: &str) -> String {
    match n {
        0 => format!("no {many}"),
        1 => format!("1 {one}"),
        _ => format!("{} {many}", grouped(n)),
    }
}

/// What a search did not look at, and why. Each count becomes a sentence
/// of the summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Skips {
    /// Hidden entries not searched (a hidden folder counts once; what is
    /// inside it is not counted).
    pub hidden: u64,
    /// Whether ignore files were read, so what they list was skipped.
    pub ignore_files: bool,
    /// Links to folders not followed, because the scope says not to.
    pub links_not_followed: u64,
    /// Links not followed because they lead out of the search.
    pub links_outside: u64,
    /// Folders not entered again: a link or a mount that leads back.
    pub loops: u64,
    /// Folders on another drive, not entered.
    pub other_drives: u64,
    /// Snapshot folders, not entered.
    pub snapshots: u64,
    /// Folders deeper than the walk's limit of 256 levels.
    pub too_deep: u64,
    /// Places marked to be skipped in searches.
    pub marked: u64,
    /// Files skipped as binary: a NUL byte, as ripgrep decides (a UTF-16
    /// file without a byte-order mark is one).
    pub binary: u64,
    /// Online-only files whose contents were not read.
    pub online_only: u64,
    /// Their size, in bytes.
    pub online_only_bytes: u64,
    /// Files whose contents could not be read.
    pub unreadable_files: u64,
    /// Entries whose name a place cannot hold, such as a name that reads
    /// as a path inside an archive.
    pub odd_names: u64,
}

impl Skips {
    /// Adds another thread's counts.
    pub(crate) fn add(&mut self, other: &Skips) {
        self.hidden += other.hidden;
        self.ignore_files |= other.ignore_files;
        self.links_not_followed += other.links_not_followed;
        self.links_outside += other.links_outside;
        self.loops += other.loops;
        self.other_drives += other.other_drives;
        self.snapshots += other.snapshots;
        self.too_deep += other.too_deep;
        self.marked += other.marked;
        self.binary += other.binary;
        self.online_only += other.online_only;
        self.online_only_bytes += other.online_only_bytes;
        self.unreadable_files += other.unreadable_files;
        self.odd_names += other.odd_names;
    }
}

/// How a search went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchSummary {
    /// Hits found, past the row cap too.
    pub hits: u64,
    /// Hits delivered to the sink: at most the row cap.
    pub shown: u64,
    /// Folders read.
    pub folders: u64,
    /// Entries looked at.
    pub entries: u64,
    /// Folders that could not be read.
    pub unreadable: u64,
    /// Whether it was cancelled.
    pub cancelled: bool,
    /// Folders still being read when a cancelled search returned: a read
    /// blocked on a dead share cannot be interrupted, so its thread is
    /// left to finish on its own.
    pub still_reading: u64,
    /// Files whose contents were read.
    pub files_read: u64,
    /// Bytes of contents read.
    pub bytes_read: u64,
    /// Milliseconds the search took.
    pub elapsed_ms: u64,
    /// The row cap the search had.
    pub row_cap: Option<u64>,
    /// What was skipped.
    pub skipped: Skips,
    /// Entries a test could not decide, by reason, most common first. A
    /// read skipped as binary, online only or unreadable answers with
    /// that reason ([`Unknown::Binary`], [`Unknown::OnlineOnly`],
    /// [`Unknown::Unreadable`]) and is counted once, in `skipped`, whose
    /// sentences say which.
    pub unknown: Vec<(Unknown, u64)>,
}

impl SearchSummary {
    /// The summary in words: "12 found in 40 folders, 1 folder unreadable".
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} found in {}",
            grouped(self.hits),
            counted(self.folders, "folder", "folders")
        );
        if self.unreadable > 0 {
            text.push_str(&format!(
                ", {} unreadable",
                counted(self.unreadable, "folder", "folders")
            ));
        }
        if self.cancelled {
            text.push_str(", cancelled");
        }
        text
    }

    /// The status line when the search ends: "Done: 214 found in 3,120
    /// folders", or "Stopped at 214 found." after a cancel.
    pub fn status_line(&self) -> String {
        if self.cancelled {
            format!("Stopped at {} found.", grouped(self.hits))
        } else {
            format!(
                "Done: {} found in {}",
                grouped(self.hits),
                counted(self.folders, "folder", "folders")
            )
        }
    }

    /// One sentence for each thing skipped or left undecided, in the order
    /// said: the row cap, hidden files, ignore files, links, other drives,
    /// unreadable folders and files, binary and online-only files, unknown
    /// answers, and the folders still being read after a stop.
    pub fn skip_sentences(&self) -> Vec<String> {
        let s = &self.skipped;
        let mut sentences = Vec::new();
        if let Some(cap) = self.row_cap
            && self.hits > self.shown
        {
            sentences.push(format!(
                "Showing the first {}. Narrow the search.",
                grouped(cap)
            ));
        }
        if s.hidden > 0 {
            sentences.push("Hidden files skipped.".to_owned());
        }
        if s.ignore_files {
            sentences.push("Files in ignore files skipped.".to_owned());
        }
        if s.links_not_followed > 0 {
            sentences.push(format!(
                "{} not followed.",
                capital(&counted(
                    s.links_not_followed,
                    "link to a folder",
                    "links to folders"
                ))
            ));
        }
        if s.links_outside > 0 {
            let verb = if s.links_outside == 1 {
                "leads"
            } else {
                "lead"
            };
            sentences.push(format!(
                "{} {verb} out of the search, not followed.",
                capital(&counted(s.links_outside, "link", "links"))
            ));
        }
        let folder_skips = [
            (s.loops, "loop", "loops", "skipped"),
            (
                s.other_drives,
                "folder on another drive",
                "folders on other drives",
                "skipped",
            ),
            (
                s.snapshots,
                "snapshot folder",
                "snapshot folders",
                "skipped",
            ),
            (
                s.too_deep,
                "folder over 256 levels deep",
                "folders over 256 levels deep",
                "skipped",
            ),
            (
                s.marked,
                "place marked Skip in searches",
                "places marked Skip in searches",
                "skipped",
            ),
            (self.unreadable, "folder", "folders", "unreadable"),
            (s.unreadable_files, "file", "files", "unreadable"),
            (s.binary, "binary file", "binary files", "skipped"),
            (
                s.odd_names,
                "name that reads as an archive path",
                "names that read as archive paths",
                "skipped",
            ),
        ];
        for (n, one, many, what) in folder_skips {
            if n > 0 {
                sentences.push(format!("{} {what}.", capital(&counted(n, one, many))));
            }
        }
        if s.online_only > 0 {
            sentences.push(format!(
                "{} skipped, {}.",
                capital(&counted(
                    s.online_only,
                    "online-only file",
                    "online-only files"
                )),
                human_size(s.online_only_bytes)
            ));
        }
        if let Some(sentence) = self.unknown_sentence() {
            sentences.push(sentence);
        }
        if self.still_reading > 0 {
            let verb = if self.still_reading == 1 { "is" } else { "are" };
            sentences.push(format!(
                "{} {verb} still being read.",
                capital(&counted(self.still_reading, "folder", "folders"))
            ));
        }
        sentences
    }

    /// The undecided part, meaning first: "3 unknown, folder size not
    /// counted.", or with several reasons "5 unknown, 3 folder size not
    /// counted, 2 type not checked."
    pub fn unknown_sentence(&self) -> Option<String> {
        let total: u64 = self.unknown.iter().map(|(_, n)| n).sum();
        match self.unknown.as_slice() {
            [] => None,
            [(why, _)] => Some(format!("{} unknown, {why}.", grouped(total))),
            many => {
                let parts: Vec<String> = many
                    .iter()
                    .map(|(why, n)| format!("{} {why}", grouped(*n)))
                    .collect();
                Some(format!("{} unknown, {}.", grouped(total), parts.join(", ")))
            }
        }
    }

    /// The whole summary as said once the search ends: the count, then
    /// what was skipped: "214 found. Hidden files skipped.", or "Stopped
    /// at 214 found. 1 folder is still being read."
    pub fn sentence(&self) -> String {
        let mut parts = vec![if self.cancelled {
            format!("Stopped at {} found.", grouped(self.hits))
        } else {
            format!("{} found.", grouped(self.hits))
        }];
        parts.extend(self.skip_sentences());
        parts.join(" ")
    }

    /// Whether only part of the place was searched: a folder or a file
    /// could not be read, or a read was left behind. `diro search` exits
    /// with 3.
    pub fn is_partial(&self) -> bool {
        self.unreadable > 0 || self.skipped.unreadable_files > 0 || self.still_reading > 0
    }

    /// Counts an undecided entry.
    pub(crate) fn add_unknown(&mut self, why: Unknown, n: u64) {
        match self.unknown.iter_mut().find(|(w, _)| *w == why) {
            Some((_, count)) => *count += n,
            None => self.unknown.push((why, n)),
        }
        self.unknown
            .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }
}

/// Text with its first letter capitalized, for a count that starts a
/// sentence ("no links" becomes "No links").
fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    #[test]
    fn the_lines_say_the_count_first() {
        let mut summary = SearchSummary {
            hits: 214,
            shown: 214,
            folders: 3_120,
            ..SearchSummary::default()
        };
        assert_eq!(summary.status_line(), "Done: 214 found in 3,120 folders");
        summary.skipped.hidden = 12;
        assert_eq!(summary.sentence(), "214 found. Hidden files skipped.");
        summary.cancelled = true;
        summary.still_reading = 1;
        assert_eq!(summary.status_line(), "Stopped at 214 found.");
        assert_eq!(
            summary.sentence(),
            "Stopped at 214 found. Hidden files skipped. 1 folder is still being read."
        );
        assert!(summary.is_partial());
    }

    #[test]
    fn every_skip_has_its_sentence_and_its_meaning_early() {
        let mut summary = SearchSummary {
            hits: 120_000,
            shown: 100_000,
            row_cap: Some(100_000),
            folders: 1,
            unreadable: 2,
            skipped: Skips {
                hidden: 1,
                ignore_files: true,
                links_not_followed: 3,
                links_outside: 1,
                loops: 1,
                other_drives: 2,
                snapshots: 1,
                too_deep: 1,
                marked: 2,
                binary: 4,
                online_only: 12,
                online_only_bytes: 1_400_000_000,
                unreadable_files: 3,
                odd_names: 1,
            },
            ..SearchSummary::default()
        };
        summary.add_unknown(Unknown::TypeNotChecked, 2);
        summary.add_unknown(Unknown::SizeNotCounted, 3);
        let sentences = summary.skip_sentences();
        assert_eq!(
            sentences,
            [
                "Showing the first 100,000. Narrow the search.",
                "Hidden files skipped.",
                "Files in ignore files skipped.",
                "3 links to folders not followed.",
                "1 link leads out of the search, not followed.",
                "1 loop skipped.",
                "2 folders on other drives skipped.",
                "1 snapshot folder skipped.",
                "1 folder over 256 levels deep skipped.",
                "2 places marked Skip in searches skipped.",
                "2 folders unreadable.",
                "3 files unreadable.",
                "4 binary files skipped.",
                "1 name that reads as an archive path skipped.",
                "12 online-only files skipped, 1.4 GB.",
                "5 unknown, 3 folder size not counted, 2 type not checked.",
            ]
        );
        // The count and the kind of skip come within the first cells.
        for sentence in &sentences {
            let head: String = sentence.chars().take(BRAILLE_CELLS).collect();
            assert!(head.split_whitespace().count() >= 2, "{sentence}");
        }
        assert!(summary.is_partial());
        assert_eq!(
            summary.describe(),
            "120,000 found in 1 folder, 2 folders unreadable"
        );
    }
}
