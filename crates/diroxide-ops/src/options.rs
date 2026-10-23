//! What a job keeps, checks and retries: the engine's options. The app
//! fills them from the settings (`keep_dates`, `verify_moves`,
//! `verify_copies`, `replace_keeps_old_in_trash`, `network_retry_count`,
//! `network_retry_seconds`, `stall_seconds`; the two retry settings are
//! real settings from R25 commit 80); the defaults are the plan's (section
//! 5.4 and research/file-operations.md).
//!
//! A lost network is retried silently with growing waits (R25, 2.3): the
//! first wait is `network_retry_wait`, each next one `network_retry_growth`
//! times longer (5, 15 and 45 seconds), `network_retries` of them, each at
//! most [`MAX_RETRY_WAIT`]. No loop is unbounded and there is no
//! "unlimited".

use std::time::Duration;

use diroxide_core::Place;

/// The engine's options. Start from [`EngineOptions::default`] and change
/// the fields that differ.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineOptions {
    /// Keep the modified time of files and folders, and the created time
    /// where the destination can set one (Keep dates; on by default). A
    /// folder's times are set after its contents, so the copy's own
    /// moment never replaces them. The accessed time is never kept.
    pub keep_dates: bool,
    /// Say in the result, as a warning, when a created time could not be
    /// kept (Linux cannot set one). Off by default: the user asked for it
    /// only when this is on.
    pub warn_created_time: bool,
    /// Read a moved file back and compare it before the source is
    /// removed (on by default).
    pub verify_moves: bool,
    /// Read a copied file back and compare it (off by default; it reads
    /// every file twice).
    pub verify_copies: bool,
    /// Send the old file to the trash before a replace, where the place
    /// has one, so Undo can bring it back (on by default).
    pub replace_keeps_old_in_trash: bool,
    /// How many times a lost network is retried before the user is asked:
    /// the retry budget, also the connection gate's tries per round.
    pub network_retries: u32,
    /// How long to wait before the first of those retries.
    pub network_retry_wait: Duration,
    /// How many times longer each next wait is than the one before (3:
    /// waits of 5, 15 and 45 seconds); 1 keeps the waits equal.
    pub network_retry_growth: u32,
    /// How long without a byte or a step before the job says it stalled.
    /// Zero turns the stall detector off.
    pub stall_after: Duration,
    /// The copy loop's chunk: pause, cancel and progress are honored
    /// between chunks.
    pub chunk_bytes: usize,
    /// Files of at least this many bytes are written so a lost connection
    /// can resume them where they stopped (R25, 2.5), where both places
    /// can; smaller ones simply start again. 4 MiB: below it the window
    /// check would read about as much as it saves.
    pub resume_over: u64,
    /// A permanent delete of more entries than this asks for the count to
    /// be typed.
    pub typed_count_over: u64,
    /// A permanent delete of this many times more entries than were
    /// selected asks for the count to be typed.
    pub typed_count_ratio: u64,
    /// Places never removed, moved or trashed without the typed name: the
    /// home folder, the state and config folders (the app adds them). Drive
    /// and volume roots are always refused.
    pub protected: Vec<Place>,
    /// The deepest a walk goes before it stops and reports.
    pub max_depth: usize,
    /// Two times this close, in seconds, are the same time ("replace only
    /// if newer" skips a tie).
    pub time_tolerance_seconds: i64,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            keep_dates: true,
            warn_created_time: false,
            verify_moves: true,
            verify_copies: false,
            replace_keeps_old_in_trash: true,
            network_retries: 3,
            network_retry_wait: Duration::from_secs(5),
            network_retry_growth: 3,
            stall_after: Duration::from_secs(20),
            chunk_bytes: 1024 * 1024,
            resume_over: 4 * 1024 * 1024,
            typed_count_over: 1000,
            typed_count_ratio: 50,
            protected: Vec::new(),
            max_depth: 256,
            time_tolerance_seconds: 2,
        }
    }
}

/// The longest single wait between two automatic retries, whatever the
/// options say: ten minutes.
pub const MAX_RETRY_WAIT: Duration = Duration::from_secs(600);

impl EngineOptions {
    /// The wait before automatic retry `retry`, counted from 1: the first
    /// wait, grown `retry - 1` times, never more than [`MAX_RETRY_WAIT`].
    pub fn retry_wait(&self, retry: u32) -> Duration {
        let growth = self.network_retry_growth.max(1);
        let mut wait = self.network_retry_wait;
        for _ in 1..retry {
            wait = wait.saturating_mul(growth);
            if wait >= MAX_RETRY_WAIT {
                return MAX_RETRY_WAIT;
            }
        }
        wait.min(MAX_RETRY_WAIT)
    }

    /// Every wait of one retry budget, in order: 5, 15 and 45 seconds by
    /// default.
    pub fn retry_waits(&self) -> Vec<Duration> {
        (1..=self.network_retries)
            .map(|retry| self.retry_wait(retry))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_wait_longer_each_time_and_are_bounded() {
        let options = EngineOptions::default();
        let seconds = |waits: Vec<Duration>| -> Vec<u64> {
            waits.into_iter().map(|wait| wait.as_secs()).collect()
        };
        assert_eq!(seconds(options.retry_waits()), [5, 15, 45]);
        let equal = EngineOptions {
            network_retry_growth: 1,
            ..EngineOptions::default()
        };
        assert_eq!(seconds(equal.retry_waits()), [5, 5, 5]);
        let many = EngineOptions {
            network_retries: 12,
            ..EngineOptions::default()
        };
        let waits = many.retry_waits();
        assert_eq!(waits.len(), 12);
        assert!(waits.iter().all(|wait| *wait <= MAX_RETRY_WAIT));
        assert_eq!(waits[11], MAX_RETRY_WAIT);
        let none = EngineOptions {
            network_retries: 0,
            ..EngineOptions::default()
        };
        assert!(none.retry_waits().is_empty());
    }

    #[test]
    fn the_defaults_are_the_plans() {
        let options = EngineOptions::default();
        assert!(options.keep_dates);
        assert!(options.verify_moves);
        assert!(!options.verify_copies);
        assert_eq!(options.chunk_bytes, 1_048_576);
        assert_eq!(options.network_retries, 3);
        assert_eq!(options.stall_after, Duration::from_secs(20));
        assert_eq!(options.max_depth, 256);
    }
}
