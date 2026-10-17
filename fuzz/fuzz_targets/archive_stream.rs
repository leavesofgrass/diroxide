//! Fuzzes compressed tar streams: the first byte picks the wrapper, the
//! rest is decoded under the strict limits' output cap, as a tar or as one
//! member. The body is `diroxide_archive::fuzzing::stream`; its seeds are
//! in `fixtures/archives/seeds/archive_stream/`. Run with
//! `-max_len=262144`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::stream(data);
});
