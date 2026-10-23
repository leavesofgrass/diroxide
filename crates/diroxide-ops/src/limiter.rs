//! The speed limit (R25, 2.6; N8): one limiter in the engine's chunk
//! loop, per job ([`crate::JobControl::set_speed_limit`]) and global
//! ([`crate::Engine::speed_limit`]), in plain units ("500 KB a second"),
//! off by default. The global one limits remote jobs only unless the
//! owner chooses all jobs ([`LimitScope`]).
//!
//! A limiter is a bucket that starts empty and fills at the limit: each
//! chunk takes its bytes from it, and a chunk that overdraws it waits until
//! the debt is paid, so the average over any stretch is the limit and the
//! only burst is one chunk. The wait reads the engine's clock and stops at
//! once for Cancel; a pause is honored between chunks as always. A change
//! of the limit takes effect at the next chunk.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use diroxide_core::format::human_size;

/// A speed limit, in bytes a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpeedLimit(NonZeroU64);

impl SpeedLimit {
    /// A limit of this many bytes a second; `None` for zero.
    pub fn bytes_per_second(bytes: u64) -> Option<Self> {
        NonZeroU64::new(bytes).map(Self)
    }

    /// The limit in bytes a second.
    pub fn get(self) -> u64 {
        self.0.get()
    }

    /// Reads a limit as the owner types it: a number and a unit, bytes,
    /// KB, MB or GB (decimal, as the jobs line says them), with or without
    /// "a second" or "/s": "500 KB", "1.5 MB a second", "800kb/s". `None`
    /// for anything else, zero included.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        let text = text
            .strip_suffix("a second")
            .or_else(|| text.strip_suffix("/s"))
            .or_else(|| text.strip_suffix("per second"))
            .unwrap_or(&text)
            .trim();
        let split = text
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(text.len());
        let (number, unit) = text.split_at(split);
        let number: f64 = number.parse().ok()?;
        let scale: f64 = match unit.trim() {
            "" | "b" | "byte" | "bytes" => 1.0,
            "kb" | "k" => 1e3,
            "mb" | "m" => 1e6,
            "gb" | "g" => 1e9,
            _ => return None,
        };
        let bytes = number * scale;
        if !bytes.is_finite() || bytes < 1.0 || bytes > u64::MAX as f64 {
            return None;
        }
        Self::bytes_per_second(bytes.round() as u64)
    }

    /// The limit in plain units: "500 KB a second".
    pub fn describe(self) -> String {
        format!("{} a second", human_size(self.get()))
    }

    /// The line said when the limit is set: "Speed limit: 500 KB a
    /// second" (R25's 28 cells).
    pub fn line(self) -> String {
        format!("Speed limit: {}", self.describe())
    }
}

impl fmt::Display for SpeedLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// Which jobs the global limit slows (R25 N8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LimitScope {
    /// Jobs that touch a remote place: the default.
    #[default]
    RemoteJobs,
    /// Every job, local ones too.
    AllJobs,
}

#[derive(Debug, Default)]
struct Bucket {
    limit: Option<SpeedLimit>,
    scope: LimitScope,
    /// Bytes that may pass now; below zero, the debt to wait for.
    tokens: f64,
    /// When the bucket was last filled, by the engine's clock.
    at: Option<Duration>,
}

/// A limiter: a job's own, or the global one every job shares. Off until a
/// limit is set.
#[derive(Debug, Default)]
pub struct Limiter {
    bucket: Mutex<Bucket>,
}

impl Limiter {
    /// A limiter that limits nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the limit, or turns it off with `None`; the next chunk of
    /// every job it limits follows the new one.
    pub fn set(&self, limit: Option<SpeedLimit>) {
        let mut bucket = self.lock();
        bucket.limit = limit;
        bucket.tokens = 0.0;
        bucket.at = None;
    }

    /// The limit, when one is set.
    pub fn limit(&self) -> Option<SpeedLimit> {
        self.lock().limit
    }

    /// Sets which jobs the global limit slows.
    pub fn set_scope(&self, scope: LimitScope) {
        self.lock().scope = scope;
    }

    /// Which jobs the global limit slows.
    pub fn scope(&self) -> LimitScope {
        self.lock().scope
    }

    /// Whether this limiter, as the global one, slows a job: one is set,
    /// and the job is remote or the scope is every job.
    pub(crate) fn applies(&self, remote: bool) -> bool {
        let bucket = self.lock();
        bucket.limit.is_some() && (remote || bucket.scope == LimitScope::AllJobs)
    }

    /// Takes `bytes` that passed at `now` (the engine's clock) and answers
    /// how long to wait before the next chunk: zero when there is no limit
    /// or the bucket covers them.
    pub(crate) fn take(&self, bytes: u64, now: Duration) -> Duration {
        let mut bucket = self.lock();
        let Some(limit) = bucket.limit else {
            return Duration::ZERO;
        };
        let rate = limit.get() as f64;
        let since = bucket
            .at
            .map_or(Duration::ZERO, |at| now.saturating_sub(at));
        // The bucket holds at most one second of the limit: an idle job
        // does not save up a burst.
        bucket.tokens = (bucket.tokens + since.as_secs_f64() * rate).min(rate);
        bucket.at = Some(now);
        bucket.tokens -= bytes as f64;
        if bucket.tokens >= 0.0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(-bucket.tokens / rate)
    }

    fn lock(&self) -> MutexGuard<'_, Bucket> {
        self.bucket.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn limits_read_and_say_plain_units() {
        let limit = SpeedLimit::parse("500 KB").unwrap();
        assert_eq!(limit.get(), 500_000);
        assert_eq!(limit.describe(), "500 KB a second");
        assert_eq!(limit.line(), "Speed limit: 500 KB a second");
        assert!(within_cells(&limit.line(), "Speed limit: 500 KB a second"));
        assert_eq!(
            SpeedLimit::parse("1.5 MB a second").unwrap().get(),
            1_500_000
        );
        assert_eq!(SpeedLimit::parse("800kb/s").unwrap().get(), 800_000);
        assert_eq!(SpeedLimit::parse("2 GB").unwrap().get(), 2_000_000_000);
        assert_eq!(SpeedLimit::parse("4096").unwrap().get(), 4096);
        for bad in ["", "0 KB", "fast", "5 TB", "-3 MB", "0.0001 b"] {
            assert_eq!(SpeedLimit::parse(bad), None, "{bad}");
        }
        assert_eq!(SpeedLimit::bytes_per_second(0), None);
    }

    #[test]
    fn a_bucket_pays_its_debt_and_saves_no_burst() {
        let limiter = Limiter::new();
        assert_eq!(
            limiter.take(1_000_000, Duration::ZERO),
            Duration::ZERO,
            "off"
        );
        limiter.set(SpeedLimit::bytes_per_second(1_000_000));
        // Starts empty: the first chunk waits for itself.
        assert_eq!(
            limiter.take(500_000, Duration::ZERO),
            Duration::from_millis(500)
        );
        // Half a second later the debt is paid; the next chunk waits again.
        let second = Duration::from_millis(500);
        assert_eq!(limiter.take(250_000, second), Duration::from_millis(250));
        // An idle minute saves at most one second.
        let later = Duration::from_secs(61);
        assert_eq!(limiter.take(1_000_000, later), Duration::ZERO);
        assert_eq!(limiter.take(1_000_000, later), Duration::from_secs(1));
        limiter.set(None);
        assert_eq!(limiter.take(9_000_000, later), Duration::ZERO);
    }

    #[test]
    fn the_global_limit_slows_remote_jobs_unless_all_are_chosen() {
        let global = Limiter::new();
        assert!(!global.applies(true), "off by default");
        global.set(SpeedLimit::parse("500 KB"));
        assert_eq!(global.scope(), LimitScope::RemoteJobs);
        assert!(global.applies(true));
        assert!(!global.applies(false));
        global.set_scope(LimitScope::AllJobs);
        assert!(global.applies(false));
    }
}
