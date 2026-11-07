//! Who may do what, the times and the owner, in the app (alpha 4, lane
//! AP, R30 commits 74 to 79; R29 4.2 and 4.3). The rows and the words are
//! AX's, in `diroxide-ops`'s access module; this holds what the app reads
//! on a worker for them and the words of its own two dialogs.
//!
//! - **The access rows of the details view** are read for the focused
//!   entry only, on the details' worker under its cancel token, through
//!   the provider's `access` ([`rows_with`]): "You can: read and write",
//!   "Owner can read and write (ada)", "Users can read and run
//!   (inherited)". A place that keeps no permissions says so in one row.
//! - **Change time** sets the modified, accessed or created time to now, a
//!   date typed in ISO form ([`parse_time`]), or the other panel's file's;
//!   after the job the times the disk kept are read back
//!   ([`AccessAsk::ReadBack`]) and a rounded one is said ("Modified time
//!   rounded by the disk").
//! - **Change owner** on Windows hands the file to the system's own
//!   Security dialog ([`AccessAsk::SecurityDialog`]), through a seam a
//!   test replaces, so no test ever opens it.

use diroxide_core::details::{DetailGroup, DetailRow};
use diroxide_core::{JobId, Place, Timestamp, UtcOffset};
use diroxide_ops::access::{TimeKind, rounded_line};
use diroxide_vfs::{CancelToken, OpContext, Registry, VfsError};

/// The line said when a typed date cannot be read (39 cells).
pub const NOT_A_DATE: &str = "Not a date: type it as 2026-10-10 22:09";

/// The access rows of one entry, read for the details view: the
/// provider's facts in AX's words, the Flags and System groups; a place
/// that keeps no permissions gives its one row. A cancel gives nothing.
pub fn rows_with(registry: &Registry, place: &Place, cancel: &CancelToken) -> Vec<DetailRow> {
    let provider = match registry.resolve(place) {
        Ok(provider) => provider,
        Err(_) => return Vec::new(),
    };
    let ctx = OpContext::new().with_cancel(cancel.clone());
    match provider.access(place, &ctx) {
        Ok(facts) => diroxide_ops::access::access_rows(&facts),
        Err(VfsError::Cancelled) => Vec::new(),
        Err(error) => vec![unread_row(&error.to_string())],
    }
}

/// The row when the access facts could not be read: "Permissions: not
/// read, permission denied".
pub fn unread_row(cause: &str) -> DetailRow {
    DetailRow::new(
        "access.unread",
        "Permissions",
        format!("not read, {cause}"),
        DetailGroup::Flags,
    )
}

/// What the app asks the runtime about access.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessAsk {
    /// After a time change: read back the times each place kept, to say
    /// one the disk rounded or did not keep.
    ReadBack {
        /// The job that set them.
        job: JobId,
        /// The places it changed.
        places: Vec<Place>,
        /// Each time set, and the time asked for.
        asked: Vec<(TimeKind, Timestamp)>,
    },
    /// Hand a file to Windows' own Security dialog, never waited for.
    SecurityDialog {
        /// The file or folder.
        place: Place,
    },
}

/// What the runtime answered.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessNote {
    /// The times read back: the lines to say, none when every time was
    /// kept as asked.
    ReadBack {
        /// The job.
        job: JobId,
        /// "Modified time rounded by the disk", each once.
        lines: Vec<String>,
    },
    /// The Security dialog was opened, or why not.
    SecurityDialog {
        /// The file.
        place: Place,
        /// Opened, or the cause in words.
        result: Result<(), String>,
    },
}

/// The times each place kept, against the times asked for: the lines of
/// the ones rounded or not kept, each line once.
pub fn read_back(
    registry: &Registry,
    places: &[Place],
    asked: &[(TimeKind, Timestamp)],
) -> Vec<String> {
    let ctx = OpContext::new();
    let mut lines: Vec<String> = Vec::new();
    for place in places {
        let Ok(entry) = registry
            .resolve(place)
            .and_then(|provider| provider.stat(place, &ctx))
        else {
            continue;
        };
        for (kind, time) in asked {
            let kept = match kind {
                TimeKind::Modified => entry.modified,
                TimeKind::Accessed => entry.accessed,
                TimeKind::Created => entry.created,
                TimeKind::Changed => continue,
            };
            // A time to the second: a disk that keeps nanoseconds keeps
            // what was asked; the job sets whole seconds.
            let kept = kept.map(|t| Timestamp::from_unix(t.unix_seconds(), 0));
            if let Some(line) = rounded_line(*kind, *time, kept)
                && !lines.contains(&line)
            {
                lines.push(line);
            }
        }
    }
    lines
}

/// A time as the typed field shows it, in ISO order: "2026-10-10 22:09".
pub fn typed_form(time: Timestamp, offset: UtcOffset) -> String {
    let civil = time.to_civil(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        civil.year, civil.month, civil.day, civil.hour, civil.minute
    )
}

/// A typed date and time, in local time at `offset`: `2026-10-10 22:09`,
/// with `T` or a space between, seconds optional; a date alone is its
/// midnight; `now` is `now`. Anything else is [`NOT_A_DATE`].
pub fn parse_time(text: &str, offset: UtcOffset, now: Timestamp) -> Result<Timestamp, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("now") {
        return Ok(Timestamp::from_unix(now.unix_seconds(), 0));
    }
    let not = || NOT_A_DATE.to_owned();
    let (date, time) = match text.split_once(['T', 't', ' ']) {
        Some((date, time)) => (date, Some(time.trim())),
        None => (text, None),
    };
    let mut parts = date.split('-');
    let year: i32 = number(parts.next()).ok_or_else(not)?;
    let month: u8 = number(parts.next()).ok_or_else(not)?;
    let day: u8 = number(parts.next()).ok_or_else(not)?;
    if parts.next().is_some() || !(1000..=9999).contains(&year) {
        return Err(not());
    }
    let (hour, minute, second) = match time {
        None | Some("") => (0, 0, 0),
        Some(time) => {
            let mut parts = time.split(':');
            let hour: u8 = number(parts.next()).ok_or_else(not)?;
            let minute: u8 = number(parts.next()).ok_or_else(not)?;
            let second: u8 = match parts.next() {
                Some(second) => number(Some(second)).ok_or_else(not)?,
                None => 0,
            };
            if parts.next().is_some() {
                return Err(not());
            }
            (hour, minute, second)
        }
    };
    Timestamp::from_civil(year, month, day, hour, minute, second, offset).ok_or_else(not)
}

/// A part of a typed date: digits only, one to four of them.
fn number<T: std::str::FromStr>(part: Option<&str>) -> Option<T> {
    let part = part?.trim();
    if part.is_empty() || part.len() > 4 || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> Timestamp {
        Timestamp::from_civil(year, month, day, hour, minute, 0, UtcOffset::UTC).unwrap()
    }

    #[test]
    fn a_typed_date_is_read_in_iso_order_or_refused_in_words() {
        let utc = UtcOffset::UTC;
        let now = at(2026, 10, 16, 9, 0);
        assert_eq!(
            parse_time("2026-10-10 22:09", utc, now),
            Ok(at(2026, 10, 10, 22, 9))
        );
        assert_eq!(
            parse_time(" 2026-10-10T22:09:30 ", utc, now),
            Ok(at(2026, 10, 10, 22, 9).plus_seconds(30))
        );
        assert_eq!(
            parse_time("2026-10-10", utc, now),
            Ok(at(2026, 10, 10, 0, 0))
        );
        assert_eq!(parse_time("now", utc, now), Ok(now));
        // Local time: two hours east is two hours earlier in UTC.
        let east = UtcOffset::from_hours(2);
        assert_eq!(
            parse_time("2026-10-10 22:09", east, now),
            Ok(at(2026, 10, 10, 20, 9))
        );
        for bad in [
            "",
            "yesterday",
            "2026-13-01",
            "2026-02-30",
            "2026-10-10 25:00",
            "10/10/2026",
            "2026-10-10 22",
            "2026-10-10 22:09:30:1",
            "-2026-10-10",
        ] {
            assert_eq!(
                parse_time(bad, utc, now),
                Err(NOT_A_DATE.to_owned()),
                "{bad}"
            );
        }
        assert!(within_cells(NOT_A_DATE, NOT_A_DATE));
        assert_eq!(typed_form(at(2026, 10, 10, 22, 9), utc), "2026-10-10 22:09");
    }

    #[test]
    fn a_time_the_disk_rounded_is_said_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"a").unwrap();
        let place = Place::parse(path.to_str().unwrap()).unwrap();
        let registry = Registry::with_local();
        let modified = registry
            .resolve(&place)
            .unwrap()
            .stat(&place, &OpContext::new())
            .unwrap()
            .modified
            .unwrap();
        let kept = Timestamp::from_unix(modified.unix_seconds(), 0);
        // Asked for what the file has: nothing to say.
        assert!(read_back(&registry, &[place.clone()], &[(TimeKind::Modified, kept)]).is_empty());
        // Asked for a second later than it kept: rounded, said once for
        // two places.
        let asked = kept.plus_seconds(1);
        let lines = read_back(
            &registry,
            &[place.clone(), place],
            &[(TimeKind::Modified, asked)],
        );
        assert_eq!(lines, ["Modified time rounded by the disk"]);
        assert!(within_cells(&lines[0], &lines[0]));
    }

    #[test]
    fn a_place_that_keeps_no_permissions_says_so_and_a_failure_says_why() {
        let row = unread_row("permission denied");
        assert_eq!(row.spoken(), "Permissions: not read, permission denied");
        assert!(within_cells(&row.spoken(), "Permissions: not read"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.txt");
        std::fs::write(&path, b"b").unwrap();
        let place = Place::parse(path.to_str().unwrap()).unwrap();
        let registry = Registry::with_local();
        let rows = rows_with(&registry, &place, &CancelToken::new());
        assert!(
            rows.iter().any(|r| r.key == "access.you-can"),
            "a file of this computer says what you can do: {rows:?}"
        );
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(rows_with(&registry, &place, &cancel).is_empty());
    }
}
