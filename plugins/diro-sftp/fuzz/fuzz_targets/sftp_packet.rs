//! Fuzzes the SFTP codec: any bytes as a stream of packets in both
//! directions. Nothing panics, nothing is allocated beyond the packet
//! bound, and every packet that decodes encodes back to the same packet.
//! The body is `diro_sftp::sftp::fuzzing::packet`, which the seeds in
//! `fixtures/sftp/seeds/sftp_packet/` also replay in normal tests.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_sftp::sftp::fuzzing::packet(data);
});
