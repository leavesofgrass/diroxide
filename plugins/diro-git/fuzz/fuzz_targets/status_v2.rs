//! Fuzzes the porcelain v2 status reader and the words of what it reads:
//! renames with their second path, conflicts, submodules, names that are
//! not UTF-8. The body is `diro_git::fuzzing::status_v2`, which the
//! plugin's own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_git::fuzzing::status_v2(data);
});
