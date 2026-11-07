//! What a conversion did, failures first because that is what a person
//! acts on (R28 5.3, question 5.2): "Report: 1 failed, 12 converted", then
//! each failure with its cause, the refusals, the skips with their
//! reasons, what was made, and a footer with the recipe, the tool and the
//! time from the clock in full.
//!
//! The report is a list in the app, like the Jobs list. The last 20 are
//! kept in the state folder ([`ReportShelf`]); a file is written only on
//! request ("Save report", `diro convert --report FILE`), from
//! [`ConvertReport::markdown`], and holds names only, never a full path.
//! Undo needs nothing more: the job's undo record names every output, so
//! Ctrl+Z trashes the files the job made.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use diroxide_core::{Clock, JobId, JobKind, Place, Timestamp};
use serde::{Deserialize, Serialize};

use super::plan::said;
use crate::engine::JobReport;

/// How many reports the state folder keeps.
pub const KEPT: usize = 20;

/// What a conversion did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertReport {
    /// The job's report: the outputs made in `done`, the failures and
    /// skips by their sources, and the undo record whose `created` names
    /// every output, so Undo trashes the files the job made.
    pub job: JobReport,
    /// Each output made, with its source: (source, output).
    pub made: Vec<(Place, Place)>,
    /// The files the plan refused, each with its reason.
    pub refused: Vec<(Place, String)>,
    /// The recipe's name.
    pub recipe: String,
    /// The tool's name as said.
    pub tool: String,
    /// When the job ended, set by [`ConvertReport::stamp`].
    pub finished: Option<Timestamp>,
}

impl ConvertReport {
    /// An empty report for a conversion job.
    pub fn new(job: JobId, recipe: &str, tool: &str) -> Self {
        Self {
            job: JobReport::new(job, JobKind::Convert),
            made: Vec::new(),
            refused: Vec::new(),
            recipe: recipe.to_owned(),
            tool: tool.to_owned(),
            finished: None,
        }
    }

    /// Notes the time the job ended, from the clock.
    pub fn stamp(&mut self, clock: &dyn Clock) {
        self.finished = Some(clock.now());
    }

    /// The report in a few words, what went wrong first: "1 failed, 12
    /// converted", "cancelled, 3 converted", "12 converted, 2 skipped".
    pub fn short(&self) -> String {
        let mut parts = Vec::new();
        if self.job.cancelled {
            parts.push("cancelled".to_owned());
        } else if self.job.stopped {
            parts.push("stopped at a failure".to_owned());
        }
        if !self.job.failed.is_empty() {
            parts.push(format!("{} failed", self.job.failed.len()));
        }
        parts.push(format!("{} converted", self.made.len()));
        if !self.refused.is_empty() {
            parts.push(format!("{} refused", self.refused.len()));
        }
        if !self.job.skipped.is_empty() {
            parts.push(format!("{} skipped", self.job.skipped.len()));
        }
        parts.join(", ")
    }

    /// The report's first line: "Report: 1 failed, 12 converted".
    pub fn summary(&self) -> String {
        format!("Report: {}", self.short())
    }

    /// The report's rows, failures first: the summary; each failure
    /// ("Failed: scan-004.pdf, ImageMagick is not installed"); each
    /// refusal; each skip ("Skipped song.flac: output exists"); each
    /// warning; each output made ("Made: song.mp3 from song.flac"); then
    /// the recipe, the tool and when it ended, in full.
    pub fn lines(&self, clock: &dyn Clock) -> Vec<String> {
        let mut out = vec![self.summary()];
        for (place, cause) in &self.job.failed {
            out.push(format!("Failed: {}, {cause}", said(place)));
        }
        for (place, reason) in &self.refused {
            out.push(format!("Refused: {}, {reason}", said(place)));
        }
        for (place, reason) in &self.job.skipped {
            out.push(format!("Skipped {}: {reason}", said(place)));
        }
        for (place, warning) in &self.job.warnings {
            out.push(format!("{warning}: {}", said(place)));
        }
        for (source, output) in &self.made {
            out.push(format!("Made: {} from {}", said(output), said(source)));
        }
        out.push(format!("Recipe: {}", self.recipe));
        out.push(format!("Tool: {}", self.tool));
        let when = self.finished.unwrap_or_else(|| clock.now());
        let civil = when.to_civil(clock.offset());
        out.push(format!(
            "Finished: {}, {}",
            civil.long_date(),
            civil.clock_time()
        ));
        out
    }

    /// The report as a Markdown file, for "Save report" and `diro convert
    /// --report FILE`: names only, never a full path, a folder's name or
    /// the computer's, so the file can be shared.
    pub fn markdown(&self, clock: &dyn Clock) -> String {
        let lines = self.lines(clock);
        let mut text = String::from("# Conversion report\n\n");
        text.push_str(&lines[0]);
        text.push_str("\n\n");
        for line in &lines[1..] {
            text.push_str("- ");
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    /// The report as the state folder keeps it.
    pub fn kept(&self, clock: &dyn Clock) -> KeptReport {
        KeptReport {
            job: self.job.job.get(),
            finished: self.finished.unwrap_or_else(|| clock.now()).unix_seconds(),
            summary: self.summary(),
            lines: self.lines(clock),
        }
    }
}

/// One report as the state folder keeps it: its words, already made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptReport {
    /// The job's id.
    pub job: u64,
    /// When it ended, in seconds since 1970.
    pub finished: i64,
    /// Its first line.
    pub summary: String,
    /// Its rows.
    pub lines: Vec<String>,
}

/// The last [`KEPT`] conversion reports, in a folder of the state folder:
/// one JSON file each, named by when it ended and its job, the oldest
/// removed as a new one comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportShelf {
    folder: PathBuf,
}

impl ReportShelf {
    /// The shelf in this folder (made when the first report is kept).
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// The shelf's folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(&self.folder)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("report-") && n.ends_with(".json"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    }

    /// Keeps a report, then removes the oldest beyond [`KEPT`].
    pub fn keep(&self, report: &KeptReport) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let name = format!(
            "report-{:012}-{:08}.json",
            report.finished.max(0),
            report.job
        );
        let text = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
        let staging = self.folder.join(format!(".{name}.part"));
        fs::write(&staging, text)?;
        fs::rename(&staging, self.folder.join(&name))?;
        let files = self.files();
        if files.len() > KEPT {
            for old in &files[..files.len() - KEPT] {
                fs::remove_file(old)?;
            }
        }
        Ok(())
    }

    /// The kept reports, the newest first; a file that cannot be read is
    /// passed over.
    pub fn list(&self) -> Vec<KeptReport> {
        self.files()
            .iter()
            .rev()
            .filter_map(|path| fs::read_to_string(path).ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::testing::{place, scratch};
    use diroxide_core::{FixedClock, UtcOffset};

    fn clock() -> FixedClock {
        // Friday, November 6, 2026, 5:40 PM at UTC minus eight hours.
        FixedClock::new(
            Timestamp::from_unix(1_794_015_600, 0),
            UtcOffset::from_seconds(-8 * 3600),
        )
    }

    /// A folder named in memory only; nothing is written there.
    fn folder() -> PathBuf {
        PathBuf::from(if cfg!(windows) { r"D:\in" } else { "/in" })
    }

    fn report() -> ConvertReport {
        let p = |name: &str| place(&folder().join(name));
        let mut report = ConvertReport::new(JobId::new(3), "Image to JPEG", "ImageMagick");
        for i in 0..12 {
            report
                .made
                .push((p(&format!("{i}.png")), p(&format!("{i}.jpg"))));
        }
        report
            .job
            .failed
            .push((p("scan-004.png"), "ImageMagick is not installed".to_owned()));
        report
            .job
            .skipped
            .push((p("song.png"), "output exists".to_owned()));
        report
    }

    #[test]
    fn failures_come_first_in_words() {
        let clock = clock();
        let mut report = report();
        assert_eq!(
            report.summary(),
            "Report: 1 failed, 12 converted, 1 skipped"
        );
        report.job.skipped.clear();
        assert_eq!(report.summary(), "Report: 1 failed, 12 converted");
        assert_eq!(report.summary().chars().count(), 30);
        assert_eq!(report.short(), "1 failed, 12 converted");
        report.stamp(&clock);
        let lines = report.lines(&clock);
        assert_eq!(
            lines[1],
            "Failed: scan-004.png, ImageMagick is not installed"
        );
        assert_eq!(lines[2], "Made: 0.jpg from 0.png");
        assert_eq!(lines[lines.len() - 3], "Recipe: Image to JPEG");
        assert_eq!(lines[lines.len() - 2], "Tool: ImageMagick");
        assert_eq!(
            lines.last().unwrap(),
            "Finished: Friday, November 6, 2026, 5:40 PM"
        );
        let mut cancelled = ConvertReport::new(JobId::new(4), "Audio to MP3", "FFmpeg");
        cancelled.job.cancelled = true;
        assert_eq!(cancelled.summary(), "Report: cancelled, 0 converted");
    }

    /// A saved report holds names only: no folder of the computer's.
    #[test]
    fn a_saved_report_holds_names_only() {
        let clock = clock();
        let report = report();
        let text = report.markdown(&clock);
        assert!(text.starts_with("# Conversion report\n\nReport: 1 failed"));
        assert!(!text.contains(&*folder().to_string_lossy()));
        assert!(!text.contains('\\'));
        assert!(text.contains("- Made: 11.jpg from 11.png\n"));
    }

    #[test]
    fn the_shelf_keeps_the_last_twenty() {
        let root = scratch();
        let shelf = ReportShelf::new(root.path().join("reports"));
        let clock = clock();
        for job in 0..25 {
            let mut report = ConvertReport::new(JobId::new(job), "Image to JPEG", "ImageMagick");
            report.finished = Some(Timestamp::from_unix(1_794_015_600 + job as i64, 0));
            shelf.keep(&report.kept(&clock)).unwrap();
        }
        let kept = shelf.list();
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept[0].job, 24);
        assert_eq!(kept[KEPT - 1].job, 5);
        assert_eq!(kept[0].summary, "Report: 0 converted");
        let names: Vec<String> = fs::read_dir(shelf.folder())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), KEPT);
        assert!(names.iter().all(|n| n.starts_with("report-")));
    }
}
