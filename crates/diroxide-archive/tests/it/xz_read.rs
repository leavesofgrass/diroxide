//! xz and tar.xz through the provider (commit 17), with research
//! archives.md section 3's four gates. Gate one runs on the xz project's
//! own test files in `xz_project.rs`; the cases here are built with the
//! encoder at test time for what those small files do not cover:
//!
//! 1. Good streams of real data decode at three presets, in many blocks of
//!    many LZMA2 chunks, and through a BCJ filter; a plain xz is sized by
//!    the indexes of several streams; a tar inside lists as the tar it is.
//! 2. Interoperability with the `xz` program, when `DIRO_CROSS_CHECK=1`
//!    is set and `xz` is on PATH.
//! 3. A decode-speed figure for the report.
//! 4. Hostile input: a cut at every 4 KiB is never a short success;
//!    padding of three bytes after a stream of data is refused by the
//!    provider though the crate takes it; a block whose dictionary is over
//!    the cap is refused before the allocation, and one over the soft
//!    limit asks first.

use std::io::{Read, Write};
use std::time::Instant;

use diroxide_archive::{Allowances, ArchiveError, Limits};
use diroxide_vfs::{OpContext, Provider};
use lzma_rust2::{CheckType, FilterType, XzOptions, XzReader, XzWriter};

use crate::build::{noise, tar_of, xz};
use crate::hostile::patch_xz_dictionary;
use crate::memory::{Peak, mib};
use crate::world::World;

/// Mixed data: some text, some noise, some runs.
fn sample(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut i = 0u64;
    while out.len() < len {
        out.extend_from_slice(format!("line {i} of the sample, ").as_bytes());
        out.extend_from_slice(&noise(64, i));
        out.extend(std::iter::repeat_n(b'x', (i % 97) as usize));
        i += 1;
    }
    out.truncate(len);
    out
}

/// An xz stream with options.
fn xz_with(data: &[u8], preset: u32, check: CheckType, block: Option<u64>, bcj: bool) -> Vec<u8> {
    let mut options = XzOptions::with_preset(preset);
    options.set_check_sum_type(check);
    options.set_block_size(block.and_then(std::num::NonZeroU64::new));
    if bcj {
        options.prepend_pre_filter(FilterType::BcjX86, 0);
    }
    let mut writer = XzWriter::new(Vec::new(), options).unwrap();
    writer.write_all(data).unwrap();
    writer.finish().unwrap()
}

/// Decodes with the provider's own settings: a memory limit, several
/// streams allowed.
fn decode(bytes: &[u8], limits: &Limits) -> Result<Vec<u8>, String> {
    let limit = u32::try_from(limits.max_dictionary.min(limits.max_memory_bytes) / 1024).unwrap();
    let mut reader = XzReader::new_mem_limit(bytes, true, limit);
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| diroxide_archive::guard::from_io(e).to_string())?;
    Ok(out)
}

/// Reads the one member of a plain xz through the provider.
fn read_single(world: &World, name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let place = world.put(name, bytes);
    let root = place.into_archive();
    let member = world
        .provider
        .list(&root, &OpContext::new())
        .map_err(|e| e.to_string())?
        .first()
        .map(|e| e.name.clone())
        .ok_or("no member")?;
    world.read(&root.join(&member).unwrap())
}

#[test]
fn gate_one_good_streams_of_real_data_decode() {
    let world = World::plain();
    let data = sample(300_000);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("preset 0", xz_with(&data, 0, CheckType::Crc64, None, false)),
        ("preset 1", xz_with(&data, 1, CheckType::Crc64, None, false)),
        ("preset 6", xz_with(&data, 6, CheckType::Crc64, None, false)),
        (
            "many blocks",
            xz_with(&data, 1, CheckType::Crc64, Some(32 << 10), false),
        ),
        ("BCJ x86", xz_with(&data, 1, CheckType::Crc64, None, true)),
    ];
    for (name, bytes) in cases {
        assert_eq!(
            decode(&bytes, &Limits::default()).as_deref(),
            Ok(&data[..]),
            "{name}"
        );
        let read = read_single(&world, "sample.xz", &bytes);
        assert_eq!(
            read.as_deref(),
            Ok(&data[..]),
            "{name} through the provider"
        );
    }
}

#[test]
fn a_plain_xz_is_one_member_sized_by_its_index() {
    let world = World::plain();
    let data = sample(123_456);
    let mut joined = xz_with(&data, 1, CheckType::Crc64, Some(40_000), false);
    joined.extend_from_slice(&[0; 4]);
    joined.extend_from_slice(&xz(b"tail", 0));
    let place = world.put("notes.txt.xz", &joined);
    let rows = world.rows(&place.clone().into_archive()).unwrap();
    assert_eq!(rows, ["notes.txt, file, 123 KB"]);
    let entry = &world.list(&place.into_archive())[0];
    assert_eq!(entry.size, Some(123_460));
    // A stream cut short has no footer: refused at open, in words.
    let cut = &xz(&data, 1)[..2_000];
    let place = world.put("cut.xz", cut);
    let error = world.rows(&place.into_archive()).unwrap_err();
    assert!(error.contains("it ends without a stream footer"), "{error}");
    // Without its index, the size is unknown (the walk's own answer).
    assert!(diroxide_archive::fuzzing::xz_size_of(cut).is_none());
}

#[test]
fn a_tar_xz_lists_and_reads() {
    let world = World::plain();
    let tar = tar_of(&[
        ("week1/", b""),
        ("week1/notes.txt", b"hello"),
        ("readme.md", b"# Course"),
    ]);
    let place = world.put("course.tar.xz", &xz(&tar, 6));
    let rows = world.rows(&place.clone().into_archive()).unwrap();
    assert_eq!(rows, ["week1, folder", "readme.md, file, 8 bytes"]);
    assert_eq!(
        world.read(&world.inside(&place, "readme.md")).unwrap(),
        b"# Course"
    );
    assert_eq!(
        world
            .read(&world.inside(&place, "week1/notes.txt"))
            .unwrap(),
        b"hello"
    );
    // A tar inside a plain .xz is listed as the tar it is.
    let place = world.put("backup.xz", &xz(&tar, 1));
    let rows = world.rows(&place.into_archive()).unwrap();
    assert_eq!(rows, ["week1, folder", "readme.md, file, 8 bytes"]);
}

#[test]
fn gate_four_cuts_and_padding_fail_without_a_large_allocation() {
    let data = noise(64 << 10, 9);
    let whole = xz(&data, 0);
    let limits = Limits::strict();
    let world = World::new(limits);
    let peak = Peak::start();
    // Cut at every 4 KiB: never a short success, alone or through the
    // provider.
    let mut at = 4096;
    while at < whole.len() {
        let result = decode(&whole[..at], &limits);
        assert!(
            result.is_err(),
            "cut at {at} decoded {:?} bytes",
            result.map(|d| d.len())
        );
        let error = read_single(&world, "cut.xz", &whole[..at]).unwrap_err();
        assert!(error.starts_with("it is damaged"), "cut at {at}: {error}");
        at += 4096;
    }
    // Padding that is not a multiple of four: the provider refuses it.
    // lzma-rust2 0.21 alone accepts the three bytes after a stream of
    // data (a gate finding: the provider holds the rule). A flipped byte
    // and data after the end are the xz project's bad files now.
    let mut padded = whole;
    padded.extend_from_slice(&[0; 3]);
    assert!(
        decode(&padded, &limits).is_ok(),
        "the crate's leniency, recorded"
    );
    let error = read_single(&world, "padded.xz", &padded).unwrap_err();
    assert!(
        error.contains("padding is not a multiple of four"),
        "{error}"
    );
    let peak = peak.stop();
    assert!(peak < 512 << 20, "{:.0} MiB", mib(peak));
}

#[test]
fn gate_four_a_dictionary_over_the_cap_is_refused_before_the_allocation() {
    let world = World::new(Limits::strict());
    let peak = Peak::start();
    // The provider reads the dictionary from the block header first.
    let place = world.put("big.xz", &crate::hostile::xz_block_over_cap());
    let error = world.rows(&place.into_archive()).unwrap_err();
    assert!(
        error.starts_with(
            "it passes the limit for dictionary size: 64 MiB, and big.xz asks for 3.9 GiB"
        ),
        "{error}"
    );
    // The decoder's own limit holds behind it, for any later block.
    let mut over = xz(&vec![0u8; 1 << 16], 0);
    patch_xz_dictionary(&mut over, 34);
    let error = decode(&over, &Limits::strict()).unwrap_err();
    assert!(error.contains("memory for one operation"), "{error}");
    let peak = peak.stop();
    assert!(peak < 512 << 20, "{:.0} MiB", mib(peak));
    // Over the soft limit, it asks.
    let world = World::new(Limits {
        ask_dictionary_bytes: 1 << 20,
        ..Limits::default()
    });
    let place = world.put("asks.xz", &xz(b"eight MiB dictionary", 6));
    let asked = world
        .provider
        .open_archive(&place, &OpContext::new(), Allowances::default())
        .unwrap_err();
    let ArchiveError::Ask(question) = asked else {
        panic!("{asked:?}");
    };
    assert_eq!(
        question.message().text,
        "Needs 8.4 MB of memory for asks.xz. Go on? y or n"
    );
}

#[test]
fn gate_three_a_decode_speed_for_the_report() {
    let data = sample(8 << 20);
    let bytes = xz_with(&data, 1, CheckType::Crc64, None, false);
    let started = Instant::now();
    let out = decode(&bytes, &Limits::default()).unwrap();
    let took = started.elapsed();
    assert_eq!(out.len(), data.len());
    eprintln!(
        "xz decode, {} MiB at preset 1: {took:?}, {:.1} MiB/s (this build's profile)",
        data.len() >> 20,
        mib(data.len() as u64) / took.as_secs_f64()
    );
}

#[test]
fn gate_two_the_xz_program_agrees() {
    if let Err(why) = crate::cross_check::oracle("xz") {
        eprintln!("cross-check against the xz program not run: {why}");
        return;
    }
    let dir = crate::spike_listing::scratch();
    let data = sample(2 << 20);
    let input = dir.path().join("sample.bin");
    std::fs::write(&input, &data).unwrap();
    let world = World::plain();
    let flag_sets: [&[&str]; 8] = [
        &["-0"],
        &["-1"],
        &["-6"],
        &["-6e"],
        &["-6", "-T2"],
        &["--check=crc32"],
        &["--check=sha256"],
        &["--x86", "--lzma2=preset=6"],
    ];
    for flags in flag_sets {
        let output = std::process::Command::new("xz")
            .args(flags)
            .args(["-c", "-k"])
            .arg(&input)
            .output()
            .unwrap();
        assert!(output.status.success(), "xz {flags:?}");
        assert_eq!(
            decode(&output.stdout, &Limits::default()).as_deref(),
            Ok(&data[..]),
            "{flags:?}"
        );
        assert_eq!(
            read_single(&world, "sample.bin.xz", &output.stdout).as_deref(),
            Ok(&data[..]),
            "{flags:?} through the provider"
        );
    }
    // What lzma-rust2 writes, the xz program tests and lists.
    let ours = dir.path().join("ours.xz");
    std::fs::write(
        &ours,
        xz_with(&data, 6, CheckType::Crc64, Some(512 << 10), false),
    )
    .unwrap();
    for check in [["-t"], ["-l"]] {
        let status = std::process::Command::new("xz")
            .args(check)
            .arg(&ours)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "xz {check:?}");
    }
    eprintln!("cross-check against the xz program ran: 8 flag sets, -t and -l");
}
