//! The log and a commit's files (R29 2.4; plan section 5.16): `git log -z`
//! with its fields between unit separators, each row the date and subject
//! first and the hash last ("2026-10-09, Fix the parser, a1b2c3d"), and
//! `--name-status -z` for what one commit changed.

/// The format `git log` is asked for: full hash, short hash, date, author
/// and subject between unit separators; `-z` ends each commit with NUL.
pub const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%ad%x1f%an%x1f%s";

/// The most commits one log lister holds.
pub const MAX_COMMITS: usize = 500;

/// One commit of a log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    /// The full hash.
    pub hash: String,
    /// The short hash.
    pub short: String,
    /// The author's date, `YYYY-MM-DD` in the commit's own time zone.
    pub date: String,
    /// The author's name.
    pub author: String,
    /// The subject line.
    pub subject: String,
}

impl Commit {
    /// The row's words: "2026-10-09, Fix the parser, a1b2c3d".
    pub fn row(&self) -> String {
        let subject = if self.subject.trim().is_empty() {
            "no subject"
        } else {
            self.subject.trim()
        };
        format!("{}, {subject}, {}", self.date, self.short)
    }
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Reads `git log -z` with [`LOG_FORMAT`] and `--date=short`; a record
/// with too few fields is passed over.
pub fn parse_log(bytes: &[u8]) -> Vec<Commit> {
    bytes
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let mut fields = text.trim_start_matches('\n').splitn(5, '\u{1f}');
            Some(Commit {
                hash: fields.next()?.trim().to_owned(),
                short: fields.next()?.to_owned(),
                date: fields.next()?.to_owned(),
                author: clean(fields.next()?),
                subject: clean(fields.next()?),
            })
        })
        .filter(|c| !c.hash.is_empty())
        .take(MAX_COMMITS)
        .collect()
}

/// What one commit did to one path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// The letter git gives: `A`, `M`, `D`, `R`, `C`, `T`.
    pub letter: char,
    /// The path after the commit.
    pub path: String,
    /// The path before, for a rename or copy.
    pub from: Option<String>,
}

impl FileChange {
    /// The state in words: "Added", "Modified", "Deleted", "Renamed",
    /// "Copied", "Type changed".
    pub fn word(&self) -> &'static str {
        match self.letter {
            'A' => "Added",
            'D' => "Deleted",
            'R' => "Renamed",
            'C' => "Copied",
            'T' => "Type changed",
            _ => "Modified",
        }
    }

    /// The row's words: "Modified: src/lib.rs", "Renamed: b.txt, from
    /// a.txt".
    pub fn row(&self) -> String {
        match &self.from {
            Some(from) => format!("{}: {}, from {from}", self.word(), self.path),
            None => format!("{}: {}", self.word(), self.path),
        }
    }
}

/// Reads `--name-status -z`: a status (a letter and, for a rename or
/// copy, its score), then one path, or two for a rename or copy.
pub fn parse_name_status(bytes: &[u8]) -> Vec<FileChange> {
    let mut out = Vec::new();
    let mut fields = bytes.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(status) = fields.next() {
        let Some(letter) = status.first().map(|b| char::from(*b)) else {
            continue;
        };
        if !letter.is_ascii_uppercase() {
            continue;
        }
        let Some(first) = fields.next() else {
            break;
        };
        let first = String::from_utf8_lossy(first).into_owned();
        if letter == 'R' || letter == 'C' {
            let Some(second) = fields.next() else {
                break;
            };
            out.push(FileChange {
                letter,
                path: String::from_utf8_lossy(second).into_owned(),
                from: Some(first),
            });
        } else {
            out.push(FileChange {
                letter,
                path: first,
                from: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_reads_date_and_subject_first_and_the_hash_last() {
        let log = b"f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57\x1ff2e5e54\x1f2026-10-09\x1fAda Example\x1fFix the parser\0\n0123456789abcdef0123456789abcdef01234567\x1f0123456\x1f2026-10-08\x1fAda Example\x1f\0";
        let commits = parse_log(log);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].row(), "2026-10-09, Fix the parser, f2e5e54");
        assert_eq!(commits[1].row(), "2026-10-08, no subject, 0123456");
        assert_eq!(commits[1].hash.len(), 40);
        assert!(parse_log(b"only\x1ftwo\0").is_empty());
        assert!(parse_log(b"").is_empty());
    }

    #[test]
    fn a_commit_s_files_read_with_renames() {
        let changes =
            parse_name_status(b"M\0src/lib.rs\0R087\0a.txt\0b.txt\0A\0new.txt\0D\0gone.txt\0");
        let rows: Vec<String> = changes.iter().map(FileChange::row).collect();
        assert_eq!(
            rows,
            [
                "Modified: src/lib.rs",
                "Renamed: b.txt, from a.txt",
                "Added: new.txt",
                "Deleted: gone.txt"
            ]
        );
        assert_eq!(parse_name_status(b"R100\0only-one\0").len(), 0);
        assert!(parse_name_status(b"\0\0x").is_empty());
    }
}
