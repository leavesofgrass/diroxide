//! Fuzzes the MBR and GPT readers: every count and size a table claims
//! bounded, a chain of logical partitions that points back ended. The
//! body is `diro_disk::fuzzing::partition_table`, which the plugin's own
//! tests also replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_disk::fuzzing::partition_table(data);
});
