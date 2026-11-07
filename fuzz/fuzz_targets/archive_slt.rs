//! Fuzzes the 7z tool backend's listing parser (alpha 4, R30 commit 80):
//! a `7z l -slt` listing, as 7-Zip prints it or as a hostile member name
//! might twist it, parsed and indexed under the strict limits; nothing
//! panics, every record points at an item that exists, and no indexed
//! path climbs out. The body is `diroxide_archive::fuzzing::slt`, whose
//! samples (`SLT_SAMPLES`) are replayed in the crate's tests. Run with
//! `-max_len=262144`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::slt(data);
});
