//! Fuzzes the squashfs reader: the superblock checks, then the crate's
//! tables. The body is `diro_disk::fuzzing::squashfs`, which the plugin's
//! own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::squashfs(data);
});
