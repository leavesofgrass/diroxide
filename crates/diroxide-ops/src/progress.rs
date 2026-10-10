//! Progress in words: "Copying 12 of 300, 1.2 of 4.0 GB, about 2 minutes
//! left". No spinners; the app says it at most twice a second.

use diroxide_core::JobKind;
use diroxide_core::format::human_size;

/// How far a job has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobProgress {
    /// What kind of job.
    pub kind: JobKind,
    /// Steps finished.
    pub items_done: u64,
    /// Steps in all.
    pub items_total: u64,
    /// Bytes finished.
    pub bytes_done: u64,
    /// Bytes in all, when known.
    pub bytes_total: Option<u64>,
    /// An estimate of the seconds left, when there is one.
    pub seconds_left: Option<u64>,
    /// The name being worked on.
    pub current: Option<String>,
}

impl JobProgress {
    /// No progress yet on a job of this kind and size.
    pub fn start(kind: JobKind, items_total: u64, bytes_total: Option<u64>) -> Self {
        Self {
            kind,
            items_done: 0,
            items_total,
            bytes_done: 0,
            bytes_total,
            seconds_left: None,
            current: None,
        }
    }

    /// The progress as one sentence, the key fact first.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} {} of {}",
            self.kind.progress_word(),
            self.items_done,
            self.items_total
        );
        if let Some(total) = self.bytes_total.filter(|&total| total > 0) {
            text.push_str(", ");
            text.push_str(&bytes_of(self.bytes_done, total));
        }
        if let Some(seconds) = self.seconds_left {
            text.push_str(", ");
            text.push_str(&time_left(seconds));
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

/// "about 2 minutes left", "about 1 hour left", "less than a minute left".
fn time_left(seconds: u64) -> String {
    match seconds {
        0..=59 => "less than a minute left".to_owned(),
        60..=3599 => {
            let minutes = seconds.div_ceil(60);
            format!(
                "about {minutes} minute{} left",
                if minutes == 1 { "" } else { "s" }
            )
        }
        _ => {
            let hours = seconds.div_ceil(3600);
            format!(
                "about {hours} hour{} left",
                if hours == 1 { "" } else { "s" }
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reads_as_a_sentence() {
        let mut p = JobProgress::start(JobKind::Copy, 300, Some(4_000_000_000));
        p.items_done = 12;
        p.bytes_done = 1_200_000_000;
        p.seconds_left = Some(110);
        assert_eq!(
            p.describe(),
            "Copying 12 of 300, 1.2 of 4.0 GB, about 2 minutes left"
        );
        p.bytes_done = 512_000_000;
        p.seconds_left = Some(30);
        assert_eq!(
            p.describe(),
            "Copying 12 of 300, 512 MB of 4.0 GB, less than a minute left"
        );
        let trash = JobProgress::start(JobKind::Trash, 3, None);
        assert_eq!(trash.describe(), "Moving to the trash 0 of 3");
    }
}
