//! Progress in words, percent first (research/file-operations.md): "Copying
//! 30 percent, 2 minutes left" on the jobs line, the counts on a detail
//! line. No spinners; the app says a change of state at once and progress
//! at most every ten seconds in screen-reader mode, at most twice a second
//! otherwise. The estimate stays hidden until the engine has one it trusts
//! (five seconds and five percent, smoothed), so `seconds_left` is `None`
//! until then.

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
            JobState::Counting => format!("{word}: counting"),
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

/// "2 minutes left", "1 hour left", "less than a minute left", with no
/// "about": the estimate is already the engine's best.
fn time_left(seconds: u64) -> String {
    match seconds {
        0..=59 => "less than a minute left".to_owned(),
        60..=3599 => {
            let minutes = seconds.div_ceil(60);
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

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn progress_reads_percent_first() {
        let mut p = JobProgress::start(JobKind::Copy, 300, Some(4_000_000_000));
        assert_eq!(p.describe(), "Copying: counting");
        p.state = JobState::Running;
        p.items_done = 12;
        p.bytes_done = 1_200_000_000;
        p.seconds_left = Some(110);
        assert_eq!(p.describe(), "Copying 30 percent, 2 minutes left");
        assert_eq!(p.detail(), "12 of 300 items, 1.2 of 4.0 GB");
        p.bytes_done = 512_000_000;
        p.seconds_left = Some(30);
        p.bytes_per_second = Some(48_000_000);
        assert_eq!(p.describe(), "Copying 12 percent, less than a minute left");
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
        let mut copy = JobProgress::start(JobKind::Copy, 1, Some(10));
        copy.state = JobState::Running;
        copy.seconds_left = Some(7200);
        assert!(copy.describe().chars().count() <= 40);
    }
}
