//! Property tests over the name checks (plan section 5.12; tag `prop` in
//! research/file-operations.md). Every name here lives in a string; no
//! test creates a hostile name on disk.

use diroxide_core::format::BRAILLE_CELLS;
use diroxide_core::names::{classify, display_name};
use diroxide_vfs::names::{
    NameIssue, NameRules, check_name_with, first_refused, is_unfinished, numbered_name,
    unfinished_name, unused_name,
};
use proptest::prelude::*;

const WINDOWS: NameRules = NameRules {
    windows: true,
    max_length: Some(255),
    length_in_utf16: true,
};

const UNIX: NameRules = NameRules {
    windows: false,
    max_length: Some(255),
    length_in_utf16: false,
};

/// Names drawn from every kind of character the checks care about:
/// letters, digits, CJK, emoji, the reserved and shell characters, dots,
/// spaces, and the hidden classes (control, format, direction, private
/// use).
fn any_name() -> impl Strategy<Value = String> {
    let pieces = prop_oneof![
        "[a-zA-Z0-9]{1,8}",
        Just(".".to_owned()),
        Just(" ".to_owned()),
        Just("\u{65E5}\u{672C}".to_owned()),
        Just("\u{1F980}".to_owned()),
        Just("e\u{301}".to_owned()),
        Just("\u{E9}".to_owned()),
        prop::sample::select(vec![
            ":", "*", "?", "\"", "<", ">", "|", "\\", "/", "$", "`"
        ])
        .prop_map(str::to_owned),
        prop::sample::select(vec![
            "\u{7}", "\u{200B}", "\u{202E}", "\u{F03A}", "\u{FEFF}", "\u{2029}"
        ])
        .prop_map(str::to_owned),
        prop::sample::select(vec!["CON", "nul", "Com1", "LPT\u{B3}", "aux"])
            .prop_map(str::to_owned),
    ];
    prop::collection::vec(pieces, 0..12).prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn the_checks_never_panic_and_refusals_are_issues(name in any_name()) {
        for rules in [WINDOWS, UNIX] {
            let issues = check_name_with(&name, &[], &rules);
            match first_refused(&name, &rules) {
                Some(issue) => prop_assert!(issues.contains(&issue)),
                None => prop_assert!(issues.iter().all(|issue| !issue.refused(&rules))),
            }
        }
    }

    #[test]
    fn windows_rules_refuse_at_least_what_unix_rules_refuse(name in any_name()) {
        // Lengths differ by rule (bytes against UTF-16 units), so compare
        // only names short enough for both.
        prop_assume!(name.len() <= 255);
        if first_refused(&name, &UNIX).is_some() {
            prop_assert!(first_refused(&name, &WINDOWS).is_some());
        }
    }

    #[test]
    fn every_issue_says_its_meaning_in_40_cells(name in any_name()) {
        for issue in check_name_with(&name, &[], &WINDOWS) {
            prop_assert!(issue.line().starts_with("Name needs care: "));
            if !matches!(issue, NameIssue::CaseTwin(_)) {
                prop_assert!(issue.line().chars().count() <= BRAILLE_CELLS, "{}", issue.line());
            }
        }
    }

    #[test]
    fn display_names_hide_every_hidden_character(name in any_name()) {
        let shown = display_name(&name);
        prop_assert!(shown.chars().all(|c| !classify(c).is_hidden()));
        prop_assert_eq!(shown.chars().count(), name.chars().count());
    }

    #[test]
    fn a_case_twin_is_found_whatever_the_case(name in "[a-zA-Z]{1,12}") {
        let twin = name.to_uppercase();
        let issues = check_name_with(&name, &[twin.as_str()], &UNIX);
        let found = issues.iter().any(|issue| matches!(issue, NameIssue::CaseTwin(_)));
        if twin == name {
            prop_assert!(!found, "a name is not its own twin");
        } else {
            prop_assert!(issues.contains(&NameIssue::CaseTwin(twin)));
        }
    }

    #[test]
    fn numbered_names_keep_the_extension(
        stem in "[a-zA-Z0-9 ]{1,10}",
        extension in prop::sample::select(vec!["", ".pdf", ".tar.gz", ".TXT"]),
        n in 2u32..1000,
    ) {
        let name = format!("{stem}{extension}");
        let numbered = numbered_name(&name, n);
        let suffix = format!(" ({n}){extension}");
        prop_assert!(numbered.ends_with(&suffix), "{numbered}");
    }

    #[test]
    fn an_unused_name_is_unused_ignoring_case(
        name in "[a-z]{1,6}\\.txt",
        taken in prop::collection::vec(2u32..6, 0..5),
    ) {
        let siblings: Vec<String> = taken
            .iter()
            .map(|n| numbered_name(&name, *n).to_uppercase())
            .chain(std::iter::once(name.clone()))
            .collect();
        let refs: Vec<&str> = siblings.iter().map(String::as_str).collect();
        let free = unused_name(&name, &refs).unwrap();
        prop_assert!(!refs.iter().any(|s| s.to_lowercase() == free.to_lowercase()));
    }

    #[test]
    fn unfinished_names_round_trip(token in any::<u32>()) {
        let name = unfinished_name(token);
        prop_assert!(is_unfinished(&name));
        prop_assert!(first_refused(&name, &WINDOWS).is_none());
        let longer = format!("{name}x");
        prop_assert!(!is_unfinished(&longer));
    }
}
