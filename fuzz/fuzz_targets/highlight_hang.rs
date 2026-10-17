//! The highlighter must never hang (R17, spike H2): arbitrary bytes, as
//! text, through each language of the allow-list, under libFuzzer's
//! timeout (run with `-timeout=2`, the viewer's deadline). The first
//! byte picks the language.

#![no_main]

use diroxide_preview::highlight::{Language, highlight_lines};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    let language = Language::ALL[usize::from(pick) % Language::ALL.len()];
    let text = String::from_utf8_lossy(rest);
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let _ = highlight_lines(language, &lines, None);
});
