//! Time: timestamps, the civil calendar, the [`Clock`] trait, and times in
//! words ("today, 2:05 PM", "Tuesday", "October 3, 2025").
//!
//! The core never reads the time itself: it asks a [`Clock`], so tests fix
//! the time and the app supplies the local offset.

use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const SECONDS_PER_DAY: i64 = 86_400;
const NANOS_PER_SECOND: u32 = 1_000_000_000;

/// A point in time: seconds and nanoseconds since the Unix epoch, in UTC.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Timestamp {
    secs: i64,
    nanos: u32,
}

impl Timestamp {
    /// January 1, 1970, at midnight UTC.
    pub const UNIX_EPOCH: Timestamp = Timestamp { secs: 0, nanos: 0 };

    /// A timestamp from seconds and nanoseconds since the epoch. Nanoseconds
    /// of a second or more carry into the seconds.
    pub const fn from_unix(secs: i64, nanos: u32) -> Self {
        let carry = (nanos / NANOS_PER_SECOND) as i64;
        Self {
            secs: secs.saturating_add(carry),
            nanos: nanos % NANOS_PER_SECOND,
        }
    }

    /// A timestamp from a [`SystemTime`], as providers read them.
    pub fn from_system_time(time: SystemTime) -> Self {
        match time.duration_since(UNIX_EPOCH) {
            Ok(after) => Self::from_duration_after_epoch(after),
            Err(before) => {
                let before = before.duration();
                let secs = i64::try_from(before.as_secs()).unwrap_or(i64::MAX);
                if before.subsec_nanos() == 0 {
                    Self::from_unix(-secs, 0)
                } else {
                    Self::from_unix(-secs - 1, NANOS_PER_SECOND - before.subsec_nanos())
                }
            }
        }
    }

    fn from_duration_after_epoch(after: Duration) -> Self {
        let secs = i64::try_from(after.as_secs()).unwrap_or(i64::MAX);
        Self::from_unix(secs, after.subsec_nanos())
    }

    /// The timestamp of a civil date and time at a given offset from UTC.
    /// Returns `None` for a month or day that does not exist.
    pub fn from_civil(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        offset: UtcOffset,
    ) -> Option<Self> {
        if !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let days = days_from_civil(i64::from(year), u32::from(month), u32::from(day));
        let local = days * SECONDS_PER_DAY
            + i64::from(hour) * 3600
            + i64::from(minute) * 60
            + i64::from(second);
        Some(Self::from_unix(local - i64::from(offset.seconds()), 0))
    }

    /// Whole seconds since the epoch.
    pub const fn unix_seconds(self) -> i64 {
        self.secs
    }

    /// The nanoseconds within the second.
    pub const fn subsec_nanos(self) -> u32 {
        self.nanos
    }

    /// This timestamp moved by a number of seconds, saturating at the ends.
    pub const fn plus_seconds(self, seconds: i64) -> Self {
        Self {
            secs: self.secs.saturating_add(seconds),
            nanos: self.nanos,
        }
    }

    /// The civil date and time at the given offset.
    pub fn to_civil(self, offset: UtcOffset) -> CivilTime {
        let local = self.secs.saturating_add(i64::from(offset.seconds()));
        let days = local.div_euclid(SECONDS_PER_DAY);
        let in_day = local.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        CivilTime {
            year: i32::try_from(year).unwrap_or(if year < 0 { i32::MIN } else { i32::MAX }),
            month: month as u8,
            day: day as u8,
            hour: (in_day / 3600) as u8,
            minute: ((in_day % 3600) / 60) as u8,
            second: (in_day % 60) as u8,
            weekday: Weekday::from_days_since_epoch(days),
        }
    }
}

/// An offset from UTC in seconds, east positive. The app reads the local
/// offset from the system; the core only carries it.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UtcOffset(i32);

impl UtcOffset {
    /// Coordinated Universal Time.
    pub const UTC: UtcOffset = UtcOffset(0);

    /// An offset of a number of seconds east of UTC, clamped to a day.
    pub const fn from_seconds(seconds: i32) -> Self {
        let limit = 86_399;
        if seconds > limit {
            Self(limit)
        } else if seconds < -limit {
            Self(-limit)
        } else {
            Self(seconds)
        }
    }

    /// An offset of whole hours east of UTC (negative for west).
    pub const fn from_hours(hours: i8) -> Self {
        Self::from_seconds(hours as i32 * 3600)
    }

    /// The offset in seconds east of UTC.
    pub const fn seconds(self) -> i32 {
        self.0
    }
}

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Weekday {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl Weekday {
    /// The weekday of a day counted from January 1, 1970, a Thursday.
    pub fn from_days_since_epoch(days: i64) -> Self {
        match days.rem_euclid(7) {
            0 => Weekday::Thursday,
            1 => Weekday::Friday,
            2 => Weekday::Saturday,
            3 => Weekday::Sunday,
            4 => Weekday::Monday,
            5 => Weekday::Tuesday,
            _ => Weekday::Wednesday,
        }
    }

    /// The weekday's name in US English.
    pub const fn name(self) -> &'static str {
        match self {
            Weekday::Monday => "Monday",
            Weekday::Tuesday => "Tuesday",
            Weekday::Wednesday => "Wednesday",
            Weekday::Thursday => "Thursday",
            Weekday::Friday => "Friday",
            Weekday::Saturday => "Saturday",
            Weekday::Sunday => "Sunday",
        }
    }
}

impl fmt::Display for Weekday {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A date and time on the civil calendar, at some offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CivilTime {
    /// The year, such as 2026.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month, 1 to 31.
    pub day: u8,
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute, 0 to 59.
    pub minute: u8,
    /// The second, 0 to 59.
    pub second: u8,
    /// The day of the week.
    pub weekday: Weekday,
}

impl CivilTime {
    /// The date in full, as the owner's rules want dates written:
    /// "Saturday, October 10, 2026".
    pub fn long_date(&self) -> String {
        format!(
            "{}, {} {}, {}",
            self.weekday.name(),
            month_name(self.month),
            self.day,
            self.year
        )
    }

    /// The time on a 12-hour clock, as US English says it: "2:05 PM".
    pub fn clock_time(&self) -> String {
        let (hour, half) = match self.hour {
            0 => (12, "AM"),
            1..=11 => (self.hour, "AM"),
            12 => (12, "PM"),
            _ => (self.hour - 12, "PM"),
        };
        format!("{hour}:{:02} {half}", self.minute)
    }

    fn same_date(&self, other: &CivilTime) -> bool {
        self.year == other.year && self.month == other.month && self.day == other.day
    }

    fn day_number(&self) -> i64 {
        days_from_civil(
            i64::from(self.year),
            u32::from(self.month),
            u32::from(self.day),
        )
    }
}

/// The name of a month, 1 to 12, in US English. Out of range gives "month".
pub const fn month_name(month: u8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "month",
    }
}

/// Where the time comes from. The app gives the core a system clock with the
/// local offset; tests give it a [`FixedClock`].
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current time.
    fn now(&self) -> Timestamp;

    /// The offset of local time from UTC.
    fn offset(&self) -> UtcOffset;
}

/// The system's clock, at an offset the app supplies.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock {
    offset: UtcOffset,
}

impl SystemClock {
    /// The system clock read at the given offset from UTC.
    pub const fn new(offset: UtcOffset) -> Self {
        Self { offset }
    }

    /// The system clock read in UTC.
    pub const fn utc() -> Self {
        Self::new(UtcOffset::UTC)
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_system_time(SystemTime::now())
    }

    fn offset(&self) -> UtcOffset {
        self.offset
    }
}

/// A clock that says what it is told, for tests.
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<Timestamp>,
    offset: UtcOffset,
}

impl FixedClock {
    /// A clock stopped at `now`, at the given offset.
    pub fn new(now: Timestamp, offset: UtcOffset) -> Self {
        Self {
            now: Mutex::new(now),
            offset,
        }
    }

    /// Sets the time.
    pub fn set(&self, now: Timestamp) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = now;
    }

    /// Moves the time forward by a number of seconds.
    pub fn advance(&self, seconds: i64) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.plus_seconds(seconds);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn offset(&self) -> UtcOffset {
        self.offset
    }
}

/// A time in words, relative to the clock's today, short enough for a row:
/// "today, 2:05 PM", "yesterday, 9:30 AM", "Tuesday" (within the last week),
/// "October 3" (this year), or "October 3, 2025" (another year, or later
/// than today).
pub fn describe_time(time: Timestamp, clock: &dyn Clock) -> String {
    let offset = clock.offset();
    let then = time.to_civil(offset);
    let today = clock.now().to_civil(offset);
    if then.same_date(&today) {
        return format!("today, {}", then.clock_time());
    }
    let days_ago = today.day_number() - then.day_number();
    match days_ago {
        1 => format!("yesterday, {}", then.clock_time()),
        2..=6 => then.weekday.name().to_owned(),
        _ if days_ago > 0 && then.year == today.year => {
            format!("{} {}", month_name(then.month), then.day)
        }
        _ => format!("{} {}, {}", month_name(then.month), then.day, then.year),
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian calendar).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date of a day counted from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> Timestamp {
        Timestamp::from_civil(year, month, day, hour, minute, 0, UtcOffset::UTC).unwrap()
    }

    #[test]
    fn known_dates_have_the_right_weekday() {
        assert_eq!(at(1970, 1, 1, 0, 0).unix_seconds(), 0);
        assert_eq!(at(2000, 1, 1, 0, 0).unix_seconds(), 946_684_800);
        let saturday = at(2000, 1, 1, 0, 0).to_civil(UtcOffset::UTC);
        assert_eq!(saturday.weekday, Weekday::Saturday);
        // The owner's rules record that September 24, 2026 was a Thursday.
        let thursday = at(2026, 9, 24, 12, 0).to_civil(UtcOffset::UTC);
        assert_eq!(thursday.weekday, Weekday::Thursday);
        assert_eq!(thursday.long_date(), "Thursday, September 24, 2026");
        let before_epoch = at(1969, 12, 31, 23, 0).to_civil(UtcOffset::UTC);
        assert_eq!(
            (before_epoch.year, before_epoch.month, before_epoch.day),
            (1969, 12, 31)
        );
        assert_eq!(before_epoch.weekday, Weekday::Wednesday);
    }

    #[test]
    fn civil_round_trips_over_leap_years() {
        for (y, m, d) in [(2024, 2, 29), (2100, 3, 1), (1600, 2, 29), (1999, 12, 31)] {
            let t = at(y, m, d, 6, 30).to_civil(UtcOffset::UTC);
            assert_eq!((t.year, t.month, t.day, t.hour, t.minute), (y, m, d, 6, 30));
        }
        assert!(Timestamp::from_civil(2026, 2, 29, 0, 0, 0, UtcOffset::UTC).is_none());
        assert!(Timestamp::from_civil(2026, 13, 1, 0, 0, 0, UtcOffset::UTC).is_none());
    }

    #[test]
    fn offsets_move_the_local_date() {
        let t = at(2026, 10, 10, 2, 0);
        let pacific = t.to_civil(UtcOffset::from_hours(-7));
        assert_eq!((pacific.day, pacific.hour), (9, 19));
        assert_eq!(pacific.clock_time(), "7:00 PM");
    }

    #[test]
    fn times_are_described_in_words() {
        let clock = FixedClock::new(at(2026, 10, 10, 15, 0), UtcOffset::UTC);
        assert_eq!(
            describe_time(at(2026, 10, 10, 14, 5), &clock),
            "today, 2:05 PM"
        );
        assert_eq!(
            describe_time(at(2026, 10, 9, 0, 30), &clock),
            "yesterday, 12:30 AM"
        );
        assert_eq!(describe_time(at(2026, 10, 6, 9, 0), &clock), "Tuesday");
        assert_eq!(describe_time(at(2026, 10, 3, 9, 0), &clock), "October 3");
        assert_eq!(
            describe_time(at(2025, 10, 3, 9, 0), &clock),
            "October 3, 2025"
        );
        assert_eq!(
            describe_time(at(2027, 1, 2, 9, 0), &clock),
            "January 2, 2027"
        );
        clock.advance(86_400);
        assert_eq!(
            describe_time(at(2026, 10, 10, 14, 5), &clock),
            "yesterday, 2:05 PM"
        );
    }

    #[test]
    fn system_times_before_the_epoch_convert() {
        let before = UNIX_EPOCH - Duration::from_millis(1500);
        let t = Timestamp::from_system_time(before);
        assert_eq!((t.unix_seconds(), t.subsec_nanos()), (-2, 500_000_000));
    }
}
