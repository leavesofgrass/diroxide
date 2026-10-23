//! Every sentence the plugin host says, in one place, so each one is
//! checked against the 40-cell rule (R22, section 9): the meaning comes
//! first, words before symbols, and in a line a plugin causes the plugin's
//! name comes last, so it cannot pass for the app.

use std::time::Duration;

use diroxide_core::format::human_size;

/// One line of the owner's Braille display, in cells: a line's meaning
/// must be whole within it.
pub const MAX_LINE_CELLS: usize = 40;

/// A plugin's dialog title, its name last so a question never sounds like
/// diroxide's own: "Trust this host? (SFTP places)"; with no title of its
/// own, "SFTP places asks".
pub fn dialog_title(title: &str, plugin: &str) -> String {
    if title.trim().is_empty() {
        format!("{plugin} asks")
    } else {
        with_source(title, plugin)
    }
}

/// A plugin's text document's title, its name last: "Diff of a.txt
/// (Git)"; with no title of its own, "Text (Git)".
pub fn document_title(title: &str, plugin: &str) -> String {
    if title.trim().is_empty() {
        with_source("Text", plugin)
    } else {
        with_source(title, plugin)
    }
}

/// The title of the host's own question before a plugin replaces a
/// file: "Replace a file? (Line counter)".
pub fn replace_title(plugin: &str) -> String {
    with_source("Replace a file?", plugin)
}

/// The lines of that question, meaning first: "Replace report.txt?" and
/// "It cannot be undone."
pub fn replace_lines(name: &str) -> Vec<String> {
    vec![
        format!("Replace {name}?"),
        "It cannot be undone.".to_owned(),
    ]
}

/// A job a plugin's hook stopped: "Copy stopped: keep.txt is protected
/// (Guard)".
pub fn hook_cancelled(verb: &str, reason: &str, plugin: &str) -> String {
    let reason = if reason.trim().is_empty() {
        "no reason given"
    } else {
        reason
    };
    with_source(&format!("{verb} stopped: {reason}"), plugin)
}

/// A hook that did not answer: "Copy goes on: no answer (Guard)", or,
/// when its manifest says silence cancels, "Copy stopped: no answer
/// (Guard)".
pub fn hook_silent(verb: &str, cancels: bool, plugin: &str) -> String {
    let what = if cancels { "stopped" } else { "goes on" };
    with_source(&format!("{verb} {what}: no answer"), plugin)
}

/// A file command on a lister's row: "This row is not a file."
pub fn not_a_file() -> String {
    "This row is not a file.".to_owned()
}

/// A lister cut at its cap: "First 5,000 shown, narrow the search."
pub fn lister_capped() -> String {
    "First 5,000 shown, narrow the search.".to_owned()
}

/// A column value not answered yet: "pending", never an empty cell.
pub fn pending() -> String {
    "pending".to_owned()
}

/// A provider's lost connection when the plugin gave no reason.
pub fn connection_lost() -> String {
    "Connection lost".to_owned()
}

/// What a text prompt says when the text does not fit the plugin's
/// pattern and the plugin gave no words of its own.
pub fn text_does_not_fit() -> String {
    "That text is not accepted here.".to_owned()
}

/// A completion that has not answered yet: "Searching for candidates".
pub fn searching() -> String {
    "Searching for candidates".to_owned()
}

/// A duration in words: "10 seconds", "1 second", "2 minutes", "under a
/// second".
pub fn duration_words(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0 => "under a second".to_owned(),
        1 => "1 second".to_owned(),
        s if s % 60 == 0 && s >= 120 => format!("{} minutes", s / 60),
        60 => "1 minute".to_owned(),
        s => format!("{s} seconds"),
    }
}

/// "Could not start Line counter: CAUSE."
pub fn could_not_start(plugin: &str, cause: &str) -> String {
    format!("Could not start {plugin}: {}.", cause.trim_end_matches('.'))
}

/// "Line counter did not start in 10 seconds."
pub fn did_not_start(plugin: &str, timeout: Duration) -> String {
    format!("{plugin} did not start in {}.", duration_words(timeout))
}

/// "Line counter stopped while starting."
pub fn stopped_while_starting(plugin: &str) -> String {
    format!("{plugin} stopped while starting.")
}

/// The refusal of a plugin of another protocol: "Line counter needs a
/// newer diroxide: it speaks protocol 2." or "Line counter is too old for
/// this diroxide: it speaks protocol 0."
pub fn other_protocol(plugin: &str, theirs: u32) -> String {
    if theirs > crate::protocol::PROTOCOL {
        format!("{plugin} needs a newer diroxide: it speaks protocol {theirs}.")
    } else {
        format!("{plugin} is too old for this diroxide: it speaks protocol {theirs}.")
    }
}

/// "Plugins are off for this session." (`--no-plugins`)
pub fn plugins_off() -> String {
    "Plugins are off for this session.".to_owned()
}

/// "Line counter is off: it did not start twice."
pub fn off_after_failed_starts(plugin: &str) -> String {
    format!("{plugin} is off: it did not start twice.")
}

/// "Line counter is off: it stopped 3 times."
pub fn off_after_crashes(plugin: &str, times: usize) -> String {
    format!("{plugin} is off: it stopped {times} times.")
}

/// "Line counter stopped unexpectedly."
pub fn stopped_unexpectedly(plugin: &str) -> String {
    format!("{plugin} stopped unexpectedly.")
}

/// "It starts again when you use it."
pub fn starts_again() -> String {
    "It starts again when you use it.".to_owned()
}

/// "Line counter was stopped for flooding."
pub fn flooded(plugin: &str) -> String {
    format!("{plugin} was stopped for flooding.")
}

/// "Count lines timed out after 30 seconds.", with a command's name or
/// the plugin's.
pub fn timed_out(what: &str, after: Duration) -> String {
    format!("{what} timed out after {}.", duration_words(after))
}

/// Words with their source last, in brackets: "Counting 12 of 300 files
/// (Line counter)". A final period of the words is dropped.
pub fn with_source(text: &str, plugin: &str) -> String {
    format!("{} ({plugin})", text.trim().trim_end_matches('.'))
}

/// A progress line: "Counting 12 of 300 files (Line counter)".
pub fn progress_line(text: &str, plugin: &str) -> String {
    with_source(text, plugin)
}

/// The start of a job: "Count lines: started (Line counter)".
pub fn progress_started(title: &str, plugin: &str) -> String {
    with_source(&format!("{title}: started"), plugin)
}

/// The end of a job with no words of its own: "Count lines done (Line
/// counter)".
pub fn progress_done(title: &str, plugin: &str) -> String {
    with_source(&format!("{title} done"), plugin)
}

/// The end of a command with a count: "Count lines done: 3 files (Line
/// counter)".
pub fn command_done(command: &str, summary: &str, plugin: &str) -> String {
    with_source(&format!("{command} done: {summary}"), plugin)
}

/// A request that waits for the owner, when the plugin gave no words:
/// "Copy to phone: waiting for you".
pub fn waiting_for_you(title: &str) -> String {
    if title.is_empty() {
        "Waiting for you".to_owned()
    } else {
        format!("{title}: waiting for you")
    }
}

/// A request that has waited two seconds: "Count lines: waiting for the
/// plugin".
pub fn waiting_for_plugin(what: &str) -> String {
    format!("{what}: waiting for the plugin")
}

/// A command the owner cancelled: "Count lines: cancelled."
pub fn cancelled(command: &str) -> String {
    format!("{command}: cancelled.")
}

/// A command that failed, in the standard form: "Could not run Count
/// lines: code 2."
pub fn could_not_run(command: &str, cause: &str) -> String {
    format!("Could not run {command}: {}.", cause.trim_end_matches('.'))
}

/// The cause's line under a failure: "Last error: cannot read a.txt."
pub fn last_error(cause: &str) -> String {
    format!("Last error: {}.", cause.trim_end_matches('.'))
}

/// The next step under a failure: "Next: diro plugin log line-counter".
pub fn next_log(plugin_id: &str) -> String {
    format!("Next: diro plugin log {plugin_id}")
}

/// Announcements held back by the rate limit: "12 more messages from
/// Line counter".
pub fn folded(count: usize, plugin: &str) -> String {
    if count == 1 {
        format!("1 more message from {plugin}")
    } else {
        format!("{count} more messages from {plugin}")
    }
}

/// A call the manifest does not allow, in two lines: "Line counter may
/// not do that." and "Its manifest does not allow it."
pub fn refused_by_manifest(plugin: &str) -> [String; 2] {
    [
        format!("{plugin} may not do that."),
        "Its manifest does not allow it.".to_owned(),
    ]
}

/// Stray output, said once a session: "Line counter wrote stray text; see
/// its log."
pub fn wrote_stray_text(plugin: &str) -> String {
    format!("{plugin} wrote stray text; see its log.")
}

/// Why a provider refused an other side it could not verify, for the
/// provider error: the plugin's words, then its hint, the next step said
/// and never run: "Host key changed. Not connected. To forget the old
/// key, run: ssh-keygen -R build.example".
pub fn untrusted_why(message: &str, hint: Option<&str>) -> String {
    let message = message.trim().trim_end_matches('.');
    let hint: Vec<&str> = hint
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if hint.is_empty() {
        format!("{message}.")
    } else {
        format!("{message}. {}", hint.join(" "))
    }
}

/// The same refusal as lines for a dialog or the messages list, one fact
/// a line: the plugin's words with its name last, then each line of its
/// hint ("Host key changed. Not connected (SFTP places)", "To forget the
/// old key, run:", "ssh-keygen -R build.example").
pub fn untrusted_lines(message: &str, hint: Option<&str>, plugin: &str) -> Vec<String> {
    let mut out = vec![with_source(message, plugin)];
    out.extend(
        hint.unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned),
    );
    out
}

/// A volume's row, meaning first: "Home on build.example, 12.0 GB free of
/// 50.0 GB"; a number the provider does not know is said so, never read as
/// zero: "Home on build.example, free space not known".
pub fn volume_row(name: &str, free: Option<u64>, total: Option<u64>) -> String {
    match (free, total) {
        (Some(free), Some(total)) => {
            format!("{name}, {} free of {}", human_size(free), human_size(total))
        }
        (Some(free), None) => format!("{name}, {} free", human_size(free)),
        (None, Some(total)) => format!("{name}, {} in all, free not known", human_size(total)),
        (None, None) => format!("{name}, free space not known"),
    }
}

/// A place a plugin offers, its state right after its name: "build,
/// saved, alice@build.example"; "nas, from ssh config".
pub fn place_row(name: &str, state: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{name}, {state}, {detail}"),
        None => format!("{name}, {state}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line of R22's section 9 that the host makes, with its example
    /// names: the text, and its meaning whole within one Braille line.
    #[test]
    fn the_lines_of_section_nine() {
        let plugin = "Line counter";
        let command = "Count lines";
        let golden: Vec<(String, &str)> = vec![
            (
                progress_started(command, plugin),
                "Count lines: started (Line counter)",
            ),
            (
                progress_line("Counting 12 of 300 files", plugin),
                "Counting 12 of 300 files (Line counter)",
            ),
            (
                waiting_for_plugin(command),
                "Count lines: waiting for the plugin",
            ),
            (
                command_done(command, "3 files", plugin),
                "Count lines done: 3 files (Line counter)",
            ),
            (cancelled(command), "Count lines: cancelled."),
            (
                could_not_run(command, "code 2"),
                "Could not run Count lines: code 2.",
            ),
            (
                last_error("cannot read a.txt"),
                "Last error: cannot read a.txt.",
            ),
            (
                next_log("line-counter"),
                "Next: diro plugin log line-counter",
            ),
            (
                timed_out(command, Duration::from_secs(30)),
                "Count lines timed out after 30 seconds.",
            ),
            (
                stopped_unexpectedly(plugin),
                "Line counter stopped unexpectedly.",
            ),
            (starts_again(), "It starts again when you use it."),
            (
                off_after_crashes(plugin, 3),
                "Line counter is off: it stopped 3 times.",
            ),
            (flooded(plugin), "Line counter was stopped for flooding."),
            (
                refused_by_manifest(plugin)[0].clone(),
                "Line counter may not do that.",
            ),
            (
                refused_by_manifest(plugin)[1].clone(),
                "Its manifest does not allow it.",
            ),
            (plugins_off(), "Plugins are off for this session."),
            (folded(12, plugin), "12 more messages from Line counter"),
            (
                did_not_start(plugin, Duration::from_secs(10)),
                "Line counter did not start in 10 seconds.",
            ),
            (
                wrote_stray_text(plugin),
                "Line counter wrote stray text; see its log.",
            ),
        ];
        for (made, expected) in &golden {
            assert_eq!(made, expected);
        }
        // R22 counted every one of its lines at 40 cells or fewer; the
        // two longer ones have their point whole by cell 40.
        for (made, _) in &golden {
            let cells = made.chars().count();
            let long_ok = made.starts_with("Line counter did not start")
                || made.starts_with("Line counter wrote stray text");
            assert!(cells <= 40 || long_ok, "{made} is {cells} cells");
        }
    }

    /// The provider additions of R25's commit 46: a refusal with its hint,
    /// the volumes and the places, golden and whole within 40 cells where
    /// their meaning is.
    #[test]
    fn the_provider_lines_of_commit_46() {
        let hint = "To forget the old key, run:\nssh-keygen -R build.example";
        assert_eq!(
            untrusted_why("Host key changed. Not connected.", Some(hint)),
            "Host key changed. Not connected. To forget the old key, run: ssh-keygen -R build.example"
        );
        assert_eq!(
            untrusted_why("the host key changed", None),
            "the host key changed."
        );
        let rows = untrusted_lines(
            "Host key changed. Not connected.",
            Some(hint),
            "SFTP places",
        );
        assert_eq!(
            rows,
            [
                "Host key changed. Not connected (SFTP places)",
                "To forget the old key, run:",
                "ssh-keygen -R build.example",
            ]
        );
        assert!(
            rows[0]
                .chars()
                .take(40)
                .collect::<String>()
                .contains("Not connected")
        );
        assert!(rows[1..].iter().all(|r| r.chars().count() <= 40));
        assert_eq!(
            volume_row(
                "Home on build.example",
                Some(12_000_000_000),
                Some(50_000_000_000)
            ),
            "Home on build.example, 12.0 GB free of 50.0 GB"
        );
        assert_eq!(
            volume_row("Home on build.example", None, None),
            "Home on build.example, free space not known"
        );
        assert_eq!(volume_row("Box", Some(512), None), "Box, 512 bytes free");
        assert_eq!(
            volume_row("Box", None, Some(2_000)),
            "Box, 2.0 KB in all, free not known"
        );
        let place = place_row("build", "saved", Some("alice@build.example"));
        assert_eq!(place, "build, saved, alice@build.example");
        assert!(place.chars().count() <= 40);
        assert_eq!(
            place_row("nas", "from ssh config", None),
            "nas, from ssh config"
        );
    }

    #[test]
    fn one_folded_message_is_singular() {
        assert_eq!(folded(1, "X"), "1 more message from X");
    }

    #[test]
    fn durations_read_as_words() {
        assert_eq!(duration_words(Duration::from_millis(150)), "under a second");
        assert_eq!(duration_words(Duration::from_secs(1)), "1 second");
        assert_eq!(duration_words(Duration::from_secs(30)), "30 seconds");
        assert_eq!(duration_words(Duration::from_secs(60)), "1 minute");
        assert_eq!(duration_words(Duration::from_secs(90)), "90 seconds");
        assert_eq!(duration_words(Duration::from_secs(300)), "5 minutes");
    }

    #[test]
    fn start_lines_fit_one_braille_line() {
        for line in [
            did_not_start("Line counter", Duration::from_secs(10)),
            stopped_while_starting("Line counter"),
            other_protocol("Line counter", 2),
        ] {
            let head: String = line.chars().take(40).collect();
            assert!(head.starts_with("Line counter "), "{line}");
        }
        assert_eq!(
            did_not_start("Line counter", Duration::from_secs(10)),
            "Line counter did not start in 10 seconds."
        );
        assert_eq!(
            could_not_start("Line counter", "no module named json."),
            "Could not start Line counter: no module named json."
        );
    }
}
