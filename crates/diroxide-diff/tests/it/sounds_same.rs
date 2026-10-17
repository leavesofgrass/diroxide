//! The nine "sounds the same" cases (research/diffing.md section 9): two
//! files that differ only by trailing spaces, tabs against spaces, case, a
//! zero-width character, composed against decomposed letters, line
//! endings, a byte-order mark, the encoding, or the final line break. A
//! screen reader would say both versions the same way, so each gets its
//! reason in words. One fixture folder per case in
//! `fixtures/diff/sounds-same/`, each a `before.txt` and an `after.txt`.

use std::path::PathBuf;

use diroxide_diff::{
    ChangeKind, CompareOptions, ContentComparison, Ignore, Sides, compare_contents,
};
use diroxide_vfs::CancelToken;

fn fixture(case: &str) -> (Vec<u8>, Vec<u8>) {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/diff/sounds-same")
        .join(case);
    let read = |name: &str| std::fs::read(folder.join(name)).unwrap();
    (read("before.txt"), read("after.txt"))
}

fn compare_with(case: &str, ignore: Ignore) -> ContentComparison {
    let (before, after) = fixture(case);
    let options = CompareOptions {
        ignore,
        ..CompareOptions::default()
    };
    compare_contents(&before, &after, &options, &CancelToken::new()).unwrap()
}

fn compare(case: &str) -> ContentComparison {
    compare_with(case, Ignore::default())
}

/// A case that changes one line: the one change, its line, its reason.
fn one_line_reason(case: &str) -> (usize, String) {
    let result = compare(case);
    let changes = result.text().changes();
    assert_eq!(changes.len(), 1, "{case}: {changes:?}");
    assert_eq!(changes[0].kind, ChangeKind::Changed, "{case}");
    let reason = result
        .reason(0)
        .unwrap_or_else(|| panic!("{case} has no reason"));
    (changes[0].first_line(), reason.to_string())
}

/// A case that changes no line: the facts said about the files.
fn facts(case: &str) -> Vec<String> {
    let result = compare(case);
    assert!(result.text().is_same(), "{case}");
    assert!(!result.identical(), "{case}");
    result
        .fact_differences()
        .iter()
        .map(|fact| fact.sentence(&Sides::panels()))
        .collect()
}

#[test]
fn trailing_spaces_are_named() {
    assert_eq!(
        one_line_reason("trailing-spaces"),
        (3, "trailing spaces added".to_owned())
    );
}

#[test]
fn tabs_against_spaces_are_named() {
    assert_eq!(
        one_line_reason("tabs-spaces"),
        (3, "tab became 4 spaces".to_owned())
    );
}

#[test]
fn case_is_named() {
    assert_eq!(one_line_reason("case"), (3, "case only".to_owned()));
}

#[test]
fn a_zero_width_character_is_named_by_its_code_point() {
    assert_eq!(
        one_line_reason("zero-width"),
        (3, "hidden character U+200B added".to_owned())
    );
}

#[test]
fn composed_against_decomposed_letters_are_named() {
    assert_eq!(
        one_line_reason("unicode-form"),
        (1, "same text in another Unicode form".to_owned())
    );
}

#[test]
fn line_endings_are_one_fact_not_a_change_on_every_line() {
    assert_eq!(
        facts("line-endings"),
        ["Line endings differ: CRLF here, LF there. Compared ignoring them."]
    );
    // With line endings compared, every line is a change, and each pair
    // says why.
    let exact = compare_with("line-endings", Ignore::NOTHING);
    assert_eq!(exact.text().changes().len(), 1);
    assert_eq!(exact.text().counts().lines_added, 4);
    for line in 0..4 {
        assert_eq!(
            exact.pair_reason(line, line).unwrap().to_string(),
            "line ending changed from LF to CRLF"
        );
    }
}

#[test]
fn a_byte_order_mark_is_one_fact() {
    assert_eq!(
        facts("byte-order-mark"),
        ["Byte-order mark here, none there."]
    );
}

#[test]
fn the_same_text_in_another_encoding_is_one_fact() {
    assert_eq!(
        facts("encoding"),
        [
            "Encodings differ: UTF-16 here, UTF-8 there.",
            "Byte-order mark here, none there."
        ]
    );
}

#[test]
fn a_missing_final_line_break_is_one_fact() {
    assert_eq!(
        facts("final-newline"),
        ["Last line has no line break here."]
    );
}

#[test]
fn the_command_line_names_the_files_instead() {
    let result = compare("final-newline");
    let said = result.fact_differences()[0].sentence(&Sides::files("old.txt", "new.txt"));
    assert_eq!(said, "Last line has no line break in new.txt.");
}
