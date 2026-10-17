//! Fuzzes the archive signature sniffer and the decision between a name
//! and the bytes: they never panic and they agree. The body is
//! `diroxide_archive::fuzzing::sniff`; its seeds are in
//! `fixtures/archives/seeds/archive_sniff/`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::sniff(data);
});
