//! Fuzzes the plugin protocol's framing: any byte stream through the
//! bounded line reader, the line sorting and the message parser. No line
//! holds more than the limit, the lines join back into the input when none
//! is over it, and every message that parses writes back to the same
//! message. The body is `diroxide_plugins::fuzzing::framing`, which the
//! seeds in `fixtures/plugins/seeds/plugin_framing/` also replay in normal
//! tests. Run with `cargo fuzz run plugin_framing
//! ../fixtures/plugins/seeds/plugin_framing`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diroxide_plugins::fuzzing::framing(data);
});
