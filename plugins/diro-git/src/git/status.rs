//! `git status --porcelain=v2 -z --branch` read (R29 2.3; spike GS1): the
//! branch headers, then one record per changed path, each ended by NUL; a
//! renamed or copied record is followed by its original path. Paths are
//! relative to the repository's top, as git's own bytes
//! (`core.quotepath=false`). Headers this version does not know, and
//! records of a kind it does not know, are passed over.
//!
//! The words for each state are spelled out, the state first: "Staged,
//! modified", "Conflict, both modified", "Untracked" (R29 2.3), never a
//! letter or a color alone.

use std::fmt;

/// What git says about the branch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branch {
    /// The commit checked out; `None` before the first commit
    /// (`(initial)`).
    pub oid: Option<String>,
    /// The branch's name; `None` when the head is detached.
    pub head: Option<String>,
    /// The upstream, such as `origin/main`.
    pub upstream: Option<String>,
    /// Commits here and not upstream, when git counted them.
    pub ahead: Option<u32>,
    /// Commits upstream and not here, when git counted them.
    pub behind: Option<u32>,
}

/// A submodule's state, from the record's four-letter `sub` field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Submodule {
    /// Its checked-out commit differs from the one recorded.
    pub commit: bool,
    /// Its tracked files are changed.
    pub modified: bool,
    /// It holds untracked files.
    pub untracked: bool,
}

/// What kind of record a change is, with its two state letters: the
/// index's (X) and the working tree's (Y), `.` for unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An ordinary change (`1`).
    Changed {
        /// The index's letter.
        x: char,
        /// The working tree's letter.
        y: char,
    },
    /// A rename or copy (`2`): `x` is `R` or `C`.
    Renamed {
        /// The index's letter.
        x: char,
        /// The working tree's letter.
        y: char,
    },
    /// A conflict (`u`), with the two letters of the unmerged pair.
    Conflict {
        /// Our side's letter.
        x: char,
        /// Their side's letter.
        y: char,
    },
    /// An untracked path (`?`); a folder ends with `/`.
    Untracked,
    /// An ignored path (`!`), listed only when asked for.
    Ignored,
}

/// One changed path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The path from the repository's top, `/` between names; a folder
    /// listed whole ends with `/`.
    pub path: String,
    /// A rename's or copy's original path.
    pub from: Option<String>,
    /// What changed.
    pub kind: Kind,
    /// The submodule's state, when the path is a submodule.
    pub submodule: Option<Submodule>,
    /// Whether the name's bytes were not UTF-8, so `path` holds stand-ins
    /// and the name cannot be handed back to git.
    pub not_unicode: bool,
}

/// A status: the branch and the changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The branch headers.
    pub branch: Branch,
    /// The changed paths, in git's order.
    pub changes: Vec<Change>,
}

/// A record git wrote that this reader could not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "git's status could not be read: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

fn text(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), false),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
    }
}

/// The record's fields before the path: `count` fields separated by single
/// spaces, then the rest is the path (which may hold spaces).
fn fields(record: &[u8], count: usize) -> Option<(Vec<&[u8]>, &[u8])> {
    let mut out = Vec::with_capacity(count);
    let mut rest = record;
    for _ in 0..count {
        let at = rest.iter().position(|b| *b == b' ')?;
        out.push(&rest[..at]);
        rest = &rest[at + 1..];
    }
    Some((out, rest))
}

fn letters(field: &[u8]) -> Option<(char, char)> {
    match field {
        [x, y] if x.is_ascii() && y.is_ascii() => Some((char::from(*x), char::from(*y))),
        _ => None,
    }
}

fn submodule(field: &[u8]) -> Option<Submodule> {
    match field {
        [b'S', c, m, u] => Some(Submodule {
            commit: *c == b'C',
            modified: *m == b'M',
            untracked: *u == b'U',
        }),
        _ => None,
    }
}

fn read_header(line: &[u8], branch: &mut Branch) {
    let Ok(line) = std::str::from_utf8(line) else {
        return;
    };
    let Some(rest) = line.strip_prefix("# ") else {
        return;
    };
    let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
    match key {
        "branch.oid" => {
            branch.oid = (value != "(initial)" && !value.is_empty()).then(|| value.to_owned());
        }
        "branch.head" => {
            branch.head = (value != "(detached)" && !value.is_empty()).then(|| value.to_owned());
        }
        "branch.upstream" => branch.upstream = Some(value.to_owned()),
        "branch.ab" => {
            for part in value.split(' ') {
                if let Some(n) = part.strip_prefix('+') {
                    branch.ahead = n.parse().ok();
                } else if let Some(n) = part.strip_prefix('-') {
                    branch.behind = n.parse().ok();
                }
            }
        }
        // stash and anything newer
        _ => {}
    }
}

/// Reads git's porcelain v2 output with `-z`.
pub fn parse(bytes: &[u8]) -> Result<Status, ParseError> {
    let mut status = Status::default();
    let mut records = bytes.split(|b| *b == 0).peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        match record[0] {
            b'#' => read_header(record, &mut status.branch),
            b'1' => {
                let (f, path) = fields(record, 8)
                    .ok_or_else(|| ParseError("an ordinary record is short".to_owned()))?;
                let (x, y) = letters(f[1])
                    .ok_or_else(|| ParseError("an ordinary record has no state".to_owned()))?;
                let (path, not_unicode) = text(path);
                status.changes.push(Change {
                    path,
                    from: None,
                    kind: Kind::Changed { x, y },
                    submodule: submodule(f[2]),
                    not_unicode,
                });
            }
            b'2' => {
                let (f, path) = fields(record, 9)
                    .ok_or_else(|| ParseError("a rename record is short".to_owned()))?;
                let (x, y) = letters(f[1])
                    .ok_or_else(|| ParseError("a rename record has no state".to_owned()))?;
                let from = records
                    .next()
                    .filter(|f| !f.is_empty())
                    .ok_or_else(|| ParseError("a rename has no original path".to_owned()))?;
                let (path, a) = text(path);
                let (from, b) = text(from);
                status.changes.push(Change {
                    path,
                    from: Some(from),
                    kind: Kind::Renamed { x, y },
                    submodule: submodule(f[2]),
                    not_unicode: a || b,
                });
            }
            b'u' => {
                let (f, path) = fields(record, 10)
                    .ok_or_else(|| ParseError("a conflict record is short".to_owned()))?;
                let (x, y) = letters(f[1])
                    .ok_or_else(|| ParseError("a conflict record has no state".to_owned()))?;
                let (path, not_unicode) = text(path);
                status.changes.push(Change {
                    path,
                    from: None,
                    kind: Kind::Conflict { x, y },
                    submodule: submodule(f[2]),
                    not_unicode,
                });
            }
            b'?' | b'!' if record.get(1) == Some(&b' ') => {
                let (path, not_unicode) = text(&record[2..]);
                status.changes.push(Change {
                    path,
                    from: None,
                    kind: if record[0] == b'?' {
                        Kind::Untracked
                    } else {
                        Kind::Ignored
                    },
                    submodule: None,
                    not_unicode,
                });
            }
            // A record of a kind this version does not know.
            _ => {}
        }
    }
    Ok(status)
}

fn index_word(x: char) -> Option<&'static str> {
    match x {
        'M' => Some("Staged"),
        'A' => Some("Added"),
        'D' => Some("Deleted, staged"),
        'R' => Some("Renamed"),
        'C' => Some("Copied"),
        'T' => Some("Type changed, staged"),
        _ => None,
    }
}

fn tree_word(y: char) -> Option<&'static str> {
    match y {
        'M' => Some("modified"),
        'D' => Some("deleted"),
        'T' => Some("type changed"),
        _ => None,
    }
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn conflict_word(x: char, y: char) -> &'static str {
    match (x, y) {
        ('D', 'D') => "Conflict, both deleted",
        ('A', 'U') => "Conflict, added by us",
        ('U', 'D') => "Conflict, deleted by them",
        ('U', 'A') => "Conflict, added by them",
        ('D', 'U') => "Conflict, deleted by us",
        ('A', 'A') => "Conflict, both added",
        _ => "Conflict, both modified",
    }
}

impl Change {
    /// The state in words, meaning first: "Modified", "Staged, modified",
    /// "Renamed", "Conflict, both modified", "Untracked", "Submodule, new
    /// commits".
    pub fn word(&self) -> String {
        if let Some(sub) = self.submodule {
            let mut word = "Submodule".to_owned();
            if sub.commit {
                word.push_str(", new commits");
            }
            if sub.modified {
                word.push_str(", modified");
            }
            if sub.untracked {
                word.push_str(", untracked content");
            }
            if !sub.commit && !sub.modified && !sub.untracked {
                // Added, deleted or moved as a whole.
                word.push_str(&match self.kind {
                    Kind::Changed { x, y } | Kind::Renamed { x, y } => {
                        match (index_word(x), tree_word(y)) {
                            (Some(i), _) => format!(", {}", i.to_lowercase()),
                            (None, Some(t)) => format!(", {t}"),
                            (None, None) => String::new(),
                        }
                    }
                    _ => String::new(),
                });
            }
            return word;
        }
        match self.kind {
            Kind::Changed { x, y } | Kind::Renamed { x, y } => {
                match (index_word(x), tree_word(y)) {
                    (Some(i), Some(t)) => {
                        // "Deleted, staged" and a tree change never meet;
                        // the staged words drop their ", staged".
                        let i = i.trim_end_matches(", staged");
                        format!("{i}, {t}")
                    }
                    (Some(i), None) => i.to_owned(),
                    (None, Some(t)) => capital(t),
                    (None, None) => "Unchanged".to_owned(),
                }
            }
            Kind::Conflict { x, y } => conflict_word(x, y).to_owned(),
            Kind::Untracked => "Untracked".to_owned(),
            Kind::Ignored => "Ignored".to_owned(),
        }
    }

    /// Whether the change is in the index: what a commit takes.
    pub fn is_staged(&self) -> bool {
        match self.kind {
            Kind::Changed { x, .. } | Kind::Renamed { x, .. } => x != '.',
            _ => false,
        }
    }

    /// Whether the working tree differs from the index: what Discard
    /// takes back.
    pub fn in_tree(&self) -> bool {
        match self.kind {
            Kind::Changed { y, .. } | Kind::Renamed { y, .. } => y != '.',
            _ => false,
        }
    }

    /// Whether it is a conflict.
    pub fn is_conflict(&self) -> bool {
        matches!(self.kind, Kind::Conflict { .. })
    }

    /// Whether it is tracked: neither untracked nor ignored.
    pub fn is_tracked(&self) -> bool {
        !matches!(self.kind, Kind::Untracked | Kind::Ignored)
    }

    /// The path without a folder's closing `/`.
    pub fn bare_path(&self) -> &str {
        self.path.trim_end_matches('/')
    }
}

impl Status {
    /// The changes under a folder (from the repository's top, empty for
    /// the top itself), ignored paths left out.
    pub fn under<'a>(&'a self, folder: &'a str) -> impl Iterator<Item = &'a Change> + 'a {
        self.changes
            .iter()
            .filter(move |c| !matches!(c.kind, Kind::Ignored) && is_under(&c.path, folder))
    }

    /// How many paths are in conflict.
    pub fn conflicts(&self) -> usize {
        self.changes.iter().filter(|c| c.is_conflict()).count()
    }

    /// The staged changes.
    pub fn staged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.is_staged())
    }
}

/// Whether a path from the top is inside `folder` (from the top; empty for
/// the top).
pub fn is_under(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    folder.is_empty()
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records as git 2.47 and 2.54 write them, one per line here, NUL in
    /// the output.
    fn v2(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(0);
        }
        out
    }

    const OID: &str = "100644 100644 100644 5626abf0f72e58d7a153368ba57db4c673c0e171 5626abf0f72e58d7a153368ba57db4c673c0e171";

    #[test]
    fn the_branch_headers_read_and_unknown_ones_pass() {
        let s = parse(&v2(&[
            "# branch.oid f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "# stash 3",
            "# branch.future something",
        ]))
        .unwrap();
        assert_eq!(
            s.branch,
            Branch {
                oid: Some("f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57".into()),
                head: Some("main".into()),
                upstream: Some("origin/main".into()),
                ahead: Some(2),
                behind: Some(1),
            }
        );
        let s = parse(&v2(&["# branch.oid (initial)", "# branch.head (detached)"])).unwrap();
        assert_eq!(s.branch.oid, None);
        assert_eq!(s.branch.head, None);
        assert!(s.changes.is_empty());
    }

    #[test]
    fn every_record_kind_reads_with_its_words() {
        let rename = format!("2 R. N... {OID} R100 b2.txt");
        let renamed_modified = format!("2 RM N... {OID} R87 new name.txt");
        let s = parse(&v2(&[
            &format!("1 .M N... {OID} a.txt"),
            &format!("1 M. N... {OID} b.txt"),
            &format!("1 MM N... {OID} sub/c.txt"),
            &format!("1 A. N... {OID} d.txt"),
            &format!("1 AM N... {OID} e.txt"),
            &format!("1 .D N... {OID} f.txt"),
            &format!("1 D. N... {OID} g.txt"),
            &format!("1 .T N... {OID} h.txt"),
            &format!("1 MD N... {OID} i.txt"),
            &rename,
            "b.txt",
            &renamed_modified,
            "old name.txt",
            "u UU N... 100644 100644 100644 100644 a1 b2 c3 both.txt",
            "u DU N... 100644 100644 100644 100644 a1 b2 c3 gone.txt",
            "? n.txt",
            "? u/",
            "! target/",
        ]))
        .unwrap();
        let words: Vec<String> = s.changes.iter().map(Change::word).collect();
        assert_eq!(
            words,
            [
                "Modified",
                "Staged",
                "Staged, modified",
                "Added",
                "Added, modified",
                "Deleted",
                "Deleted, staged",
                "Type changed",
                "Staged, deleted",
                "Renamed",
                "Renamed, modified",
                "Conflict, both modified",
                "Conflict, deleted by us",
                "Untracked",
                "Untracked",
                "Ignored",
            ]
        );
        assert_eq!(s.changes[9].from.as_deref(), Some("b.txt"));
        assert_eq!(s.changes[10].path, "new name.txt");
        assert_eq!(s.changes[10].from.as_deref(), Some("old name.txt"));
        assert_eq!(s.changes[14].bare_path(), "u");
        assert_eq!(s.conflicts(), 2);
        assert_eq!(s.staged().count(), 8);
    }

    #[test]
    fn odd_names_keep_their_bytes_and_say_when_they_are_not_unicode() {
        let s = parse(&v2(&[
            &format!("1 .M N... {OID} with  two spaces.txt"),
            &format!("1 .M N... {OID} \"quoted\" name.txt"),
            &format!("1 .M N... {OID} caf\u{e9}/\u{65e5}\u{672c}.txt"),
            &format!("1 .M N... {OID} -leading-dash.txt"),
            &format!("1 .M N... {OID} line\nbreak.txt"),
        ]))
        .unwrap();
        let paths: Vec<&str> = s.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "with  two spaces.txt",
                "\"quoted\" name.txt",
                "caf\u{e9}/\u{65e5}\u{672c}.txt",
                "-leading-dash.txt",
                "line\nbreak.txt",
            ]
        );
        let mut bytes = format!("1 .M N... {OID} bad-").into_bytes();
        bytes.extend_from_slice(b"\xff\xfe.txt\0");
        let s = parse(&bytes).unwrap();
        assert!(s.changes[0].not_unicode);
        assert!(s.changes[0].path.starts_with("bad-"));
    }

    #[test]
    fn submodules_say_what_changed_inside() {
        let s = parse(&v2(&[
            &format!("1 .M SC.. {OID} libs/one"),
            &format!("1 .M S.MU {OID} libs/two"),
            &format!("1 A. S... {OID} libs/three"),
        ]))
        .unwrap();
        let words: Vec<String> = s.changes.iter().map(Change::word).collect();
        assert_eq!(
            words,
            [
                "Submodule, new commits",
                "Submodule, modified, untracked content",
                "Submodule, added"
            ]
        );
    }

    #[test]
    fn short_or_broken_records_are_refused_in_words_and_unknown_kinds_pass() {
        assert_eq!(
            parse(b"1 .M N...\0").unwrap_err().to_string(),
            "git's status could not be read: an ordinary record is short"
        );
        let rename = format!("2 R. N... {OID} R100 b2.txt\0");
        assert!(parse(rename.as_bytes()).is_err());
        assert!(parse(b"z something new\0? n.txt\0").unwrap().changes.len() == 1);
        assert!(parse(b"").unwrap().changes.is_empty());
        assert!(parse(b"?\0!x\0").unwrap().changes.is_empty());
    }

    #[test]
    fn under_a_folder_means_inside_it() {
        assert!(is_under("sub/c.txt", "sub"));
        assert!(is_under("sub/deep/c.txt", "sub/"));
        assert!(!is_under("subway.txt", "sub"));
        assert!(!is_under("sub", "sub"));
        assert!(is_under("anything", ""));
    }
}
