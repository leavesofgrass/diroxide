//! Dates in the filter language: calendar dates written year first
//! (`2026-10-11`, `2026-10`, `2026`, `2026-10-11T14:30`) or spelled as the
//! read-back says them ("October 11, 2026"), the date words (today,
//! yesterday, this and last week, month and year), and durations (`3
//! days`, `2w`, `90 min`).
//!
//! Dates such as 10/11/2026 are refused, because they mean different days
//! in different countries.

use std::fmt;

use crate::clock::{Timestamp, UtcOffset, month_name};

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
