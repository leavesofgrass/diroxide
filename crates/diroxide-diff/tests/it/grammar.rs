//! The message grammar at 40 cells: the verdict line, the report in its
//! reading order, the rows, one change in full, and the unified view.
//! Snapshots keep the wording; the Braille label test keeps each line's
//! meaning in its first 40 cells; a property keeps the verdict line at 40
//! cells for names up to 255 characters, the names cut last.

use std::path::PathBuf;

use diroxide_core::format::{BRAILLE_CELLS, first_cells, within_cells};
use diroxide_diff::grammar::{counts_sentence, ignore_note};
use diroxide_diff::{
    CompareOptions, ContentComparison, Context, Ignore, Sides, Verdict, compare_contents,
    verdict_line, verdict_sentence,
};
use diroxide_vfs::CancelToken;
use proptest::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diff")
}

fn compare_files(before: &str, after: &str, options: &CompareOptions) -> ContentComparison {
    let read = |name: &str| std::fs::read(fixtures().join(name)).unwrap();
    compare_contents(&read(before), &read(after), options, &CancelToken::new()).unwrap()
}

fn report() -> ContentComparison {
    compare_files(
        "report/old.txt",
        "report/report.txt",
        &CompareOptions::default(),
    )
}

/// Everything a listener hears of a compare, in order.
fn transcript(result: &ContentComparison, before: &str, after: &str, sides: &Sides) -> String {
    let options = CompareOptions::default();
    let mut out = String::new();
    for line in result.summary(before, after, sides, &options) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("-- rows\n");
    for row in result.rows(&options) {
        out.push_str(&row);
        out.push('\n');
    }
    for index in 0..result.text().changes().len() {
        out.push_str(&format!("-- detail {}\n", index + 1));
        for line in result.detail(index, &options) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn the_report_reads_in_order() {
    let result = report();
    insta::assert_snapshot!(
        "report_in_the_panels",
        transcript(&result, "old.txt", "report.txt", &Sides::panels())
    );
}

#[test]
fn swap_reads_the_report_the_other_way() {
    let result = report().swapped();
    insta::assert_snapshot!(
        "report_swapped",
        transcript(&result, "report.txt", "old.txt", &Sides::panels_swapped())
    );
}

#[test]
fn the_unified_view_has_two_lines_of_context() {
    let result = report();
    let rows: Vec<String> = result
        .unified(Context::default())
        .iter()
        .map(|row| row.spoken())
        .collect();
    insta::assert_snapshot!("report_unified", rows.join("\n"));
    let whole = result.unified(Context::WholeFile);
    assert_eq!(
        whole
            .iter()
            .filter(|r| r.kind == diroxide_diff::UnifiedKind::Same)
            .count(),
        56
    );
    let none = result.unified(Context::Lines(0));
    assert!(
        none.iter()
            .all(|r| r.kind != diroxide_diff::UnifiedKind::Same)
    );
}

#[test]
fn the_nine_sounds_the_same_cases_read_as_rows() {
    let cases = [
        "trailing-spaces",
        "tabs-spaces",
        "case",
        "zero-width",
        "unicode-form",
        "line-endings",
        "byte-order-mark",
        "encoding",
        "final-newline",
    ];
    let mut out = String::new();
    for case in cases {
        out.push_str(&format!("-- {case}\n"));
        let result = compare_files(
            &format!("sounds-same/{case}/before.txt"),
            &format!("sounds-same/{case}/after.txt"),
            &CompareOptions::default(),
        );
        out.push_str(&transcript(
            &result,
            "before.txt",
            "after.txt",
            &Sides::panels(),
        ));
    }
    insta::assert_snapshot!("sounds_the_same", out);
}

#[test]
fn the_command_line_names_the_files() {
    let result = report();
    let said = result.summary(
        "old.txt",
        "report.txt",
        &Sides::files("old.txt", "report.txt"),
        &CompareOptions::default(),
    );
    assert_eq!(said[0], "5 changes between old.txt and report.txt");
    assert_eq!(said[2], "From old.txt to report.txt.");
}

#[test]
fn what_was_ignored_is_said() {
    let options = CompareOptions {
        ignore: Ignore {
            trailing_space: true,
            ..Ignore::default()
        },
        ..CompareOptions::default()
    };
    let result = compare_files("report/old.txt", "report/report.txt", &options);
    let said = result.summary("old.txt", "report.txt", &Sides::panels(), &options);
    assert!(said.contains(&"Ignoring changes in trailing spaces.".to_owned()));
    assert!(said.contains(&"1 line differs only in what was ignored.".to_owned()));
    assert_eq!(said[0], "4 changes between report.txt and old.txt");
}

/// The words that carry each line's meaning, which the owner's display
/// must show in its first 40 cells.
#[test]
fn every_line_says_its_meaning_in_40_cells() {
    let options = CompareOptions::default();
    let result = report();
    for line in result.summary("old.txt", "report.txt", &Sides::panels(), &options) {
        assert!(
            line.chars().count() <= BRAILLE_CELLS || line.starts_with("Changed"),
            "{line}"
        );
        let key = first_cells(&line, BRAILLE_CELLS);
        assert!(
            ["changes between", "Changed", "From the other panel"]
                .iter()
                .any(|k| key.contains(k)),
            "{line}"
        );
    }
    for row in result.rows(&options) {
        let start: String = row.split(',').next().unwrap().to_owned();
        assert!(within_cells(&row, &start), "{row}");
        assert!(
            ["Changed line", "Added line", "Removed line"]
                .iter()
                .any(|k| within_cells(&row, k)),
            "{row}"
        );
    }
    for row in result.unified(Context::default()) {
        let spoken = row.spoken();
        assert!(
            ["Change ", "Same ", "Added ", "Removed ", "same line"]
                .iter()
                .any(|k| within_cells(&spoken, k)),
            "{spoken}"
        );
    }
    // The facts and notes say their subject first.
    let lines = [
        "Line endings differ: CRLF here, LF there. Compared ignoring them.",
        "Byte-order mark here, none there.",
        "Encodings differ: UTF-16 here, UTF-8 there.",
        "Last line has no line break here.",
        "Ignoring changes in trailing spaces, case and blank lines.",
        "Stopped after 2 seconds; the changes may be more than the least possible.",
    ];
    for (line, key) in lines.iter().zip([
        "Line endings differ",
        "Byte-order mark",
        "Encodings differ",
        "no line break",
        "Ignoring changes",
        "Stopped after 2 seconds",
    ]) {
        assert!(within_cells(line, key), "{line}");
    }
    assert!(within_cells(
        &counts_sentence(&result.text().counts()),
        "Changed"
    ));
    assert_eq!(ignore_note(&Ignore::default()), "");
}

fn verdicts() -> impl Strategy<Value = Verdict> {
    prop_oneof![
        Just(Verdict::Same),
        Just(Verdict::SameText),
        Just(Verdict::SameFile),
        (1usize..100_000_000).prop_map(Verdict::Changes),
        (1u64..100_000_000).prop_map(Verdict::BinaryBytes),
        Just(Verdict::BinarySizes),
        Just(Verdict::TooLarge),
        Just(Verdict::NotCompared),
        Just(Verdict::ChangedWhileComparing),
    ]
}

/// Names of 1 to 255 characters, from plain letters to accented,
/// wide and hidden characters.
fn names() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            prop::char::range('a', 'z'),
            Just('.'),
            Just('-'),
            Just('\u{e9}'),
            Just('\u{4e2d}'),
            Just('\u{200b}'),
            Just('\u{202e}'),
        ],
        1..=255,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_verdict_line_fits_40_cells_and_cuts_the_names_last(
        verdict in verdicts(),
        first in names(),
        second in names(),
    ) {
        let line = verdict_line(verdict, &first, &second);
        let cells = line.chars().count();
        prop_assert!(cells <= BRAILLE_CELLS, "{} cells: {}", cells, line);
        // The verdict's own words come first and are never cut.
        let sentence = verdict_sentence(verdict, &first, &second);
        let bare = verdict_sentence(verdict, "", "");
        let words = bare
            .trim_end_matches(" and ")
            .trim_end_matches([':', ' '])
            .trim_end_matches(" between");
        prop_assert!(line.starts_with(words), "{} against {}", line, sentence);
        // Nothing hidden reaches the display.
        let hidden = line.contains('\u{200b}') || line.contains('\u{202e}');
        prop_assert!(!hidden, "a hidden character reached the line: {}", line);
        // A compare's verdict with room for names is exactly 40 cells
        // whenever the whole sentence is longer.
        if matches!(verdict, Verdict::Changes(n) if n < 10_000_000)
            || matches!(verdict, Verdict::Same | Verdict::SameText | Verdict::SameFile)
        {
            if sentence.chars().count() > BRAILLE_CELLS {
                prop_assert_eq!(cells, BRAILLE_CELLS, "{}", line);
            } else {
                prop_assert_eq!(&line, &sentence);
            }
        }
    }
}
