//! What a watch says, and how often: every line puts its meaning in its
//! first 40 cells, words before symbols, the name of the file or folder
//! after the fact ("Arrived: scan-004.pdf, 1.2 MB", 29 cells); the
//! [`Teller`] says at most one line an interval. A file name's hidden
//! characters are said as a question mark, as the panels show them; the
//! real name stays in [`Arrival::name`] for scripts.

use std::time::Duration;

use diroxide_core::Timestamp;
use diroxide_core::format::{count, human_size};
use diroxide_core::names::display_name;

use super::between;
use super::machine::{Arrival, SkipReason};

/// One arrival in words: "Arrived: scan-004.pdf, 1.2 MB", or "Arrived
/// again: scan-004.pdf, 3.0 MB" when the name had settled before.
pub fn arrived_line(arrival: &Arrival) -> String {
    let word = if arrival.again {
        "Arrived again"
    } else {
        "Arrived"
    };
    match arrival.size {
        Some(size) => format!(
            "{word}: {}, {}",
            display_name(&arrival.name),
            human_size(size)
        ),
        None => format!("{word}: {}", display_name(&arrival.name)),
    }
}

/// Several arrivals in one line: "Watching Downloads: 3 arrived", the
/// count since watching began.
pub fn summary_line(folder: &str, arrived: usize) -> String {
    format!("Watching {folder}: {arrived} arrived")
}

/// What the status line shows while files settle, never said: "Waiting
/// for scan-004.pdf to settle", or "Waiting for 3 files to settle";
/// `None` when nothing waits.
pub fn waiting_line(names: &[String]) -> Option<String> {
    match names {
        [] => None,
        [one] => Some(format!("Waiting for {} to settle", display_name(one))),
        many => Some(format!(
            "Waiting for {} to settle",
            count(many.len() as u64, "file", "files")
        )),
    }
}

/// A skipped file, said: "Skipped scan-004.pdf: online only".
pub fn skipped_line(name: &str, reason: SkipReason) -> String {
    format!("Skipped {}: {}", display_name(name), reason.words())
}

/// A file that kept changing too long: "Gave up waiting for app.log".
pub fn gave_up_line(name: &str) -> String {
    format!("Gave up waiting for {}", display_name(name))
}

/// The first line of a watch: "Watching Downloads for new files. Escape
/// stops." With `every`, the folder is polled: "Watching share for new
/// files, checking every 5 seconds. Ctrl+C stops." `stops` is the
/// frontend's own words for stopping, or empty.
pub fn start_line(folder: &str, every: Option<Duration>, stops: &str) -> String {
    let mut line = format!("Watching {folder} for new files");
    if let Some(every) = every {
        line.push_str(&format!(
            ", checking every {}",
            count(every.as_secs().max(1), "second", "seconds")
        ));
    }
    line.push('.');
    if !stops.is_empty() {
        line.push(' ');
        line.push_str(stops);
    }
    line
}

/// The last line of a watch stopped on request: "Stopped watching
/// Downloads", with the count when anything arrived ("Stopped watching
/// Downloads: 3 arrived").
pub fn stopped_line(folder: &str, arrived: usize) -> String {
    if arrived == 0 {
        format!("Stopped watching {folder}")
    } else {
        format!("Stopped watching {folder}: {arrived} arrived")
    }
}

/// The last line of a watch whose folder went away: "Stopped watching
/// Downloads: the folder is gone".
pub fn gone_line(folder: &str) -> String {
    format!("Stopped watching {folder}: the folder is gone")
}

/// Paces what a watch says: at most one line every interval. An arrival
/// with no line said in the last interval is said at once; arrivals inside
/// the interval wait, and when the interval is over the one waiting is
/// said, or, when several wait, the summary ("Watching Downloads: 3
/// arrived"). One line a settled arrival at most, never one a write.
#[derive(Clone, Debug)]
pub struct Teller {
    folder: String,
    interval: Duration,
    last: Option<Timestamp>,
    held: Vec<Arrival>,
    total: usize,
}

impl Teller {
    /// A teller for the folder of this name, saying at most one line every
    /// `interval`.
    pub fn new(folder: impl Into<String>, interval: Duration) -> Self {
        Self {
            folder: folder.into(),
            interval,
            last: None,
            held: Vec::new(),
            total: 0,
        }
    }

    /// Takes an arrival: its line when it may be said now, else `None`
    /// and it waits.
    pub fn arrived(&mut self, arrival: Arrival, now: Timestamp) -> Option<String> {
        self.total += 1;
        self.held.push(arrival);
        self.due(now)
    }

    /// The waiting line, when its time has come.
    pub fn due(&mut self, now: Timestamp) -> Option<String> {
        if self.held.is_empty() || !self.may_say(now) {
            return None;
        }
        let line = match self.held.as_slice() {
            [one] => arrived_line(one),
            _ => summary_line(&self.folder, self.total),
        };
        self.held.clear();
        self.last = Some(now);
        Some(line)
    }

    /// How long until a waiting line may be said; `None` when nothing
    /// waits.
    pub fn next_due(&self, now: Timestamp) -> Option<Duration> {
        if self.held.is_empty() {
            return None;
        }
        Some(match self.last {
            Some(last) => self.interval.saturating_sub(between(last, now)),
            None => Duration::ZERO,
        })
    }

    /// How many arrivals it has taken.
    pub fn total(&self) -> usize {
        self.total
    }

    /// How many arrivals wait to be said.
    pub fn waiting(&self) -> usize {
        self.held.len()
    }

    fn may_say(&self, now: Timestamp) -> bool {
        self.last
            .is_none_or(|last| between(last, now) >= self.interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix(
            1_800_000_000 + ms.div_euclid(1000),
            (ms.rem_euclid(1000) * 1_000_000) as u32,
        )
    }

    fn arrival(name: &str, size: u64) -> Arrival {
        Arrival {
            name: name.to_owned(),
            size: Some(size),
            again: false,
            waited: Duration::from_secs(2),
        }
    }

    #[test]
    fn the_lines_say_their_meaning_first() {
        let line = arrived_line(&arrival("scan-004.pdf", 1_200_000));
        assert_eq!(line, "Arrived: scan-004.pdf, 1.2 MB");
        assert_eq!(line.chars().count(), 29);
        let again = Arrival {
            again: true,
            ..arrival("scan-004.pdf", 3_000_000)
        };
        assert_eq!(arrived_line(&again), "Arrived again: scan-004.pdf, 3.0 MB");
        let unknown = Arrival {
            size: None,
            ..arrival("x", 0)
        };
        assert_eq!(arrived_line(&unknown), "Arrived: x");
        assert_eq!(
            summary_line("Downloads", 3),
            "Watching Downloads: 3 arrived"
        );
        assert_eq!(
            waiting_line(&["scan-004.pdf".to_owned()]).unwrap(),
            "Waiting for scan-004.pdf to settle"
        );
        assert_eq!(
            waiting_line(&["a".to_owned(), "b".to_owned(), "c".to_owned()]).unwrap(),
            "Waiting for 3 files to settle"
        );
        assert_eq!(waiting_line(&[]), None);
        assert_eq!(
            skipped_line("scan-004.pdf", SkipReason::OnlineOnly),
            "Skipped scan-004.pdf: online only"
        );
        assert_eq!(gave_up_line("app.log"), "Gave up waiting for app.log");
        assert_eq!(stopped_line("Downloads", 0), "Stopped watching Downloads");
        assert_eq!(stopped_line("in", 3), "Stopped watching in: 3 arrived");
        assert_eq!(gone_line("in"), "Stopped watching in: the folder is gone");
        assert_eq!(
            start_line("in", None, "Escape stops."),
            "Watching in for new files. Escape stops."
        );
        assert_eq!(
            start_line("share", Some(Duration::from_secs(5)), "Ctrl+C stops."),
            "Watching share for new files, checking every 5 seconds. Ctrl+C stops."
        );
        assert_eq!(start_line("in", None, ""), "Watching in for new files.");
        // A hidden character is said as a question mark.
        let bell = Arrival {
            name: "scan\u{7}.pdf".to_owned(),
            ..arrival("x", 5)
        };
        assert_eq!(arrived_line(&bell), "Arrived: scan?.pdf, 5 bytes");
        assert_eq!(
            waiting_line(std::slice::from_ref(&bell.name)).unwrap(),
            "Waiting for scan?.pdf to settle"
        );
        // The fact comes before the folder's or the file's name, so the
        // first 40 cells carry it for any name.
        let long = "a-very-long-file-name-from-a-scanner-0001.pdf";
        for (line, fact) in [
            (arrived_line(&arrival(long, 10)), "Arrived: "),
            (
                summary_line("Downloads", 12),
                "Watching Downloads: 12 arrived",
            ),
            (waiting_line(&[long.to_owned()]).unwrap(), "Waiting for "),
            (skipped_line(long, SkipReason::OnlineOnly), "Skipped "),
            (gave_up_line(long), "Gave up waiting for "),
            (
                stopped_line("Downloads", 3),
                "Stopped watching Downloads: 3 arrived",
            ),
            (
                gone_line("Downloads"),
                "Stopped watching Downloads: the folder",
            ),
            (
                start_line("Downloads", None, "Escape stops."),
                "Watching Downloads for new files.",
            ),
        ] {
            assert!(within_cells(&line, fact), "{line}");
        }
    }

    #[test]
    fn one_line_an_interval_and_the_summary_for_several() {
        let mut teller = Teller::new("in", Duration::from_secs(10));
        assert_eq!(teller.next_due(at(0)), None);
        // The first arrival is said at once.
        assert_eq!(
            teller.arrived(arrival("a.pdf", 1_000), at(0)).as_deref(),
            Some("Arrived: a.pdf, 1.0 KB")
        );
        // The next inside the interval waits, and is said alone when the
        // interval is over.
        assert_eq!(teller.arrived(arrival("b.pdf", 2_000), at(3_000)), None);
        assert_eq!(teller.next_due(at(3_000)), Some(Duration::from_secs(7)));
        assert_eq!(teller.due(at(9_999)), None);
        assert_eq!(
            teller.due(at(10_000)).as_deref(),
            Some("Arrived: b.pdf, 2.0 KB")
        );
        assert_eq!(teller.next_due(at(10_000)), None);
        // Three inside one interval become the summary, with the total.
        assert_eq!(teller.arrived(arrival("c.pdf", 1), at(11_000)), None);
        assert_eq!(teller.arrived(arrival("d.pdf", 1), at(12_000)), None);
        assert_eq!(teller.arrived(arrival("e.pdf", 1), at(13_000)), None);
        assert_eq!(teller.waiting(), 3);
        assert_eq!(
            teller.due(at(20_000)).as_deref(),
            Some("Watching in: 5 arrived")
        );
        assert_eq!(teller.total(), 5);
        // After a quiet interval, an arrival is said at once again.
        assert_eq!(
            teller.arrived(arrival("f.pdf", 1), at(45_000)).as_deref(),
            Some("Arrived: f.pdf, 1 byte")
        );
    }
}
