//! What a job keeps, checks and retries: the engine's options. The app
//! fills them from the settings (`keep_dates`, `verify_moves`,
//! `verify_copies`, `replace_keeps_old_in_trash`, `network_retry_count`,
//! `network_retry_seconds`, `stall_seconds`); the defaults are the plan's
//! (section 5.4 and research/file-operations.md).

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
    /// How many times a lost network is retried before the user is asked.
    pub network_retries: u32,
    /// How long to wait between those retries.
    pub network_retry_wait: Duration,
    /// How long without a byte or a step before the job says it stalled.
    /// Zero turns the stall detector off.
    pub stall_after: Duration,
    /// The copy loop's chunk: pause, cancel and progress are honored
    /// between chunks.
    pub chunk_bytes: usize,
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
            stall_after: Duration::from_secs(20),
            chunk_bytes: 1024 * 1024,
            typed_count_over: 1000,
            typed_count_ratio: 50,
            protected: Vec::new(),
            max_depth: 256,
            time_tolerance_seconds: 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
