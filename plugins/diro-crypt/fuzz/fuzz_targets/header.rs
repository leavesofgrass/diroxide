//! Fuzzes the look at an age header before any secret is asked (armored
//! or not, the scrypt work factor read and capped), and the names made
//! from a file's name. The body is `diro_crypt::fuzzing::header`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_crypt::fuzzing::header(data);
});
