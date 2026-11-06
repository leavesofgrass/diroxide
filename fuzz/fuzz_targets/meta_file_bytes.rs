//! Fuzzes the metadata readers with arbitrary bytes as a file's contents:
//! they must never panic, hang or read past their limits. A stub from
//! alpha 4's contract step until the metadata lane builds the readers.
//! Run with `cargo fuzz run meta_file_bytes`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = diroxide_meta::fuzzing::file_bytes(data);
});
