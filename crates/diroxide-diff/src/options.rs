//! What a compare leaves out, how it lines the two files up, and how long
//! it may take.

use std::time::Duration;

/// The differences a compare leaves out. Each one that is on is said in
/// the report ("Ignoring changes in spaces."), so a hidden rule never
/// shapes a result in silence (R18, "Ignore options, in words").
///
/// Space means any character Unicode calls white space, tabs included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ignore {
    /// Line endings: CRLF, LF and CR compare the same. On by default; when
    /// the files' endings differ, the report says so once as a fact about
    /// the files, never as a change on every line.
    pub line_endings: bool,
    /// Space at the end of a line.
    pub trailing_space: bool,
    /// Any change in the amount of space: a run of space equals any other
    /// run, and space at the end of a line is left out (git's
    /// `--ignore-space-change`).
    pub space_amount: bool,
    /// All space: lines compare with every space removed.
    pub all_space: bool,
    /// Changes whose lines are all blank.
    pub blank_lines: bool,
    /// Upper and lower case.
    pub case: bool,
}

impl Ignore {
    /// Nothing ignored: every character of every line counts, its ending
    /// included.
    pub const NOTHING: Ignore = Ignore {
        line_endings: false,
        trailing_space: false,
        space_amount: false,
        all_space: false,
        blank_lines: false,
        case: false,
    };

    /// Whether anything other than line endings is ignored: the options
    /// the ignore note names in every report.
    pub fn beyond_line_endings(&self) -> bool {
        Ignore {
            line_endings: false,
            ..*self
        } != Ignore::NOTHING
    }
}

impl Default for Ignore {
    /// Line endings ignored, everything else compared.
    fn default() -> Self {
        Ignore {
            line_endings: true,
            ..Ignore::NOTHING
        }
    }
}

/// How lines are lined up before the changes are counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineAlgorithm {
    /// Anchor on lines that appear once in each file (patience): fewer
    /// misaligned changes to hear around repeated lines such as closing
    /// braces or blank lines. The default (R18).
    #[default]
    Patience,
    /// Prefer rare lines as anchors (histogram): good for logs and other
    /// text with many repeated lines.
    Histogram,
    /// Close to the fewest changed lines (Myers, as git and GNU diff use
    /// by default), which can split a change around a repeated line.
    Myers,
}

/// How long the line and word passes may run before they settle for a
/// longer list of changes than the least possible: two seconds, R18's
/// starting value for the bench gate to settle.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(2);

/// Everything a text compare takes besides the two texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompareOptions {
    /// What is left out.
    pub ignore: Ignore,
    /// How lines are lined up.
    pub algorithm: LineAlgorithm,
    /// How long a run may take: [`DEFAULT_DEADLINE`].
    pub deadline: Duration,
}

impl Default for CompareOptions {
    fn default() -> Self {
        CompareOptions {
            ignore: Ignore::default(),
            algorithm: LineAlgorithm::default(),
            deadline: DEFAULT_DEADLINE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_alone_are_the_default_and_not_named() {
        let ignore = Ignore::default();
        assert!(ignore.line_endings);
        assert!(!ignore.beyond_line_endings());
        assert!(!Ignore::NOTHING.beyond_line_endings());
        let case = Ignore {
            case: true,
            ..Ignore::NOTHING
        };
        assert!(case.beyond_line_endings());
        assert_eq!(CompareOptions::default().deadline, Duration::from_secs(2));
        assert_eq!(CompareOptions::default().algorithm, LineAlgorithm::Patience);
    }
}
