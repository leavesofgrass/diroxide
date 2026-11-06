//! Fuzzes the ext reader: superblock, group descriptors, inodes, extents
//! and block maps. The body is `diro_disk::fuzzing::ext`, which the
//! plugin's own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::ext(data);
});
