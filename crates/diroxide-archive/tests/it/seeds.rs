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
    let course = build::tar_of(&[("week1/", b""), ("week1/notes.txt", b"hello")]);
    let tars: [(&str, Vec<u8>); 4] = [
        ("plain.tar", course.clone()),
        ("pax-size.tar", hostile::pax_size_disagrees()),
        ("links.tar", hostile::tar_links_out_of_root()),
        (
            "long-name.tar",
            build::tar_of(&[(&"d".repeat(150), b"deep")]),
        ),
    ];
    for (file, bytes) in tars {
        seeds.push(("archive_tar", file.to_owned(), bytes));
    }
    let mut gz_tar = vec![0u8];
    gz_tar.extend_from_slice(&build::gzip(&course));
    let mut gz_one = vec![0u8];
    gz_one.extend_from_slice(&build::gzip(b"one compressed file"));
    let mut gz_zeros = vec![0u8];
    gz_zeros.extend_from_slice(&build::gzip(&vec![0u8; 1 << 20]));
    for (file, bytes) in [
        ("gzip-tar.bin", gz_tar),
        ("gzip-one.bin", gz_one),
        ("gzip-zeros.bin", gz_zeros),
    ] {
        seeds.push(("archive_stream", file.to_owned(), bytes));
    }
    let mut xz_tar = vec![1u8];
    xz_tar.extend_from_slice(&build::xz(&course, 1));
    seeds.push(("archive_stream", "xz-tar.bin".to_owned(), xz_tar));
    let mut joined = build::xz(b"first ", 0);
    joined.extend_from_slice(&[0; 4]);
    joined.extend_from_slice(&build::xz(b"second", 0));
    let xzs: [(&str, Vec<u8>); 4] = [
        ("plain.xz", build::xz(b"one compressed file", 1)),
        ("joined.xz", joined),
        ("over-cap.xz", hostile::xz_block_over_cap()),
        (
            "cut.xz",
            build::xz(&build::noise(8 << 10, 5), 0)[..4096].to_vec(),
        ),
    ];
    for (file, bytes) in xzs {
        seeds.push(("archive_xz", file.to_owned(), bytes));
    }
    let with_byte = |pick: u8, bytes: Vec<u8>| {
        let mut out = vec![pick];
        out.extend_from_slice(&bytes);
        out
    };
    let mut locked = vec![1u8];
    locked.extend_from_slice(b"seedpass");
    locked.extend_from_slice(&build::sevenz_of(
        &[("a.txt", b"locked")],
        false,
        Some("seedpass"),
    ));
    let sevenzs: [(&str, Vec<u8>); 6] = [
        (
            "plain.7z",
            with_byte(
                0,
                build::sevenz_of(&[("dir/a.txt", b"one"), ("b.txt", b"two")], false, None),
            ),
        ),
        (
            "solid.7z",
            with_byte(
                0,
                build::sevenz_of(&[("a.txt", b"one"), ("b.txt", b"two")], true, None),
            ),
        ),
        ("locked.7z", locked),
        (
            "header-dictionary.7z",
            with_byte(0, hostile::sevenz_four_gib_dictionary()),
        ),
        (
            "content-dictionary.7z",
            with_byte(0, hostile::sevenz_four_gib_content_dictionary()),
        ),
        (
            "empty-files.7z",
            with_byte(
                0,
                build::sevenz_empty_files((0..20).map(|i| format!("f{i}"))),
            ),
        ),
    ];
    for (file, bytes) in sevenzs {
        seeds.push(("archive_7z", file.to_owned(), bytes));
    }
    seeds
}

/// Runs a seed through its target's body.
fn replay(target: &str, bytes: &[u8]) {
    match target {
        "archive_names" => fuzzing::names(bytes),
        "archive_sniff" => fuzzing::sniff(bytes),
        "archive_zip" => fuzzing::zip(bytes),
        "archive_tar" => fuzzing::tar(bytes),
        "archive_stream" => fuzzing::stream(bytes),
        "archive_xz" => fuzzing::xz(bytes),
        "archive_7z" => fuzzing::sevenz(bytes),
        "archive_roundtrip" => fuzzing::roundtrip(bytes),
        other => panic!("no body for the target {other}"),
    }
}

/// The bytes with what the zip writer takes from the platform zeroed: in
/// each central directory header, the system half of "version made by"
/// (DOS on Windows, Unix elsewhere) and the two DOS attribute bytes of the
/// external attributes (the folder bit on Windows). Seeds are written as
/// generated, so a checked-in zip carries its generating platform's bytes
/// and the comparison still holds on every other; nothing else differs.
fn platform_free(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i + 42 <= out.len() {
        if out[i..i + 4] == [b'P', b'K', 1, 2] {
            out[i + 5] = 0;
            out[i + 38] = 0;
            out[i + 39] = 0;
            i += 42;
        } else {
            i += 1;
        }
    }
    out
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
        assert_eq!(
            platform_free(&on_disk),
            platform_free(&bytes),
            "{} is not the generator's",
            path.display()
        );
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
