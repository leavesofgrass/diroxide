//! git's progress lines in words (R29 2.4; spike GS2): with `--progress`,
//! git rewrites a line in place with carriage returns ("Receiving objects:
//!  45% (450/1000), 1.20 MiB | 2.00 MiB/s"), which the host's `split =
//! "cr"` hands over one rewrite at a time. Each becomes a sentence, the
//! job first: "Cloning: receiving objects, 45 percent" (38), said at the
//! host's progress interval.

/// One progress line read: the phase and its percent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The phase in lowercase words: "receiving objects".
    pub phase: String,
    /// Its percent, when the line has one.
    pub percent: Option<u8>,
}

/// Reads one of git's progress lines; `None` for a line that is not one
/// ("Cloning into 'x'...", a warning).
pub fn read(line: &str) -> Option<Step> {
    let line = line.trim();
    let line = line.strip_prefix("remote: ").unwrap_or(line).trim();
    let (phase, rest) = line.split_once(':')?;
    let phase = phase.trim();
    // A phase is two words or more ("Receiving objects"); "warning",
    // "error" and "hint" are one.
    if !phase.contains(' ')
        || phase.len() > 40
        || !phase.chars().all(|c| c.is_ascii_alphabetic() || c == ' ')
    {
        return None;
    }
    let percent = rest.split_whitespace().find_map(|word| {
        let digits = word.strip_suffix('%')?;
        digits.parse::<u8>().ok().filter(|p| *p <= 100)
    });
    Some(Step {
        phase: phase.to_ascii_lowercase(),
        percent,
    })
}

/// The sentence of a step, the job's verb first: "Cloning: receiving
/// objects, 45 percent". A phase whose sentence would pass 40 cells drops
/// its last word ("Cloning: compressing, 100 percent").
pub fn words(verb: &str, step: &Step) -> String {
    let sentence = |phase: &str| match step.percent {
        Some(percent) => format!("{verb}: {phase}, {percent} percent"),
        None => format!("{verb}: {phase}"),
    };
    let whole = sentence(&step.phase);
    if whole.chars().count() <= diroxide_core::format::BRAILLE_CELLS {
        return whole;
    }
    match step.phase.rsplit_once(' ') {
        Some((short, _)) => sentence(short),
        None => whole,
    }
}

/// Keeps the last sentence said, so a rewrite that changes nothing a
/// listener hears is not said again.
#[derive(Debug, Default)]
pub struct Said {
    last: Option<String>,
}

impl Said {
    /// The sentence for a line when it says something new.
    pub fn next(&mut self, verb: &str, line: &str) -> Option<String> {
        let step = read(line)?;
        let sentence = words(verb, &step);
        if self.last.as_deref() == Some(sentence.as_str()) {
            return None;
        }
        self.last = Some(sentence.clone());
        Some(sentence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn clone_progress_reads_as_words() {
        let step = read("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s").unwrap();
        assert_eq!(step.phase, "receiving objects");
        assert_eq!(step.percent, Some(45));
        let line = words("Cloning", &step);
        assert_eq!(line, "Cloning: receiving objects, 45 percent");
        assert_eq!(line.chars().count(), 38);
        assert!(within_cells(&line, "45 percent"));
        assert_eq!(
            words(
                "Fetching",
                &read("remote: Counting objects: 100% (5/5), done.").unwrap()
            ),
            "Fetching: counting objects, 100 percent"
        );
        assert_eq!(
            words("Cloning", &read("Enumerating objects: 5, done.").unwrap()),
            "Cloning: enumerating objects"
        );
        assert_eq!(
            words(
                "Cloning",
                &read("Compressing objects: 100% (5/5), done.").unwrap()
            ),
            "Cloning: compressing, 100 percent"
        );
        assert!(read("Cloning into 'repo'...").is_none());
        assert!(read("warning: you appear to have cloned an empty repository.").is_none());
        assert!(read("").is_none());
    }

    #[test]
    fn a_rewrite_that_says_nothing_new_is_not_said_again() {
        let mut said = Said::default();
        assert!(
            said.next("Cloning", "Receiving objects:  45% (450/1000)")
                .is_some()
        );
        assert!(
            said.next("Cloning", "Receiving objects:  45% (451/1000), 1 MiB")
                .is_none()
        );
        assert!(
            said.next("Cloning", "Receiving objects:  46% (460/1000)")
                .is_some()
        );
        assert!(said.next("Cloning", "Cloning into 'x'...").is_none());
    }
}
