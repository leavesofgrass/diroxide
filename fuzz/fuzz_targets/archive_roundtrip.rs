//! Fuzzes the writers against the readers: a list of entries, plain and
//! hostile names with small contents, is written in a format at a fast
//! level (with a password and hidden names where the format takes them),
//! read back under the strict limits, and compared. The body is
//! `diroxide_archive::fuzzing::roundtrip`; its seeds are in
//! `fixtures/archives/seeds/archive_roundtrip/`. Run with
//! `-max_len=1048576`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::roundtrip(data);
});
