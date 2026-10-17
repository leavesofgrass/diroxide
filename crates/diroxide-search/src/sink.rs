//! Where hits go, and how a running search is stopped or held: the
//! [`SearchSink`] gets each hit in arrival order on the thread that called
//! the search, and the [`SearchControl`] cancels, pauses and resumes it
//! from any thread.

use std::fmt;
use std::ops::ControlFlow;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use diroxide_core::format::grouped;
use diroxide_vfs::{CancelToken, OpContext};

use crate::summary::counted;

/// Where a search's hits go. Hits arrive in the order they are found and
/// never move; the sink is called on the thread that runs the search.
pub trait SearchSink {
    /// A hit. `Break` stops the search, as a cancel does.
    fn found(&mut self, hit: crate::SearchHit) -> ControlFlow<()>;

    /// How far the search has got, at most ten times a second. The app
    /// says it at the progress interval, never on every call.
    fn progress(&mut self, progress: &SearchProgress) {
        let _ = progress;
    }
}

/// A sink made of a closure that takes every hit.
pub struct FnSink<F>(pub F);

impl<F: FnMut(crate::SearchHit)> SearchSink for FnSink<F> {
    fn found(&mut self, hit: crate::SearchHit) -> ControlFlow<()> {
        (self.0)(hit);
        ControlFlow::Continue(())
    }
}

/// A pause shared by the search's threads.
#[derive(Clone, Debug, Default)]
struct Pause(Arc<(Mutex<bool>, Condvar)>);

impl Pause {
    fn set(&self, paused: bool) {
        let (lock, wake) = &*self.0;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = paused;
        wake.notify_all();
    }

    fn is_set(&self) -> bool {
        *self.0.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait(&self, cancel: &CancelToken) {
        let (lock, wake) = &*self.0;
        let mut paused = lock.lock().unwrap_or_else(PoisonError::into_inner);
        while *paused && !cancel.is_cancelled() {
            paused = wake
                .wait_timeout(paused, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Stops, holds and resumes a running search from any thread. Clones
/// share one search.
#[derive(Clone, Default)]
pub struct SearchControl {
    cancel: CancelToken,
    pause: Pause,
}

impl fmt::Debug for SearchControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchControl")
            .field("cancelled", &self.is_cancelled())
            .field("paused", &self.is_paused())
            .finish()
    }
}

impl SearchControl {
    /// A control for a new search.
    pub fn new() -> Self {
        Self::default()
    }

    /// A control that answers to an existing cancel token, such as a
    /// job's.
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Stops the search: Escape. Threads stop at their next entry; a read
    /// blocked on a dead share is left behind and said in the summary.
    pub fn cancel(&self) {
        self.cancel.cancel();
        self.pause.set(false);
    }

    /// Whether the search was stopped.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Holds the search's threads at their next entry.
    pub fn pause(&self) {
        self.pause.set(true);
    }

    /// Lets a held search go on.
    pub fn resume(&self) {
        self.pause.set(false);
    }

    /// Whether the search is held.
    pub fn is_paused(&self) -> bool {
        self.pause.is_set()
    }

    /// The cancel token the providers are given.
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// The context for provider calls: this search's cancel token.
    pub fn op_context(&self) -> OpContext {
        OpContext::new().with_cancel(self.cancel.clone())
    }

    /// Waits while the search is held; returns `true` when it was
    /// cancelled.
    pub(crate) fn checkpoint(&self) -> bool {
        self.pause.wait(&self.cancel);
        self.cancel.is_cancelled()
    }
}

/// How far a search has got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchProgress {
    /// Hits so far.
    pub found: u64,
    /// Folders read so far.
    pub folders: u64,
    /// Entries looked at so far.
    pub entries: u64,
    /// Files whose contents were read.
    pub files_read: u64,
    /// Bytes of contents read.
    pub bytes_read: u64,
    /// Milliseconds since the search started.
    pub elapsed_ms: u64,
    /// Whether the search is held.
    pub paused: bool,
    /// Whether the search is over a network, where the rate is said.
    pub network: bool,
}

impl SearchProgress {
    /// Entries a second so far.
    pub fn rate(&self) -> u64 {
        self.entries
            .saturating_mul(1000)
            .checked_div(self.elapsed_ms)
            .unwrap_or(0)
    }

    /// The status line, meaning first: "Searching: 214 found, 3,120
    /// folders", "Paused at 214 found.", or over a network "Searching
    /// over the network: 400 items a second".
    pub fn line(&self) -> String {
        if self.paused {
            format!("Paused at {} found.", grouped(self.found))
        } else if self.network {
            format!(
                "Searching over the network: {} items a second",
                grouped(self.rate())
            )
        } else {
            format!(
                "Searching: {} found, {}",
                grouped(self.found),
                counted(self.folders, "folder", "folders")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn progress_lines_fit_and_say_the_count_first() {
        let progress = SearchProgress {
            found: 214,
            folders: 3_120,
            ..SearchProgress::default()
        };
        assert_eq!(progress.line(), "Searching: 214 found, 3,120 folders");
        let paused = SearchProgress {
            paused: true,
            ..progress.clone()
        };
        assert_eq!(paused.line(), "Paused at 214 found.");
        let network = SearchProgress {
            entries: 4_000,
            elapsed_ms: 10_000,
            network: true,
            ..progress
        };
        assert_eq!(
            network.line(),
            "Searching over the network: 400 items a second"
        );
        for line in [paused.line(), network.line()] {
            assert!(line.chars().count() <= 47, "{line}");
        }
    }

    #[test]
    fn a_cancel_wakes_a_paused_thread() {
        let control = SearchControl::new();
        control.pause();
        assert!(control.is_paused());
        let woke = Arc::new(AtomicBool::new(false));
        let thread = {
            let control = control.clone();
            let woke = Arc::clone(&woke);
            std::thread::spawn(move || {
                let cancelled = control.checkpoint();
                woke.store(true, Ordering::SeqCst);
                cancelled
            })
        };
        std::thread::sleep(Duration::from_millis(60));
        assert!(!woke.load(Ordering::SeqCst));
        control.cancel();
        assert!(thread.join().unwrap());
        assert!(!control.is_paused());
        let resumed = SearchControl::new();
        resumed.pause();
        resumed.resume();
        assert!(!resumed.checkpoint());
    }
}
