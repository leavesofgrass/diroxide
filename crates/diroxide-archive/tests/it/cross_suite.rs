//! The cross-check suite (plan commit 36, research/archives.md section
//! 11), in three directions, the oracle versions said in the output:
//!
//! - **One:** archives other tools made, read in place by diroxide and held
//!   to the tree they came from.
//! - **Two:** archives diroxide made, unpacked by other tools to the same
//!   tree (with the listing and header checks in `cross_check`).
//! - **Three:** archives diroxide made, read in place and unpacked by
//!   diroxide, in every format, with no tool at all: this one always runs.
//!
//! Directions one and two run only with `DIRO_CROSS_CHECK=1` and the tool
//! on PATH; otherwise each says why it did not run and passes. Natively on
//! the owner's machine: the Windows system tar (bsdtar), Info-ZIP's unzip,
//! gzip and xz from Git for Windows; in the container, 7-Zip, GNU tar,
//! bsdtar, unzip, gzip and xz.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use diroxide_archive::{
    Allowances, ArchiveFormat, ArchiveProvider, FolderRule, JobEnv, JobSecrets, Limits,
    UnpackRequest, plan_unpack, run_unpack,
};
use diroxide_core::{EntryKind, JobId, Place};
use diroxide_ops::{Engine, JobControl};
use diroxide_vfs::{NoSecrets, OpContext, Registry, SecretSource};

use crate::cross_check::{
    oracle, pack_on_disk, pack_on_disk_with, source_tree, tool_says, version_of,
};
use crate::jobs::Sink;
use crate::prompt::Prompt;

/// A tree: each path (forward slashes) with its bytes, or `None` for a
/// folder.
pub type Tree = BTreeMap<String, Option<Vec<u8>>>;

/// The tree on disk under `root`, its own name first in every path.
pub fn disk_tree(root: &Path) -> Tree {
    let mut tree = Tree::new();
    let base = root.parent().unwrap();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let relative: Vec<String> = path
            .strip_prefix(base)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let relative = relative.join("/");
        if path.is_dir() {
            tree.insert(relative, None);
            for item in std::fs::read_dir(&path).unwrap() {
                stack.push(item.unwrap().path());
            }
        } else {
            tree.insert(relative, Some(std::fs::read(&path).unwrap()));
        }
    }
    tree
}

/// The secrets a check asks: the prompt typing `password`, or none.
fn secrets_for(password: Option<&'static str>) -> Arc<dyn SecretSource> {
    match password {
        Some(password) => Prompt::always(password),
        None => Arc::new(NoSecrets),
    }
}

/// What diroxide reads in place from an archive on disk, through the
/// provider and the real local registry.
pub fn archive_tree(archive: &Path, password: Option<&'static str>) -> Tree {
    let (_registry, provider) = ArchiveProvider::install(Registry::with_local(), Limits::default());
    let ctx = JobSecrets::new(secrets_for(password)).context(&OpContext::new());
    let place = Place::parse(&archive.to_string_lossy()).unwrap();
    let mut tree = Tree::new();
    let mut stack = vec![(place.into_archive(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        let listing = provider
            .list_folder(&folder, &ctx, Allowances::default())
            .unwrap();
        for entry in listing.entries {
            let path = format!("{prefix}{}", entry.name);
            let member = folder.join(&entry.name).unwrap();
            if entry.kind == EntryKind::Folder {
                tree.insert(path.clone(), None);
                stack.push((member, format!("{path}/")));
            } else {
                let mut reader = provider
                    .read_member(&member, &ctx, Allowances::default(), None)
                    .unwrap();
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut reader, &mut data).unwrap();
                tree.insert(path, Some(data));
            }
        }
    }
    tree
}

/// Unpacks an archive on disk with diroxide into `into` (made new): what
/// the runner wrote there, as a tree.
pub fn unpack_on_disk(archive: &Path, into: &Path, password: Option<&'static str>) -> Tree {
    std::fs::create_dir_all(into).unwrap();
    let (registry, archives) = ArchiveProvider::install(Registry::with_local(), Limits::default());
    let engine = Engine::new(registry.clone());
    let request = UnpackRequest {
        folder_rule: FolderRule::Never,
        ..UnpackRequest::new(
            Place::parse(&archive.to_string_lossy()).unwrap(),
            Place::parse(&into.to_string_lossy()).unwrap(),
        )
    };
    let job = JobId::new(8);
    let secrets = JobSecrets::new(secrets_for(password));
    let plan = plan_unpack(
        &archives,
        &registry,
        &request,
        job,
        &secrets.context(&OpContext::new()),
        Allowances::default(),
    )
    .unwrap();
    let control = JobControl::new();
    let sink = Sink::skipping();
    let env = JobEnv::new(&engine, &archives, job, &control, &sink).with_secrets(secrets);
    let outcome = run_unpack(&plan, None, Allowances::default(), &env);
    assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
    let mut tree = Tree::new();
    for item in std::fs::read_dir(into).unwrap() {
        tree.extend(disk_tree(&item.unwrap().path()));
    }
    tree
}

/// One kind of archive the round trips cover.
struct Kind {
    name: &'static str,
    format: ArchiveFormat,
    password: Option<&'static str>,
    solid: bool,
}

/// The formats and settings the round trips cover.
fn every_kind() -> Vec<Kind> {
    let kind = |name, format, password, solid| Kind {
        name,
        format,
        password,
        solid,
    };
    vec![
        kind("plain.zip", ArchiveFormat::Zip, None, false),
        kind(
            "locked.zip",
            ArchiveFormat::Zip,
            Some("round-trip-password"),
            false,
        ),
        kind("solid.7z", ArchiveFormat::SevenZ, None, true),
        kind("blocks.7z", ArchiveFormat::SevenZ, None, false),
        kind(
            "locked.7z",
            ArchiveFormat::SevenZ,
            Some("round-trip-password"),
            true,
        ),
        kind("course.tar", ArchiveFormat::Tar, None, false),
        kind("course.tar.gz", ArchiveFormat::TarGz, None, false),
        kind("course.tar.xz", ArchiveFormat::TarXz, None, false),
    ]
}

#[test]
fn direction_three_every_format_round_trips_through_diroxide() {
    let dir = crate::spike_listing::scratch();
    let source = source_tree(dir.path());
    let expected = disk_tree(&source);
    for kind in every_kind() {
        let archive =
            pack_on_disk_with(&source, kind.name, kind.format, kind.password, |request| {
                request.solid = kind.solid;
            });
        assert_eq!(
            archive_tree(&archive, kind.password),
            expected,
            "{}: read in place",
            kind.name
        );
        let into = dir
            .path()
            .join(format!("out-{}", kind.name.replace('.', "-")));
        assert_eq!(
            unpack_on_disk(&archive, &into, kind.password),
            expected,
            "{}: unpacked",
            kind.name
        );
    }
    let notes = source.join("notes.txt");
    let xz = pack_on_disk(&notes, "notes.txt.xz", ArchiveFormat::Xz, None);
    let read = archive_tree(&xz, None);
    assert_eq!(
        read.get("notes.txt"),
        Some(&Some(std::fs::read(&notes).unwrap()))
    );
    eprintln!("cross-check direction three: nine kinds round-trip, no tool needed");
}

#[test]
fn direction_two_diroxide_archives_unpack_in_other_tools() {
    let tools: Vec<&str> = ["tar", "7z"]
        .into_iter()
        .filter(|tool| match oracle(tool) {
            Ok(()) => true,
            Err(why) => {
                eprintln!("cross-check direction two against {tool} not run: {why}");
                false
            }
        })
        .collect();
    if tools.is_empty() {
        return;
    }
    let bsdtar = version_of("tar", "--version").contains("bsdtar");
    let dir = crate::spike_listing::scratch();
    let source = source_tree(dir.path());
    let expected = disk_tree(&source);
    for kind in every_kind() {
        let archive =
            pack_on_disk_with(&source, kind.name, kind.format, kind.password, |request| {
                request.solid = kind.solid;
            });
        let is_7z = kind.name.ends_with(".7z");
        let is_zip = kind.name.ends_with(".zip");
        for tool in &tools {
            // GNU tar reads tars only; bsdtar reads zips and 7z too, but
            // not a locked 7z.
            if *tool == "tar" && (is_zip || is_7z) && !bsdtar {
                continue;
            }
            if *tool == "tar" && is_7z && kind.password.is_some() {
                continue;
            }
            let into = dir
                .path()
                .join(format!("{tool}-{}", kind.name.replace('.', "-")));
            std::fs::create_dir_all(&into).unwrap();
            let mut args: Vec<std::ffi::OsString> = Vec::new();
            if *tool == "7z" {
                args.push("x".into());
                if let Some(password) = kind.password {
                    args.push(format!("-p{password}").into());
                }
                args.push(format!("-o{}", into.display()).into());
                args.push("-y".into());
                args.push(archive.clone().into_os_string());
            } else {
                args.push("-x".into());
                if let Some(password) = kind.password {
                    args.push("--passphrase".into());
                    args.push(password.into());
                }
                args.push("-f".into());
                args.push(archive.clone().into_os_string());
                args.push("-C".into());
                args.push(into.clone().into_os_string());
            }
            let args: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
            let (ok, said) = tool_says(tool, &args);
            assert!(ok, "{tool} on {}: {said}", kind.name);
            // 7-Zip takes one layer off a compressed tar, leaving the tar
            // beside it; a second pass unpacks that.
            if *tool == "7z"
                && let Some(inner) = kind
                    .name
                    .strip_suffix(".gz")
                    .or(kind.name.strip_suffix(".xz"))
            {
                let tar = into.join(inner);
                let args: Vec<std::ffi::OsString> = vec![
                    "x".into(),
                    format!("-o{}", into.display()).into(),
                    "-y".into(),
                    tar.clone().into_os_string(),
                ];
                let args: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
                let (ok, said) = tool_says(tool, &args);
                assert!(ok, "{tool} on {} (second layer): {said}", tar.display());
            }
            assert_eq!(
                disk_tree(&into.join("course")),
                expected,
                "{tool} unpacked {} to the same tree",
                kind.name
            );
            let version = if *tool == "7z" {
                version_of("7z", "i")
            } else {
                version_of("tar", "--version")
            };
            eprintln!(
                "cross-check direction two: {} unpacked by {tool} ({version}) to the same tree",
                kind.name
            );
        }
    }
}

#[test]
fn direction_one_archives_made_by_tools_read_in_diroxide() {
    let dir = crate::spike_listing::scratch();
    let source = source_tree(dir.path());
    let expected = disk_tree(&source);
    let mut made = 0;
    let run_in = |tool: &str, args: &[&str]| {
        let output = Command::new(tool)
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{tool} {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    match oracle("tar") {
        Ok(()) => {
            let version = version_of("tar", "--version");
            let mut cases = vec![
                ("tool.tar", vec!["-cf", "tool.tar", "course"]),
                ("tool.tar.gz", vec!["-czf", "tool.tar.gz", "course"]),
            ];
            if oracle("xz").is_ok() || version.contains("bsdtar") {
                cases.push(("tool.tar.xz", vec!["-cJf", "tool.tar.xz", "course"]));
            }
            if version.contains("bsdtar") {
                cases.push((
                    "tool.zip",
                    vec!["--format", "zip", "-cf", "tool.zip", "course"],
                ));
                cases.push((
                    "tool.7z",
                    vec!["--format", "7zip", "-cf", "tool.7z", "course"],
                ));
            }
            for (name, args) in cases {
                run_in("tar", &args);
                assert_eq!(
                    archive_tree(&dir.path().join(name), None),
                    expected,
                    "{name}"
                );
                eprintln!(
                    "cross-check direction one: {name} made by tar ({version}) reads in diroxide"
                );
                made += 1;
            }
        }
        Err(why) => eprintln!("cross-check direction one against tar not run: {why}"),
    }
    match oracle("xz") {
        Ok(()) => {
            let output = Command::new("xz")
                .args(["-c", "-6"])
                .arg(source.join("notes.txt"))
                .output()
                .unwrap();
            let path = dir.path().join("notes.txt.xz");
            std::fs::write(&path, output.stdout).unwrap();
            let read = archive_tree(&path, None);
            assert_eq!(read.get("notes.txt"), expected.get("course/notes.txt"));
            eprintln!(
                "cross-check direction one: notes.txt.xz made by xz ({}) reads in diroxide",
                version_of("xz", "--version")
            );
            made += 1;
        }
        Err(why) => eprintln!("cross-check direction one against xz not run: {why}"),
    }
    match oracle("gzip") {
        Ok(()) => {
            // Two gzip members one after the other read as one file.
            let mut joined = Vec::new();
            for part in ["first ", "second"] {
                let mut child = Command::new("gzip")
                    .arg("-c")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let mut stdin = child.stdin.take().unwrap();
                std::io::Write::write_all(&mut stdin, part.as_bytes()).unwrap();
                drop(stdin);
                joined.extend(child.wait_with_output().unwrap().stdout);
            }
            let path = dir.path().join("joined.txt.gz");
            std::fs::write(&path, joined).unwrap();
            let read = archive_tree(&path, None);
            assert_eq!(
                read.get("joined.txt"),
                Some(&Some(b"first second".to_vec()))
            );
            eprintln!(
                "cross-check direction one: a two-member gzip from gzip ({}) reads whole",
                version_of("gzip", "--version")
            );
            made += 1;
        }
        Err(why) => eprintln!("cross-check direction one against gzip not run: {why}"),
    }
    match oracle("7z") {
        Ok(()) => {
            let version = version_of("7z", "i");
            let cases: [(&str, &[&str], Option<&'static str>); 5] = [
                ("seven.zip", &["a", "-tzip", "seven.zip", "course"], None),
                (
                    "seven-solid.7z",
                    &["a", "-t7z", "-ms=on", "seven-solid.7z", "course"],
                    None,
                ),
                (
                    "seven-blocks.7z",
                    &["a", "-t7z", "-ms=off", "seven-blocks.7z", "course"],
                    None,
                ),
                (
                    "seven-locked.7z",
                    &[
                        "a",
                        "-t7z",
                        "-psecret-7z",
                        "-mhe=on",
                        "seven-locked.7z",
                        "course",
                    ],
                    Some("secret-7z"),
                ),
                (
                    "seven-aes.zip",
                    &[
                        "a",
                        "-tzip",
                        "-mem=AES256",
                        "-psecret-7z",
                        "seven-aes.zip",
                        "course",
                    ],
                    Some("secret-7z"),
                ),
            ];
            for (name, args, password) in cases {
                run_in("7z", args);
                assert_eq!(
                    archive_tree(&dir.path().join(name), password),
                    expected,
                    "{name}"
                );
                eprintln!(
                    "cross-check direction one: {name} made by 7z ({version}) reads in diroxide"
                );
                made += 1;
            }
        }
        Err(why) => eprintln!("cross-check direction one against 7z not run: {why}"),
    }
    eprintln!("cross-check direction one: {made} archives made by other tools checked");
}
