//! Fuzzes the whole open: container, table or file system, the root and
//! one level listed. The body is `diro_disk::fuzzing::image`, which the
//! plugin's own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::image(data);
});
