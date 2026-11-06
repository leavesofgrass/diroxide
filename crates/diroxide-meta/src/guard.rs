//! The guard every reader runs under (R12, "Safety of the readers
//! themselves"; R28 3.2): readers parse hostile files, so each runs
//!
//! - **on a thread of its own** with an 8 MiB stack, so a deeply nested
//!   file has room;
//! - **under a catch**, so a reader that panics fails one file and never
//!   the program: the cell says "unreadable: the reader failed";
//! - **with a deadline**, after which its answer is dropped ("unreadable:
//!   it took too long to read"); and
//! - **with a byte limit**, through [`Bounded`], the source every reader
//!   gets: it counts the bytes read, and past the limit, the deadline or a
//!   cancel every further read fails, so a reader left running past its
//!   deadline stops at its next read rather than reading on.
//!
//! A reader cannot be stopped from outside in Rust, so a guard runs at
//! most [`Guard::most`] readers at once, those left past their deadline
//! among them; past that the next read is refused at once ("busy with an
//! earlier file", a cell that still says "reading"), so threads stay flat
//! however many hostile files are read. Clones share the count.

use std::io::{self, Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_vfs::{CancelToken, ReadSeek, VfsError};

use crate::reader::{MetaError, ReadLimits};

/// The stack a reader gets.
pub const STACK_BYTES: usize = 8 * 1024 * 1024;

/// The most readers a guard runs at once by default: the pool of four
/// R28 3.2 gives the columns.
pub const MOST_RUNNING: usize = 4;

/// Runs readers on their own threads with a stack, a catch and a
/// deadline. See the module documentation.
#[derive(Clone, Debug)]
pub struct Guard {
    most: usize,
    running: Arc<AtomicUsize>,
}

impl Default for Guard {
    fn default() -> Self {
        Self::new(MOST_RUNNING)
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
    /// A guard that runs at most `most` readers at once (at least one).
    pub fn new(most: usize) -> Self {
        Self {
            most: most.max(1),
            running: Arc::new(AtomicUsize::new(0)),
        }
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

    /// Runs `work` on a reader thread with an 8 MiB stack under a catch,
    /// waiting at most `deadline` for its answer: [`MetaError::TooSlow`]
    /// past it, [`MetaError::ReaderFailed`] when it panicked,
    /// [`MetaError::Busy`] when the guard already runs its most.
    pub fn run<T, F>(&self, deadline: Duration, work: F) -> Result<T, MetaError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, MetaError> + Send + 'static,
    {
        if self.running.fetch_add(1, Ordering::SeqCst) >= self.most {
            self.running.fetch_sub(1, Ordering::SeqCst);
            return Err(MetaError::Busy);
        }
        let count = Running(Arc::clone(&self.running));
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = thread::Builder::new()
            .name("diroxide-meta".to_owned())
            .stack_size(STACK_BYTES)
            .spawn(move || {
                let _count = count;
                let answer = catch_unwind(AssertUnwindSafe(work));
                let _ = tx.send(answer);
            });
        if spawned.is_err() {
            // The closure, and the count in it, were dropped.
            return Err(MetaError::ReaderFailed);
        }
        match rx.recv_timeout(deadline) {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => Err(MetaError::ReaderFailed),
            Err(RecvTimeoutError::Timeout) => Err(MetaError::TooSlow),
        }
    }
}

/// Which limit a [`Bounded`] source reached first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Tripped {
    /// Nothing yet.
    No = 0,
    /// It read more than its byte limit.
    Bytes = 1,
    /// It read past its deadline.
    Deadline = 2,
    /// Its token was cancelled.
    Cancelled = 3,
}

/// Shared word of which limit a source reached, read by the pipeline
/// after its reader returns, so a reader's own error words ("failed to
/// fill whole buffer") give way to the real cause.
#[derive(Clone, Debug, Default)]
pub struct Trip(Arc<AtomicU8>);

impl Trip {
    /// Which limit was reached, if any.
    pub fn get(&self) -> Tripped {
        match self.0.load(Ordering::SeqCst) {
            1 => Tripped::Bytes,
            2 => Tripped::Deadline,
            3 => Tripped::Cancelled,
            _ => Tripped::No,
        }
    }

    fn set(&self, tripped: Tripped) {
        let _ = self
            .0
            .compare_exchange(0, tripped as u8, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// A reader's error, replaced by the limit's own error when a limit
    /// was reached.
    pub fn explain(&self, error: MetaError) -> MetaError {
        match self.get() {
            Tripped::No => error,
            Tripped::Bytes => MetaError::TooLarge,
            Tripped::Deadline => MetaError::TooSlow,
            Tripped::Cancelled => MetaError::Place(VfsError::Cancelled),
        }
    }
}

/// The source a reader reads: any seekable source held to a byte limit,
/// a deadline and a cancel token. Every read past one of them fails.
pub struct Bounded {
    inner: Box<dyn ReadSeek>,
    max_bytes: u64,
    read: u64,
    ends: Instant,
    cancel: CancelToken,
    trip: Trip,
}

impl std::fmt::Debug for Bounded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bounded")
            .field("max_bytes", &self.max_bytes)
            .field("read", &self.read)
            .finish_non_exhaustive()
    }
}

impl Bounded {
    /// `inner` held to `limits` from now, stopped by `cancel`.
    pub fn new(inner: Box<dyn ReadSeek>, limits: &ReadLimits, cancel: CancelToken) -> Self {
        Self {
            inner,
            max_bytes: limits.max_bytes,
            read: 0,
            ends: Instant::now() + limits.deadline,
            cancel,
            trip: Trip::default(),
        }
    }

    /// Bytes in memory as a source, for the fuzz targets and the tests.
    pub fn from_bytes(bytes: Vec<u8>, limits: &ReadLimits) -> Self {
        Self::new(Box::new(io::Cursor::new(bytes)), limits, CancelToken::new())
    }

    /// The bytes read so far.
    pub fn bytes_read(&self) -> u64 {
        self.read
    }

    /// The word of which limit it reached, shared.
    pub fn trip(&self) -> Trip {
        self.trip.clone()
    }

    fn check(&self) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            self.trip.set(Tripped::Cancelled);
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if Instant::now() >= self.ends {
            self.trip.set(Tripped::Deadline);
            return Err(io::Error::new(io::ErrorKind::TimedOut, "too slow"));
        }
        if self.read > self.max_bytes {
            self.trip.set(Tripped::Bytes);
            return Err(io::Error::other("too large"));
        }
        Ok(())
    }
}

impl Read for Bounded {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        // Never more than one byte past the limit, so a reader learns it
        // went over without reading far over.
        let room = self.max_bytes.saturating_sub(self.read).saturating_add(1);
        let take = buf.len().min(usize::try_from(room).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..take])?;
        self.read += n as u64;
        Ok(n)
    }
}

impl Seek for Bounded {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.check()?;
        self.inner.seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_words_and_a_slow_reader_is_dropped() {
        let guard = Guard::new(2);
        let failed: Result<(), MetaError> = guard.run(Duration::from_secs(2), || panic!("boom"));
        assert_eq!(failed, Err(MetaError::ReaderFailed));
        assert_eq!(MetaError::ReaderFailed.to_string(), "the reader failed",);
        let slow: Result<(), MetaError> = guard.run(Duration::from_millis(20), || {
            thread::sleep(Duration::from_millis(300));
            Ok(())
        });
        assert_eq!(slow, Err(MetaError::TooSlow));
        let fine = guard.run(Duration::from_secs(2), || Ok(7));
        assert_eq!(fine, Ok(7));
    }

    #[test]
    fn a_full_guard_refuses_at_once() {
        let guard = Guard::new(1);
        let (hold, wait) = mpsc::channel::<()>();
        let busy = guard.clone();
        let first = thread::spawn(move || {
            busy.run(Duration::from_secs(5), move || {
                let _ = wait.recv_timeout(Duration::from_secs(5));
                Ok(())
            })
        });
        // Wait until the first reader is counted.
        let start = Instant::now();
        while guard.running() == 0 && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            guard.run(Duration::from_secs(1), || Ok(())),
            Err(MetaError::Busy)
        );
        hold.send(()).unwrap();
        assert_eq!(first.join().unwrap(), Ok(()));
    }

    #[test]
    fn the_source_stops_at_its_limits() {
        let limits = ReadLimits {
            max_bytes: 10,
            deadline: Duration::from_secs(5),
        };
        let mut source = Bounded::from_bytes(vec![7; 100], &limits);
        let trip = source.trip();
        let mut all = Vec::new();
        assert!(source.read_to_end(&mut all).is_err());
        assert_eq!(all.len(), 11);
        assert_eq!(trip.get(), Tripped::Bytes);
        assert_eq!(
            trip.explain(MetaError::Unreadable("short".to_owned())),
            MetaError::TooLarge
        );

        let cancel = CancelToken::new();
        let mut source = Bounded::new(
            Box::new(io::Cursor::new(vec![1u8; 10])),
            &ReadLimits::default(),
            cancel.clone(),
        );
        cancel.cancel();
        let mut one = [0u8; 1];
        assert!(source.read(&mut one).is_err());
        assert_eq!(source.trip().get(), Tripped::Cancelled);

        let mut source = Bounded::from_bytes(
            vec![1u8; 10],
            &ReadLimits {
                max_bytes: 100,
                deadline: Duration::ZERO,
            },
        );
        assert!(source.read(&mut one).is_err());
        assert_eq!(source.trip().get(), Tripped::Deadline);
        assert_eq!(
            source.trip().explain(MetaError::TooLarge),
            MetaError::TooSlow
        );
    }
}
