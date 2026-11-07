//! The ledger (R16 3.4; R28 6.3): an append-only text file per task,
//! `hot/NAME/ledger.txt`, one line per handled file, the outcome first,
//! then the name, the file's identity (volume, inode, size, modified
//! time), the time and, for a committed output, its path.
//!
//! It is what makes a run never do anything twice:
//!
//! - A **made** line is written, and flushed to the disk, after the
//!   output is committed and before the input moves. A crash between the
//!   two leaves the input in `in` with a made line of its identity, so the
//!   next start finishes the move and converts nothing again.
//! - **done**, **failed**, **skipped** and **told** lines say a file was
//!   handled: in `leave` mode, and for a tell task, the file stays, and its
//!   identity in the ledger keeps it from being handled again; a changed
//!   size or time, or a new file under the same name, is a new arrival.
//! - **cleared** is "Put back": the name is new again.
//!
//! Names and paths are written so they read back exactly: a backslash,
//! a tab and line breaks are escaped, and every hidden character is
//! spelled `\u{XXXX}`. The ledger never holds a secret. Past its size cap
//! it is compacted to the newest line of each name still in `in`.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use diroxide_core::names::classify;
use diroxide_core::{Entry, Timestamp};

/// The ledger's file name in a task's folder.
pub const LEDGER_FILE: &str = "ledger.txt";

/// The size past which the ledger is compacted.
pub const LEDGER_CAP: u64 = 1024 * 1024;

/// What happened to a file, the first word of its ledger line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// The output is committed; the input has not moved yet.
    Made,
    /// Handled: the output made and the input moved, or left in place.
    Done,
    /// It failed: moved to `failed`, or left in place in `leave` mode.
    Failed,
    /// Nothing was needed: the output exists, or is up to date.
    Skipped,
    /// A tell task said it arrived.
    Told,
    /// Put back: the name is new again.
    Cleared,
}

impl Outcome {
    /// The word on the line.
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Made => "made",
            Outcome::Done => "done",
            Outcome::Failed => "failed",
            Outcome::Skipped => "skipped",
            Outcome::Told => "told",
            Outcome::Cleared => "cleared",
        }
    }

    /// Reads the word.
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "made" => Outcome::Made,
            "done" => Outcome::Done,
            "failed" => Outcome::Failed,
            "skipped" => Outcome::Skipped,
            "told" => Outcome::Told,
            "cleared" => Outcome::Cleared,
            _ => return None,
        })
    }
}

/// What tells one file from another under the same name: its volume and
/// inode where known, its size and its modified time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Identity {
    /// The volume, when known.
    pub volume: Option<u64>,
    /// The inode or file index, when known.
    pub inode: Option<u64>,
    /// The size, when known.
    pub size: Option<u64>,
    /// The modified time, when known.
    pub modified: Option<Timestamp>,
}

impl Identity {
    /// A listed or looked-at entry's identity.
    pub fn of(entry: &Entry) -> Self {
        Self {
            volume: entry.volume,
            inode: entry.inode,
            size: entry.size,
            modified: entry.modified,
        }
    }

    /// Whether two looks found the same file: the same size and modified
    /// time, and, where both know them, the same volume and inode.
    pub fn same(&self, other: &Identity) -> bool {
        let known = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        self.size == other.size
            && self.modified == other.modified
            && known(self.volume, other.volume)
            && known(self.inode, other.inode)
    }
}

/// One line of the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// What happened.
    pub outcome: Outcome,
    /// The file's name in `in`.
    pub name: String,
    /// The file as it was.
    pub identity: Identity,
    /// When, from the clock.
    pub at: Timestamp,
    /// The output committed, for a made or done line.
    pub output: Option<String>,
}

/// Escapes a name or a path for a line: `\` as `\\`, a tab as `\t`, line
/// breaks as `\n` and `\r`, every other hidden character as `\u{XXXX}`.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if classify(c).is_hidden() => out.push_str(&format!("\\u{{{:04X}}}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// Reads back what [`escape`] wrote; `None` for text it never writes.
pub fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut hex = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        h if h.is_ascii_hexdigit() && hex.len() < 6 => hex.push(h),
                        _ => return None,
                    }
                }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn number(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

fn time(value: Option<Timestamp>) -> String {
    value.map_or_else(
        || "-".to_owned(),
        |t| format!("{}.{:09}", t.unix_seconds(), t.subsec_nanos()),
    )
}

fn read_number(text: &str) -> Option<Option<u64>> {
    if text == "-" {
        Some(None)
    } else {
        text.parse().ok().map(Some)
    }
}

fn read_time(text: &str) -> Option<Option<Timestamp>> {
    if text == "-" {
        return Some(None);
    }
    let (seconds, nanos) = text.split_once('.')?;
    Some(Some(Timestamp::from_unix(
        seconds.parse().ok()?,
        nanos.parse().ok()?,
    )))
}

impl Line {
    /// The line as written, without its line break: the fields separated
    /// by tabs, the outcome first.
    pub fn text(&self) -> String {
        let id = &self.identity;
        format!(
            "{}\t{}\tv={}\ti={}\ts={}\tm={}\tt={}\to={}",
            self.outcome.word(),
            escape(&self.name),
            number(id.volume),
            number(id.inode),
            number(id.size),
            time(id.modified),
            time(Some(self.at)),
            self.output
                .as_deref()
                .map_or_else(|| "-".to_owned(), escape),
        )
    }

    /// Reads a line; `None` for one that is not whole (the end of a file
    /// a crash cut short).
    pub fn read(text: &str) -> Option<Self> {
        let fields: Vec<&str> = text.split('\t').collect();
        let [outcome, name, v, i, s, m, t, o] = fields.as_slice() else {
            return None;
        };
        let field = |text: &'_ str, key: &str| -> Option<String> {
            text.strip_prefix(key)?.strip_prefix('=').map(str::to_owned)
        };
        Some(Self {
            outcome: Outcome::from_word(outcome)?,
            name: unescape(name)?,
            identity: Identity {
                volume: read_number(&field(v, "v")?)?,
                inode: read_number(&field(i, "i")?)?,
                size: read_number(&field(s, "s")?)?,
                modified: read_time(&field(m, "m")?)?,
            },
            at: read_time(&field(t, "t")?)??,
            output: match field(o, "o")?.as_str() {
                "-" => None,
                other => Some(unescape(other)?),
            },
        })
    }
}

/// A task's ledger: the file, and the newest line of each name read from
/// it. See the module documentation.
#[derive(Debug)]
pub struct Ledger {
    path: PathBuf,
    cap: u64,
    newest: HashMap<String, Line>,
    /// The file ends in a line a crash cut short: the next line starts on
    /// a line of its own.
    cut_short: bool,
}

impl Ledger {
    /// Opens the ledger at `path` (made on the first line written) and
    /// reads it. Lines that are not whole are passed over.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let mut newest = HashMap::new();
        let mut cut_short = false;
        match fs::read_to_string(&path) {
            Ok(text) => {
                cut_short = !text.is_empty() && !text.ends_with('\n');
                for line in text.lines().filter_map(Line::read) {
                    newest.insert(line.name.clone(), line);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(Self {
            path,
            cap: LEDGER_CAP,
            newest,
            cut_short,
        })
    }

    /// The same ledger compacted past `cap` bytes instead of
    /// [`LEDGER_CAP`].
    pub fn with_cap(mut self, cap: u64) -> Self {
        self.cap = cap;
        self
    }

    /// The ledger's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends a line and flushes it to the disk before returning, so a
    /// crash after it never loses it.
    pub fn append(&mut self, line: Line) -> io::Result<()> {
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let start = if self.cut_short { "\n" } else { "" };
        file.write_all(format!("{start}{}\n", line.text()).as_bytes())?;
        file.sync_data()?;
        self.cut_short = false;
        self.newest.insert(line.name.clone(), line);
        Ok(())
    }

    /// The newest line of a name, if any.
    pub fn newest(&self, name: &str) -> Option<&Line> {
        self.newest.get(name)
    }

    /// The newest line of a name if it is about this same file and was
    /// not cleared: what the file came to before.
    pub fn handled(&self, name: &str, identity: &Identity) -> Option<&Line> {
        self.newest
            .get(name)
            .filter(|line| line.outcome != Outcome::Cleared && line.identity.same(identity))
    }

    /// The made lines with no line after them: outputs committed whose
    /// inputs did not move, in name order.
    pub fn unfinished(&self) -> Vec<&Line> {
        let mut out: Vec<&Line> = self
            .newest
            .values()
            .filter(|line| line.outcome == Outcome::Made)
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Whether the file has grown past its cap.
    pub fn is_over_cap(&self) -> bool {
        fs::metadata(&self.path).is_ok_and(|m| m.len() > self.cap)
    }

    /// Rewrites the ledger with the newest line of each name in `present`
    /// (the names in `in` now), cleared names left out, and returns how
    /// many lines it keeps. Written whole beside it and renamed into
    /// place.
    pub fn compact(&mut self, present: &BTreeSet<String>) -> io::Result<usize> {
        self.newest
            .retain(|name, line| line.outcome != Outcome::Cleared && present.contains(name));
        let mut lines: Vec<&Line> = self.newest.values().collect();
        lines.sort_by(|a, b| (a.at, &a.name).cmp(&(b.at, &b.name)));
        let mut text = String::new();
        for line in &lines {
            text.push_str(&line.text());
            text.push('\n');
        }
        let new = self.path.with_extension("txt.new");
        {
            let mut file = fs::File::create(&new)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&new, &self.path)?;
        self.cut_short = false;
        Ok(lines.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::testing::scratch;

    fn identity(size: u64, modified: i64) -> Identity {
        Identity {
            volume: Some(7),
            inode: Some(1000 + size),
            size: Some(size),
            modified: Some(Timestamp::from_unix(modified, 250)),
        }
    }

    fn line(outcome: Outcome, name: &str, size: u64) -> Line {
        Line {
            outcome,
            name: name.to_owned(),
            identity: identity(size, 1_790_000_000),
            at: Timestamp::from_unix(1_790_000_100, 0),
            output: (outcome == Outcome::Made).then(|| format!("D:\\scans\\out\\{name}.txt")),
        }
    }

    /// Hostile names are built in memory only, never made on the disk.
    #[test]
    fn every_name_reads_back_exactly() {
        for name in [
            "scan-004.pdf",
            "two\twords",
            "line\nbreak.pdf",
            "carriage\rreturn",
            "back\\slash",
            "-starts-with-dash.pdf",
            "private\u{f03a}use",
            "right\u{202e}fdp.exe",
            "zero\u{200b}width",
            "caf\u{e9}.pdf",
            "\\u{41}",
        ] {
            let written = escape(name);
            assert!(!written.contains(['\t', '\n', '\r']), "{written:?}");
            assert_eq!(unescape(&written).as_deref(), Some(name), "{written}");
            let made = Line {
                name: name.to_owned(),
                ..line(Outcome::Made, "x", 3)
            };
            assert_eq!(Line::read(&made.text()), Some(made));
        }
        assert_eq!(escape("private\u{f03a}use"), "private\\u{F03A}use");
        assert_eq!(unescape("bad\\q"), None);
        assert_eq!(Line::read("made\tcut short"), None);
    }

    #[test]
    fn the_outcome_comes_first_and_identity_decides_what_was_handled() {
        let made = line(Outcome::Made, "scan-004.pdf", 300);
        assert!(
            made.text().starts_with("made\tscan-004.pdf\tv=7\t"),
            "{}",
            made.text()
        );
        let dir = scratch();
        let path = dir.path().join("scans").join(LEDGER_FILE);
        let mut ledger = Ledger::open(&path).unwrap();
        assert!(ledger.unfinished().is_empty());
        ledger.append(made.clone()).unwrap();
        ledger.append(line(Outcome::Done, "a.pdf", 10)).unwrap();
        // Read again, as the next start does.
        let ledger = Ledger::open(&path).unwrap();
        let pending: Vec<&str> = ledger
            .unfinished()
            .iter()
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(pending, ["scan-004.pdf"]);
        assert_eq!(
            ledger
                .handled("scan-004.pdf", &identity(300, 1_790_000_000))
                .map(|l| l.outcome),
            Some(Outcome::Made)
        );
        // A different size or time under the same name is a new file.
        assert!(
            ledger
                .handled("scan-004.pdf", &identity(301, 1_790_000_000))
                .is_none()
        );
        assert!(
            ledger
                .handled("scan-004.pdf", &identity(300, 1_790_000_001))
                .is_none()
        );
        // An identity a listing does not know is not a change.
        let bare = Identity {
            volume: None,
            inode: None,
            ..identity(300, 1_790_000_000)
        };
        assert!(ledger.handled("scan-004.pdf", &bare).is_some());
    }

    #[test]
    fn a_line_cut_short_by_a_crash_is_passed_over_and_cleared_names_are_new() {
        let dir = scratch();
        let path = dir.path().join(LEDGER_FILE);
        let mut ledger = Ledger::open(&path).unwrap();
        ledger.append(line(Outcome::Failed, "bad.pdf", 5)).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"made\tgood.pdf\tv=7\ti=").unwrap();
        drop(file);
        let mut ledger = Ledger::open(&path).unwrap();
        assert!(ledger.newest("good.pdf").is_none());
        assert_eq!(
            ledger.newest("bad.pdf").map(|l| l.outcome),
            Some(Outcome::Failed)
        );
        // A line written after the cut starts on a line of its own, so
        // it reads.
        ledger.append(line(Outcome::Cleared, "bad.pdf", 5)).unwrap();
        ledger.append(line(Outcome::Done, "next.pdf", 6)).unwrap();
        let ledger = Ledger::open(&path).unwrap();
        assert!(
            ledger
                .handled("bad.pdf", &identity(5, 1_790_000_000))
                .is_none()
        );
        assert!(ledger.newest("next.pdf").is_some());
    }

    #[test]
    fn compaction_keeps_the_newest_line_of_each_name_still_waiting() {
        let dir = scratch();
        let path = dir.path().join(LEDGER_FILE);
        let mut ledger = Ledger::open(&path).unwrap().with_cap(200);
        for n in 0..6 {
            ledger.append(line(Outcome::Told, "kept.pdf", n)).unwrap();
            ledger
                .append(line(Outcome::Done, &format!("gone-{n}.pdf"), n))
                .unwrap();
        }
        ledger
            .append(line(Outcome::Cleared, "put-back.pdf", 1))
            .unwrap();
        assert!(ledger.is_over_cap());
        let present: BTreeSet<String> = ["kept.pdf", "put-back.pdf"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(ledger.compact(&present).unwrap(), 1);
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with("told\tkept.pdf\t"));
        assert!(!ledger.is_over_cap());
        let again = Ledger::open(&path).unwrap();
        assert_eq!(
            again.newest("kept.pdf").map(|l| l.identity.size),
            Some(Some(5))
        );
        assert!(!dir.path().join("ledger.txt.new").exists());
    }
}
