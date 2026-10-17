//! The properties every compare keeps (R18, "Tests worth writing first"):
//! applying the change list to the before text gives the after text; the
//! counts add up; Swap reads the same changes the other way.

use diroxide_diff::{
    ChangeKind, CompareOptions, Ignore, LineAlgorithm, TextComparison, compare_text,
};
use diroxide_vfs::CancelToken;
use proptest::prelude::*;

/// Property tests write no regression files into the crate: a failure
/// prints its minimal input instead.
fn quiet() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Lines from a small set, so the two texts share many lines and the
/// alignment has real work to do; with space, case and blank variants
/// for the ignore options.
const WORDS: [&str; 10] = ["a", "b", "c", "", "  ", " a", "a ", "A", "a\tb", "a  b"];
const ENDINGS: [&str; 3] = ["\n", "\r\n", "\r"];

/// Texts with every kind of line ending.
fn text() -> impl Strategy<Value = String> {
    text_with(ENDINGS.len())
}

/// Texts with LF and CRLF endings only. Under the ignore options a same
/// line keeps the before file's characters, which may be emptier than the
/// after file's ("  " against "" with space ignored), and a lone CR before
/// an emptied line would read back as one CRLF: an artifact of putting
/// text back together, not of the compare, which the exact property
/// covers with lone CRs.
fn text_without_lone_cr() -> impl Strategy<Value = String> {
    text_with(2)
}

fn text_with(endings: usize) -> impl Strategy<Value = String> {
    (
        prop::collection::vec((0..WORDS.len(), 0..endings), 0..24),
        any::<bool>(),
    )
        .prop_map(|(lines, final_break)| {
            let mut text = String::new();
            let count = lines.len();
            for (at, (word, ending)) in lines.into_iter().enumerate() {
                text.push_str(WORDS[word]);
                if at + 1 < count || final_break {
                    text.push_str(ENDINGS[ending]);
                }
            }
            text
        })
}

fn algorithm() -> impl Strategy<Value = LineAlgorithm> {
    prop_oneof![
        Just(LineAlgorithm::Patience),
        Just(LineAlgorithm::Histogram),
        Just(LineAlgorithm::Myers),
    ]
}

fn ignore() -> impl Strategy<Value = Ignore> {
    (any::<[bool; 6]>()).prop_map(|flags| Ignore {
        line_endings: flags[0],
        trailing_space: flags[1],
        space_amount: flags[2],
        all_space: flags[3],
        blank_lines: flags[4],
        case: flags[5],
    })
}

fn run(before: &str, after: &str, ignore: Ignore, algorithm: LineAlgorithm) -> TextComparison {
    let options = CompareOptions {
        ignore,
        algorithm,
        ..CompareOptions::default()
    };
    compare_text(before, after, &options, &CancelToken::new()).unwrap()
}

/// The lines of a text as the ignore options see them: no endings when
/// endings are ignored, no blank lines when blank lines are. With
/// `written_back`, for text the compare wrote back, an emptied last line
/// with no line break is dropped too (see below).
fn seen(text: &str, ignore: &Ignore, written_back: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let cut = rest.find(['\n', '\r']).unwrap_or(rest.len());
        let ending_len = if rest[cut..].starts_with("\r\n") {
            2
        } else {
            usize::from(cut < rest.len())
        };
        let line = &rest[..cut];
        let ending = &rest[cut..cut + ending_len];
        rest = &rest[cut + ending_len..];
        if ignore.blank_lines && line.chars().all(char::is_whitespace) {
            continue;
        }
        let mut key: String = if ignore.all_space {
            line.chars().filter(|c| !c.is_whitespace()).collect()
        } else if ignore.space_amount {
            // Space at the end goes; every other run of space is one space.
            let trimmed = line.trim_end();
            let mut out = String::new();
            for (at, word) in trimmed.split(char::is_whitespace).enumerate() {
                if at > 0 && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(word);
            }
            out
        } else if ignore.trailing_space {
            line.trim_end().to_owned()
        } else {
            line.to_owned()
        };
        if ignore.case {
            key = key.to_lowercase();
        }
        if !ignore.line_endings {
            key.push_str(ending);
        }
        // A last line that the options empty and that has no line break
        // cannot be written back as text: the before file's same line
        // ("" against "  " with space ignored) is no line at all. Drop it
        // on both sides; the change list still pairs it.
        if written_back && key.is_empty() && ending.is_empty() {
            continue;
        }
        lines.push(key);
    }
    lines
}

proptest! {
    #![proptest_config(quiet())]

    #[test]
    fn applying_the_changes_to_before_gives_after(
        before in text(),
        after in text(),
        algorithm in algorithm(),
    ) {
        let result = run(&before, &after, Ignore::NOTHING, algorithm);
        prop_assert_eq!(result.apply_to_before(), after.clone());
        prop_assert_eq!(result.is_same(), before == after);
        let counts = result.counts();
        prop_assert!(counts.adds_up());
        prop_assert_eq!(counts.changes, result.changes().len());
        // The lines outside changes are the same lines on both sides.
        prop_assert_eq!(
            result.before_lines() - counts.lines_removed,
            result.after_lines() - counts.lines_added
        );
        // Changes come in order and never touch.
        for pair in result.changes().windows(2) {
            prop_assert!(pair[0].before.end < pair[1].before.start
                || pair[0].after.end < pair[1].after.start);
        }
    }

    #[test]
    fn swapping_turns_additions_into_removals(
        before in text(),
        after in text(),
        algorithm in algorithm(),
    ) {
        let result = run(&before, &after, Ignore::NOTHING, algorithm);
        let swapped = result.swapped();
        prop_assert_eq!(swapped.apply_to_before(), before.clone());
        prop_assert_eq!(swapped.counts().changes, result.counts().changes);
        prop_assert_eq!(swapped.counts().added, result.counts().removed);
        prop_assert_eq!(swapped.counts().removed, result.counts().added);
        prop_assert_eq!(swapped.counts().changed, result.counts().changed);
        for (one, other) in result.changes().iter().zip(swapped.changes()) {
            let expected = match one.kind {
                ChangeKind::Added => ChangeKind::Removed,
                ChangeKind::Removed => ChangeKind::Added,
                ChangeKind::Changed => ChangeKind::Changed,
            };
            prop_assert_eq!(other.kind, expected);
            prop_assert_eq!(&other.before, &one.after);
        }
        prop_assert_eq!(swapped.swapped(), result);
    }

    #[test]
    fn under_any_ignore_options_the_changes_lead_to_after(
        before in text_without_lone_cr(),
        after in text_without_lone_cr(),
        ignore in ignore(),
        algorithm in algorithm(),
    ) {
        let result = run(&before, &after, ignore, algorithm);
        prop_assert!(result.counts().adds_up());
        prop_assert_eq!(
            seen(&result.apply_to_before(), &ignore, true),
            seen(&after, &ignore, true)
        );
        // With blank lines ignored, a move around a blank line may still
        // count as a change, as in git; otherwise "same" means exactly
        // the same lines as the options see them.
        if !ignore.blank_lines {
            prop_assert_eq!(
                result.is_same(),
                seen(&before, &ignore, false) == seen(&after, &ignore, false)
            );
        }
    }
}
