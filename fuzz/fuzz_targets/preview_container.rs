//! Fuzzes the previews' container read path (alpha 4): any bytes as a
//! zip-based document, through its member checks and its nesting cut,
//! with small caps; it must never panic, hang or unpack past its caps.
//! The body is `diroxide_preview::fuzzing::container`. Run with
//! `-max_len=1048576`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_preview::fuzzing::container(data);
});
