//! Fuzzes the FAT reader behind its read-only wrapper: boot records,
//! folders, cluster chains. The body is `diro_disk::fuzzing::fat`, which
//! the plugin's own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::fat(data);
});
