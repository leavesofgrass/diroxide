//! Fuzzes the name checks with arbitrary text: they must never panic,
//! every refusal must be one of the issues found, and the display name
//! must hide every hidden character. Run with `cargo fuzz run names`.

#![no_main]

use diroxide_core::names::{classify, display_name, spell_out, split_extension};
use diroxide_vfs::names::{
    NameRules, check_name_with, first_refused, is_unfinished, numbered_name, unused_name,
};
use libfuzzer_sys::fuzz_target;

const RULES: [NameRules; 2] = [
    NameRules {
        windows: true,
        max_length: Some(255),
        length_in_utf16: true,
    },
    NameRules {
        windows: false,
        max_length: Some(255),
        length_in_utf16: false,
    },
];

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // The first line is the name; the rest are its siblings.
    let mut lines = text.split('\n');
    let name = lines.next().unwrap_or_default();
    let siblings: Vec<&str> = lines.take(16).collect();
    for rules in &RULES {
        let issues = check_name_with(name, &siblings, rules);
        if let Some(refused) = first_refused(name, rules) {
            assert!(issues.contains(&refused));
        }
        for issue in &issues {
            assert!(issue.line().starts_with("Name needs care: "));
        }
    }
    assert!(display_name(name).chars().all(|c| !classify(c).is_hidden()));
    let _ = spell_out(name);
    let (stem, extension) = split_extension(name);
    assert_eq!(format!("{stem}{extension}"), name);
    let _ = numbered_name(name, 2);
    let _ = unused_name(name, &siblings);
    let _ = is_unfinished(name);
});
