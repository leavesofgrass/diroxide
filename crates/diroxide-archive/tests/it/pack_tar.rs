//! Pack to tar, tar.gz, tar.xz and xz (plan commit 33): GNU format, no
//! owner names, no sparse entries, times and modes from the sources; xz
//! holds one file; tar takes no password.

use std::io::Read;

use diroxide_archive::{ArchiveError, ArchiveFormat, PackRequest, plan_pack};
use diroxide_core::{JobId, Secret};
use diroxide_vfs::OpContext;

use crate::jobs::{Sink, out};
use crate::pack_zip::{plan_of, run_plan, sources};
use crate::world::World;

/// One header of a tar, as the tar crate reads it.
type Header = (String, u64, u32, u64, u64, u64, String, String, bool);

/// Each header of a tar: path, size, mode, mtime, uid, gid, owner and
/// group names, type, and whether it is GNU's format.
fn headers(tar: &[u8]) -> Vec<Header> {
    let mut archive = tar::Archive::new(tar);
    let mut found = Vec::new();
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        let header = entry.header();
        found.push((
            entry.path().unwrap().to_string_lossy().into_owned(),
            header.size().unwrap(),
            header.mode().unwrap(),
            header.mtime().unwrap(),
            header.uid().unwrap(),
            header.gid().unwrap(),
            header.username().unwrap().unwrap_or("").to_owned(),
            header.groupname().unwrap().unwrap_or("").to_owned(),
            header.as_gnu().is_some(),
        ));
    }
    found
}

fn unxz(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    lzma_rust2::XzReader::new(bytes, true)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn ungzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap();
    out
}

#[test]
fn the_tar_family_packs_in_gnu_format_without_owner_names() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    for format in [
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
    ] {
        let destination = out.join(&format!("docs{}", format.extension())).unwrap();
        let request = PackRequest::new(vec![docs.clone()], destination.clone(), format);
        let plan = plan_of(&world, &request);
        let rows = plan.rows();
        assert!(
            rows.contains(&"Owner names: left out".to_owned()),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.starts_with("Password: not for tar")),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.starts_with("Solid: not for tar")),
            "{rows:?}"
        );
        let outcome = run_plan(&world, &plan, &Sink::skipping());
        assert_eq!(
            outcome.lines,
            [
                "Pack finished: 6 done.",
                "Checked 6 of 6 entries, all good."
            ],
            "{format:?}"
        );
        let bytes = world.fake.contents(&destination).unwrap();
        let tar = match format {
            ArchiveFormat::Tar => bytes,
            ArchiveFormat::TarGz => ungzip(&bytes),
            _ => unxz(&bytes),
        };
        let headers = headers(&tar);
        assert_eq!(headers.len(), 6, "{format:?}: {headers:?}");
        for (path, _, mode, mtime, uid, gid, owner, group, gnu) in &headers {
            assert!(gnu, "{path}: GNU format");
            assert_eq!((*uid, *gid), (0, 0), "{path}");
            assert!(
                owner.is_empty() && group.is_empty(),
                "{path}: no owner names"
            );
            if path.ends_with(".txt") {
                assert_eq!(*mode, 0o640, "{path}");
                assert!(*mtime >= 1_760_000_000, "{path}: the source's time");
            }
        }
        assert_eq!(headers[2].0, "docs/notes.txt");
        assert_eq!(headers[2].1, 36);
    }
}

#[test]
fn long_names_and_levels_read_back() {
    let world = World::plain();
    let deep = world.root.parent().unwrap().join("deep").unwrap();
    let long = "a-name-long-enough-to-need-the-gnu-long-name-entry-in-a-tar-header-of-one-hundred-bytes.txt";
    world.fake.add_file(&deep.join(long).unwrap(), b"long");
    let out = out(&world);
    for level in [
        diroxide_archive::CompressionLevel::Fastest,
        diroxide_archive::CompressionLevel::Ultra,
    ] {
        let destination = out.join(&format!("deep-{}.tar.xz", level.word())).unwrap();
        let request = PackRequest {
            level,
            ..PackRequest::new(
                vec![deep.clone()],
                destination.clone(),
                ArchiveFormat::TarXz,
            )
        };
        let outcome = run_plan(&world, &plan_of(&world, &request), &Sink::skipping());
        assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
        let tar = unxz(&world.fake.contents(&destination).unwrap());
        assert_eq!(headers(&tar)[1].0, format!("deep/{long}"));
    }
}

#[test]
fn xz_holds_one_file() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let notes = docs.join("notes.txt").unwrap();
    let destination = out.join("notes.txt.xz").unwrap();
    let request = PackRequest::new(vec![notes.clone()], destination.clone(), ArchiveFormat::Xz);
    let plan = plan_of(&world, &request);
    assert!(
        plan.rows()
            .contains(&"Format: xz, level normal (6 of 9)".to_owned())
    );
    let outcome = run_plan(&world, &plan, &Sink::skipping());
    assert_eq!(outcome.lines[1], "Checked 1 of 1 entries, all good.");
    assert_eq!(
        unxz(&world.fake.contents(&destination).unwrap()),
        b"the notes, again and again and again"
    );
    let refuse = |sources: Vec<diroxide_core::Place>| {
        plan_pack(
            &world.registry,
            &PackRequest::new(sources, out.join("many.xz").unwrap(), ArchiveFormat::Xz),
            JobId::new(41),
            &OpContext::new(),
        )
        .unwrap_err()
    };
    let folder = refuse(vec![docs.clone()]);
    assert_eq!(
        folder.to_string(),
        "Could not pack many.xz: xz holds one file. Use tar.xz."
    );
    let two = refuse(vec![notes.clone(), docs.join("draft.tmp").unwrap()]);
    assert_eq!(two.to_string(), folder.to_string());
}

/// W2g's finding (P2b): a plain xz whose name is not the file's own name
/// and `.xz` (`diro archive pack one.xz notes.txt`) failed its own test
/// after packing, since an xz keeps no name and reads back as its own
/// stem. The member is stored and checked as `one`, and the plan says so.
#[test]
fn an_xz_named_otherwise_holds_its_file_as_its_stem() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let notes = docs.join("notes.txt").unwrap();
    let destination = out.join("one.xz").unwrap();
    let request = PackRequest::new(vec![notes], destination.clone(), ArchiveFormat::Xz);
    let plan = plan_of(&world, &request);
    assert!(
        plan.rows()
            .contains(&"Unpacks as one: xz keeps no name".to_owned()),
        "{:?}",
        plan.rows()
    );
    let outcome = run_plan(&world, &plan, &Sink::skipping());
    assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
    assert_eq!(outcome.lines[1], "Checked 1 of 1 entries, all good.");
    assert_eq!(
        unxz(&world.fake.contents(&destination).unwrap()),
        b"the notes, again and again and again"
    );
    assert!(!world.fake.exists(&plan.temp), "nothing is left behind");
    // The name the file's own and `.xz` needs no such row.
    let same = PackRequest::new(
        vec![docs.join("notes.txt").unwrap()],
        out.join("notes.txt.xz").unwrap(),
        ArchiveFormat::Xz,
    );
    assert!(
        !plan_of(&world, &same)
            .rows()
            .iter()
            .any(|row| row.starts_with("Unpacks as"))
    );
}

#[test]
fn tar_takes_no_password() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let request = PackRequest {
        password: Some(Secret::new("a password".to_owned())),
        ..PackRequest::new(
            vec![docs],
            out.join("locked.tar.gz").unwrap(),
            ArchiveFormat::TarGz,
        )
    };
    let refused =
        plan_pack(&world.registry, &request, JobId::new(41), &OpContext::new()).unwrap_err();
    assert!(matches!(refused, ArchiveError::Refused(_)));
    assert_eq!(
        refused.message("pack", "locked.tar.gz").text,
        "Could not pack locked.tar.gz: tar.gz keeps no password. Use 7z or zip."
    );
}
