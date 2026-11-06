//! Fuzzes the ISO 9660 reader: the record-claim guard, Joliet and Rock
//! Ridge names, relocated folders. The body is `diro_disk::fuzzing::iso`,
//! which the plugin's own tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::iso(data);
});
