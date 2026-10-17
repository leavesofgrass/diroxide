//! Fuzzes xz decoding with the strict limits' memory cap, with and without
//! several streams, under an output cap, and the provider's checks of an
//! xz file: its index walk, its end and its first dictionary. The body is
//! `diroxide_archive::fuzzing::xz`; its seeds are in
//! `fixtures/archives/seeds/archive_xz/`. Run with `-max_len=262144`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::xz(data);
});
