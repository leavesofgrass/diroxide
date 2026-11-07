//! The version check after a helper is installed (alpha 4, R28 1.3): the
//! stand-in program plays the helper, so no real helper is ever run. It is
//! packed in a zip as `pandoc`, served by the fake fetcher, downloaded and
//! unpacked for this computer; then its version line is said, or that it
//! did not start, and nothing is removed either way. Nothing here opens a
//! connection.

#![cfg(feature = "fake-tools")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;

use diroxide_components::fake::{FakeAnswer, FakeFetcher};
use diroxide_components::messages;
use diroxide_components::{
    Action, Component, Consent, Downloader, FilePin, Kind, Sources, Status, host_target,
    install_from,
};
use diroxide_vfs::OpContext;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ADDRESS: &str = "https://example.invalid/pandoc-3.13.zip";

/// The program's name in the component, as this system starts it.
fn program() -> String {
    format!("pandoc{}", std::env::consts::EXE_SUFFIX)
}

/// A zip holding the stand-in as `pandoc` under a top folder, with what
/// it prints and its exit code beside it, and a file `keep` leaves out.
fn helper_zip(stdout: &[u8], exit: &str) -> Vec<u8> {
    let stand_in = fs::read(env!("CARGO_BIN_EXE_diroxide-fake-tool")).unwrap();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, bytes) in [
        (format!("pandoc-3.13/{}", program()), stand_in.as_slice()),
        ("pandoc-3.13/pandoc-stdout.bin".to_owned(), stdout),
        ("pandoc-3.13/pandoc-exit.txt".to_owned(), exit.as_bytes()),
        ("pandoc-3.13/MANUAL.html".to_owned(), b"manual".as_slice()),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn helper(bytes: &[u8]) -> Component {
    let mut pin = FilePin::of_bytes("pandoc-3.13.zip", bytes);
    pin.url = Some(ADDRESS.to_owned());
    pin.target = host_target().to_owned();
    pin.strip = 1;
    pin.keep = vec!["pandoc*".into()];
    pin.exec = Some(program());
    let mut c = Component::new("pandoc", "pandoc", vec![pin]);
    c.kind = Kind::Helper;
    c.version = "3.13".into();
    c.license = "GPL-2.0-or-later".into();
    c.action = Action::Unpack;
    c.unpacked_size = bytes.len() as u64 * 4;
    c.tools = vec!["pandoc".into()];
    c
}

fn download(c: &Component, bytes: Vec<u8>) -> diroxide_components::Outcome {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let fake = Arc::new(FakeFetcher::new());
    fake.serve(ADDRESS, FakeAnswer::Bytes(bytes));
    let downloader = Downloader::new(root.clone(), Sources::offline(), fake);
    let outcome = downloader
        .download(c, &Consent::yes(c), &OpContext::new(), &mut |_| {})
        .unwrap();
    // Nothing is removed, whatever the check said.
    assert_eq!(c.status_in(&root, host_target()), Status::Installed);
    assert!(c.dir_in(&root).join(program()).is_file());
    assert!(!c.dir_in(&root).join("MANUAL.html").exists(), "not kept");
    outcome
}

#[test]
fn a_helper_that_starts_says_its_version() {
    let bytes = helper_zip(b"\npandoc 3.13\nFeatures: +server +lua\n", "0");
    let c = helper(&bytes);
    let outcome = download(&c, bytes);
    assert_eq!(outcome.started, ["pandoc: 3.13, from components"]);
    assert_eq!(
        messages::installed(&c, &outcome),
        ["Ready: pandoc 3.13.", "pandoc: 3.13, from components"]
    );
}

#[test]
fn a_helper_that_says_nothing_did_not_start_and_stays() {
    let bytes = helper_zip(b"", "1");
    let c = helper(&bytes);
    let outcome = download(&c, bytes);
    assert_eq!(outcome.started, ["Installed pandoc, but it did not start."]);
}

/// An install from a file runs the same check.
#[test]
fn an_install_from_a_file_checks_the_version_too() {
    let bytes = helper_zip(b"pandoc 3.13\n", "0");
    let c = helper(&bytes);
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("pandoc-3.13.zip");
    fs::write(&from, &bytes).unwrap();
    let root = tmp.path().join("components");
    let outcome = install_from(&c, &from, &root, host_target(), &OpContext::new()).unwrap();
    assert_eq!(outcome.started, ["pandoc: 3.13, from components"]);
    // For another computer's files nothing is run.
    let other = install_from(
        &c,
        &from,
        &tmp.path().join("other"),
        "x86_64-unknown-freebsd",
        &OpContext::new(),
    );
    assert!(other.is_err(), "no build for that computer");
}
