//! The round-trip target's body under proptest (plan commit 37): random
//! lists of entries, hostile names among them, in every format, so the
//! normal tests do a short fuzz run of their own beside the seeds'
//! replay. The libFuzzer run itself is the container's.

use diroxide_archive::fuzzing::{ROUND_FILE, ROUND_FOLDER, roundtrip, roundtrip_input};
use proptest::prelude::*;

/// A name of a few parts, from an alphabet with the hostile characters
/// in it: slashes, dots, backslashes, spaces, an archive marker, a drive
/// colon, an accent, a direction override, a control character.
fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a", "b", "C", "/", ".", "..", "\\", " ", "!", ".zip!", ":", "\u{e9}", "\u{202e}",
            "\u{1}", "x.txt",
        ]),
        1..8,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 160,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn written_archives_read_back_as_written(
        format in 0u8..6,
        level in 0u8..2,
        flags in 0u8..8,
        entries in prop::collection::vec(
            (name(), any::<bool>(), prop::collection::vec(any::<u8>(), 0..300)),
            1..10,
        ),
    ) {
        let entries: Vec<(String, u8, Vec<u8>)> = entries
            .into_iter()
            .map(|(name, folder, data)| (name, if folder { ROUND_FOLDER } else { ROUND_FILE }, data))
            .collect();
        let borrowed: Vec<(&str, u8, &[u8])> = entries
            .iter()
            .map(|(n, k, d)| (n.as_str(), *k, d.as_slice()))
            .collect();
        roundtrip(&roundtrip_input(format, level, flags, &borrowed));
    }

    #[test]
    fn any_bytes_are_said_or_refused_never_a_panic(data in prop::collection::vec(any::<u8>(), 0..400)) {
        roundtrip(&data);
    }
}
