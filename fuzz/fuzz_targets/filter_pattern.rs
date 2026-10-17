//! Fuzzes the pattern module's compile step with arbitrary text: globs for
//! names and paths, regular expressions with the limits set, and the first
//! rung's patterns. Nothing may panic, a refusal must say itself inside 40
//! cells, and a compiled pattern must match in time linear in the text.
//! Run with `cargo fuzz run filter_pattern`.

#![no_main]

use diroxide_core::Explain;
use diroxide_core::pattern::{
    CaseRule, Glob, MAX_PATTERN_CHARS, Pattern, PatternError, compile_regex, regex_reason,
};
use diroxide_core::sort::NameCase;
use libfuzzer_sys::fuzz_target;

fn said_well(error: &PatternError) {
    assert!(error.line().chars().count() <= 40);
    assert!(!error.next_step().is_empty());
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // The first line is the pattern; the rest are names to try it on.
    let mut lines = text.split('\n');
    let source = lines.next().unwrap_or_default();
    let names: Vec<&str> = lines.take(8).collect();
    for case in [NameCase::Ignore, NameCase::Respect] {
        match compile_regex(source, case) {
            Ok(regex) => {
                for name in &names {
                    let _ = regex.is_match(name);
                }
            }
            Err(error) => {
                said_well(&error);
                if error == PatternError::BadRegex {
                    assert!(regex_reason(source).is_some());
                }
                if source.chars().count() > MAX_PATTERN_CHARS {
                    assert_eq!(error, PatternError::TooLong);
                }
            }
        }
        for glob in [Glob::new(source, case), Glob::for_path(source, case)] {
            match glob {
                Ok(glob) => {
                    for name in &names {
                        let _ = glob.is_match(name);
                    }
                }
                Err(error) => said_well(&error),
            }
        }
        match Pattern::parse_with(source, case) {
            Ok(pattern) => {
                for name in &names {
                    let _ = pattern.matches(name, true);
                    let _ = pattern.matches(name, false);
                }
            }
            Err(error) => said_well(&error),
        }
    }
    let _ = CaseRule::Smart.resolve(source);
    let _ = CaseRule::Smart.resolve_regex(source);
});
