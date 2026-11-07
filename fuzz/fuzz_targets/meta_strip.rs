//! Fuzzes the strip into a copy with arbitrary bytes as a picture: it
//! must never panic, and a copy it writes, read again, must hold nothing
//! the strip takes out. Run with `cargo fuzz run meta_strip`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = diroxide_meta::fuzzing::strip(data);
});
