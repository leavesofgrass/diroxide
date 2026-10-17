//! Fuzzes the archive member-name rules, `Place` parsing with `!`, `join`
//! and junk: a safe name never holds `..`, a leading slash, a drive, NUL
//! or a control character, normalizing twice is stable, and a member place
//! prints and parses back equal. The body is
//! `diroxide_archive::fuzzing::names`, which the seeds in
//! `fixtures/archives/seeds/archive_names/` also replay in normal tests.
//! Run with `cargo fuzz run archive_names
//! ../fixtures/archives/seeds/archive_names`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_archive::fuzzing::names(data);
});
