//! Fuzzes the metadata readers with arbitrary bytes as a file's contents:
//! every built-in reader (audio, zip, photo, PDF, Office, OpenDocument,
//! EPUB, text), the Exif reader and the internet mark's readers. They
//! must never panic, hang or read past their limits. Run with `cargo
//! fuzz run meta_file_bytes`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = diroxide_meta::fuzzing::file_bytes(data);
});
