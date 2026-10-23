//! Fuzzes the script header reader: any bytes as a one-file script. No
//! panic; every problem names its field and ends its fix with a period;
//! with no error the name is a plain name of one Braille line, and an
//! unattended command has a timeout and asks nothing. The body is
//! `diroxide_plugins::script::header::fuzz_header`, which the scripts in
//! `fixtures/scripts/headers/` also replay in normal tests. Run with
//! `cargo fuzz run script_header ../fixtures/scripts/headers`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_plugins::script::header::fuzz_header(data);
});
