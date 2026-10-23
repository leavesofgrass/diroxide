//! Timeouts, leases, the idle timer, and the crash and flood policies
//! (R3, sections 6.2 to 6.6; R22, section 5.6), as a state machine driven
//! by a clock, so tests move time by hand and never sleep.
//!
//! - **Timeouts are inactivity limits.** A request's timer restarts on
//!   progress, a log line or a partial result; it pauses while the plugin
//!   has a request of its own outstanding to the host (a dialog waiting for
//!   the owner) and while the plugin says it is waiting for the owner (up
//!   to five minutes). When it runs out the host sends `$/cancel_request`
//!   and waits two seconds; then the caller gets -32034 and the plugin is
//!   unresponsive. A second time-out within five minutes restarts it.
//! - **Leases** keep a plugin alive: a request in flight, a lister showing
//!   its places, a write session, a watch, a job it started. The idle timer
//!   starts when the last one goes; after five minutes by default the host
//!   stops the plugin politely.
//! - **Crashes:** three in 60 seconds turn the plugin off until the owner
//!   turns it on again.
//! - **Floods:** more than 20 lines that do not parse in a minute, more
//!   than 500 stray lines in a minute, or more than 200 announcements
//!   folded away in a minute stop the plugin, with one sentence.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::protocol::Id;

/// How long the host waits for the answer to a cancelled request.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// How long the owner may be waited for before a request's timer counts
/// again.
pub const WAITING_LIMIT: Duration = Duration::from_secs(300);

/// The window in which a second time-out restarts a plugin.
pub const UNRESPONSIVE_WINDOW: Duration = Duration::from_secs(300);

/// After how long a request is said to be waiting for the plugin.
pub const SLOW_NOTICE: Duration = Duration::from_secs(2);

/// A monotonic clock: time since some start. The supervisor reads it;
/// tests move a [`ManualTicker`] by hand.
pub trait Ticker: Send + Sync + fmt::Debug {
    /// The time now, since the ticker's start.
    fn now(&self) -> Duration;
}

/// The system's monotonic clock.
#[derive(Clone, Copy, Debug)]
pub struct SystemTicker {
    start: Instant,
}

impl SystemTicker {
    /// A ticker that starts now.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemTicker {
    fn default() -> Self {
        Self::new()
    }
}

impl Ticker for SystemTicker {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
}

/// A clock that moves only when told, for tests.
#[derive(Debug, Default)]
pub struct ManualTicker {
    now: Mutex<Duration>,
}

impl ManualTicker {
    /// A ticker at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the time forward.
    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }
}

impl Ticker for ManualTicker {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The inactivity timer of one request.
#[derive(Clone, Debug)]
struct Timer {
    limit: Option<Duration>,
    started: Duration,
    idle: Duration,
    last: Duration,
    waiting_since: Option<Duration>,
    cancelled_at: Option<Duration>,
    noticed: bool,
}

impl Timer {
    fn new(limit: Option<Duration>, now: Duration) -> Self {
        Self {
            limit,
            started: now,
            idle: Duration::ZERO,
            last: now,
            waiting_since: None,
            cancelled_at: None,
            noticed: false,
        }
    }

    /// Counts the time since the last look, unless paused: a host call
    /// pauses the whole step, and a wait for the owner pauses the step up
    /// to five minutes after the wait began.
    fn advance(&mut self, now: Duration, host_calls: bool) {
        let from = self.last;
        self.last = now;
        if host_calls {
            return;
        }
        let start = match self.waiting_since {
            Some(since) => from.max(since + WAITING_LIMIT),
            None => from,
        };
        self.idle += now.saturating_sub(start);
    }
}

/// What the supervisor asks its driver to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Say that a request waits for the plugin ("Count lines: waiting for
    /// the plugin"), once, two seconds after it was sent.
    Slow(Id),
    /// A request's timer ran out: send `$/cancel_request`.
    Cancel(Id),
    /// The cancelled request was not answered in two seconds: answer its
    /// caller -32034, and say it timed out.
    TimedOut(Id),
    /// A second time-out within five minutes: kill the plugin and start it
    /// again at the next use.
    Restart,
    /// The idle timer ran out: stop the plugin politely.
    IdleStop,
}

/// One plugin's timers and leases.
#[derive(Debug)]
pub struct Supervisor {
    timers: HashMap<Id, Timer>,
    leases: usize,
    idle_timeout: Option<Duration>,
    idle_since: Option<Duration>,
    last_timeout: Option<Duration>,
    unresponsive: bool,
}

impl Supervisor {
    /// A supervisor for a plugin that started now, with its idle timeout
    /// (`None`: never stop for idleness).
    pub fn new(idle_timeout: Option<Duration>, now: Duration) -> Self {
        Self {
            timers: HashMap::new(),
            leases: 0,
            idle_timeout,
            idle_since: Some(now),
            last_timeout: None,
            unresponsive: false,
        }
    }

    /// Starts timing a request with its inactivity limit (`None`: no
    /// limit, but it still holds a lease).
    pub fn track(&mut self, id: Id, limit: Option<Duration>, now: Duration) {
        self.timers.insert(id, Timer::new(limit, now));
        self.idle_since = None;
    }

    /// The request was answered or given up.
    pub fn finish(&mut self, id: &Id, now: Duration) {
        self.timers.remove(id);
        self.check_idle(now);
    }

    /// Progress, a partial result or a log line for a request: its timer
    /// restarts.
    pub fn activity(&mut self, id: &Id, now: Duration) {
        if let Some(timer) = self.timers.get_mut(id) {
            timer.idle = Duration::ZERO;
            timer.last = now;
        }
        self.unresponsive = false;
    }

    /// A log line: every request's timer restarts (a log line is not tied
    /// to a request).
    pub fn activity_all(&mut self, now: Duration) {
        let ids: Vec<Id> = self.timers.keys().cloned().collect();
        for id in ids {
            self.activity(&id, now);
        }
    }

    /// The plugin says a request waits for the owner, or no longer does.
    pub fn waiting(&mut self, id: &Id, waiting: bool, now: Duration) {
        if let Some(timer) = self.timers.get_mut(id) {
            timer.advance(now, false);
            timer.waiting_since = waiting.then_some(now);
        }
    }

    /// Takes a lease: the plugin stays alive while any is held.
    pub fn lease(&mut self) {
        self.leases += 1;
        self.idle_since = None;
    }

    /// Gives a lease back.
    pub fn release(&mut self, now: Duration) {
        self.leases = self.leases.saturating_sub(1);
        self.check_idle(now);
    }

    fn check_idle(&mut self, now: Duration) {
        if self.leases == 0 && self.timers.is_empty() && self.idle_since.is_none() {
            self.idle_since = Some(now);
        }
    }

    /// How many leases are held, counting requests in flight.
    pub fn leases(&self) -> usize {
        self.leases + self.timers.len()
    }

    /// Whether a request timed out and nothing has been heard since.
    pub fn is_unresponsive(&self) -> bool {
        self.unresponsive
    }

    /// Looks at the time: what to do now. `host_calls` is whether the
    /// plugin has a request of its own outstanding to the host, which
    /// pauses every timer.
    pub fn tick(&mut self, now: Duration, host_calls: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut ids: Vec<Id> = self.timers.keys().cloned().collect();
        ids.sort_by_key(|id| format!("{id:?}"));
        for id in ids {
            let Some(timer) = self.timers.get_mut(&id) else {
                continue;
            };
            timer.advance(now, host_calls);
            if !timer.noticed && now.saturating_sub(timer.started) >= SLOW_NOTICE {
                timer.noticed = true;
                actions.push(Action::Slow(id.clone()));
            }
            match (timer.limit, timer.cancelled_at) {
                (Some(limit), None) if timer.idle >= limit => {
                    timer.cancelled_at = Some(now);
                    actions.push(Action::Cancel(id.clone()));
                }
                (_, Some(at)) if now.saturating_sub(at) >= CANCEL_GRACE => {
                    self.timers.remove(&id);
                    actions.push(Action::TimedOut(id.clone()));
                    let again = self
                        .last_timeout
                        .is_some_and(|last| now.saturating_sub(last) < UNRESPONSIVE_WINDOW);
                    self.last_timeout = Some(now);
                    self.unresponsive = true;
                    if again {
                        actions.push(Action::Restart);
                    }
                }
                _ => {}
            }
        }
        self.check_idle(now);
        if let (Some(since), Some(limit)) = (self.idle_since, self.idle_timeout)
            && now.saturating_sub(since) >= limit
        {
            self.idle_since = None;
            actions.push(Action::IdleStop);
        }
        actions
    }
}

/// What a crash means for a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashVerdict {
    /// It starts again at the next use.
    Restartable,
    /// Three crashes in 60 seconds: it is off until the owner turns it on.
    Disabled,
}

/// Counts crashes in a window.
#[derive(Clone, Debug)]
pub struct CrashPolicy {
    window: Duration,
    limit: usize,
    times: VecDeque<Duration>,
}

impl Default for CrashPolicy {
    fn default() -> Self {
        Self::new(Duration::from_secs(60), 3)
    }
}

impl CrashPolicy {
    /// A policy of `limit` crashes in `window`.
    pub fn new(window: Duration, limit: usize) -> Self {
        Self {
            window,
            limit: limit.max(1),
            times: VecDeque::new(),
        }
    }

    /// A policy for failures to start: two in a minute.
    pub fn for_starts() -> Self {
        Self::new(Duration::from_secs(60), 2)
    }

    /// Records a crash now.
    pub fn record(&mut self, now: Duration) -> CrashVerdict {
        self.times.push_back(now);
        while self
            .times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= self.window)
        {
            self.times.pop_front();
        }
        if self.times.len() >= self.limit {
            CrashVerdict::Disabled
        } else {
            CrashVerdict::Restartable
        }
    }

    /// How many crashes are in the window now.
    pub fn count(&self) -> usize {
        self.times.len()
    }

    /// Forgets the crashes (the owner turned the plugin on again).
    pub fn reset(&mut self) {
        self.times.clear();
    }
}

/// What a plugin floods with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flood {
    /// Lines that do not parse.
    ParseFailures,
    /// Output lines that are not messages.
    Stray,
    /// Announcements folded away by the rate limit.
    Folded,
}

impl Flood {
    /// How many in a minute stop the plugin.
    pub const fn limit(self) -> usize {
        match self {
            Flood::ParseFailures => 20,
            Flood::Stray => 500,
            Flood::Folded => 200,
        }
    }
}

/// Counts what a plugin floods with, per kind, in a minute.
#[derive(Clone, Debug, Default)]
pub struct FloodGuard {
    seen: HashMap<Flood, VecDeque<Duration>>,
}

impl FloodGuard {
    /// Records one; true when the plugin must be stopped.
    pub fn record(&mut self, kind: Flood, now: Duration) -> bool {
        let times = self.seen.entry(kind).or_default();
        times.push_back(now);
        while times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= Duration::from_secs(60))
        {
            times.pop_front();
        }
        times.len() > kind.limit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn at(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn a_silent_request_is_cancelled_then_timed_out() {
        let mut sup = Supervisor::new(Some(at(300)), at(0));
        let id = Id::Number(1);
        sup.track(id.clone(), Some(at(30)), at(0));
        assert_eq!(sup.tick(at(1), false), []);
        assert_eq!(sup.tick(at(2), false), [Action::Slow(id.clone())]);
        assert_eq!(sup.tick(at(29), false), []);
        assert_eq!(sup.tick(at(30), false), [Action::Cancel(id.clone())]);
        assert_eq!(sup.tick(at(31), false), []);
        assert_eq!(sup.tick(at(32), false), [Action::TimedOut(id.clone())]);
        assert!(sup.is_unresponsive());
    }

    #[test]
    fn progress_restarts_the_timer() {
        let mut sup = Supervisor::new(None, at(0));
        let id = Id::Number(1);
        sup.track(id.clone(), Some(at(30)), at(0));
        for t in (10..=120).step_by(10) {
            sup.activity(&id, at(t));
            let actions = sup.tick(at(t), false);
            assert!(!actions.contains(&Action::Cancel(id.clone())), "{t}");
        }
        assert_eq!(sup.tick(at(149), false), []);
        assert_eq!(sup.tick(at(150), false), [Action::Cancel(id)]);
    }

    #[test]
    fn host_calls_pause_the_timer() {
        let mut sup = Supervisor::new(None, at(0));
        let id = Id::Number(1);
        sup.track(id.clone(), Some(at(30)), at(0));
        sup.tick(at(20), false);
        // The owner takes ten minutes to answer the plugin's question.
        assert!(
            !sup.tick(at(620), true)
                .contains(&Action::Cancel(id.clone()))
        );
        assert_eq!(sup.tick(at(629), false), []);
        assert_eq!(sup.tick(at(630), false), [Action::Cancel(id)]);
    }

    #[test]
    fn waiting_for_the_owner_pauses_up_to_five_minutes() {
        let mut sup = Supervisor::new(None, at(0));
        let id = Id::Number(1);
        sup.track(id.clone(), Some(at(30)), at(0));
        sup.tick(at(10), false);
        sup.waiting(&id, true, at(10));
        assert!(
            !sup.tick(at(300), false)
                .contains(&Action::Cancel(id.clone()))
        );
        // Five minutes after the waiting began the timer counts again,
        // from the ten seconds it had.
        assert_eq!(sup.tick(at(310), false), []);
        assert_eq!(sup.tick(at(329), false), []);
        assert_eq!(sup.tick(at(330), false), [Action::Cancel(id)]);
    }

    #[test]
    fn a_second_time_out_within_five_minutes_restarts_the_plugin() {
        let mut sup = Supervisor::new(None, at(0));
        sup.track(Id::Number(1), Some(at(5)), at(0));
        sup.tick(at(5), false);
        assert_eq!(sup.tick(at(7), false), [Action::TimedOut(Id::Number(1))]);
        sup.track(Id::Number(2), Some(at(5)), at(100));
        sup.tick(at(105), false);
        assert_eq!(
            sup.tick(at(107), false),
            [Action::TimedOut(Id::Number(2)), Action::Restart]
        );
        // A third, more than five minutes later, does not.
        sup.track(Id::Number(3), Some(at(5)), at(500));
        sup.tick(at(505), false);
        assert_eq!(sup.tick(at(507), false), [Action::TimedOut(Id::Number(3))]);
    }

    #[test]
    fn the_idle_stop_comes_five_minutes_after_the_last_lease() {
        let mut sup = Supervisor::new(Some(at(300)), at(0));
        sup.lease();
        sup.track(Id::Number(1), None, at(0));
        assert_eq!(sup.leases(), 2);
        assert!(!sup.tick(at(1000), false).contains(&Action::IdleStop));
        sup.finish(&Id::Number(1), at(1000));
        assert!(!sup.tick(at(2000), false).contains(&Action::IdleStop));
        sup.release(at(2000));
        assert_eq!(sup.tick(at(2299), false), []);
        assert_eq!(sup.tick(at(2300), false), [Action::IdleStop]);
        assert_eq!(sup.tick(at(9000), false), []);
    }

    #[test]
    fn never_idle_means_never() {
        let mut sup = Supervisor::new(None, at(0));
        assert_eq!(sup.tick(at(100_000), false), []);
    }

    #[test]
    fn three_crashes_in_a_minute_turn_a_plugin_off() {
        let mut policy = CrashPolicy::default();
        assert_eq!(policy.record(at(0)), CrashVerdict::Restartable);
        assert_eq!(policy.record(at(30)), CrashVerdict::Restartable);
        assert_eq!(policy.record(at(59)), CrashVerdict::Disabled);
        let mut spread = CrashPolicy::default();
        spread.record(at(0));
        spread.record(at(40));
        assert_eq!(spread.record(at(70)), CrashVerdict::Restartable);
        assert_eq!(spread.count(), 2);
        let mut starts = CrashPolicy::for_starts();
        assert_eq!(starts.record(at(0)), CrashVerdict::Restartable);
        assert_eq!(starts.record(at(10)), CrashVerdict::Disabled);
    }

    #[test]
    fn a_flood_stops_a_plugin_past_its_limit() {
        let mut guard = FloodGuard::default();
        for i in 0..20 {
            assert!(!guard.record(Flood::ParseFailures, at(i)));
        }
        assert!(guard.record(Flood::ParseFailures, at(20)));
        let mut slow = FloodGuard::default();
        for i in 0..100 {
            assert!(!slow.record(Flood::ParseFailures, at(i * 4)), "{i}");
        }
    }

    #[test]
    fn the_manual_ticker_moves_only_when_told() {
        let ticker = ManualTicker::new();
        assert_eq!(ticker.now(), Duration::ZERO);
        ticker.advance(S * 3);
        assert_eq!(ticker.now(), at(3));
        let system = SystemTicker::new();
        assert!(system.now() < at(60));
    }
}
