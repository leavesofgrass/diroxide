//! Fuzzes the recipients reader: a pasted key or a recipients file, with
//! comments, blank lines, SSH, plugin and hardware keys refused in words.
//! The body is `diro_crypt::fuzzing::recipients`, which the plugin's own
//! tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_crypt::fuzzing::recipients(data);
});
