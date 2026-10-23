//! The components list reader (W4c): any bytes as `components.toml`
//! never panic, every component read passes its own check, and a list
//! written back reads as the same components. The body is
//! `diroxide_components::manifest::fuzz_manifest`, so the fixture lists
//! under `fixtures/components/manifests/` replay as seeds in the crate's
//! normal tests. Run with `cargo fuzz run component_manifest
//! fixtures/components/manifests`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_components::manifest::fuzz_manifest(data);
});
