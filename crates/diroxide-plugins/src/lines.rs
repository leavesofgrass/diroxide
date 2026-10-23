//! Every sentence the plugin host says, in one place, so each one is
//! checked against the 40-cell rule (R22, section 9): the meaning comes
//! first, words before symbols, and in a line a plugin causes the plugin's
//! name comes last, so it cannot pass for the app.

use std::time::Duration;

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

#[cfg(test)]
mod tests {
    use super::*;

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
