//! Progress in words, percent first (research/file-operations.md): "Copying
//! 30 percent, 2 minutes left" on the jobs line, the counts on a detail
//! line. No spinners; the app says a change of state at once and progress
//! at most every ten seconds in screen-reader mode, at most twice a second
//! otherwise. The estimate stays hidden until the engine has one it trusts
//! (five seconds and five percent, smoothed), so `seconds_left` is `None`
//! until then; the sentence, estimate included, fits 40 cells for every
//! job that has one.

use std::time::Duration;

use diroxide_core::format::{count, human_size};
use diroxide_core::{JobKind, JobState};

/// How far a job has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobProgress {
    /// What kind of job.
    pub kind: JobKind,
    /// What the job is doing now.
    pub state: JobState,
    /// Steps finished.
    pub items_done: u64,
    /// Steps in all.
    pub items_total: u64,
    /// Bytes finished.
    pub bytes_done: u64,
    /// Bytes in all, when known.
    pub bytes_total: Option<u64>,
    /// The smoothed rate, in bytes a second, when there is one.
    pub bytes_per_second: Option<u64>,
    /// An estimate of the seconds left, when there is one worth saying.
    pub seconds_left: Option<u64>,
    /// The name being worked on.
    pub current: Option<String>,
}

impl JobProgress {
    /// No progress yet on a job of this kind and size, counting.
    pub fn start(kind: JobKind, items_total: u64, bytes_total: Option<u64>) -> Self {
        Self {
            kind,
            state: JobState::Counting,
            items_done: 0,
            items_total,
            bytes_done: 0,
            bytes_total,
            bytes_per_second: None,
            seconds_left: None,
            current: None,
        }
    }

    /// The share done, in whole percent: by bytes when their total is
    /// known, else by items.
    pub fn percent(&self) -> u64 {
        let (done, total) = match self.bytes_total.filter(|&total| total > 0) {
            Some(total) => (self.bytes_done, total),
            None => (self.items_done, self.items_total),
        };
        if total == 0 {
            return 0;
        }
        (u128::from(done.min(total)) * 100 / u128::from(total)) as u64
    }

    /// The jobs line, the key fact first and inside 40 cells: "Copying 30
    /// percent, 2 minutes left", "Copying paused at 30 percent", "Copying
    /// waits for an answer".
    pub fn describe(&self) -> String {
        let word = self.kind.progress_word();
        let percent = self.percent();
        match self.state {
            JobState::Counting if self.items_done > 0 => {
                format!("{word}: counting, {} so far", self.items_done)
            }
            JobState::Counting => format!("{word}: counting"),
            JobState::Running if matches!(self.kind, JobKind::Split | JobKind::Join) => {
                self.parts_line(word)
            }
            JobState::Running => match self.seconds_left {
                Some(seconds) => format!("{word} {percent} percent, {}", time_left(seconds)),
                None => format!("{word} {percent} percent"),
            },
            JobState::Paused => format!("{word} paused at {percent} percent"),
            JobState::Stalled => format!("{word} stalled at {percent} percent"),
            JobState::WaitingForAnswer => format!("{word} waits for an answer"),
            JobState::Verifying => format!("Verifying {percent} percent"),
            JobState::Finished => format!("{word} finished"),
        }
    }

    /// A split's or a join's line, by parts: "Splitting 4 of 12 parts, 1.4
    /// of 4.0 GB", the part at work counted; the estimate only where the
    /// line still fits 40 cells.
    fn parts_line(&self, word: &str) -> String {
        let total = self.items_total.max(1);
        let current = (self.items_done + 1).min(total);
        let mut line = format!("{word} {current} of {}", count(total, "part", "parts"));
        if let Some(bytes) = self.bytes_total.filter(|&bytes| bytes > 0) {
            line.push_str(&format!(", {}", bytes_of(self.bytes_done, bytes)));
        }
        if let Some(seconds) = self.seconds_left {
            let longer = format!("{line}, {}", time_left(seconds));
            if longer.chars().count() <= diroxide_core::format::BRAILLE_CELLS {
                return longer;
            }
        }
        line
    }

    /// The detail line: "12 of 300 items, 1.2 of 4.0 GB, 48 MB a second".
    pub fn detail(&self) -> String {
        let mut text = format!(
            "{} of {}",
            self.items_done,
            count(self.items_total, "item", "items")
        );
        if self.items_total == 0 {
            text = "no items".to_owned();
        }
        if let Some(total) = self.bytes_total.filter(|&total| total > 0) {
            text.push_str(", ");
            text.push_str(&bytes_of(self.bytes_done, total));
        }
        if let Some(rate) = self.bytes_per_second.filter(|&rate| rate > 0) {
            text.push_str(&format!(", {} a second", human_size(rate)));
        }
        text
    }
}

/// "1.2 of 4.0 GB" when both share a unit, else "512 MB of 4.0 GB".
fn bytes_of(done: u64, total: u64) -> String {
    let done_text = human_size(done);
    let total_text = human_size(total);
    match (done_text.rsplit_once(' '), total_text.rsplit_once(' ')) {
        (Some((number, unit)), Some((_, total_unit))) if unit == total_unit => {
            format!("{number} of {total_text}")
        }
        _ => format!("{done_text} of {total_text}"),
    }
}

/// "2 minutes left", "1 hour left", rounded up, with no "about": the
/// estimate is already the engine's best. Anything under a minute is "1
/// minute left", so the longest progress word, the percent and the
/// estimate fit 40 cells together.
fn time_left(seconds: u64) -> String {
    match seconds {
        0..=3599 => {
            let minutes = seconds.div_ceil(60).max(1);
            format!(
                "{minutes} minute{} left",
                if minutes == 1 { "" } else { "s" }
            )
        }
        _ => {
            let hours = seconds.div_ceil(3600);
            format!("{hours} hour{} left", if hours == 1 { "" } else { "s" })
        }
    }
}

/// The smoothed throughput and the estimate (research/file-operations.md,
/// "Progress"): the byte count sampled once a second, an exponentially
/// weighted rate with a factor of 0.2, and no estimate until five seconds
/// have passed and five percent is done. A pause, a stall or a new total
/// starts it over.
#[derive(Clone, Debug, Default)]
pub(crate) struct Estimator {
    started: Option<Duration>,
    sample: Option<(Duration, u64)>,
    rate: Option<f64>,
}

impl Estimator {
    /// How long a sample is.
    const SAMPLE: Duration = Duration::from_secs(1);
    /// How much a new sample counts.
    const WEIGHT: f64 = 0.2;
    /// How long before an estimate is said.
    const HIDDEN_FOR: Duration = Duration::from_secs(5);

    /// Takes the count at `now` and returns the rate and the estimate.
    pub(crate) fn update(
        &mut self,
        now: Duration,
        done: u64,
        total: Option<u64>,
    ) -> (Option<u64>, Option<u64>) {
        let started = *self.started.get_or_insert(now);
        match self.sample {
            None => self.sample = Some((now, done)),
            Some((at, bytes)) if now >= at + Self::SAMPLE => {
                let seconds = (now - at).as_secs_f64();
                let instant = done.saturating_sub(bytes) as f64 / seconds;
                self.rate = Some(match self.rate {
                    Some(rate) => rate + Self::WEIGHT * (instant - rate),
                    None => instant,
                });
                self.sample = Some((now, done));
            }
            Some(_) => {}
        }
        let rate = self.rate.filter(|rate| *rate > 0.0);
        let per_second = rate.map(|rate| rate.round() as u64);
        let left = match (rate, total) {
            (Some(rate), Some(total))
                if total > 0
                    && now >= started + Self::HIDDEN_FOR
                    && done.saturating_mul(20) >= total =>
            {
                Some((total.saturating_sub(done) as f64 / rate).ceil() as u64)
            }
            _ => None,
        };
        (per_second, left)
    }

    /// Starts over: after a pause, a stall, or a change of the total.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn progress_reads_percent_first() {
        let mut p = JobProgress::start(JobKind::Copy, 300, Some(4_000_000_000));
        assert_eq!(p.describe(), "Copying: counting");
        p.items_done = 1200;
        assert_eq!(p.describe(), "Copying: counting, 1200 so far");
        p.items_done = 0;
        p.state = JobState::Running;
        p.items_done = 12;
        p.bytes_done = 1_200_000_000;
        p.seconds_left = Some(110);
        assert_eq!(p.describe(), "Copying 30 percent, 2 minutes left");
        assert_eq!(p.detail(), "12 of 300 items, 1.2 of 4.0 GB");
        p.bytes_done = 512_000_000;
        p.seconds_left = Some(30);
        p.bytes_per_second = Some(48_000_000);
        assert_eq!(p.describe(), "Copying 12 percent, 1 minute left");
        assert_eq!(
            p.detail(),
            "12 of 300 items, 512 MB of 4.0 GB, 48.0 MB a second"
        );
        let mut trash = JobProgress::start(JobKind::Trash, 3, None);
        trash.state = JobState::Running;
        trash.items_done = 1;
        assert_eq!(trash.describe(), "Moving to the trash 33 percent");
    }

    #[test]
    fn every_state_says_its_fact_in_40_cells() {
        let mut p = JobProgress::start(JobKind::Trash, 3, None);
        p.items_done = 3;
        let keys = [
            (JobState::Counting, "Moving to the trash: counting"),
            (JobState::Running, "Moving to the trash 100 percent"),
            (JobState::Paused, "Moving to the trash paused"),
            (JobState::Stalled, "Moving to the trash stalled"),
            (JobState::WaitingForAnswer, "Moving to the trash waits"),
            (JobState::Verifying, "Verifying 100 percent"),
            (JobState::Finished, "Moving to the trash finished"),
        ];
        for (state, key) in keys {
            p.state = state;
            assert!(within_cells(&p.describe(), key), "{}", p.describe());
        }
    }

    #[test]
    fn the_estimate_fits_40_cells_for_every_job_that_has_one() {
        let kinds = [
            JobKind::Copy,
            JobKind::Move,
            JobKind::Duplicate,
            JobKind::Pack,
            JobKind::Unpack,
            JobKind::TestArchive,
            JobKind::Split,
            JobKind::Join,
            JobKind::Convert,
            JobKind::Download,
            JobKind::Undo,
            JobKind::Rename,
            JobKind::DeletePermanently,
        ];
        for kind in kinds {
            for seconds in [1, 59, 61, 3599, 3600, 86_400, 359_999] {
                let mut p = JobProgress::start(kind, 1, Some(10));
                p.state = JobState::Running;
                p.bytes_done = 10;
                p.seconds_left = Some(seconds);
                let line = p.describe();
                assert!(line.chars().count() <= 40, "{line}");
                assert!(!line.contains("about"), "{line}");
            }
        }
    }

    #[test]
    fn the_estimate_hides_until_five_seconds_and_five_percent() {
        let mut estimate = Estimator::default();
        let total = Some(1000);
        let second = Duration::from_secs(1);
        assert_eq!(estimate.update(Duration::ZERO, 0, total), (None, None));
        assert_eq!(estimate.update(second, 10, total), (Some(10), None));
        let (_, left) = estimate.update(second * 4, 40, total);
        assert_eq!(left, None, "under five seconds");
        let (rate, left) = estimate.update(second * 6, 60, total);
        assert_eq!(rate, Some(10));
        assert_eq!(left, Some(94), "five seconds and six percent");
        estimate.reset();
        assert_eq!(estimate.update(second * 7, 70, total), (None, None));
        let (_, unknown) = Estimator::default().update(second * 9, 900, None);
        assert_eq!(unknown, None, "no total, no estimate");
    }
}
