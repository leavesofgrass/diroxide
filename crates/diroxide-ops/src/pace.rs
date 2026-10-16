//! Time for the engine: how long a job has run, and waiting. The estimate
//! and the waits between automatic retries read a [`Pacer`], so a test can
//! run five-second waits in no time with a [`FakePacer`]. The stall
//! detector always uses the real clock, since it watches a call that does
//! not return.

use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_vfs::CancelToken;

/// A monotonic clock that can wait.
pub trait Pacer: Send + Sync + fmt::Debug {
    /// The time since the pacer started.
    fn now(&self) -> Duration;

    /// Waits for a while.
    fn sleep(&self, duration: Duration);
}

/// The real clock.
#[derive(Clone, Copy, Debug)]
pub struct SystemPacer {
    origin: Instant,
}

impl SystemPacer {
    /// A pacer that starts now.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemPacer {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacer for SystemPacer {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// A clock for tests: it stands still until told to move, and a wait
/// moves it forward at once.
#[derive(Debug, Default)]
pub struct FakePacer {
    now: Mutex<Duration>,
    slept: Mutex<Vec<Duration>>,
}

impl FakePacer {
    /// A pacer at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the time forward.
    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now += by;
    }

    /// Every wait asked for, in order.
    pub fn waits(&self) -> Vec<Duration> {
        self.slept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Pacer for FakePacer {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sleep(&self, duration: Duration) {
        self.slept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        self.advance(duration);
    }
}

/// Waits for `duration` in short slices, and returns early, `false`, when
/// the token is cancelled.
pub(crate) fn wait(pacer: &dyn Pacer, duration: Duration, cancel: &CancelToken) -> bool {
    const SLICE: Duration = Duration::from_millis(50);
    let end = pacer.now() + duration;
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let now = pacer.now();
        if now >= end {
            return true;
        }
        pacer.sleep((end - now).min(SLICE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fake_pacer_waits_in_no_time() {
        let pacer = FakePacer::new();
        let start = Instant::now();
        assert!(wait(&pacer, Duration::from_secs(5), &CancelToken::new()));
        assert_eq!(pacer.now(), Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(1));
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(!wait(&pacer, Duration::from_secs(5), &cancelled));
    }

    #[test]
    fn the_system_pacer_moves_forward() {
        let pacer = SystemPacer::new();
        let before = pacer.now();
        pacer.sleep(Duration::from_millis(2));
        assert!(pacer.now() > before);
    }
}
