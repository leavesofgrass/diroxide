//! Spike X1 (research/alpha2-plan.md 4.13): diroxide's parts and sidecar on
//! real files, put back by the tools other people use. Ignored by default,
//! since it runs programs: `cargo nextest run -p diroxide-ops --run-ignored
//! only -E 'test(interop)' --no-capture`.
//!
//! A 5 MB file is split by the engine into parts of 1,000,000 bytes (the
//! email preset would make one part of a 5 MB file), joined back by `cat`
//! where it is on PATH and by `copy /b` from cmd on Windows, each compared
//! by SHA-256 with the whole; the sidecar's CRC-32 is compared with 7-Zip's
//! when `7z` is on PATH, and 7-Zip is fed the parts as a `.001` set. Each
//! tool that is missing is said, never failed.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use diroxide_core::job::PartSize;
use diroxide_core::{JobId, JobKind, JobRequest, Place};
use diroxide_ops::{Engine, JobControl, SidecarRead, Verdict};
use diroxide_vfs::Registry;
use sha2::{Digest, Sha256};

use crate::support::Answers;

fn place_of(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

fn sha_of(path: &Path) -> Vec<u8> {
    Sha256::digest(fs::read(path).unwrap()).to_vec()
}

fn found(program: &str) -> bool {
    Command::new(program)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[test]
#[ignore = "runs cat, cmd and 7z where they are found: --run-ignored only"]
fn interop_parts_join_with_cat_copy_b_and_7z() {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let (source, parts, joined) = (
        root.path().join("source"),
        root.path().join("parts"),
        root.path().join("joined"),
    );
    for folder in [&source, &parts, &joined] {
        fs::create_dir(folder).unwrap();
    }
    let whole: Vec<u8> = (0..5_000_003u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let original = source.join("report.bin");
    fs::write(&original, &whole).unwrap();
    let engine = Engine::new(Arc::new(Registry::with_local()));
    let split = JobRequest::new(JobId::new(1), JobKind::Split, vec![place_of(&original)])
        .to(place_of(&parts))
        .split_into(PartSize::Bytes(1_000_000));
    let plan = engine.plan(&split).unwrap();
    println!("X1 plan: {}", plan.facts.split.as_ref().unwrap().rows()[0]);
    let report = engine.run(&plan, &JobControl::new(), &Answers::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let names: Vec<String> = (1..=6).map(|n| format!("report.bin.{n:03}")).collect();
    for name in &names {
        assert!(parts.join(name).exists(), "{name}");
    }
    let sidecar = SidecarRead::parse(&fs::read(parts.join("report.bin.crc")).unwrap());
    let crc = sidecar.crc32.unwrap();
    assert_eq!(crc, crc32fast::hash(&whole));
    let expected = sha_of(&original);

    if found("cat") {
        let out = fs::File::create(joined.join("by-cat.bin")).unwrap();
        let status = Command::new("cat")
            .args(&names)
            .current_dir(&parts)
            .stdout(out)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(sha_of(&joined.join("by-cat.bin")), expected, "cat");
        println!("X1 cat: SHA-256 matches");
    } else {
        println!("X1 cat: not on PATH, not run");
    }

    if cfg!(windows) {
        let list = names.join("+");
        let target = joined.join("by-copy.bin");
        // cmd reads a forward slash as a switch: the path in backslashes.
        let written = target.to_str().unwrap().replace('/', "\\");
        let status = Command::new("cmd")
            .args(["/C", "copy", "/b", &list, &written])
            .current_dir(&parts)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(sha_of(&target), expected, "copy /b");
        println!("X1 copy /b: SHA-256 matches");
    } else {
        println!("X1 copy /b: not Windows, not run");
    }

    if found("7z") {
        let output = Command::new("7z")
            .args(["h", "-scrcCRC32"])
            .arg(&original)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout).to_uppercase();
        assert!(text.contains(&format!("{crc:08X}")), "7z: {text}");
        let status = Command::new("7z")
            .args(["x", "-y"])
            .arg(format!("-o{}", joined.join("by-7z").display()))
            .arg(parts.join("report.bin.001"))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(sha_of(&joined.join("by-7z").join("report.bin")), expected);
        println!("X1 7z: CRC-32 matches the sidecar, the .001 set extracts whole");
    } else {
        println!("X1 7z: not on PATH, not run");
    }

    // And diroxide's own join, from a middle part, verified.
    let join = JobRequest::new(
        JobId::new(2),
        JobKind::Join,
        vec![place_of(&parts.join("report.bin.003"))],
    )
    .to(place_of(&joined));
    let plan = engine.plan(&join).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Answers::default());
    let outcome = report.joined.unwrap();
    assert_eq!(outcome.verdict, Verdict::Matches);
    assert_eq!(sha_of(&joined.join("report.bin")), expected);
    println!("X1 diroxide: {}", outcome.sentence());
}
