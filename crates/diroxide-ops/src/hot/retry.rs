//! What a failure means for a task (R16 2.6; R28 6.3): the same file
//! tried again later, the file filed in `failed`, or the whole task
//! paused with every file left in `in`.
//!
//! - **Again** (in use, timed out, stopped by a signal, changed during the
//!   run): after 10 seconds, then 1 minute, then 10 minutes, as many times
//!   as the task's `retries` says. No retry loop is unbounded.
//! - **The file** (unreadable, damaged, refused, a tool's error for this
//!   file, a password needed): the original goes to `failed` with its
//!   reason in the log, and the next file goes on.
//! - **The task** (a tool not installed or not starting, a drive full or
//!   read only, the network lost, a folder missing): the task pauses and
//!   says why once; nothing more is tried until the owner resumes it.
//!
//! The circuit breaker: ten files in a row failing with the same cause
//! pause the task ("Scans paused: 10 failures in a row"), since that is
//! almost always a wrong recipe for the files and not ten bad files. A
//! file is counted once, after its last try, so one bad file retried
//! never trips it; a success, or a failure with another cause, starts the
//! count again.

use std::time::Duration;

/// The waits before each retry: 10 seconds, 1 minute, 10 minutes; later
/// retries wait the last.
pub const RETRY_WAITS: [Duration; 3] = [
    Duration::from_secs(10),
    Duration::from_secs(60),
    Duration::from_secs(600),
];

/// How many files in a row failing with one cause pause a task.
pub const BREAKER_AT: u32 = 10;

/// What a failure's cause means for the task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CauseClass {
    /// The same file may succeed later: try it again.
    Again,
    /// Something about this file: file it in `failed`.
    File,
    /// Something about the surroundings or the task: pause it.
    Task,
}

/// Words that say the surroundings or the task are the cause, as the
/// engine, the tools runner and the file system say them.
const TASK_WORDS: [&str; 10] = [
    "is not installed",
    "could not be started",
    " encoder",
    "the drive is full",
    "the quota on this drive is used up",
    "the drive is read only",
    "the network connection was lost",
    "nothing is set up to open",
    "needs a privilege",
    "the target folder",
];

/// Words that say the same file may succeed later.
const AGAIN_WORDS: [&str; 7] = [
    "another program is using it",
    "is in use",
    "being used by another process",
    "took longer than",
    "was stopped",
    "it changed since it was listed",
    "changed during the run",
];

/// What a cause in words means for the task.
pub fn classify(cause: &str) -> CauseClass {
    let lower = cause.to_lowercase();
    if TASK_WORDS.iter().any(|w| lower.contains(w)) {
        CauseClass::Task
    } else if AGAIN_WORDS.iter().any(|w| lower.contains(w)) {
        CauseClass::Again
    } else {
        CauseClass::File
    }
}

/// The wait before retry number `n` (1 for the first).
pub fn wait_before(n: u32, waits: &[Duration]) -> Duration {
    let at = usize::try_from(n.saturating_sub(1)).unwrap_or(usize::MAX);
    waits
        .get(at)
        .or_else(|| waits.last())
        .copied()
        .unwrap_or(RETRY_WAITS[0])
}

/// A cause with the file's own name taken out, so ten bad files of the
/// same kind give one cause: "ImageMagick ended with code 1: improper
/// image header {file}".
pub fn cause_key(cause: &str, name: &str) -> String {
    let mut key = cause.to_owned();
    if !name.is_empty() {
        key = key.replace(name, "{file}");
        if let Some((stem, _)) = name.rsplit_once('.')
            && stem.len() > 1
        {
            key = key.replace(stem, "{file}");
        }
    }
    key
}

/// The circuit breaker. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breaker {
    limit: u32,
    cause: Option<String>,
    count: u32,
}

impl Breaker {
    /// A breaker that trips at `limit` failures in a row with one cause.
    pub fn new(limit: u32) -> Self {
        Self {
            limit: limit.max(1),
            cause: None,
            count: 0,
        }
    }

    /// Counts a file that failed with this cause key; true when that makes
    /// the limit, and the breaker trips.
    pub fn failed(&mut self, key: &str) -> bool {
        if self.cause.as_deref() == Some(key) {
            self.count += 1;
        } else {
            self.cause = Some(key.to_owned());
            self.count = 1;
        }
        self.count >= self.limit
    }

    /// A file succeeded: the count starts again.
    pub fn succeeded(&mut self) {
        self.cause = None;
        self.count = 0;
    }

    /// How many in a row now.
    pub fn count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn causes_are_sorted_by_what_they_mean() {
        for (cause, class) in [
            ("pdftotext is not installed", CauseClass::Task),
            (
                "ImageMagick could not be started: access is denied",
                CauseClass::Task,
            ),
            ("this FFmpeg has no libx264 encoder", CauseClass::Task),
            ("the drive is full", CauseClass::Task),
            ("the drive is read only", CauseClass::Task),
            ("the network connection was lost", CauseClass::Task),
            ("another program is using it", CauseClass::Again),
            (
                "pdftotext took longer than 600 seconds and was stopped",
                CauseClass::Again,
            ),
            ("FFmpeg was stopped", CauseClass::Again),
            ("Changed during the run", CauseClass::Again),
            (
                "ImageMagick ended with code 1: improper image header",
                CauseClass::File,
            ),
            ("a password is needed", CauseClass::File),
            ("it is damaged: a bad header", CauseClass::File),
            ("Refused: over 500 MB, the recipe's limit", CauseClass::File),
        ] {
            assert_eq!(classify(cause), class, "{cause}");
        }
    }

    #[test]
    fn waits_grow_and_stop_at_the_last() {
        assert_eq!(wait_before(1, &RETRY_WAITS), Duration::from_secs(10));
        assert_eq!(wait_before(2, &RETRY_WAITS), Duration::from_secs(60));
        assert_eq!(wait_before(3, &RETRY_WAITS), Duration::from_secs(600));
        assert_eq!(wait_before(5, &RETRY_WAITS), Duration::from_secs(600));
        assert_eq!(wait_before(1, &[]), Duration::from_secs(10));
    }

    #[test]
    fn ten_in_a_row_with_one_cause_trip_and_anything_else_starts_again() {
        let mut breaker = Breaker::new(BREAKER_AT);
        let cause = |n: u32| {
            cause_key(
                &format!("ImageMagick ended with code 1: improper image header `bad-{n}.png'"),
                &format!("bad-{n}.png"),
            )
        };
        assert_eq!(cause(1), cause(2));
        for n in 1..=9 {
            assert!(!breaker.failed(&cause(n)), "{n}");
        }
        // A success in between starts the count again.
        breaker.succeeded();
        for n in 1..=9 {
            assert!(!breaker.failed(&cause(n)));
        }
        // So does another cause.
        assert!(!breaker.failed("a password is needed"));
        assert_eq!(breaker.count(), 1);
        for n in 1..=9 {
            assert!(!breaker.failed(&cause(n)));
        }
        assert!(breaker.failed(&cause(10)));
    }
}
