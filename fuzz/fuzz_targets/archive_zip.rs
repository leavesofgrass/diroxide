//! Fuzzes zip reading: open under the strict limits, the index made into
//! rows, the first 64 members read through the limiter chain. The body is
//! `diroxide_archive::fuzzing::zip`; its seeds are in
//! `fixtures/archives/seeds/archive_zip/`. Run with `-max_len=1048576`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::zip(data);
});
