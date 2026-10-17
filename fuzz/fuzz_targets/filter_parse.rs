//! Fuzzes the filter language's parser with arbitrary text: it must never
//! panic, a refusal must start "Filter problem: " and point inside the
//! text, and a filter that reads must read back to the same tree. Run
//! with `cargo fuzz run filter_parse`.

#![no_main]

use diroxide_core::filter::{Filter, FilterOptions, PROBLEM_START};
use diroxide_core::pattern::CaseRule;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for case in CaseRule::ALL {
        let options = FilterOptions {
            case,
            ..FilterOptions::default()
        };
        match Filter::parse_with(text, &options) {
            Ok(filter) => {
                let sentence = filter.sentence();
                let again = Filter::parse_with(&sentence, &options)
                    .unwrap_or_else(|e| panic!("{sentence:?}: {}", e.sentences()));
                assert_eq!(again.expr(), filter.expr(), "{text:?} read back as {sentence:?}");
            }
            Err(error) => {
                assert!(error.line().starts_with(PROBLEM_START));
                assert!(error.span().end <= text.len());
                assert!(error.word() >= 1);
            }
        }
    }
});
