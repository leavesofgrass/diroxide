//! The fuzz targets' bodies replayed in the normal tests (R30 commit 106):
//! every seed in `fixtures/seeds`, every cut of it, copies with bytes
//! bent, and random bytes. None may panic.

use std::path::Path;

use proptest::prelude::*;

use diro_git::fuzzing;

type Body = fn(&[u8]);

const TARGETS: [(&str, Body); 4] = [
    ("status_v2", fuzzing::status_v2),
    ("log_record", fuzzing::log_record),
    ("config_scan", fuzzing::config_scan),
    ("other_readers", fuzzing::other_readers),
];

fn seeds(target: &str) -> Vec<Vec<u8>> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("seeds")
        .join(target);
    let mut out: Vec<Vec<u8>> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| std::fs::read(e.unwrap().path()).unwrap())
        .collect();
    out.sort();
    out
}

#[test]
fn every_seed_cut_and_bent_reads_without_a_panic() {
    for (target, body) in TARGETS {
        let seeds = seeds(target);
        assert!(!seeds.is_empty(), "{target} has no seeds");
        for seed in &seeds {
            body(seed);
            for cut in 0..seed.len() {
                body(&seed[..cut]);
            }
            for at in 0..seed.len() {
                for bent in [0u8, b'\n', b'\x1f', b' ', 0xff, b'2', b'u', b'#'] {
                    let mut copy = seed.clone();
                    copy[at] = bent;
                    body(&copy);
                }
            }
        }
        // Every target's seeds, through every other target too.
        for (_, other) in TARGETS {
            for seed in &seeds {
                other(seed);
            }
        }
    }
}

proptest! {
    #[test]
    fn random_bytes_read_without_a_panic(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        for (_, body) in TARGETS {
            body(&data);
        }
    }
}
