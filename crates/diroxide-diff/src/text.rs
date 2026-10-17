//! Comparing two texts line by line: each line becomes a key under the
//! ignore options, the keys are lined up by the engine, and every
//! stretch that differs becomes one [`Change`].

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use diroxide_vfs::CancelToken;

use crate::change::{Change, Counts};
use crate::engine;
use crate::error::DiffError;
use crate::lines::{LineEnding, Text};
use crate::options::{CompareOptions, Ignore};

/// The result of comparing two texts: the changes in order, their counts,
/// and what shaped them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextComparison {
    before: Text,
    after: Text,
    segments: Vec<Segment>,
    changes: Vec<Change>,
    counts: Counts,
    ignore: Ignore,
    ignored_lines: usize,
    stopped_early: bool,
}

/// What a stretch of the alignment is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    /// The same lines on both sides, pair by pair, under the options.
    Same,
    /// Blank lines only, left out under the blank-lines option.
    Blank,
    /// The change of this index in the change list.
    Change(usize),
}

/// One stretch of the alignment, kept so that same lines can be paired
/// (the unified view's context, the endings in [`TextComparison::apply_to_before`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) kind: SegmentKind,
    pub(crate) before: Range<usize>,
    pub(crate) after: Range<usize>,
}

impl Segment {
    fn swapped(&self) -> Segment {
        Segment {
            kind: self.kind,
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// Compares two texts line by line under `options`, honoring `cancel`
/// and the deadline in the options.
///
/// A run that reaches the deadline still returns a correct list of
/// changes, perhaps longer than the least possible, and
/// [`TextComparison::stopped_early`] says so. A cancelled run returns
/// [`DiffError::Cancelled`].
pub fn compare_text(
    before: &str,
    after: &str,
    options: &CompareOptions,
    cancel: &CancelToken,
) -> Result<TextComparison, DiffError> {
    let deadline = Instant::now().checked_add(options.deadline);
    if cancel.is_cancelled() {
        return Err(DiffError::Cancelled);
    }
    let before = Text::new(before.to_owned());
    let after = Text::new(after.to_owned());
    let ignore = options.ignore;
    let (before_ids, after_ids) = {
        let mut interner = Interner::default();
        let before_ids: Vec<u32> = before
            .iter()
            .map(|(line, ending)| interner.id(line_key(line, ending, &ignore)))
            .collect();
        let after_ids: Vec<u32> = after
            .iter()
            .map(|(line, ending)| interner.id(line_key(line, ending, &ignore)))
            .collect();
        (before_ids, after_ids)
    };
    let stretches = engine::align(&before_ids, &after_ids, options.algorithm, deadline, cancel)
        .map_err(|_| DiffError::Cancelled)?;
    let mut changes = Vec::new();
    let mut segments = Vec::with_capacity(stretches.len());
    let mut ignored_lines = 0;
    for stretch in stretches {
        let kind = if stretch.same {
            ignored_lines += stretch
                .before
                .clone()
                .zip(stretch.after.clone())
                .filter(|&(b, a)| before.line(b) != after.line(a))
                .count();
            SegmentKind::Same
        } else if ignore.blank_lines
            && stretch.before.clone().all(|i| is_blank(before.line(i)))
            && stretch.after.clone().all(|i| is_blank(after.line(i)))
        {
            ignored_lines += stretch.before.len() + stretch.after.len();
            SegmentKind::Blank
        } else {
            changes.push(Change::between(
                stretch.before.clone(),
                stretch.after.clone(),
            ));
            SegmentKind::Change(changes.len() - 1)
        };
        segments.push(Segment {
            kind,
            before: stretch.before,
            after: stretch.after,
        });
    }
    let stopped_early =
        !changes.is_empty() && deadline.is_some_and(|deadline| Instant::now() > deadline);
    if cancel.is_cancelled() {
        return Err(DiffError::Cancelled);
    }
    Ok(TextComparison {
        counts: Counts::of(&changes),
        before,
        after,
        segments,
        changes,
        ignore,
        ignored_lines,
        stopped_early,
    })
}

impl TextComparison {
    /// The changes, in the order of the files.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// The counts of the changes.
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Whether the texts are the same under the ignore options.
    pub fn is_same(&self) -> bool {
        self.changes.is_empty()
    }

    /// What was left out.
    pub fn ignore(&self) -> Ignore {
        self.ignore
    }

    /// How many lines differ only in what was ignored (space, case, blank
    /// lines); differences in line endings alone are a fact about the
    /// files, not counted here.
    pub fn ignored_lines(&self) -> usize {
        self.ignored_lines
    }

    /// Whether the run reached its deadline: the changes are correct but
    /// may be more than the least possible, and the report says so.
    pub fn stopped_early(&self) -> bool {
        self.stopped_early
    }

    /// How many lines the before text has.
    pub fn before_lines(&self) -> usize {
        self.before.len()
    }

    /// How many lines the after text has.
    pub fn after_lines(&self) -> usize {
        self.after.len()
    }

    /// A line of the before text without its ending, counted from 0.
    pub fn before_line(&self, index: usize) -> Option<&str> {
        self.before.line(index)
    }

    /// A line of the after text without its ending, counted from 0.
    pub fn after_line(&self, index: usize) -> Option<&str> {
        self.after.line(index)
    }

    /// How a line of the before text ends.
    pub fn before_ending(&self, index: usize) -> Option<LineEnding> {
        self.before.ending(index)
    }

    /// How a line of the after text ends.
    pub fn after_ending(&self, index: usize) -> Option<LineEnding> {
        self.after.ending(index)
    }

    /// The before text, whole.
    pub fn before_text(&self) -> &str {
        self.before.as_str()
    }

    /// The after text, whole.
    pub fn after_text(&self) -> &str {
        self.after.as_str()
    }

    /// The same comparison read the other way (Swap): before and after
    /// exchanged, every addition a removal and the other way about, with
    /// the same number of changes. Nothing is compared again.
    pub fn swapped(&self) -> TextComparison {
        TextComparison {
            before: self.after.clone(),
            after: self.before.clone(),
            segments: self.segments.iter().map(Segment::swapped).collect(),
            changes: self.changes.iter().map(Change::swapped).collect(),
            counts: self.counts.swapped(),
            ignore: self.ignore,
            ignored_lines: self.ignored_lines,
            stopped_early: self.stopped_early,
        }
    }

    /// The before text with every change applied: the after text, when
    /// nothing was ignored, and otherwise the after text as the options
    /// see it. Same lines keep the before file's characters with the after
    /// file's line endings; blank lines left out take the after file's.
    /// The check that the change list is complete.
    pub fn apply_to_before(&self) -> String {
        let mut out = String::with_capacity(self.after.as_str().len());
        for segment in &self.segments {
            match segment.kind {
                SegmentKind::Same => {
                    for (b, a) in segment.before.clone().zip(segment.after.clone()) {
                        out.push_str(self.before.line(b).unwrap_or_default());
                        out.push_str(self.after.ending(a).unwrap_or(LineEnding::None).as_str());
                    }
                }
                // Blank lines left out take the after file's lines, as a
                // change does, so the text keeps the after file's breaks.
                SegmentKind::Blank | SegmentKind::Change(_) => {
                    for index in segment.after.clone() {
                        out.push_str(self.after.raw(index));
                    }
                }
            }
        }
        out
    }
}

/// Gives each distinct key one small number, so the engine compares
/// numbers instead of strings.
#[derive(Default)]
struct Interner<'a> {
    ids: HashMap<Cow<'a, str>, u32>,
}

impl<'a> Interner<'a> {
    fn id(&mut self, key: Cow<'a, str>) -> u32 {
        let next = u32::try_from(self.ids.len()).unwrap_or(u32::MAX);
        *self.ids.entry(key).or_insert(next)
    }
}

/// Whether a line holds nothing but space.
fn is_blank(line: Option<&str>) -> bool {
    line.is_none_or(|line| line.chars().all(char::is_whitespace))
}

/// A line as the compare sees it under the ignore options.
pub(crate) fn line_key<'a>(line: &'a str, ending: LineEnding, ignore: &Ignore) -> Cow<'a, str> {
    let mut key: Cow<'a, str> = if ignore.all_space {
        Cow::Owned(line.chars().filter(|c| !c.is_whitespace()).collect())
    } else if ignore.space_amount {
        Cow::Owned(collapse_space(line))
    } else if ignore.trailing_space {
        Cow::Borrowed(line.trim_end())
    } else {
        Cow::Borrowed(line)
    };
    if ignore.case {
        key = Cow::Owned(key.to_lowercase());
    }
    if !ignore.line_endings && ending != LineEnding::None {
        // A line never holds a line break, so the ending's own characters
        // mark it without any clash.
        key.to_mut().push_str(ending.as_str());
    }
    key
}

/// Every run of space as one space, and none at the end.
fn collapse_space(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_space = false;
    for c in line.trim_end().chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::change::ChangeKind;

    fn compare(before: &str, after: &str, ignore: Ignore) -> TextComparison {
        let options = CompareOptions {
            ignore,
            ..CompareOptions::default()
        };
        compare_text(before, after, &options, &CancelToken::new()).unwrap()
    }

    fn kinds(result: &TextComparison) -> Vec<(ChangeKind, usize, usize)> {
        result
            .changes()
            .iter()
            .map(|c| (c.kind, c.first_line(), c.last_line()))
            .collect()
    }

    #[test]
    fn the_same_text_has_no_changes() {
        let result = compare("a\nb\n", "a\nb\n", Ignore::NOTHING);
        assert!(result.is_same());
        assert_eq!(result.counts(), Counts::default());
        assert!(compare("", "", Ignore::NOTHING).is_same());
    }

    #[test]
    fn changes_are_found_and_numbered() {
        let before = "one\ntwo\nthree\nfour\n";
        let after = "one\n2\nthree\nfour\nfive\nsix\n";
        let result = compare(before, after, Ignore::default());
        assert_eq!(
            kinds(&result),
            [(ChangeKind::Changed, 2, 2), (ChangeKind::Added, 5, 6)]
        );
        let removed = compare("a\nb\nc\n", "a\nc\n", Ignore::default());
        assert_eq!(kinds(&removed), [(ChangeKind::Removed, 2, 2)]);
        assert_eq!(compare("", "x\ny", Ignore::NOTHING).counts().lines_added, 2);
    }

    #[test]
    fn each_ignore_option_leaves_its_difference_out() {
        let cases = [
            (
                "a \n",
                "a\n",
                Ignore {
                    trailing_space: true,
                    ..Ignore::default()
                },
            ),
            (
                "a  b\n",
                "a\tb \n",
                Ignore {
                    space_amount: true,
                    ..Ignore::default()
                },
            ),
            (
                "a b\n",
                "ab\n",
                Ignore {
                    all_space: true,
                    ..Ignore::default()
                },
            ),
            (
                "Total\n",
                "total\n",
                Ignore {
                    case: true,
                    ..Ignore::default()
                },
            ),
            (
                "a\nb\n",
                "a\n\n  \nb\n",
                Ignore {
                    blank_lines: true,
                    ..Ignore::default()
                },
            ),
            ("a\r\nb\r\n", "a\nb\n", Ignore::default()),
        ];
        for (before, after, ignore) in cases {
            let result = compare(before, after, ignore);
            assert!(result.is_same(), "{before:?} against {after:?}");
            assert!(
                !compare(before, after, Ignore::NOTHING).is_same(),
                "{before:?} against {after:?} with nothing ignored"
            );
        }
    }

    #[test]
    fn lines_that_differ_only_in_what_was_ignored_are_counted() {
        let ignore = Ignore {
            case: true,
            ..Ignore::default()
        };
        let result = compare("A\nb\nC\n", "a\nb\nc\n", ignore);
        assert!(result.is_same());
        assert_eq!(result.ignored_lines(), 2);
        // Line endings alone are a fact about the files, not counted.
        assert_eq!(
            compare("a\r\n", "a\n", Ignore::default()).ignored_lines(),
            0
        );
        let blank = Ignore {
            blank_lines: true,
            ..Ignore::default()
        };
        assert_eq!(compare("a\nb", "a\n\n\nb", blank).ignored_lines(), 2);
    }

    #[test]
    fn space_amount_keeps_space_at_the_start_and_end_rules() {
        assert_eq!(collapse_space("  a \t b  "), " a b");
        assert_eq!(collapse_space(""), "");
        let ignore = Ignore {
            space_amount: true,
            ..Ignore::NOTHING
        };
        assert_eq!(line_key("a  b ", LineEnding::None, &ignore), "a b");
        assert_ne!(
            line_key("a", LineEnding::None, &ignore),
            line_key(" a", LineEnding::None, &ignore)
        );
    }

    #[test]
    fn swapping_reads_the_changes_the_other_way() {
        let result = compare("a\nb\nc\n", "a\nc\nd\n", Ignore::NOTHING);
        let swapped = result.swapped();
        assert_eq!(swapped.counts().changes, result.counts().changes);
        assert_eq!(swapped.counts().added, result.counts().removed);
        assert_eq!(swapped.apply_to_before(), "a\nb\nc\n");
        assert_eq!(swapped.swapped(), result);
    }

    #[test]
    fn a_deadline_that_passes_still_gives_a_correct_list() {
        let before: String = (0..3000).map(|i| format!("{}\n", i * 7 % 101)).collect();
        let after: String = (0..3000).map(|i| format!("{}\n", i * 13 % 97)).collect();
        let options = CompareOptions {
            ignore: Ignore::NOTHING,
            deadline: Duration::ZERO,
            ..CompareOptions::default()
        };
        let result = compare_text(&before, &after, &options, &CancelToken::new()).unwrap();
        assert!(result.stopped_early());
        assert_eq!(result.apply_to_before(), after);
        // A deadline never marks a result with nothing to change.
        let same = compare_text(&before, &before, &options, &CancelToken::new()).unwrap();
        assert!(same.is_same() && !same.stopped_early());
    }

    #[test]
    fn a_cancelled_compare_is_not_a_result() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            compare_text("a", "b", &CompareOptions::default(), &cancel),
            Err(DiffError::Cancelled)
        );
    }
}
