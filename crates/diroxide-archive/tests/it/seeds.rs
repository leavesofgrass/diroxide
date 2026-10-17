//! The fuzz seeds under `fixtures/archives/seeds/`: generated (the text
//! seeds by `diroxide_archive::fuzzing::seeds`, the archive seeds by this
//! crate's own builders), so they are reproducible, checked here against
//! the generators, and replayed through each target's body. Set
//! `DIRO_WRITE_SEEDS=1` to write them after a generator changes.

use std::path::PathBuf;

use diroxide_archive::fuzzing;

use crate::build;
use crate::hostile;

/// The seeds folder in the repository.
fn seeds_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/archives/seeds")
}

/// Every seed: the target, a plain file name, the bytes.
fn generated() -> Vec<(&'static str, String, Vec<u8>)> {
    let mut seeds: Vec<(&'static str, String, Vec<u8>)> = fuzzing::seeds()
        .into_iter()
        .map(|(target, file, bytes)| (target, file.to_owned(), bytes))
        .collect();
    let zips: [(&str, Vec<u8>); 8] = [
        (
            "plain.zip",
            build::zip_of(&[("week1/", b""), ("week1/notes.txt", b"hello")]),
        ),
        ("stored.zip", build::zip_stored("a.txt", b"stored")),
        ("overlap.zip", build::quoted_overlap(3, 4096)),
        ("claims.zip", hostile::claims_four_gb()),
        ("names-disagree.zip", hostile::names_disagree()),
        ("hostile-names.zip", hostile::hostile_names()),
        ("links.zip", hostile::links_out_of_root()),
        (
            "zipcrypto.zip",
            build::zipcrypto_zip("secret.txt", b"the plans", "seed"),
        ),
    ];
    for (file, bytes) in zips {
        seeds.push(("archive_zip", file.to_owned(), bytes));
    }
    seeds
}

/// Runs a seed through its target's body.
fn replay(target: &str, bytes: &[u8]) {
    match target {
        "archive_names" => fuzzing::names(bytes),
        "archive_sniff" => fuzzing::sniff(bytes),
        "archive_zip" => fuzzing::zip(bytes),
        other => panic!("no body for the target {other}"),
    }
}

#[test]
fn the_checked_in_seeds_are_the_generators() {
    let write = std::env::var_os("DIRO_WRITE_SEEDS").is_some_and(|v| v == "1");
    let mut total = 0usize;
    for (target, file, bytes) in generated() {
        assert!(
            file.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b)),
            "a plain name: {file}"
        );
        let path = seeds_dir().join(target).join(&file);
        if write {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let on_disk = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{}: {e}; run with DIRO_WRITE_SEEDS=1", path.display()));
        assert_eq!(on_disk, bytes, "{} is not the generator's", path.display());
        total += bytes.len();
    }
    assert!(total < 1 << 20, "seeds stay under 1 MB in all: {total}");
}

#[test]
fn every_seed_replays() {
    for (target, _, bytes) in generated() {
        replay(target, &bytes);
    }
}
