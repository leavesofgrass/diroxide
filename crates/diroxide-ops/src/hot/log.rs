//! The log (R16 3.5; R28 6.3): a plain text file per task,
//! `hot/NAME/log.txt`, one line per outcome, rotated at 1 MiB with five
//! kept (`log.1.txt` the newest of them, `log.5.txt` the oldest).
//!
//! Each line puts the outcome first and the time last, so the first 40
//! cells of a Braille line say what happened:
//!
//! `done  scan-004.pdf  pdf-to-text  3 seconds  out\scan-004.txt  2026-11-06 20:41:07`
//!
//! Fields are separated by two spaces. Hidden characters in names and
//! causes are spelled out (`[U+202E]`), so nothing in the log hides from
//! sight or speech. The log never holds a secret and never leaves the
//! machine.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use diroxide_core::names::spell_out;
use diroxide_core::{Timestamp, UtcOffset};

/// The log's file name in a task's folder.
pub const LOG_FILE: &str = "log.txt";

/// The size at which the log is rotated.
pub const LOG_CAP: u64 = 1024 * 1024;

/// How many rotated logs are kept beside the current one.
pub const LOGS_KEPT: u32 = 5;

/// The time as the log writes it, in local time: `2026-11-06 20:41:07`.
pub fn log_time(at: Timestamp, offset: UtcOffset) -> String {
    let c = at.to_civil(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// A log line: the outcome word, then the fields with hidden characters
/// spelled out, then the time.
pub fn log_line(outcome: &str, fields: &[&str], at: Timestamp, offset: UtcOffset) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(fields.len() + 2);
    parts.push(outcome.to_owned());
    parts.extend(
        fields
            .iter()
            .filter(|f| !f.is_empty())
            .map(|f| spell_out(f).into_owned()),
    );
    parts.push(log_time(at, offset));
    parts.join("  ")
}

/// A task's log file, rotated as it grows. See the module documentation.
#[derive(Clone, Debug)]
pub struct TaskLog {
    path: PathBuf,
    cap: u64,
}

impl TaskLog {
    /// The log at `path`, made on the first line written.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cap: LOG_CAP,
        }
    }

    /// The same log rotated at `cap` bytes instead of [`LOG_CAP`].
    pub fn with_cap(mut self, cap: u64) -> Self {
        self.cap = cap;
        self
    }

    /// The current log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path of the `n`th rotated log: `log.1.txt`.
    fn rotated(&self, n: u32) -> PathBuf {
        self.path.with_extension(format!("{n}.txt"))
    }

    /// Appends one line, rotating first when it would pass the cap.
    pub fn write(&self, line: &str) -> io::Result<()> {
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        let size = fs::metadata(&self.path).map_or(0, |m| m.len());
        if size > 0 && size + line.len() as u64 + 1 > self.cap {
            self.rotate()?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(format!("{line}\n").as_bytes())
    }

    /// Moves `log.txt` to `log.1.txt` and each older one up a place; the
    /// one past [`LOGS_KEPT`] goes. Only the task's own log files are
    /// touched.
    fn rotate(&self) -> io::Result<()> {
        let oldest = self.rotated(LOGS_KEPT);
        match fs::remove_file(&oldest) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        for n in (1..LOGS_KEPT).rev() {
            let from = self.rotated(n);
            if from.exists() {
                fs::rename(&from, self.rotated(n + 1))?;
            }
        }
        fs::rename(&self.path, self.rotated(1))
    }

    /// The newest `limit` lines, newest first, across the current log and
    /// the rotated ones: the Log list's rows.
    pub fn newest(&self, limit: usize) -> Vec<String> {
        let mut out = Vec::new();
        let files =
            std::iter::once(self.path.clone()).chain((1..=LOGS_KEPT).map(|n| self.rotated(n)));
        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            for line in text.lines().rev() {
                if out.len() == limit {
                    return out;
                }
                out.push(line.to_owned());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::testing::scratch;
    use diroxide_core::format::within_cells;

    fn at() -> Timestamp {
        // Friday, November 6, 2026, 8:41:07 PM in UTC.
        Timestamp::from_unix(1_793_997_667, 0)
    }

    #[test]
    fn a_line_says_the_outcome_first_and_the_time_last() {
        let line = log_line(
            "done",
            &[
                "scan-004.pdf",
                "pdf-to-text",
                "3 seconds",
                "out\\scan-004.txt",
            ],
            at(),
            UtcOffset::UTC,
        );
        assert_eq!(
            line,
            "done  scan-004.pdf  pdf-to-text  3 seconds  out\\scan-004.txt  2026-11-06 20:41:07"
        );
        assert!(within_cells(&line, "done  scan-004.pdf  pdf-to-text"));
        // Hidden characters are spelled out, line breaks included (names
        // built in memory only).
        let line = log_line(
            "failed",
            &["right\u{202e}fdp.exe", "a cause\non two lines"],
            at(),
            UtcOffset::UTC,
        );
        assert_eq!(
            line,
            "failed  right[U+202E]fdp.exe  a cause[U+000A]on two lines  2026-11-06 20:41:07"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn the_log_rotates_at_its_cap_and_keeps_five() {
        let dir = scratch();
        let log = TaskLog::new(dir.path().join("scans").join(LOG_FILE)).with_cap(100);
        for n in 0..40 {
            log.write(&format!("done  file-{n:02}.pdf  2026-11-06 20:41:07"))
                .unwrap();
        }
        let folder = dir.path().join("scans");
        let mut names: Vec<String> = fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "log.1.txt",
                "log.2.txt",
                "log.3.txt",
                "log.4.txt",
                "log.5.txt",
                "log.txt"
            ]
        );
        for name in &names {
            assert!(fs::metadata(folder.join(name)).unwrap().len() <= 100);
        }
        let newest = log.newest(3);
        assert_eq!(newest.len(), 3);
        assert!(newest[0].contains("file-39.pdf"), "{newest:?}");
        assert!(newest[1].contains("file-38.pdf"), "{newest:?}");
        // The oldest lines rotated away; the newest twelve are kept.
        let all = log.newest(1000);
        assert_eq!(all.len(), 12);
        assert!(all.last().unwrap().contains("file-28.pdf"), "{all:?}");
    }
}
