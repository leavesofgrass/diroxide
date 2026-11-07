//! Fuzzes the checksum-list parser (`SHA256SUMS`, BSD tagged lists,
//! one-file lists, `.sfv`) with arbitrary bytes: it must never panic, a
//! name that climbs out of its folder or is absolute must never be
//! taken, and a list written from what was read must read back with the
//! same names. Run with `cargo fuzz run meta_checksum_list`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = diroxide_meta::fuzzing::checksum_list(data);
});
