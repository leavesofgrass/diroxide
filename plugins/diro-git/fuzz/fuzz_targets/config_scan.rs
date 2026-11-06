//! Fuzzes the trust scan over a configuration listing with scopes: what a
//! hostile repository's own configuration could hold. The body is
//! `diro_git::fuzzing::config_scan`, which the plugin's own tests also
//! replay.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_git::fuzzing::config_scan(data);
});
