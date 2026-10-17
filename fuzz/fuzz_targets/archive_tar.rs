//! Fuzzes uncompressed tar reading: the header walk under the strict
//! limits, the index made into rows, the first members read through the
//! limiter chain. The body is `diroxide_archive::fuzzing::tar`; its seeds
//! are in `fixtures/archives/seeds/archive_tar/`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::tar(data);
});
