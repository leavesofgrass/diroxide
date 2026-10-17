//! The arrival tracker: when a file that appeared in a watched folder has
//! settled, so it can be said ("Arrived: scan-004.pdf, 1.2 MB") and, in a
//! later alpha, acted on (research/hot-folders.md, R16; the alpha.3 plan's
//! "The arrival tracker").
//!
//! - **One record a name.** A first sighting gives a name a record: its
//!   size, modified time and identity, and when it was seen. Every later
//!   look that finds any of them different resets its quiet timer.
//! - **The settle rule.** A file is ready only when the same facts have
//!   been seen for the whole quiet period: 2 seconds in a watched local
//!   folder, 10 in a polled place ([`SettleRules`]), half a second for a
//!   name a producer renamed in. On Windows a file that a writer still
//!   holds open is never ready, however long it has been quiet (the
//!   writer probe), so a producer that pauses mid-write is not taken
//!   early; elsewhere a producer that pauses longer than the quiet period
//!   is said early and then again, with its final size ("Arrived again"),
//!   and a longer quiet period is the remedy.
//! - **The tick.** The settling files are looked at again every half
//!   second, and only while one is settling: an idle watch costs nothing.
//! - **Events wake the scanner; the listing is the truth.** A watch event
//!   only says which name to look at; a rescan lists the folder and
//!   compares. Folders, links and anything that is not a file are
//!   skipped; online-only files are skipped and said.
//! - **Ignored names** never get a record: unfinished downloads and
//!   copies, editors' swap and lock files, sync clients' temporary copies
//!   ([`is_ignored`]).
//! - **The clock is injected.** The state machine ([`Tracker`]) takes the
//!   time of each look, the steppable watch ([`ArrivalWatch`]) takes it
//!   from its caller, and only [`spawn_watch`] waits in real time, on the
//!   clock it is given, so the whole machine runs in tests against a fixed
//!   clock.
//!
//! What is said is paced by a [`Teller`]: at most one line every interval
//! (the app's progress interval in screen-reader mode), the arrival itself
//! when one waits, or the summary ("Watching Downloads: 3 arrived") when
//! several do.

mod ignore;
mod lines;
mod machine;
mod probe;
#[cfg(test)]
mod tests;
mod watch;

use std::time::Duration;

use diroxide_core::Timestamp;

pub use ignore::{IGNORED_ENDINGS, IGNORED_NAMES, IGNORED_PREFIXES, is_ignored};
pub use lines::{
    Teller, arrived_line, gave_up_line, gone_line, skipped_line, start_line, stopped_line,
    summary_line, waiting_line,
};
pub use machine::{
    Arrival, ArrivalState, Facts, Look, SettleRules, SkipReason, Tracker, TrackerEvent,
};
pub use probe::writer_open;
pub use watch::{ArrivalWatch, POLL_EVERY, WatchNote, WatchOptions, WatchRun, spawn_watch};

/// The time from `earlier` to `later`, or zero when `later` is not after
/// it.
pub fn between(earlier: Timestamp, later: Timestamp) -> Duration {
    let secs = i128::from(later.unix_seconds()) - i128::from(earlier.unix_seconds());
    let nanos = i128::from(later.subsec_nanos()) - i128::from(earlier.subsec_nanos());
    let total = secs * 1_000_000_000 + nanos;
    if total <= 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(u64::try_from(total).unwrap_or(u64::MAX))
    }
}

/// The moment `after` past `time`.
pub fn later_by(time: Timestamp, after: Duration) -> Timestamp {
    let secs = i64::try_from(after.as_secs()).unwrap_or(i64::MAX);
    let at = time.plus_seconds(secs);
    Timestamp::from_unix(
        at.unix_seconds(),
        at.subsec_nanos().saturating_add(after.subsec_nanos()),
    )
}
