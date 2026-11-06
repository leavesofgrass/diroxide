//! Fuzzes the NTFS reader: the boot sector, file records, indexes and
//! data runs. The body is `diro_disk::fuzzing::ntfs`, which the plugin's
//! own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::ntfs(data);
});
