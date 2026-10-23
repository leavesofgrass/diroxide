//! Every sentence a script command causes, in one place, each checked
//! against the 40-cell rule (R22 section 9): the meaning first, words
//! before symbols, and in a line that carries a script's or a plugin's
//! own text, its name last ("By Total size of selection.").
//!
//! The exit codes (R22 section 2.6): 0 done; 1 failed with nothing
//! changed; 2 wrong use; 3 partly done, the output saying which; 64 to
//! 113 free for a command's own meanings, said through `exit_words`; the
//! host's own sentences for 126, 127, a cancel and a timeout.

use std::collections::BTreeMap;
use std::time::Duration;

use diroxide_core::Message;
use diroxide_core::format::grouped;

use crate::lines;
use crate::services::{attribute, clean_text};

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The program exited with a code.
    Exited(i32),
    /// The program was ended by a signal (Linux and macOS).
    Signal,
    /// The owner cancelled.
    Cancelled,
    /// The command ran past its timeout.
    TimedOut(Duration),
    /// The program could not be started; the system's words.
    NotStarted(String),
}

impl Ending {
    /// Whether the run did what it was asked.
    pub fn is_done(&self) -> bool {
        matches!(self, Ending::Exited(0))
    }

    /// The exit code `diro plugin run` passes on: 0 done, 1 failed, 2
    /// wrong use, 3 partly done; a cancel or a timeout is 1.
    pub fn exit_code(&self) -> i32 {
        match self {
            Ending::Exited(code @ 0..=3) => *code,
            _ => 1,
        }
    }
}

/// The lines said when a run ends: the first is the verdict; a failure
/// adds the last error line. Empty for a run that ended well and said
/// its own words.
pub fn ending_lines(
    title: &str,
    source: &str,
    ending: &Ending,
    exit_words: &BTreeMap<i32, String>,
    last_error: Option<&str>,
) -> Vec<Message> {
    if let Ending::Exited(code) = ending
        && let Some(words) = exit_words.get(code)
    {
        let words = attribute(&clean_text(words, 200), source);
        return vec![if *code == 0 {
            Message::done(words)
        } else {
            Message::error(words)
        }];
    }
    let first = match ending {
        Ending::Exited(0) => return vec![Message::done(format!("{title} done."))],
        Ending::Exited(1) => Message::error(lines::could_not_run(title, "code 1, nothing changed")),
        Ending::Exited(2) => Message::error(lines::could_not_run(title, "code 2, wrong use")),
        Ending::Exited(3) => Message::warning(format!("{title} partly done: code 3.")),
        Ending::Exited(126) => Message::error(lines::could_not_run(title, "it is not executable")),
        Ending::Exited(127) => {
            Message::error(lines::could_not_run(title, "a program was not found"))
        }
        Ending::Exited(code) => {
            Message::error(lines::could_not_run(title, &format!("code {code}")))
        }
        Ending::Signal => Message::error(lines::could_not_run(title, "it was stopped by a signal")),
        Ending::Cancelled => return vec![Message::status(lines::cancelled(title))],
        Ending::TimedOut(after) => Message::error(lines::timed_out(title, *after)),
        Ending::NotStarted(cause) => {
            Message::error(lines::could_not_run(title, &clean_text(cause, 120)))
        }
    };
    let mut said = vec![first];
    if let Some(cause) = last_error
        .map(|e| clean_text(e, 120))
        .filter(|e| !e.is_empty())
    {
        said.push(Message::info(lines::last_error(&cause)));
    }
    said
}

/// A lister's title, the count first: "214 results for report, from
/// Everything".
pub fn results_title(count: usize, query: &str, from: &str) -> String {
    let count = grouped(count as u64);
    let query = clean_text(query, 60);
    match (query.is_empty(), count.as_str()) {
        (true, "1") => format!("1 result, from {from}"),
        (true, _) => format!("{count} results, from {from}"),
        (false, "1") => format!("1 result for {query}, from {from}"),
        (false, _) => format!("{count} results for {query}, from {from}"),
    }
}

/// No rows: "No results for report."
pub fn no_results(query: &str) -> String {
    let query = clean_text(query, 60);
    if query.is_empty() {
        "No results.".to_owned()
    } else {
        format!("No results for {query}.")
    }
}

/// The cap: "First 5,000 shown, narrow the search."
pub fn first_shown(max_rows: u32) -> String {
    format!(
        "First {} shown, narrow the search.",
        grouped(u64::from(max_rows))
    )
}

/// A row whose place has gone since the list was made: "Missing,
/// report-old.pdf".
pub fn missing_row(name: &str) -> String {
    format!("Missing, {name}")
}

/// A launch: "Started meld."
pub fn started(program: &str) -> String {
    format!("Started {program}.")
}

/// A long list of files in batches: "Running in 3 batches."
pub fn in_batches(count: usize) -> String {
    format!("Running in {count} batches.")
}

/// A tool a command needs, missing: "Needs es, not found."
pub fn needs_tool(tool: &str) -> String {
    format!("Needs {tool}, not found.")
}

/// A command shown disabled: "Search with Everything, disabled".
pub fn disabled(title: &str) -> String {
    format!("{title}, disabled")
}

/// The question before a terminal-wait command returns: "Press Enter to
/// go back."
pub fn press_enter() -> String {
    "Press Enter to go back.".to_owned()
}

/// Output cut at the cap: "Output cut at 1 MB."
pub fn output_cut() -> String {
    "Output cut at 1 MB.".to_owned()
}

/// A command not made for this system: "Search with Spotlight is not for
/// this system."
pub fn other_system(title: &str) -> String {
    format!("{title} is not for this system.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    fn texts(lines: Vec<Message>) -> Vec<String> {
        lines.into_iter().map(|m| m.text).collect()
    }

    #[test]
    fn exit_codes_in_words() {
        let none = BTreeMap::new();
        let say = |ending: Ending, last: Option<&str>| {
            texts(ending_lines(
                "Count lines",
                "Line counter",
                &ending,
                &none,
                last,
            ))
        };
        assert_eq!(say(Ending::Exited(0), None), ["Count lines done."]);
        assert_eq!(
            say(Ending::Exited(1), Some("cannot read a.txt")),
            [
                "Could not run Count lines: code 1, nothing changed.",
                "Last error: cannot read a.txt."
            ]
        );
        assert_eq!(
            say(Ending::Exited(2), None),
            ["Could not run Count lines: code 2, wrong use."]
        );
        assert_eq!(
            say(Ending::Exited(3), None),
            ["Count lines partly done: code 3."]
        );
        assert_eq!(
            say(Ending::Exited(126), None),
            ["Could not run Count lines: it is not executable."]
        );
        assert_eq!(
            say(Ending::Exited(127), None),
            ["Could not run Count lines: a program was not found."]
        );
        assert_eq!(
            say(Ending::Exited(70), None),
            ["Could not run Count lines: code 70."]
        );
        assert_eq!(
            say(Ending::Cancelled, Some("ignored")),
            ["Count lines: cancelled."]
        );
        assert_eq!(
            say(Ending::TimedOut(Duration::from_secs(30)), None),
            ["Count lines timed out after 30 seconds."]
        );
        assert_eq!(
            say(Ending::Signal, None),
            ["Could not run Count lines: it was stopped by a signal."]
        );
    }

    #[test]
    fn a_commands_own_exit_words_carry_its_name_last() {
        let mut words = BTreeMap::new();
        words.insert(
            8,
            "Everything is not running. Start it, then try again.".to_owned(),
        );
        let said = texts(ending_lines(
            "Search with Everything",
            "Search the whole computer",
            &Ending::Exited(8),
            &words,
            None,
        ));
        assert_eq!(
            said,
            ["Everything is not running. Start it, then try again. By Search the whole computer."]
        );
        assert!(diroxide_core::format::within_cells(
            &said[0],
            "Everything is not running."
        ));
    }

    #[test]
    fn exit_codes_pass_on() {
        assert_eq!(Ending::Exited(0).exit_code(), 0);
        assert_eq!(Ending::Exited(3).exit_code(), 3);
        assert_eq!(Ending::Exited(70).exit_code(), 1);
        assert_eq!(Ending::Cancelled.exit_code(), 1);
    }

    #[test]
    fn the_lines_of_section_nine_fit_one_braille_line() {
        let golden = [
            (
                results_title(214, "report", "Everything"),
                "214 results for report, from Everything",
            ),
            (no_results("report"), "No results for report."),
            (first_shown(5000), "First 5,000 shown, narrow the search."),
            (missing_row("report-old.pdf"), "Missing, report-old.pdf"),
            (needs_tool("es"), "Needs es, not found."),
            (
                disabled("Search with Everything"),
                "Search with Everything, disabled",
            ),
            (started("meld"), "Started meld."),
            (in_batches(3), "Running in 3 batches."),
            (press_enter(), "Press Enter to go back."),
            (output_cut(), "Output cut at 1 MB."),
        ];
        for (made, expected) in &golden {
            assert_eq!(made, expected);
            assert!(made.chars().count() <= BRAILLE_CELLS, "{made}");
        }
        assert_eq!(results_title(1, "", "plocate"), "1 result, from plocate");
    }
}
