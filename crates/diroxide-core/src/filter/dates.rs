//! Dates in the filter language: calendar dates written year first
//! (`2026-10-11`, `2026-10`, `2026`, `2026-10-11T14:30`) or spelled as the
//! read-back says them ("October 11, 2026"), the date words (today,
//! yesterday, this and last week, month and year), and durations (`3
//! days`, `2w`, `90 min`).
//!
//! Dates such as 10/11/2026 are refused, because they mean different days
//! in different countries.
//!
//! Every date stands for a [`Period`] of local time, computed on the
//! core's clock with the clock's offset from UTC ([`DateContext`]). The
//! offset has no daylight-saving rules, so a file changed in the hour a
//! clock changed may land on the wrong side of midnight (a known limit).
//! The first day of the week is a setting; the read-back spells the real
//! days, so a listener hears which week was meant.

use std::fmt;

use crate::clock::{Clock, Timestamp, UtcOffset, Weekday, month_name};

const SECONDS_PER_DAY: i64 = 86_400;

/// A date word, evaluated against the clock each time the filter runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateWord {
    /// Since local midnight, until the next.
    Today,
    /// The day before today.
    Yesterday,
    /// The week holding today, from the first day of the week.
    ThisWeek,
    /// The week before this one.
    LastWeek,
    /// The calendar month holding today.
    ThisMonth,
    /// The month before this one.
    LastMonth,
    /// The calendar year holding today.
    ThisYear,
    /// The year before this one.
    LastYear,
}

impl DateWord {
    /// Every date word, in the order completion offers them.
    pub const ALL: [DateWord; 8] = [
        DateWord::Today,
        DateWord::Yesterday,
        DateWord::ThisWeek,
        DateWord::LastWeek,
        DateWord::ThisMonth,
        DateWord::LastMonth,
        DateWord::ThisYear,
        DateWord::LastYear,
    ];

    /// The words, as typed and said: "today", "last week".
    pub const fn words(self) -> &'static str {
        match self {
            DateWord::Today => "today",
            DateWord::Yesterday => "yesterday",
            DateWord::ThisWeek => "this week",
            DateWord::LastWeek => "last week",
            DateWord::ThisMonth => "this month",
            DateWord::LastMonth => "last month",
            DateWord::ThisYear => "this year",
            DateWord::LastYear => "last year",
        }
    }

    /// The date word one word names: today or yesterday.
    pub fn from_one(word: &str) -> Option<DateWord> {
        match word.to_ascii_lowercase().as_str() {
            "today" => Some(DateWord::Today),
            "yesterday" => Some(DateWord::Yesterday),
            _ => None,
        }
    }

    /// The date word two words name: "this week", "last year".
    pub fn from_two(first: &str, second: &str) -> Option<DateWord> {
        let second = second.to_ascii_lowercase();
        let this = match first.to_ascii_lowercase().as_str() {
            "this" => true,
            "last" => false,
            _ => return None,
        };
        Some(match (this, second.as_str()) {
            (true, "week") => DateWord::ThisWeek,
            (false, "week") => DateWord::LastWeek,
            (true, "month") => DateWord::ThisMonth,
            (false, "month") => DateWord::LastMonth,
            (true, "year") => DateWord::ThisYear,
            (false, "year") => DateWord::LastYear,
            _ => return None,
        })
    }

    /// Whether a word starts a two-word date word: "this" or "last".
    pub fn starts_two(word: &str) -> bool {
        word.eq_ignore_ascii_case("this") || word.eq_ignore_ascii_case("last")
    }
}

/// How much of a calendar date was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Precision {
    /// A year: `2026`.
    Year,
    /// A month: `2026-10`, "October 2026".
    Month,
    /// A day: `2026-10-11`, "October 11, 2026".
    Day,
    /// A minute: `2026-10-11T14:30`.
    Minute,
    /// A second: `2026-10-11T14:30:15`.
    Second,
}

/// A calendar date in local time, as precise as it was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CalendarDate {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    precision: Precision,
}

/// Why a written date was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateProblem {
    /// Not a date at all.
    NotUnderstood,
    /// Written with the year last, such as 10/11/2026.
    YearFirst,
    /// A month or day that does not exist, such as 2026-02-30.
    NoSuchDay,
}

impl CalendarDate {
    /// A date, checked: the month and day must exist, and the year must
    /// have four digits.
    pub fn new(
        year: i32,
        month: Option<u8>,
        day: Option<u8>,
        time: Option<(u8, u8, Option<u8>)>,
    ) -> Result<Self, DateProblem> {
        if !(1..=9999).contains(&year) {
            return Err(DateProblem::NotUnderstood);
        }
        let precision = match (month, day, time) {
            (None, None, None) => Precision::Year,
            (Some(_), None, None) => Precision::Month,
            (Some(_), Some(_), None) => Precision::Day,
            (Some(_), Some(_), Some((_, _, None))) => Precision::Minute,
            (Some(_), Some(_), Some((_, _, Some(_)))) => Precision::Second,
            _ => return Err(DateProblem::NotUnderstood),
        };
        let month_number = month.unwrap_or(1);
        let day_number = day.unwrap_or(1);
        let (hour, minute, second) = time.map_or((0, 0, 0), |(h, m, s)| (h, m, s.unwrap_or(0)));
        if Timestamp::from_civil(
            year,
            month_number,
            day_number,
            hour,
            minute,
            second,
            UtcOffset::UTC,
        )
        .is_none()
        {
            return Err(DateProblem::NoSuchDay);
        }
        Ok(Self {
            year,
            month: month_number,
            day: day_number,
            hour,
            minute,
            second,
            precision,
        })
    }

    /// The year.
    pub fn year(&self) -> i32 {
        self.year
    }

    /// The month, 1 to 12 (1 when only a year was written).
    pub fn month(&self) -> u8 {
        self.month
    }

    /// The day, 1 to 31 (1 when only a year or month was written).
    pub fn day(&self) -> u8 {
        self.day
    }

    /// The hour, minute and second (zero when no time was written).
    pub fn time(&self) -> (u8, u8, u8) {
        (self.hour, self.minute, self.second)
    }

    /// How much was written.
    pub fn precision(&self) -> Precision {
        self.precision
    }

    /// The date in the year-first form: "2026-10-11", "2026-10", "2026",
    /// "2026-10-11T14:30".
    pub fn iso(&self) -> String {
        match self.precision {
            Precision::Year => format!("{:04}", self.year),
            Precision::Month => format!("{:04}-{:02}", self.year, self.month),
            Precision::Day => format!("{:04}-{:02}-{:02}", self.year, self.month, self.day),
            Precision::Minute => format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}",
                self.year, self.month, self.day, self.hour, self.minute
            ),
            Precision::Second => format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                self.year, self.month, self.day, self.hour, self.minute, self.second
            ),
        }
    }

    /// The date spelled, as the read-back says it: "October 11, 2026",
    /// "October 2026", "2026", "October 11, 2026, 2:30 PM".
    pub fn spelled(&self) -> String {
        let day = format!("{} {}, {}", month_name(self.month), self.day, self.year);
        match self.precision {
            Precision::Year => self.year.to_string(),
            Precision::Month => format!("{} {}", month_name(self.month), self.year),
            Precision::Day => day,
            Precision::Minute | Precision::Second => {
                let (hour, half) = match self.hour {
                    0 => (12, "AM"),
                    1..=11 => (self.hour, "AM"),
                    12 => (12, "PM"),
                    _ => (self.hour - 12, "PM"),
                };
                if self.precision == Precision::Second {
                    format!("{day}, {hour}:{:02}:{:02} {half}", self.minute, self.second)
                } else {
                    format!("{day}, {hour}:{:02} {half}", self.minute)
                }
            }
        }
    }
}

impl fmt::Display for CalendarDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.spelled())
    }
}

/// A date the filter names: a date word or a calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateValue {
    /// A date word, such as last week.
    Word(DateWord),
    /// A calendar date.
    Date(CalendarDate),
}

/// A unit of time for a duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TimeUnit {
    /// Seconds.
    Second,
    /// Minutes.
    Minute,
    /// Hours.
    Hour,
    /// Days.
    Day,
    /// Weeks.
    Week,
    /// Calendar months.
    Month,
    /// Calendar years.
    Year,
}

impl TimeUnit {
    /// Every unit, smallest first.
    pub const ALL: [TimeUnit; 7] = [
        TimeUnit::Second,
        TimeUnit::Minute,
        TimeUnit::Hour,
        TimeUnit::Day,
        TimeUnit::Week,
        TimeUnit::Month,
        TimeUnit::Year,
    ];

    /// The unit's word for one ("day") or several ("days").
    pub const fn said(self, one: bool) -> &'static str {
        match (self, one) {
            (TimeUnit::Second, true) => "second",
            (TimeUnit::Second, false) => "seconds",
            (TimeUnit::Minute, true) => "minute",
            (TimeUnit::Minute, false) => "minutes",
            (TimeUnit::Hour, true) => "hour",
            (TimeUnit::Hour, false) => "hours",
            (TimeUnit::Day, true) => "day",
            (TimeUnit::Day, false) => "days",
            (TimeUnit::Week, true) => "week",
            (TimeUnit::Week, false) => "weeks",
            (TimeUnit::Month, true) => "month",
            (TimeUnit::Month, false) => "months",
            (TimeUnit::Year, true) => "year",
            (TimeUnit::Year, false) => "years",
        }
    }

    /// The unit a word names, ignoring case: the word, its plural, or a
    /// short form (`s`, `min`, `h`, `d`, `w`, `mo`, `y`).
    pub fn from_word(word: &str) -> Option<TimeUnit> {
        Some(match word.to_ascii_lowercase().as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => TimeUnit::Second,
            "min" | "mins" | "minute" | "minutes" => TimeUnit::Minute,
            "h" | "hr" | "hrs" | "hour" | "hours" => TimeUnit::Hour,
            "d" | "day" | "days" => TimeUnit::Day,
            "w" | "wk" | "wks" | "week" | "weeks" => TimeUnit::Week,
            "mo" | "month" | "months" => TimeUnit::Month,
            "y" | "yr" | "yrs" | "year" | "years" => TimeUnit::Year,
            _ => return None,
        })
    }
}

/// A length of time: a count and a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Duration {
    /// How many.
    pub count: u32,
    /// Of what.
    pub unit: TimeUnit,
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.count, self.unit.said(self.count == 1))
    }
}

/// The month a name gives, ignoring case: the full name or its first three
/// letters.
pub fn month_from_name(word: &str) -> Option<u8> {
    let lower = word.to_ascii_lowercase();
    (1..=12u8).find(|&m| {
        let name = month_name(m).to_ascii_lowercase();
        name == lower || (lower.len() == 3 && name.starts_with(&lower))
    })
}

fn digits(text: &str, count: usize) -> Option<u32> {
    (text.len() == count && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn small(text: &str, max: u32) -> Option<u8> {
    let n: u32 = (!text.is_empty() && text.len() <= 2 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()?;
    (n <= max).then(|| u8::try_from(n).ok()).flatten()
}

/// Reads a time of day: "14:30", "2:30", "14:30:15". The hour is 0 to 23.
pub fn parse_clock_time(text: &str) -> Option<(u8, u8, Option<u8>)> {
    let mut parts = text.split(':');
    let hour = small(parts.next()?, 23)?;
    let minute = parts.next()?;
    let minute = (minute.len() == 2).then(|| small(minute, 59)).flatten()?;
    let second = match parts.next() {
        Some(s) => Some((s.len() == 2).then(|| small(s, 59)).flatten()?),
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((hour, minute, second))
}

/// Reads a year-first date in one word. `Ok(None)` when the word does not
/// look like a date at all; an error when it looks like one but is not
/// (10/11/2026, 2026-02-30).
pub fn parse_iso(text: &str) -> Result<Option<CalendarDate>, DateProblem> {
    if text.contains('/') && text.bytes().any(|b| b.is_ascii_digit()) {
        return Err(DateProblem::YearFirst);
    }
    let (date, time) = match text.split_once(['T', 't']) {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Ok(None);
    }
    let Some(year) = digits(parts[0], 4) else {
        // A day or month first, with the year last: 11-10-2026.
        if parts.len() == 3 && digits(parts[2], 4).is_some() {
            return Err(DateProblem::YearFirst);
        }
        return Ok(None);
    };
    let year = i32::try_from(year).map_err(|_| DateProblem::NotUnderstood)?;
    let field = |i: usize, max: u32| -> Result<Option<u8>, DateProblem> {
        match parts.get(i) {
            None => Ok(None),
            Some(p) if p.len() == 2 => small(p, max).map(Some).ok_or(DateProblem::NoSuchDay),
            Some(_) => Err(DateProblem::NotUnderstood),
        }
    };
    if parts.len() > 3 {
        return Err(DateProblem::NotUnderstood);
    }
    let month = field(1, 99)?;
    let day = field(2, 99)?;
    let time = match time {
        Some(t) => Some(parse_clock_time(t).ok_or(DateProblem::NotUnderstood)?),
        None => None,
    };
    if time.is_some() && day.is_none() {
        return Err(DateProblem::NotUnderstood);
    }
    CalendarDate::new(year, month, day, time).map(Some)
}

/// Reads a duration in one word: "3d", "3days", "90min", "1y".
pub fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (count, unit) = text.split_at(split);
    Some(Duration {
        count: count.parse().ok()?,
        unit: TimeUnit::from_word(unit)?,
    })
}

/// A stretch of time: from `start`, up to but not including `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Period {
    /// The first moment in it.
    pub start: Timestamp,
    /// The first moment after it.
    pub end: Timestamp,
}

impl Period {
    /// Whether a time is in it.
    pub fn contains(&self, time: Timestamp) -> bool {
        self.start <= time && time < self.end
    }
}

/// A local day, as a count of days since January 1, 1970.
type Day = i64;

fn day_of_civil(year: i32, month: u8, day: u8) -> Day {
    Timestamp::from_civil(year, month, day, 0, 0, 0, UtcOffset::UTC)
        .map_or(0, |t| t.unix_seconds().div_euclid(SECONDS_PER_DAY))
}

fn civil_of_day(day: Day) -> (i32, u8, u8, Weekday) {
    let civil =
        Timestamp::from_unix(day.saturating_mul(SECONDS_PER_DAY), 0).to_civil(UtcOffset::UTC);
    (civil.year, civil.month, civil.day, civil.weekday)
}

/// The days of a month, from 28 to 31.
fn days_in_month(year: i32, month: u8) -> u8 {
    (28..=31u8)
        .rev()
        .find(|&d| Timestamp::from_civil(year, month, d, 0, 0, 0, UtcOffset::UTC).is_some())
        .unwrap_or(28)
}

/// A month moved by a number of months (negative for earlier).
fn shift_month(year: i32, month: u8, by: i64) -> (i32, u8) {
    let total = i64::from(year) * 12 + i64::from(month) - 1 + by;
    let year =
        i32::try_from(total.div_euclid(12)).unwrap_or(if total < 0 { i32::MIN } else { i32::MAX });
    (year, (total.rem_euclid(12) + 1) as u8)
}

/// A weekday's place in the week from Monday, 0 to 6.
fn weekday_index(day: Weekday) -> i64 {
    match day {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}

/// What date words and durations are measured against: the clock's time
/// and offset, read once per run, and the first day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateContext {
    now: Timestamp,
    offset: UtcOffset,
    week_start: Weekday,
}

impl DateContext {
    /// The clock read now.
    pub fn new(clock: &dyn Clock, week_start: Weekday) -> Self {
        Self::at(clock.now(), clock.offset(), week_start)
    }

    /// A fixed moment, for tests and for a saved search run at a time.
    pub fn at(now: Timestamp, offset: UtcOffset, week_start: Weekday) -> Self {
        Self {
            now,
            offset,
            week_start,
        }
    }

    /// The moment the context was read.
    pub fn now(&self) -> Timestamp {
        self.now
    }

    fn today(&self) -> Day {
        self.now
            .unix_seconds()
            .saturating_add(i64::from(self.offset.seconds()))
            .div_euclid(SECONDS_PER_DAY)
    }

    /// Local midnight at the start of a day.
    fn midnight(&self, day: Day) -> Timestamp {
        Timestamp::from_unix(
            day.saturating_mul(SECONDS_PER_DAY) - i64::from(self.offset.seconds()),
            0,
        )
    }

    fn days(&self, first: Day, after: Day) -> Period {
        Period {
            start: self.midnight(first),
            end: self.midnight(after),
        }
    }

    /// The first day of the week holding a day.
    fn week_of(&self, day: Day) -> Day {
        let (_, _, _, weekday) = civil_of_day(day);
        day - (weekday_index(weekday) - weekday_index(self.week_start)).rem_euclid(7)
    }

    fn month_period(&self, year: i32, month: u8) -> Period {
        let (next_year, next_month) = shift_month(year, month, 1);
        self.days(
            day_of_civil(year, month, 1),
            day_of_civil(next_year, next_month, 1),
        )
    }

    fn year_period(&self, year: i32) -> Period {
        self.days(
            day_of_civil(year, 1, 1),
            day_of_civil(year.saturating_add(1), 1, 1),
        )
    }

    /// The local days a date word covers, first and last.
    fn word_days(&self, word: DateWord) -> (Day, Day) {
        let today = self.today();
        let (year, month, _, _) = civil_of_day(today);
        match word {
            DateWord::Today => (today, today),
            DateWord::Yesterday => (today - 1, today - 1),
            DateWord::ThisWeek => {
                let start = self.week_of(today);
                (start, start + 6)
            }
            DateWord::LastWeek => {
                let start = self.week_of(today) - 7;
                (start, start + 6)
            }
            DateWord::ThisMonth => (
                day_of_civil(year, month, 1),
                day_of_civil(year, month, days_in_month(year, month)),
            ),
            DateWord::LastMonth => {
                let (y, m) = shift_month(year, month, -1);
                (
                    day_of_civil(y, m, 1),
                    day_of_civil(y, m, days_in_month(y, m)),
                )
            }
            DateWord::ThisYear => (day_of_civil(year, 1, 1), day_of_civil(year, 12, 31)),
            DateWord::LastYear => {
                let y = year.saturating_sub(1);
                (day_of_civil(y, 1, 1), day_of_civil(y, 12, 31))
            }
        }
    }

    /// The stretch of local time a date stands for: a whole day, week,
    /// month or year, or a minute or second when a time was written.
    pub fn period(&self, value: &DateValue) -> Period {
        match value {
            DateValue::Word(word) => {
                let (first, last) = self.word_days(*word);
                self.days(first, last + 1)
            }
            DateValue::Date(date) => match date.precision {
                Precision::Year => self.year_period(date.year),
                Precision::Month => self.month_period(date.year, date.month),
                Precision::Day => {
                    let day = day_of_civil(date.year, date.month, date.day);
                    self.days(day, day + 1)
                }
                Precision::Minute | Precision::Second => {
                    let start = Timestamp::from_civil(
                        date.year,
                        date.month,
                        date.day,
                        date.hour,
                        date.minute,
                        date.second,
                        self.offset,
                    )
                    .unwrap_or_default();
                    let length = if date.precision == Precision::Minute {
                        60
                    } else {
                        1
                    };
                    Period {
                        start,
                        end: start.plus_seconds(length),
                    }
                }
            },
        }
    }

    /// The moment a duration ago: fixed seconds for seconds to weeks, the
    /// same time on the calendar for months and years (a day past a
    /// month's end becomes its last day).
    pub fn ago(&self, duration: &Duration) -> Timestamp {
        let count = i64::from(duration.count);
        let seconds = match duration.unit {
            TimeUnit::Second => 1,
            TimeUnit::Minute => 60,
            TimeUnit::Hour => 3_600,
            TimeUnit::Day => SECONDS_PER_DAY,
            TimeUnit::Week => 7 * SECONDS_PER_DAY,
            TimeUnit::Month | TimeUnit::Year => {
                let months = if duration.unit == TimeUnit::Year {
                    count * 12
                } else {
                    count
                };
                let now = self.now.to_civil(self.offset);
                let (year, month) = shift_month(now.year, now.month, -months);
                let day = now.day.min(days_in_month(year, month));
                return Timestamp::from_civil(
                    year,
                    month,
                    day,
                    now.hour,
                    now.minute,
                    now.second,
                    self.offset,
                )
                .unwrap_or(Timestamp::from_unix(i64::MIN / 2, 0));
            }
        };
        self.now
            .plus_seconds(count.saturating_mul(seconds).saturating_neg())
    }

    /// The real days a date word covers, as the read-back adds them:
    /// "October 11, 2026", "October 4 to 10", "September 27 to October 3",
    /// "December 27, 2026 to January 2, 2027", "October 2026", "2026".
    pub fn spelled(&self, word: DateWord) -> String {
        let (first, last) = self.word_days(word);
        let (this_year, ..) = civil_of_day(self.today());
        let (y1, m1, d1, _) = civil_of_day(first);
        let (y2, m2, d2, _) = civil_of_day(last);
        match word {
            DateWord::Today | DateWord::Yesterday => {
                format!("{} {d1}, {y1}", month_name(m1))
            }
            DateWord::ThisMonth | DateWord::LastMonth => format!("{} {y1}", month_name(m1)),
            DateWord::ThisYear | DateWord::LastYear => y1.to_string(),
            DateWord::ThisWeek | DateWord::LastWeek => {
                if y1 != y2 {
                    format!(
                        "{} {d1}, {y1} to {} {d2}, {y2}",
                        month_name(m1),
                        month_name(m2)
                    )
                } else {
                    let range = if m1 == m2 {
                        format!("{} {d1} to {d2}", month_name(m1))
                    } else {
                        format!("{} {d1} to {} {d2}", month_name(m1), month_name(m2))
                    };
                    if y1 == this_year {
                        range
                    } else {
                        format!("{range}, {y1}")
                    }
                }
            }
        }
    }

    /// A moment in words, for the details line: "October 8, 2026, 2:09
    /// PM".
    pub fn moment(&self, time: Timestamp) -> String {
        let civil = time.to_civil(self.offset);
        format!(
            "{} {}, {}, {}",
            month_name(civil.month),
            civil.day,
            civil.year,
            civil.clock_time()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_first_dates_read_at_every_precision() {
        let day = parse_iso("2026-10-11").unwrap().unwrap();
        assert_eq!((day.year(), day.month(), day.day()), (2026, 10, 11));
        assert_eq!(day.precision(), Precision::Day);
        assert_eq!(day.spelled(), "October 11, 2026");
        assert_eq!(day.iso(), "2026-10-11");
        let month = parse_iso("2026-10").unwrap().unwrap();
        assert_eq!(month.spelled(), "October 2026");
        let year = parse_iso("2026").unwrap().unwrap();
        assert_eq!(year.spelled(), "2026");
        let minute = parse_iso("2026-10-11T14:30").unwrap().unwrap();
        assert_eq!(minute.spelled(), "October 11, 2026, 2:30 PM");
        assert_eq!(minute.iso(), "2026-10-11T14:30");
        let second = parse_iso("2026-10-11T00:05:09").unwrap().unwrap();
        assert_eq!(second.spelled(), "October 11, 2026, 12:05:09 AM");
        let leap = parse_iso("2028-02-29").unwrap().unwrap();
        assert_eq!(leap.spelled(), "February 29, 2028");
    }

    #[test]
    fn dates_that_are_not_dates_say_why() {
        assert_eq!(parse_iso("10/11/2026"), Err(DateProblem::YearFirst));
        assert_eq!(parse_iso("11-10-2026"), Err(DateProblem::YearFirst));
        assert_eq!(parse_iso("2026-02-29"), Err(DateProblem::NoSuchDay));
        assert_eq!(parse_iso("2026-13"), Err(DateProblem::NoSuchDay));
        assert_eq!(
            parse_iso("2026-10-11T25:00"),
            Err(DateProblem::NotUnderstood)
        );
        assert_eq!(parse_iso("2026-1-5"), Err(DateProblem::NotUnderstood));
        assert_eq!(parse_iso("report"), Ok(None));
        assert_eq!(parse_iso("123"), Ok(None));
        assert_eq!(parse_iso("0000"), Err(DateProblem::NotUnderstood));
    }

    fn local(date: (i32, u8, u8), hours: i8) -> Timestamp {
        let (y, m, d) = date;
        Timestamp::from_civil(y, m, d, 0, 0, 0, UtcOffset::from_hours(hours)).unwrap()
    }

    fn context(now: (i32, u8, u8, u8, u8), hours: i8, week: Weekday) -> DateContext {
        let (y, m, d, h, mi) = now;
        let offset = UtcOffset::from_hours(hours);
        DateContext::at(
            Timestamp::from_civil(y, m, d, h, mi, 0, offset).unwrap(),
            offset,
            week,
        )
    }

    /// The weekdays below were computed with Python's calendar, not
    /// recalled: October 11, 2026 is a Sunday, December 31, 2026 a
    /// Thursday, January 1, 2027 a Friday, February 29, 2028 a Tuesday.
    #[test]
    fn every_date_word_on_a_fixed_clock() {
        use DateWord as W;
        use Weekday::{Monday as Mon, Saturday as Sat, Sunday as Sun};
        type Row = (
            (i32, u8, u8, u8, u8),
            i8,
            Weekday,
            DateWord,
            (i32, u8, u8),
            (i32, u8, u8),
            &'static str,
        );
        let sunday = (2026, 10, 11, 14, 9);
        let year_end = (2026, 12, 31, 23, 30);
        let new_year = (2027, 1, 1, 0, 15);
        let leap = (2028, 2, 29, 12, 0);
        let table: [Row; 34] = [
            // The day the plan was written, Pacific daylight time.
            (
                sunday,
                -7,
                Sun,
                W::Today,
                (2026, 10, 11),
                (2026, 10, 12),
                "October 11, 2026",
            ),
            (
                sunday,
                -7,
                Sun,
                W::Yesterday,
                (2026, 10, 10),
                (2026, 10, 11),
                "October 10, 2026",
            ),
            (
                sunday,
                -7,
                Sun,
                W::ThisWeek,
                (2026, 10, 11),
                (2026, 10, 18),
                "October 11 to 17",
            ),
            (
                sunday,
                -7,
                Sun,
                W::LastWeek,
                (2026, 10, 4),
                (2026, 10, 11),
                "October 4 to 10",
            ),
            (
                sunday,
                -7,
                Sun,
                W::ThisMonth,
                (2026, 10, 1),
                (2026, 11, 1),
                "October 2026",
            ),
            (
                sunday,
                -7,
                Sun,
                W::LastMonth,
                (2026, 9, 1),
                (2026, 10, 1),
                "September 2026",
            ),
            (
                sunday,
                -7,
                Sun,
                W::ThisYear,
                (2026, 1, 1),
                (2027, 1, 1),
                "2026",
            ),
            (
                sunday,
                -7,
                Sun,
                W::LastYear,
                (2025, 1, 1),
                (2026, 1, 1),
                "2025",
            ),
            (
                sunday,
                -7,
                Mon,
                W::ThisWeek,
                (2026, 10, 5),
                (2026, 10, 12),
                "October 5 to 11",
            ),
            (
                sunday,
                -7,
                Mon,
                W::LastWeek,
                (2026, 9, 28),
                (2026, 10, 5),
                "September 28 to October 4",
            ),
            (
                sunday,
                -7,
                Sat,
                W::ThisWeek,
                (2026, 10, 10),
                (2026, 10, 17),
                "October 10 to 16",
            ),
            // A year end, Pacific standard time.
            (
                year_end,
                -8,
                Sun,
                W::Today,
                (2026, 12, 31),
                (2027, 1, 1),
                "December 31, 2026",
            ),
            (
                year_end,
                -8,
                Sun,
                W::ThisWeek,
                (2026, 12, 27),
                (2027, 1, 3),
                "December 27, 2026 to January 2, 2027",
            ),
            (
                year_end,
                -8,
                Sun,
                W::LastWeek,
                (2026, 12, 20),
                (2026, 12, 27),
                "December 20 to 26",
            ),
            (
                year_end,
                -8,
                Sun,
                W::ThisMonth,
                (2026, 12, 1),
                (2027, 1, 1),
                "December 2026",
            ),
            (
                year_end,
                -8,
                Sun,
                W::ThisYear,
                (2026, 1, 1),
                (2027, 1, 1),
                "2026",
            ),
            (
                year_end,
                -8,
                Mon,
                W::ThisWeek,
                (2026, 12, 28),
                (2027, 1, 4),
                "December 28, 2026 to January 3, 2027",
            ),
            // A quarter of an hour into the new year.
            (
                new_year,
                -8,
                Sun,
                W::Today,
                (2027, 1, 1),
                (2027, 1, 2),
                "January 1, 2027",
            ),
            (
                new_year,
                -8,
                Sun,
                W::Yesterday,
                (2026, 12, 31),
                (2027, 1, 1),
                "December 31, 2026",
            ),
            (
                new_year,
                -8,
                Sun,
                W::ThisWeek,
                (2026, 12, 27),
                (2027, 1, 3),
                "December 27, 2026 to January 2, 2027",
            ),
            (
                new_year,
                -8,
                Sun,
                W::LastWeek,
                (2026, 12, 20),
                (2026, 12, 27),
                "December 20 to 26, 2026",
            ),
            (
                new_year,
                -8,
                Sun,
                W::LastMonth,
                (2026, 12, 1),
                (2027, 1, 1),
                "December 2026",
            ),
            (
                new_year,
                -8,
                Sun,
                W::ThisYear,
                (2027, 1, 1),
                (2028, 1, 1),
                "2027",
            ),
            (
                new_year,
                -8,
                Sun,
                W::LastYear,
                (2026, 1, 1),
                (2027, 1, 1),
                "2026",
            ),
            // A leap day.
            (
                leap,
                -8,
                Sun,
                W::Today,
                (2028, 2, 29),
                (2028, 3, 1),
                "February 29, 2028",
            ),
            (
                leap,
                -8,
                Sun,
                W::Yesterday,
                (2028, 2, 28),
                (2028, 2, 29),
                "February 28, 2028",
            ),
            (
                leap,
                -8,
                Sun,
                W::ThisWeek,
                (2028, 2, 27),
                (2028, 3, 5),
                "February 27 to March 4",
            ),
            (
                leap,
                -8,
                Sun,
                W::LastWeek,
                (2028, 2, 20),
                (2028, 2, 27),
                "February 20 to 26",
            ),
            (
                leap,
                -8,
                Sun,
                W::ThisMonth,
                (2028, 2, 1),
                (2028, 3, 1),
                "February 2028",
            ),
            (
                leap,
                -8,
                Sun,
                W::LastMonth,
                (2028, 1, 1),
                (2028, 2, 1),
                "January 2028",
            ),
            (
                leap,
                -8,
                Mon,
                W::ThisWeek,
                (2028, 2, 28),
                (2028, 3, 6),
                "February 28 to March 5",
            ),
            (
                leap,
                -8,
                Mon,
                W::LastWeek,
                (2028, 2, 21),
                (2028, 2, 28),
                "February 21 to 27",
            ),
            // One moment, read at two offsets an hour apart, falls on
            // either side of midnight.
            (
                (2026, 10, 12, 0, 30),
                -7,
                Sun,
                W::Today,
                (2026, 10, 12),
                (2026, 10, 13),
                "October 12, 2026",
            ),
            (
                (2026, 10, 11, 23, 30),
                -8,
                Sun,
                W::Today,
                (2026, 10, 11),
                (2026, 10, 12),
                "October 11, 2026",
            ),
        ];
        for (now, hours, week, word, first, after, said) in table {
            let dates = context(now, hours, week);
            let period = dates.period(&DateValue::Word(word));
            let row = format!("{word:?} at {now:?} {hours}, weeks from {week}");
            assert_eq!(period.start, local(first, hours), "{row}");
            assert_eq!(period.end, local(after, hours), "{row}");
            assert_eq!(dates.spelled(word), said, "{row}");
            let this = matches!(word, W::Today | W::ThisWeek | W::ThisMonth | W::ThisYear);
            assert_eq!(period.contains(dates.now()), this, "{row}");
        }
        assert_eq!(
            context((2026, 10, 12, 0, 30), -7, Sun).now(),
            context((2026, 10, 11, 23, 30), -8, Sun).now(),
            "the two offset rows are one moment"
        );
    }

    #[test]
    fn calendar_dates_stand_for_their_whole_span() {
        let dates = context((2026, 10, 11, 14, 9), -7, Weekday::Sunday);
        let span = |text: &str| {
            let date = parse_iso(text).unwrap().unwrap();
            dates.period(&DateValue::Date(date))
        };
        assert_eq!(span("2026-10").start, local((2026, 10, 1), -7));
        assert_eq!(span("2026-10").end, local((2026, 11, 1), -7));
        assert_eq!(span("2028").end, local((2029, 1, 1), -7));
        assert_eq!(span("2028-02").end, local((2028, 3, 1), -7));
        assert_eq!(span("2026-12-31").end, local((2027, 1, 1), -7));
        let minute = span("2026-10-11T14:30");
        assert_eq!(minute.end.unix_seconds() - minute.start.unix_seconds(), 60);
        let second = span("2026-10-11T14:30:15");
        assert_eq!(second.end.unix_seconds() - second.start.unix_seconds(), 1);
    }

    #[test]
    fn durations_reach_back_on_the_clock_and_the_calendar() {
        let sunday = context((2026, 10, 11, 14, 9), -7, Weekday::Sunday);
        let ago = |dates: &DateContext, count: u32, unit: TimeUnit| {
            dates.moment(dates.ago(&Duration { count, unit }))
        };
        assert_eq!(ago(&sunday, 3, TimeUnit::Day), "October 8, 2026, 2:09 PM");
        assert_eq!(ago(&sunday, 1, TimeUnit::Week), "October 4, 2026, 2:09 PM");
        assert_eq!(
            ago(&sunday, 90, TimeUnit::Minute),
            "October 11, 2026, 12:39 PM"
        );
        assert_eq!(
            ago(&sunday, 1, TimeUnit::Month),
            "September 11, 2026, 2:09 PM"
        );
        assert_eq!(ago(&sunday, 1, TimeUnit::Year), "October 11, 2025, 2:09 PM");
        assert_eq!(
            ago(&sunday, 30, TimeUnit::Second),
            "October 11, 2026, 2:08 PM"
        );
        let leap = context((2028, 2, 29, 12, 0), -8, Weekday::Sunday);
        assert_eq!(ago(&leap, 1, TimeUnit::Year), "February 28, 2027, 12:00 PM");
        assert_eq!(ago(&leap, 4, TimeUnit::Year), "February 29, 2024, 12:00 PM");
        assert_eq!(ago(&leap, 1, TimeUnit::Month), "January 29, 2028, 12:00 PM");
        let month_end = context((2026, 3, 31, 10, 0), -7, Weekday::Sunday);
        assert_eq!(
            ago(&month_end, 1, TimeUnit::Month),
            "February 28, 2026, 10:00 AM"
        );
        let january = context((2027, 1, 15, 9, 0), -8, Weekday::Sunday);
        assert_eq!(
            ago(&january, 1, TimeUnit::Month),
            "December 15, 2026, 9:00 AM"
        );
        // Absurd durations stay in range.
        let _ = sunday.ago(&Duration {
            count: u32::MAX,
            unit: TimeUnit::Year,
        });
        let _ = sunday.ago(&Duration {
            count: u32::MAX,
            unit: TimeUnit::Week,
        });
    }

    #[test]
    fn durations_and_words() {
        assert_eq!(
            parse_duration("3d"),
            Some(Duration {
                count: 3,
                unit: TimeUnit::Day
            })
        );
        assert_eq!(parse_duration("90min").unwrap().to_string(), "90 minutes");
        assert_eq!(parse_duration("1y").unwrap().to_string(), "1 year");
        assert_eq!(parse_duration("2mo").unwrap().unit, TimeUnit::Month);
        assert_eq!(parse_duration("d"), None);
        assert_eq!(parse_duration("3x"), None);
        assert_eq!(DateWord::from_two("LAST", "Week"), Some(DateWord::LastWeek));
        assert_eq!(DateWord::from_two("next", "week"), None);
        assert_eq!(month_from_name("oct"), Some(10));
        assert_eq!(month_from_name("September"), Some(9));
        assert_eq!(month_from_name("sept"), None);
        assert_eq!(parse_clock_time("2:30"), Some((2, 30, None)));
        assert_eq!(parse_clock_time("2:3"), None);
    }
}
