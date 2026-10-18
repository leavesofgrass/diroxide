//! The xz project's own test files (`fixtures/archives/xz/`, XZ Utils
//! 5.8.4, 0BSD; see its `SOURCES.md`), read as the project's `README`
//! there describes them: research archives.md section 3, gate one.
//!
//! - The good `.xz` files list as one member, sized by their index, and
//!   read whole.
//! - The bad `.xz` files are refused at opening with the catalog's
//!   sentence, "Could not open NAME: it is damaged: CAUSE. Get a fresh
//!   copy if you can.", quickly and without a large allocation, never a
//!   panic or a hang.
//! - The unsupported `.xz` files say what they use that is not supported.
//! - The `.lzma` and `.lz` files are formats diroxide does not open as
//!   archives: each is refused in words. The `.lzma` files also go
//!   through `lzma-rust2`'s LZMA decoder, the one zip and 7z members use.
//!
//! The crate alone takes some of the bad files whole; that list is
//! recorded here, so a bump of `lzma-rust2` that changes it is seen.

use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use diroxide_archive::{Limits, member_row};
use diroxide_core::Message;
use diroxide_vfs::{OpContext, Provider, VfsError};
use lzma_rust2::{LzmaReader, XzReader};

use crate::memory::{Peak, mib};
use crate::world::World;

/// The folder of the xz project's files in the repository.
fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/archives/xz")
}

/// Every test file whose name ends in `extension`, with its bytes, by
/// name.
fn files(extension: &str) -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<(String, Vec<u8>)> = std::fs::read_dir(folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(extension))
        .map(|name| {
            let bytes = std::fs::read(folder().join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    found.sort();
    found
}

/// The files of one kind ("good", "bad", "unsupported") and extension.
fn kind(prefix: &str, extension: &str) -> Vec<(String, Vec<u8>)> {
    files(extension)
        .into_iter()
        .filter(|(name, _)| name.starts_with(&format!("{prefix}-")))
        .collect()
}

/// What the crate alone makes of an xz file, with the provider's memory
/// limit and several streams allowed: the bytes, or the error in words.
fn crate_alone(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let limits = Limits::default();
    let limit = u32::try_from(limits.max_dictionary.min(limits.max_memory_bytes) / 1024).unwrap();
    let mut out = Vec::new();
    XzReader::new_mem_limit(bytes, true, limit)
        .read_to_end(&mut out)
        .map_err(|e| diroxide_archive::guard::from_io(e).to_string())?;
    Ok(out)
}

/// A file opened through the provider: its root rows, its one member's
/// listed size, and that member read whole.
struct Opened {
    rows: Vec<String>,
    size: Option<u64>,
    bytes: Vec<u8>,
}

/// Opens a file through the provider and reads its first member whole.
fn open(world: &World, name: &str, bytes: &[u8]) -> Result<Opened, VfsError> {
    let root = world.put(name, bytes).into_archive();
    let entries = world.provider.list(&root, &OpContext::new())?;
    let first = entries.first().ok_or(VfsError::NotFound)?;
    let mut reader = world
        .provider
        .read(&root.join(&first.name).unwrap(), &OpContext::new())?;
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(diroxide_archive::guard::from_io)?;
    Ok(Opened {
        rows: entries.iter().map(member_row).collect(),
        size: first.size,
        bytes: out,
    })
}

/// The sentence the owner hears when opening fails.
fn refusal(world: &World, name: &str, bytes: &[u8]) -> String {
    match open(world, name, bytes) {
        Ok(opened) => panic!("{name} opened: {:?}", opened.rows),
        Err(error) => Message::from_error("open", name, &error).text,
    }
}

/// The text most of the small files hold.
const HELLO: &[u8] = b"Hello\nWorld!\n";

#[test]
fn the_fixtures_are_the_set_sources_md_records() {
    let sources = std::fs::read_to_string(folder().join("SOURCES.md")).unwrap();
    let readme = std::fs::read_to_string(folder().join("README")).unwrap();
    let mut recorded = Vec::new();
    for line in sources.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if let [_, name, size, digest, _] = cells[..]
            && digest.len() == 64
        {
            recorded.push((name.trim_matches('`').to_owned(), size.replace(',', "")));
        }
    }
    let mut present: Vec<String> = std::fs::read_dir(folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !matches!(name.as_str(), "LICENSE" | "SOURCES.md"))
        .collect();
    present.sort();
    let names: Vec<String> = recorded.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(names, present, "SOURCES.md and the folder disagree");
    assert_eq!(present.len(), 98);
    for (name, size) in &recorded {
        let len = std::fs::metadata(folder().join(name)).unwrap().len();
        assert_eq!(size, &len.to_string(), "{name}");
        if name != "README" {
            assert!(
                readme.contains(name.as_str()),
                "the README does not describe {name}"
            );
        }
    }
    let total: u64 = recorded
        .iter()
        .map(|(_, size)| size.parse::<u64>().unwrap())
        .sum();
    assert!(total < 1 << 20, "{total} bytes");
}

#[test]
fn the_good_xz_files_list_as_one_member_and_read_whole() {
    let world = World::plain();
    let good = kind("good", ".xz");
    assert_eq!(good.len(), 22);
    let mut by_size: std::collections::BTreeMap<usize, Vec<u8>> = Default::default();
    for (name, bytes) in &good {
        let opened = open(&world, name, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let member = name.trim_end_matches(".xz");
        assert_eq!(opened.rows.len(), 1, "{name}: {:?}", opened.rows);
        assert!(
            opened.rows[0].starts_with(&format!("{member}, file, ")),
            "{name}: {:?}",
            opened.rows
        );
        // The size from the index is the size read.
        assert_eq!(opened.size, Some(opened.bytes.len() as u64), "{name}");
        assert_eq!(
            crate_alone(bytes).as_deref(),
            Ok(&opened.bytes[..]),
            "{name}"
        );
        // Files of one size hold one content, however it was coded.
        let same = by_size
            .entry(opened.bytes.len())
            .or_insert_with(|| opened.bytes.clone());
        assert_eq!(same, &opened.bytes, "{name}");
    }
    let sizes: Vec<usize> = by_size.keys().copied().collect();
    assert_eq!(sizes, [0, 13, 457, 8_576, 929_138]);
    assert_eq!(by_size[&13], HELLO);
    let (name, tiff) = good
        .iter()
        .find(|(name, _)| name == "good-1-delta-lzma2.tiff.xz")
        .unwrap();
    let rows = open(&world, name, tiff).unwrap().rows;
    assert_eq!(rows, ["good-1-delta-lzma2.tiff, file, 929 KB"]);
}

/// The bad files `lzma-rust2` 0.21 decodes without an error when used
/// alone; the provider's own walk refuses each (read_xz.rs).
const CRATE_TAKES: [&str; 9] = [
    "bad-0-backward_size.xz",
    "bad-0pad-empty.xz",
    "bad-1-block_header-4.xz",
    "bad-1-block_header-5.xz",
    "bad-1-index-huge-uncomp.xz",
    "bad-1-vli-1.xz",
    "bad-2-index-1.xz",
    "bad-2-index-2.xz",
    "bad-3-index-uncomp-overflow.xz",
];

#[test]
fn the_bad_xz_files_are_refused_as_damaged_in_words() {
    let world = World::new(Limits::strict());
    let bad = kind("bad", ".xz");
    assert_eq!(bad.len(), 44);
    let peak = Peak::start();
    let mut taken = Vec::new();
    for (name, bytes) in &bad {
        let started = Instant::now();
        let said = refusal(&world, name, bytes);
        let took = started.elapsed();
        assert!(
            said.starts_with(&format!("Could not open {name}: it is damaged: ")),
            "{said}"
        );
        assert!(said.ends_with(". Get a fresh copy if you can."), "{said}");
        assert!(took < Duration::from_secs(5), "{name} took {took:?}");
        if crate_alone(bytes).is_ok() {
            taken.push(name.as_str());
        }
    }
    let peak = peak.stop();
    assert!(peak < 256 << 20, "{:.0} MiB", mib(peak));
    assert_eq!(taken, CRATE_TAKES, "the crate's leniency changed");
}

#[test]
fn the_bad_xz_files_say_what_is_wrong() {
    let world = World::plain();
    let cases = [
        ("bad-0-backward_size.xz", "its index is damaged"),
        ("bad-0-index-1.xz", "its index is damaged"),
        (
            "bad-0pad-empty.xz",
            "its stream padding is not a multiple of four bytes",
        ),
        (
            "bad-0-empty-truncated.xz",
            "it ends without a stream footer: it is cut short or has data after its end",
        ),
        ("bad-1-block_header-3.xz", "a block header is damaged"),
        (
            "bad-1-block_header-4.xz",
            "its index does not match its blocks",
        ),
        ("bad-1-block_header-5.xz", "a block header is damaged"),
        ("bad-1-check-crc32.xz", "invalid block checksum"),
        (
            "bad-1-index-huge-uncomp.xz",
            "its index does not match its blocks",
        ),
        ("bad-1-lzma2-2.xz", "its compressed data does not decode"),
        ("bad-1-lzma2-6.xz", "its compressed data is damaged"),
        (
            "bad-1-stream_flags-1.xz",
            "its stream header and footer disagree",
        ),
        ("bad-1-stream_flags-2.xz", "its stream header is damaged"),
        ("bad-1-stream_flags-3.xz", "its stream footer is damaged"),
        ("bad-1-vli-1.xz", "a block header is damaged"),
        ("bad-2-index-1.xz", "its index does not match its blocks"),
        ("bad-2-index-2.xz", "its index does not match its blocks"),
        (
            "bad-3-index-uncomp-overflow.xz",
            "its sizes add up to more than the format allows",
        ),
    ];
    for (name, cause) in cases {
        let bytes = std::fs::read(folder().join(name)).unwrap();
        assert_eq!(
            refusal(&world, name, &bytes),
            format!("Could not open {name}: it is damaged: {cause}. Get a fresh copy if you can."),
        );
    }
}

#[test]
fn the_unsupported_xz_files_say_what_is_not_supported() {
    let world = World::plain();
    let unsupported = kind("unsupported", ".xz");
    let names: Vec<&str> = unsupported.iter().map(|(name, _)| name.as_str()).collect();
    let cases = [
        (
            "unsupported-block_header.xz",
            "an xz block header field newer than this reader",
        ),
        ("unsupported-check.xz", "xz check type 2"),
        ("unsupported-filter_flags-1.xz", "xz filter number 127"),
        (
            "unsupported-filter_flags-2.xz",
            "an xz filter chain that does not end in LZMA2",
        ),
        (
            "unsupported-filter_flags-3.xz",
            "an xz filter chain with LZMA2 before its end",
        ),
    ];
    assert_eq!(names, cases.map(|(name, _)| name));
    for ((name, bytes), (_, what)) in unsupported.iter().zip(cases) {
        assert_eq!(
            refusal(&world, name, bytes),
            format!(
                "Could not open {name}: it uses {what}, which is not supported. \
                 Open it with a program that reads it."
            ),
        );
    }
}

#[test]
fn the_lzma_and_lzip_files_are_refused_in_words() {
    let world = World::plain();
    let mut seen = 0;
    for extension in [".lzma", ".lz"] {
        for (name, bytes) in files(extension) {
            // lzip by its signature; a broken signature and the old .lzma
            // format have none, so no format is found.
            let what = if bytes.starts_with(b"LZIP") {
                "lzip compression"
            } else {
                "no archive format diroxide reads"
            };
            assert_eq!(
                refusal(&world, &name, &bytes),
                format!(
                    "Could not open {name}: it uses {what}, which is not supported. \
                     Open it with a program that reads it."
                ),
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 26);
}

#[test]
fn the_lzma_decoder_zip_and_7z_use_reads_the_lzma_files() {
    let decode = |bytes: &[u8]| -> Result<Vec<u8>, String> {
        let mut reader =
            LzmaReader::new_mem_limit(bytes, 64 << 10, None).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    };
    let good = kind("good", ".lzma");
    assert_eq!(good.len(), 3);
    for (name, bytes) in &good {
        assert_eq!(decode(bytes).as_deref(), Ok(HELLO), "{name}");
    }
    // Two of the five bad files end in a short success: with a size in
    // the header, the decoder stops there without looking for the end.
    // A zip or 7z member read through the provider is held to its CRC
    // and its size, so that output is never shown as whole.
    let mut short = Vec::new();
    for (name, bytes) in kind("bad", ".lzma") {
        if let Ok(out) = decode(&bytes) {
            assert!(HELLO.starts_with(&out) && out.len() < HELLO.len(), "{name}");
            short.push(name);
        }
    }
    assert_eq!(
        short,
        [
            "bad-too_small_size-without_eopm-1.lzma",
            "bad-too_small_size-without_eopm-2.lzma"
        ],
        "the LZMA decoder's leniency changed"
    );
}
