//! Unpacking and installing from a file (R22 section 6.6): the hostile
//! archive set, the ceiling, the member count, strip, the receipt, Remove,
//! and installs from a folder and from zips.

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use diroxide_vfs::OpContext;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::component::{Action, Component, FileState, Status};
use crate::download::{Consent, Downloader, Sources};
use crate::error::ComponentError;
use crate::fake::{FakeAnswer, FakeFetcher};
use crate::pin::{ANY_TARGET, FilePin};
use crate::receipt::Receipt;
use crate::unpack::{install_from, unpack_with};

/// A zip of files (name, bytes) and links (name, target), as written.
fn zip_of(files: &[(&str, &[u8])], links: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    for (name, target) in links {
        zip.add_symlink(*name, *target, options).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn pack(bytes: &[u8], unpacked: u64) -> Component {
    let mut c = Component::new(
        "pack",
        "Example pack",
        vec![FilePin::of_bytes("pack-0.1.0.zip", bytes)],
    );
    c.action = Action::Unpack;
    c.unpacked_size = unpacked;
    c
}

/// Places the archive in the component's folder, as a download's finish
/// does, and unpacks it.
fn place_and_unpack(
    root: &Path,
    c: &Component,
    bytes: &[u8],
    max: usize,
) -> Result<Vec<(String, String)>, ComponentError> {
    let dir = c.dir_in(root);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(&c.files[0].name), bytes).unwrap();
    unpack_with(c, &dir, &OpContext::new(), max)
}

/// Every file under `root`, as paths with `/`, sorted.
fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in fs::read_dir(&dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_hostile_archive_writes_only_plain_files_inside_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let bytes = zip_of(
        &[
            ("good/readme.txt", b"fine"),
            ("../escape.txt", b"out"),
            ("/abs.txt", b"abs"),
            ("dir\\back.txt", b"back"),
            ("con.txt", b"device"),
            ("aux", b"device"),
            ("a:stream.txt", b"stream"),
            ("C:/drive.txt", b"drive"),
            ("x\u{202e}txt.exe", b"bidi"),
            ("ctl\u{7}.txt", b"bell"),
            ("Same.txt", b"one"),
            ("same.txt", b"two"),
            ("trailing.", b"dot"),
        ],
        &[("link", "../../outside")],
    );
    let c = pack(&bytes, 4096);
    let left_out = place_and_unpack(&root, &c, &bytes, 1000).unwrap();
    let dir = c.dir_in(&root);
    assert_eq!(
        tree(tmp.path()),
        [
            "components/pack/Same.txt",
            "components/pack/good/readme.txt",
            "components/pack/pack-0.1.0.zip.unpacked"
        ]
    );
    let reasons: Vec<String> = left_out
        .iter()
        .map(|(name, why)| format!("{name}: {why}"))
        .collect();
    let said = reasons.join("\n");
    for words in [
        "name leaves the archive",
        "abs.txt: leading slash removed",
        "back.txt: backslashes read as folders",
        "con.txt: con.txt is not a plain name",
        "aux: aux is not a plain name",
        "stream.txt: drive removed",
        "drive.txt: drive removed",
        "x[U+202E]txt.exe: has hidden characters",
        "contains a control character",
        "same.txt: differs only by case",
        "link: a link or a special file",
        "trailing.: trailing. is not a plain name",
    ] {
        assert!(said.contains(words), "{words} missing from:\n{said}");
    }
    let receipt = Receipt::matching(&dir, &c.files[0]).unwrap();
    let listed: Vec<&str> = receipt.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(listed, ["good/readme.txt", "Same.txt"]);
    assert_eq!(c.status_in(&root, ANY_TARGET), Status::Installed);
    assert!(
        c.verify_in(&root, ANY_TARGET)
            .iter()
            .all(|f| f.state == FileState::Good)
    );
    // Remove takes exactly what the receipt lists; the owner's file stays.
    fs::write(dir.join("good").join("mine.txt"), b"mine").unwrap();
    let removal = c.removal_in(&root);
    assert_eq!(removal.summary(), "Removes 3 files. Yours stay.");
    removal.run().unwrap();
    assert_eq!(tree(tmp.path()), ["components/pack/good/mine.txt"]);
}

#[test]
fn an_archive_that_expands_past_its_size_is_stopped_and_cleaned_up() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let zeros = vec![0u8; 2 << 20];
    let bytes = zip_of(&[("a.txt", b"small"), ("bomb.bin", &zeros)], &[]);
    let c = pack(&bytes, 1000);
    let error = place_and_unpack(&root, &c, &bytes, 1000).unwrap_err();
    assert_eq!(error.to_string(), "it unpacks to more than its list says");
    assert_eq!(
        tree(&root),
        ["pack/pack-0.1.0.zip"],
        "only the archive is left"
    );
    assert_eq!(c.status_in(&root, ANY_TARGET), Status::NotInstalled);
}

#[test]
fn an_archive_with_too_many_members_is_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let files: Vec<(String, Vec<u8>)> = (0..8)
        .map(|i| (format!("f{i}.txt"), vec![b'x'; 3]))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let bytes = zip_of(&refs, &[]);
    let c = pack(&bytes, 4096);
    assert!(place_and_unpack(&root, &c, &bytes, 5).is_err());
    assert_eq!(tree(&root), ["pack/pack-0.1.0.zip"]);
}

#[test]
fn strip_drops_the_top_folder_and_leaves_out_what_is_above_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let bytes = zip_of(
        &[
            ("top/a.txt", b"a"),
            ("top/sub/b.txt", b"b"),
            ("loose.txt", b"l"),
        ],
        &[],
    );
    let mut c = pack(&bytes, 4096);
    c.files[0].strip = 1;
    let left_out = place_and_unpack(&root, &c, &bytes, 1000).unwrap();
    assert_eq!(
        tree(&root),
        [
            "pack/a.txt",
            "pack/pack-0.1.0.zip.unpacked",
            "pack/sub/b.txt"
        ]
    );
    assert_eq!(
        left_out,
        [(
            "loose.txt".to_owned(),
            "outside the folder that is stripped".to_owned()
        )]
    );
}

#[cfg(unix)]
#[test]
fn only_the_named_program_gets_the_executable_bit_and_links_are_not_followed() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let bytes = zip_of(
        &[
            ("bin/tool", b"#!/bin/sh\n"),
            ("data.txt", b"d"),
            ("into/x.txt", b"x"),
        ],
        &[],
    );
    let mut c = pack(&bytes, 4096);
    c.kind = crate::component::Kind::Helper;
    c.files[0].exec = Some("bin/tool".into());
    let dir = c.dir_in(&root);
    fs::create_dir_all(&dir).unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("into")).unwrap();
    let left_out = place_and_unpack(&root, &c, &bytes, 1000).unwrap();
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir.join("bin").join("tool")) & 0o111, 0o111);
    assert_eq!(mode(&dir.join("data.txt")) & 0o111, 0);
    assert!(
        left_out
            .iter()
            .any(|(n, why)| n == "into/x.txt" && why == "a link is in the way")
    );
    assert!(
        fs::read_dir(&outside).unwrap().next().is_none(),
        "nothing written through the link"
    );
}

#[test]
fn a_download_of_a_pack_unpacks_it_and_a_bad_unpack_keeps_the_archive_staged() {
    let tmp = tempfile::tempdir().unwrap();
    let bytes = zip_of(
        &[
            ("plugins/a/plugin.toml", b"name = \"a\""),
            ("README.md", b"r"),
        ],
        &[],
    );
    let c = pack(&bytes, 4096);
    let fake = Arc::new(FakeFetcher::new());
    fake.serve(
        "https://public.invalid/r/pack-0.1.0/pack-0.1.0.zip",
        FakeAnswer::Bytes(bytes.clone()),
    );
    let sources = Sources {
        source: None,
        releases: Some("https://public.invalid/r".into()),
    };
    let d = Downloader::new(tmp.path().join("components"), sources, fake.clone())
        .with_target(ANY_TARGET);
    let outcome = d
        .download(&c, &Consent::yes(&c), &OpContext::new(), &mut |_| {})
        .unwrap();
    assert_eq!(outcome.fetched, ["pack-0.1.0.zip"]);
    assert!(outcome.left_out.is_empty());
    let root = d.components_dir();
    assert_eq!(
        tree(root),
        [
            "pack/README.md",
            "pack/pack-0.1.0.zip.unpacked",
            "pack/plugins/a/plugin.toml"
        ]
    );
    assert_eq!(c.status_in(root, ANY_TARGET), Status::Installed);
    // A second download keeps it and fetches nothing.
    d.download(&c, &Consent::yes(&c), &OpContext::new(), &mut |_| {})
        .unwrap();
    assert_eq!(fake.requests().len(), 1);
    // A list whose unpacked size is too small: the unpack stops, and the
    // checked archive waits in the staging folder for the next try.
    let mut small = c.clone();
    small.id = "small".into();
    small.folder = "small".into();
    small.unpacked_size = 1;
    let zeros = zip_of(&[("big.bin", &vec![0u8; 1 << 20])], &[]);
    small.files = vec![FilePin::of_bytes("small-0.1.0.zip", &zeros)];
    fake.serve(
        "https://public.invalid/r/small-0.1.0/small-0.1.0.zip",
        FakeAnswer::Bytes(zeros),
    );
    let error = d.download(
        &small,
        &Consent::yes(&small),
        &OpContext::new(),
        &mut |_| {},
    );
    assert!(matches!(error, Err(ComponentError::Unpack(_))), "{error:?}");
    assert_eq!(small.status_in(root, ANY_TARGET), Status::NotInstalled);
    assert!(small.staging_in(root).join("small-0.1.0.zip").is_file());
}

#[test]
fn install_from_a_folder_finds_files_by_name_and_refuses_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let bytes = zip_of(&[("a.txt", b"a")], &[]);
    let c = pack(&bytes, 4096);
    let folder = tmp.path().join("stick");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("pack-0.1.0.zip"), &bytes).unwrap();
    fs::write(folder.join("notes.txt"), b"other").unwrap();
    let outcome = install_from(&c, &folder, &root, ANY_TARGET, &OpContext::new()).unwrap();
    assert_eq!(outcome.fetched, ["pack-0.1.0.zip"]);
    assert_eq!(
        outcome.left_out,
        [(
            "notes.txt".to_owned(),
            "not one of the component's files".to_owned()
        )]
    );
    assert_eq!(c.status_in(&root, ANY_TARGET), Status::Installed);
    assert!(
        folder.join("pack-0.1.0.zip").is_file(),
        "the source is copied, never moved"
    );
}

#[test]
fn install_from_a_folder_finds_a_renamed_file_by_size_and_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let c = Component::new(
        "data",
        "Data",
        vec![FilePin::of_bytes("words.txt", b"some words")],
    );
    let folder = tmp.path().join("stick");
    fs::create_dir_all(folder.join("data-0.1.0")).unwrap();
    fs::write(folder.join("data-0.1.0").join("renamed.txt"), b"some words").unwrap();
    install_from(&c, &folder, &root, ANY_TARGET, &OpContext::new()).unwrap();
    assert_eq!(
        fs::read(c.dir_in(&root).join("words.txt")).unwrap(),
        b"some words"
    );
}

#[test]
fn install_from_a_zip_that_is_the_pinned_archive_or_holds_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let inner = zip_of(&[("plugins/x.toml", b"x")], &[]);
    let c = pack(&inner, 4096);
    // The archive itself, under any name.
    let direct = tmp.path().join("downloaded.zip");
    fs::write(&direct, &inner).unwrap();
    install_from(&c, &direct, &root, ANY_TARGET, &OpContext::new()).unwrap();
    assert_eq!(c.status_in(&root, ANY_TARGET), Status::Installed);
    c.removal_in(&root).run().unwrap();
    // A zip that holds it, with a stranger beside it.
    let outer = zip_of(
        &[
            ("bundle/pack-0.1.0.zip", &inner),
            ("bundle/other.txt", b"o"),
        ],
        &[],
    );
    let container = tmp.path().join("bundle.zip");
    fs::write(&container, &outer).unwrap();
    let outcome = install_from(&c, &container, &root, ANY_TARGET, &OpContext::new()).unwrap();
    assert_eq!(c.status_in(&root, ANY_TARGET), Status::Installed);
    assert!(outcome.left_out.contains(&(
        "bundle/other.txt".to_owned(),
        "not one of the component's files".to_owned()
    )));
}

#[test]
fn install_from_a_folder_without_the_file_installs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("components");
    let c = Component::new(
        "data",
        "Data",
        vec![
            FilePin::of_bytes("a.bin", b"aaa"),
            FilePin::of_bytes("b.bin", b"bbb"),
        ],
    );
    let folder = tmp.path().join("stick");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("a.bin"), b"aaa").unwrap();
    fs::write(folder.join("b.bin"), b"bad").unwrap();
    let error = install_from(&c, &folder, &root, ANY_TARGET, &OpContext::new()).unwrap_err();
    assert_eq!(error.to_string(), "b.bin: it was not found in stick");
    assert!(!c.dir_in(&root).exists());
    assert!(!c.staging_in(&root).exists());
}
