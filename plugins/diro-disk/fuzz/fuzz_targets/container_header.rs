//! Fuzzes the five containers' headers (VHD, VHDX, VDI, VMDK, QCOW2): the
//! claims bounded before any table is read. The body is
//! `diro_disk::fuzzing::container_header`, which the plugin's own tests
//! also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::container_header(data);
});
