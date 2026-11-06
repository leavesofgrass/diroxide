//! Fuzzes the checksum-list parser (`SHA256SUMS`, BSD tagged lists,
//! `.sfv`) with arbitrary bytes: it must never panic, and a name that
//! climbs out of its folder must never be read. A stub from alpha 4's
//! contract step until the checksum engine is built. Run with `cargo fuzz
//! run meta_checksum_list`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = diroxide_meta::fuzzing::checksum_list(data);
});
