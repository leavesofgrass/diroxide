//! The guard every document reader runs under (research/alpha2-plan.md
//! part 4.5): a thread of its own with an 8 MiB stack, so a deeply nested
//! file has room; `catch_unwind`, so a reader that panics fails one
//! preview and not the program; and a deadline, so a file that would
//! hold a reader for minutes (carta took 157 seconds on an Org list
//! nested 3,000 deep, spike C1) is cut off with "took too long".
//!
//! A reader cannot be stopped from outside in Rust, so a reader past its
//! deadline is left to finish on its own and its answer is dropped. A
//! [`Guard`] runs at most [`Guard::most`] readers at once, those left
//! past their deadline among them; past that the next read is refused at
//! once ("an earlier document is still being read") rather than starting
//! another, so threads stay flat however many hostile files are
//! previewed. The quick view keeps one guard; clones share its count.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use super::DocError;

/// The stack a reader gets.
pub const STACK_BYTES: usize = 8 * 1024 * 1024;

/// The most readers a guard runs at once by default.
pub const MOST_RUNNING: usize = 3;

/// The deadline a guard gives a reader by default: the quick view's
/// `deadline_ms` setting's default.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// Runs readers on their own threads with a stack, a catch and a
/// deadline. See the module documentation.
#[derive(Clone, Debug)]
pub struct Guard {
    deadline: Duration,
    most: usize,
    running: Arc<AtomicUsize>,
}

impl Default for Guard {
    fn default() -> Self {
        Self::new(DEADLINE, MOST_RUNNING)
    }
}

/// Counts a reader while its thread lives.
struct Running(Arc<AtomicUsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Guard {
    /// A guard with a deadline that runs at most `most` readers at once
    /// (at least one).
    pub fn new(deadline: Duration, most: usize) -> Self {
        Self {
            deadline,
            most: most.max(1),
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The same guard, sharing its count, with another deadline.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// The deadline.
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// The most readers it runs at once.
    pub fn most(&self) -> usize {
        self.most
    }

    /// How many of its readers run now, those left past their deadline
    /// among them.
    pub fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// Runs `work` on a reader thread with an 8 MiB stack under
    /// `catch_unwind`, waiting at most the deadline for its answer:
    /// [`DocError::TookTooLong`] past it, [`DocError::Failed`] when it
    /// panicked, [`DocError::Busy`] when the guard already runs its most.
    pub fn run<T, F>(&self, work: F) -> Result<T, DocError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        if self.running.fetch_add(1, Ordering::SeqCst) >= self.most {
            self.running.fetch_sub(1, Ordering::SeqCst);
            return Err(DocError::Busy);
        }
        let count = Running(Arc::clone(&self.running));
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = thread::Builder::new()
            .name("diroxide-doc".to_owned())
            .stack_size(STACK_BYTES)
            .spawn(move || {
                let _count = count;
                let answer = catch_unwind(AssertUnwindSafe(work)).map_err(|_| ());
                let _ = tx.send(answer);
            });
        if spawned.is_err() {
            // The closure, and the count in it, were dropped.
            return Err(DocError::Failed);
        }
        match rx.recv_timeout(self.deadline) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(())) | Err(RecvTimeoutError::Disconnected) => Err(DocError::Failed),
            Err(RecvTimeoutError::Timeout) => Err(DocError::TookTooLong),
        }
    }
}
