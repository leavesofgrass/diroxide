//! Fuzzes the image words' header readers (alpha 4): any bytes as a
//! picture's file, read with small limits; they must never panic, hang or
//! read past their limits, and a header that reads must give words. The
//! body is `diroxide_preview::fuzzing::image_header`. Run with
//! `-max_len=1048576`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_preview::fuzzing::image_header(data);
});
