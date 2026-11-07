//! What a running task says (R16 5.1 and 5.2; R28 6.7), each line with
//! its meaning in the first 40 cells and words, never symbols. The task's
//! title (`Scans`) and a file's name (`scan-004.pdf`) are the samples the
//! counts are made with; a longer name pushes the tail, not the meaning.
//! Hidden characters in names are spelled out (`[U+202E]`).
//!
//! Nothing a task says interrupts: every line is polite, the summary at
//! most once a progress interval, a pause once.

use std::time::Duration;

use diroxide_core::format::{count, human_size};
use diroxide_core::names::spell_out;

use super::words::duration_words;

/// The polite summary of everything since the last: "Scans: 3 done, 1
/// failed" (23), "Scans: 3 done" (13). Skips come last.
pub fn summary_line(title: &str, done: usize, failed: usize, skipped: usize) -> String {
    let mut parts = Vec::new();
    if done > 0 || (failed == 0 && skipped == 0) {
        parts.push(format!("{done} done"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    format!("{title}: {}", parts.join(", "))
}

/// A task paused, said once: "Scans paused: pdftotext is not installed"
/// (40), "Scans paused: 10 failures in a row" (34).
pub fn paused_line(title: &str, reason: &str) -> String {
    format!("{title} paused: {reason}")
}

/// The breaker's reason: "10 failures in a row".
pub fn breaker_reason(n: u32) -> String {
    format!("{n} failures in a row")
}

/// The low-space reason: "only 200 MB free".
pub fn space_reason(free: u64) -> String {
    format!("only {} free", human_size(free))
}

/// The missing-folder reason.
pub const FOLDER_MISSING: &str = "folder is missing";

/// The status line while files wait, never spoken: "Scans: 3 waiting"
/// (16), "Scans: 2 waiting, 1 retrying".
pub fn status_line(title: &str, waiting: usize, retrying: usize) -> Option<String> {
    match (waiting, retrying) {
        (0, 0) => None,
        (w, 0) => Some(format!("{title}: {w} waiting")),
        (0, r) => Some(format!("{title}: {r} retrying")),
        (w, r) => Some(format!("{title}: {w} waiting, {r} retrying")),
    }
}

/// Catch-up at start: "14 files were waiting in Scans" (30).
pub fn caught_up_line(title: &str, waiting: usize) -> String {
    if waiting == 1 {
        format!("1 file was waiting in {title}")
    } else {
        format!("{waiting} files were waiting in {title}")
    }
}

/// The first turn-on's question, starting on No above 20 files: "Process
/// the 14 files in Scans? y or n" (37).
pub fn catch_up_question(title: &str, waiting: usize) -> String {
    if waiting == 1 {
        format!("Process the 1 file in {title}? y or n")
    } else {
        format!("Process the {waiting} files in {title}? y or n")
    }
}

/// Above this many waiting files the catch-up question starts on No.
pub const CATCH_UP_YES_UP_TO: usize = 20;

/// A second runner: "Scans is run by another diro" (28).
pub fn locked_line(title: &str) -> String {
    format!("{title} is run by another diro")
}

/// The queue's cap reached: "Scans: 10000 files waiting" (26).
pub fn full_line(title: &str, waiting: usize) -> String {
    format!("{title}: {waiting} files waiting")
}

/// A place that cannot be watched: "Scans: checking every 30 seconds" (32).
pub fn polled_line(title: &str, every: Duration) -> String {
    format!("{title}: checking every {}", duration_words(every))
}

/// Partial outputs a crash left, listed and never removed: "Found 2
/// partial files in out" (28).
pub fn partial_line(found: usize, folder: &str) -> String {
    format!(
        "Found {} in {folder}",
        count(found as u64, "partial file", "partial files")
    )
}

/// "Put back 2 files from failed? y or n" (36).
pub fn put_back_question(files: usize) -> String {
    if files == 1 {
        "Put back 1 file from failed? y or n".to_owned()
    } else {
        format!("Put back {files} files from failed? y or n")
    }
}

/// How long a run took, in whole seconds: "under a second", "3 seconds",
/// "1 minute 30 seconds".
pub fn took_words(took: Duration) -> String {
    if took < Duration::from_secs(1) {
        "under a second".to_owned()
    } else {
        duration_words(Duration::from_secs(took.as_secs()))
    }
}

/// A file done: "Done: scan-004.pdf, 3 seconds" (29).
pub fn done_line(name: &str, took: Duration) -> String {
    format!("Done: {}, {}", spell_out(name), took_words(took))
}

/// A file failed, its cause as the tool or the system said it:
/// "Failed: scan-004.pdf, unreadable" (32).
pub fn failed_line(name: &str, cause: &str) -> String {
    format!("Failed: {}, {cause}", spell_out(name))
}

/// A file to be tried again: "Retrying scan-004.pdf in 1 minute" (33).
pub fn retry_line(name: &str, wait: Duration) -> String {
    format!("Retrying {} in {}", spell_out(name), duration_words(wait))
}

/// A file that changed while it ran: "Changed during the run:
/// scan-004.pdf" (36). It settles and runs again.
pub fn changed_line(name: &str) -> String {
    format!("Changed during the run: {}", spell_out(name))
}

/// A file passed over: "Skipped scan-004.pdf: output exists" (35).
pub fn skipped_line(name: &str, why: &str) -> String {
    format!("Skipped {}: {why}", spell_out(name))
}

/// A tell task's arrival: "Arrived: scan-004.pdf, 1.2 MB" (29).
pub fn told_line(name: &str, size: Option<u64>) -> String {
    match size {
        Some(size) => format!("Arrived: {}, {}", spell_out(name), human_size(size)),
        None => format!("Arrived: {}", spell_out(name)),
    }
}

/// `diro watch --once`: a file whose retry waits for the next run:
/// "Left for the next run: scan-004.pdf" (35).
pub fn left_line(name: &str) -> String {
    format!("Left for the next run: {}", spell_out(name))
}

/// Moving the original crossed drives: "Moved across drives: scan-004.pdf".
pub fn across_line(name: &str) -> String {
    format!("Moved across drives: {}", spell_out(name))
}

/// The dry run's line for a file that would run: "Would convert
/// scan-004.pdf" (26).
pub fn would_line(verb: &str, name: &str) -> String {
    format!("Would {verb} {}", spell_out(name))
}

/// The dry run's summary: "Dry run: 3 would convert, 1 skipped" (35).
pub fn dry_run_line(verb: &str, would: usize, skipped: usize) -> String {
    format!("Dry run: {would} would {verb}, {skipped} skipped")
}

/// A task's state, said when it changes: "Hot folder Scans is on" (22),
/// "Hot folder Scans is paused" (26).
pub fn state_line(title: &str, on: bool, paused: bool) -> String {
    let state = match (on, paused) {
        (true, true) => "paused",
        (true, false) => "on",
        _ => "off",
    };
    format!("Hot folder {title} is {state}")
}

/// A row of the Watched folders list, its position said after it:
/// "Scans, on, 3 done today, 1 failed" (33).
pub fn list_row(title: &str, on: bool, paused: bool, done: usize, failed: usize) -> String {
    let state = match (on, paused) {
        (true, true) => "paused",
        (true, false) => "on",
        _ => "off",
    };
    let mut row = format!("{title}, {state}");
    if done > 0 || failed > 0 {
        row.push_str(&format!(", {done} done today"));
    }
    if failed > 0 {
        row.push_str(&format!(", {failed} failed"));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    /// R16's and R28's counted lines, each at 40 cells or fewer.
    #[test]
    fn every_line_fits_the_braille_display() {
        let lines = [
            (
                summary_line("Scans", 3, 1, 0),
                "Scans: 3 done, 1 failed",
                23,
            ),
            (summary_line("Scans", 3, 0, 0), "Scans: 3 done", 13),
            (
                summary_line("Scans", 0, 1, 2),
                "Scans: 1 failed, 2 skipped",
                26,
            ),
            (
                paused_line("Scans", "pdftotext is not installed"),
                "Scans paused: pdftotext is not installed",
                40,
            ),
            (
                paused_line("Scans", &breaker_reason(10)),
                "Scans paused: 10 failures in a row",
                34,
            ),
            (
                paused_line("Scans", FOLDER_MISSING),
                "Scans paused: folder is missing",
                31,
            ),
            (
                paused_line("Scans", &space_reason(200_000_000)),
                "Scans paused: only 200 MB free",
                30,
            ),
            (status_line("Scans", 3, 0).unwrap(), "Scans: 3 waiting", 16),
            (
                caught_up_line("Scans", 14),
                "14 files were waiting in Scans",
                30,
            ),
            (
                catch_up_question("Scans", 14),
                "Process the 14 files in Scans? y or n",
                37,
            ),
            (locked_line("Scans"), "Scans is run by another diro", 28),
            (full_line("Scans", 10_000), "Scans: 10000 files waiting", 26),
            (
                polled_line("Scans", Duration::from_secs(30)),
                "Scans: checking every 30 seconds",
                32,
            ),
            (partial_line(2, "out"), "Found 2 partial files in out", 28),
            (
                put_back_question(2),
                "Put back 2 files from failed? y or n",
                36,
            ),
            (
                done_line("scan-004.pdf", Duration::from_secs(3)),
                "Done: scan-004.pdf, 3 seconds",
                29,
            ),
            (
                failed_line("scan-004.pdf", "unreadable"),
                "Failed: scan-004.pdf, unreadable",
                32,
            ),
            (
                retry_line("scan-004.pdf", Duration::from_secs(60)),
                "Retrying scan-004.pdf in 1 minute",
                33,
            ),
            (
                changed_line("scan-004.pdf"),
                "Changed during the run: scan-004.pdf",
                36,
            ),
            (
                skipped_line("scan-004.pdf", "output exists"),
                "Skipped scan-004.pdf: output exists",
                35,
            ),
            (
                told_line("scan-004.pdf", Some(1_200_000)),
                "Arrived: scan-004.pdf, 1.2 MB",
                29,
            ),
            (
                left_line("scan-004.pdf"),
                "Left for the next run: scan-004.pdf",
                35,
            ),
            (
                across_line("scan-004.pdf"),
                "Moved across drives: scan-004.pdf",
                33,
            ),
            (
                would_line("convert", "scan-004.pdf"),
                "Would convert scan-004.pdf",
                26,
            ),
            (
                dry_run_line("convert", 3, 1),
                "Dry run: 3 would convert, 1 skipped",
                35,
            ),
            (
                state_line("Scans", true, false),
                "Hot folder Scans is on",
                22,
            ),
            (
                state_line("Scans", true, true),
                "Hot folder Scans is paused",
                26,
            ),
            (
                state_line("Scans", false, false),
                "Hot folder Scans is off",
                23,
            ),
            (
                list_row("Scans", true, false, 3, 1),
                "Scans, on, 3 done today, 1 failed",
                33,
            ),
            (list_row("Scans", false, false, 0, 0), "Scans, off", 10),
        ];
        for (line, expected, cells) in lines {
            assert_eq!(line, expected);
            assert_eq!(line.chars().count(), cells, "{line}");
            assert!(cells <= 40, "{line}");
        }
        // A name with a hidden character is spelled out, the verdict
        // first (built in memory only).
        let line = failed_line("right\u{202e}fdp.exe", "a password is needed");
        assert_eq!(line, "Failed: right[U+202E]fdp.exe, a password is needed");
        assert!(within_cells(&line, "Failed: right[U+202E]fdp.exe"));
        assert_eq!(caught_up_line("Scans", 1), "1 file was waiting in Scans");
        assert_eq!(
            catch_up_question("Scans", 1),
            "Process the 1 file in Scans? y or n"
        );
    }
}
