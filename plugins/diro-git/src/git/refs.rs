//! Branches and stashes (plan section 5.16): `git for-each-ref` and `git
//! stash list`, each row the name first.

/// The format `git for-each-ref refs/heads` is asked for, fields between
/// unit separators, one branch a line.
pub const BRANCH_FORMAT: &str = "--format=%(HEAD)%1f%(refname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(objectname:short)%1f%(committerdate:short)%1f%(contents:subject)";

/// The format `git stash list -z` is asked for.
pub const STASH_FORMAT: &str = "--format=%gd%x1f%H%x1f%gs%x1f%cs";

/// One local branch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchRow {
    /// Its name.
    pub name: String,
    /// Whether it is checked out.
    pub current: bool,
    /// Its upstream, such as `origin/main`.
    pub upstream: Option<String>,
    /// git's words for the upstream: "ahead 2, behind 1", "gone", or empty
    /// when even.
    pub track: String,
    /// Its commit's short hash.
    pub short: String,
    /// Its commit's date.
    pub date: String,
    /// Its commit's subject.
    pub subject: String,
}

impl BranchRow {
    /// Where it stands against its upstream, in words: "2 ahead, 1
    /// behind", "up to date", "upstream gone", "no upstream".
    pub fn standing(&self) -> String {
        if self.upstream.is_none() {
            return "no upstream".to_owned();
        }
        let track = self.track.trim();
        if track.is_empty() {
            return "up to date".to_owned();
        }
        if track == "gone" {
            return "upstream gone".to_owned();
        }
        // git says "ahead 2, behind 1"; the words put the number first.
        track
            .split(", ")
            .map(|part| match part.split_once(' ') {
                Some((word, n)) => format!("{n} {word}"),
                None => part.to_owned(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The row's words: "main, current, 2 ahead", "topic, no upstream".
    pub fn row(&self) -> String {
        if self.current {
            format!("{}, current, {}", self.name, self.standing())
        } else {
            format!("{}, {}", self.name, self.standing())
        }
    }
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Reads `git for-each-ref` with [`BRANCH_FORMAT`].
pub fn parse_branches(text: &str) -> Vec<BranchRow> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\u{1f}');
            let head = f.next()?;
            let name = f.next()?.to_owned();
            if name.is_empty() {
                return None;
            }
            let upstream = f.next()?;
            Some(BranchRow {
                current: head.trim() == "*",
                name,
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
                track: f.next().unwrap_or("").to_owned(),
                short: f.next().unwrap_or("").to_owned(),
                date: f.next().unwrap_or("").to_owned(),
                subject: clean(f.next().unwrap_or("")),
            })
        })
        .collect()
}

/// One stash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stash {
    /// Its number: 0 is the newest.
    pub number: u32,
    /// Its commit's full hash, checked before a pop or a drop.
    pub hash: String,
    /// Its message without git's "On main: " or "WIP on main: ".
    pub message: String,
    /// The branch it was made on.
    pub branch: Option<String>,
    /// When it was made.
    pub date: String,
}

impl Stash {
    /// Its selector for git: `stash@{2}`.
    pub fn selector(&self) -> String {
        format!("stash@{{{}}}", self.number)
    }

    /// The row's words: "Stash 0: diroxide discard, on main, 2026-11-06".
    pub fn row(&self) -> String {
        let mut row = format!("Stash {}: {}", self.number, self.message);
        if let Some(branch) = &self.branch {
            row.push_str(&format!(", on {branch}"));
        }
        if !self.date.is_empty() {
            row.push_str(&format!(", {}", self.date));
        }
        row
    }
}

/// Reads `git stash list -z` with [`STASH_FORMAT`].
pub fn parse_stashes(bytes: &[u8]) -> Vec<Stash> {
    bytes
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let mut f = text.trim_start_matches('\n').split('\u{1f}');
            let selector = f.next()?;
            let number = selector
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
            let hash = f.next()?.to_owned();
            let subject = clean(f.next().unwrap_or(""));
            let (branch, message) = split_subject(&subject);
            Some(Stash {
                number,
                hash,
                message,
                branch,
                date: f.next().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// Splits a stash's reflog subject, "On main: my message" or "WIP on main:
/// a1b2c3d subject", into its branch and its message.
fn split_subject(subject: &str) -> (Option<String>, String) {
    let rest = subject
        .strip_prefix("WIP on ")
        .or_else(|| subject.strip_prefix("On "));
    match rest.and_then(|r| r.split_once(": ")) {
        Some((branch, message)) => {
            let message = if subject.starts_with("WIP on ") {
                format!("work in progress, {message}")
            } else {
                message.to_owned()
            };
            (Some(branch.to_owned()), message)
        }
        None => (None, subject.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_read_name_first_with_their_standing() {
        let text = "*\u{1f}main\u{1f}origin/main\u{1f}ahead 2, behind 1\u{1f}f2e5e54\u{1f}2026-11-06\u{1f}First\n \u{1f}topic\u{1f}\u{1f}\u{1f}0123456\u{1f}2026-11-05\u{1f}Try\n \u{1f}old\u{1f}origin/old\u{1f}gone\u{1f}1\u{1f}2\u{1f}3\n \u{1f}even\u{1f}origin/even\u{1f}\u{1f}1\u{1f}2\u{1f}3\n";
        let rows: Vec<String> = parse_branches(text).iter().map(BranchRow::row).collect();
        assert_eq!(
            rows,
            [
                "main, current, 2 ahead, 1 behind",
                "topic, no upstream",
                "old, upstream gone",
                "even, up to date"
            ]
        );
        assert!(parse_branches("").is_empty());
        assert!(parse_branches("*").is_empty());
    }

    #[test]
    fn stashes_read_number_and_message_first() {
        let bytes = b"stash@{0}\x1f238ace184a78226b47540f8639c73f7918fd663f\x1fOn main: diroxide discard 2026-11-06 14:30 UTC\x1f2026-11-06\0stash@{1}\x1fabc\x1fWIP on topic: f2e5e54 First\x1f2026-11-05\0";
        let stashes = parse_stashes(bytes);
        assert_eq!(stashes[0].selector(), "stash@{0}");
        assert_eq!(
            stashes[0].row(),
            "Stash 0: diroxide discard 2026-11-06 14:30 UTC, on main, 2026-11-06"
        );
        assert_eq!(
            stashes[1].row(),
            "Stash 1: work in progress, f2e5e54 First, on topic, 2026-11-05"
        );
        assert!(parse_stashes(b"stash@{x}\x1fa\0junk\0").is_empty());
    }
}
