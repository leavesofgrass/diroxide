//! Fuzzes the log record reader (fields between unit separators, commits
//! ended by NUL) and a commit's name-status reader. The body is
//! `diro_git::fuzzing::log_record`, which the plugin's own tests also
//! replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_git::fuzzing::log_record(data);
});
