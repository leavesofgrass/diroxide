//! Fuzzes the plugin manifest reader: any text as `plugin.toml`, read for
//! each system. A manifest comes back exactly when there is no error, every
//! problem names its field and a fix, and an accepted manifest has a plain
//! id and a name of one Braille line. The body is
//! `diroxide_plugins::fuzzing::manifest_toml`, which the manifests in
//! `fixtures/plugins/manifests/` also replay in normal tests. Run with
//! `cargo fuzz run manifest_toml ../fixtures/plugins/manifests/good`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_plugins::fuzzing::manifest_toml(data);
});
