//! Fuzzes the 7z l -slt parser: any text, never a panic. The body is
//! `diro_disk::fuzzing::seven_listing`, which the plugin's own tests also
//! replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::seven_listing(data);
});
