//! The committed fixtures: images made in the container by
//! `docker/disk/make-images.sh` with the real tools (mkntfs, mksquashfs,
//! xorriso, mkfs.fat, qemu-img), gzipped under `fixtures/`, each checked
//! against the SHA-256 the script recorded, so Windows and macOS read
//! them without the tools.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// A fixture image's bytes, checked against its recorded digest.
pub fn fixture(name: &str) -> Vec<u8> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let gz = std::fs::read(folder.join(format!("{name}.gz"))).unwrap();
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(gz.as_slice())
        .read_to_end(&mut bytes)
        .unwrap();
    let sums = std::fs::read_to_string(folder.join("SHA256SUMS")).unwrap();
    let want = sums
        .lines()
        .find_map(|line| {
            let (digest, file) = line.split_once("  ")?;
            (file == name).then_some(digest)
        })
        .unwrap_or_else(|| panic!("{name} is not in SHA256SUMS"));
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(got, want, "{name}");
    bytes
}

#[test]
fn every_fixture_matches_its_digest_and_stays_small() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let sums = std::fs::read_to_string(folder.join("SHA256SUMS")).unwrap();
    let mut count = 0;
    for line in sums.lines() {
        let (_, name) = line.split_once("  ").unwrap();
        let gz = std::fs::metadata(folder.join(format!("{name}.gz"))).unwrap();
        assert!(gz.len() <= 64 * 1024, "{name}.gz is {} bytes", gz.len());
        fixture(name);
        count += 1;
    }
    assert!(count >= 12, "{count}");
}
