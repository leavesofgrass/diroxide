//! Fuzzes 7z reading: open under the strict limits with the coders'
//! pre-scan, an optional password from the input's first bytes, the
//! index made into rows, the first members read through the limiter
//! chain, and the whole visited in order. The body is
//! `diroxide_archive::fuzzing::sevenz`; its seeds are in
//! `fixtures/archives/seeds/archive_7z/`. Run with `-max_len=1048576`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::sevenz(data);
});
