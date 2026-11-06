//! Fuzzes the other readers of git's words: blame, the branch and stash
//! lists, and progress lines. The body is
//! `diro_git::fuzzing::other_readers`, which the plugin's own tests also
//! replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_git::fuzzing::other_readers(data);
});
