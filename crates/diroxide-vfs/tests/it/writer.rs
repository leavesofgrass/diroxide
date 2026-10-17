//! The committing writer and the fast copy (research/file-operations.md
//! C1 and the copy edge cases): a write that fails or is dropped part way
//! leaves neither a half file nor an unfinished one, a new file never
//! replaces one that appeared meanwhile, sizes around the 1 MiB chunk copy
//! exactly, and a fast copy the volume cannot make answers "unsupported"
//! without touching the destination.

use std::fs;
use std::io::{Read, Write};

use diroxide_core::Place;
use diroxide_vfs::{CopyHint, LocalProvider, OpContext, Provider, VfsError, WriteMode};

use crate::local::place_of;
use crate::scratch;

/// Every name in a folder, the unfinished ones included.
fn everything_in(folder: &tempfile::TempDir) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder.path())
        .unwrap()
        .map(|item| item.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn child(folder: &tempfile::TempDir, name: &str) -> Place {
    place_of(folder).join(name).unwrap()
}

#[test]
fn a_write_dropped_part_way_leaves_nothing() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let target = child(&folder, "report.txt");
    let mut writer = provider
        .write(&target, WriteMode::CreateNew, &OpContext::new())
        .unwrap();
    // The bytes go to an unfinished name, never the real one.
    writer.write_all(&[7u8; 4096]).unwrap();
    let during = everything_in(&folder);
    assert_eq!(during.len(), 1);
    assert!(diroxide_vfs::names::is_unfinished(&during[0]), "{during:?}");
    // A fault at byte N: the job drops the writer.
    drop(writer);
    assert!(everything_in(&folder).is_empty());
}

/// The writer says where its bytes are until the commit, so the journal
/// can tie the unfinished name to its target (W1b's contract request 1).
#[test]
fn the_writer_says_its_unfinished_place() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let target = child(&folder, "report.txt");
    let mut writer = provider
        .write(&target, WriteMode::CreateNew, &OpContext::new())
        .unwrap();
    writer.write_all(b"draft").unwrap();
    let unfinished = writer.unfinished_place().expect("the local writer says");
    assert_eq!(unfinished.parent(), target.parent(), "beside its target");
    let name = unfinished.name().unwrap().to_owned();
    assert!(diroxide_vfs::names::is_unfinished(&name), "{name}");
    assert_eq!(everything_in(&folder), [name]);
    writer.commit(false).unwrap();
    assert_eq!(everything_in(&folder), ["report.txt"]);
}

/// W2b's request 1 (P2b): the local writer seeks in its unfinished file,
/// so a header written last lands in front, and nothing reaches the
/// target before the commit.
#[test]
fn the_local_writer_seeks_in_its_unfinished_file() {
    use std::io::SeekFrom;
    let folder = scratch();
    let provider = LocalProvider::new();
    let target = child(&folder, "archive.7z");
    let mut writer = provider
        .write(&target, WriteMode::CreateNew, &OpContext::new())
        .unwrap();
    assert_eq!(writer.seek_to(SeekFrom::Start(4)), Ok(4));
    writer.write_all(b"BODY").unwrap();
    assert_eq!(writer.seek_to(SeekFrom::Current(0)), Ok(8));
    assert_eq!(writer.seek_to(SeekFrom::Start(0)), Ok(0));
    writer.write_all(b"HEAD").unwrap();
    assert_eq!(writer.seek_to(SeekFrom::End(0)), Ok(8));
    assert!(
        !folder.path().join("archive.7z").exists(),
        "not before the commit"
    );
    writer.commit(false).unwrap();
    assert_eq!(
        fs::read(folder.path().join("archive.7z")).unwrap(),
        b"HEADBODY"
    );
    assert_eq!(everything_in(&folder), ["archive.7z"]);
}

#[test]
fn a_new_file_never_replaces_one_that_appeared_meanwhile() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let target = child(&folder, "race.txt");
    let mut writer = provider
        .write(&target, WriteMode::CreateNew, &OpContext::new())
        .unwrap();
    writer.write_all(b"mine").unwrap();
    fs::write(folder.path().join("race.txt"), b"theirs").unwrap();
    assert_eq!(writer.commit(false), Err(VfsError::AlreadyExists));
    assert_eq!(fs::read(folder.path().join("race.txt")).unwrap(), b"theirs");
    assert_eq!(everything_in(&folder), vec!["race.txt".to_owned()]);
}

#[test]
fn an_overwrite_replaces_only_at_the_commit() {
    let folder = scratch();
    fs::write(folder.path().join("notes.txt"), b"old").unwrap();
    let provider = LocalProvider::new();
    let target = child(&folder, "notes.txt");
    let mut writer = provider
        .write(&target, WriteMode::Overwrite, &OpContext::new())
        .unwrap();
    writer.write_all(b"new").unwrap();
    assert_eq!(fs::read(folder.path().join("notes.txt")).unwrap(), b"old");
    writer.commit(true).unwrap();
    assert_eq!(fs::read(folder.path().join("notes.txt")).unwrap(), b"new");
    assert_eq!(everything_in(&folder), vec!["notes.txt".to_owned()]);
}

#[test]
fn sizes_around_the_chunk_size_write_exactly() {
    const MIB: usize = 1024 * 1024;
    let folder = scratch();
    let provider = LocalProvider::new();
    for size in [0, 1, MIB - 1, MIB, MIB + 1] {
        let name = format!("size-{size}.bin");
        let bytes: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
        let mut writer = provider
            .write(
                &child(&folder, &name),
                WriteMode::CreateNew,
                &OpContext::new(),
            )
            .unwrap();
        writer.write_all(&bytes).unwrap();
        writer.commit(false).unwrap();
        let mut back = Vec::new();
        provider
            .read(&child(&folder, &name), &OpContext::new())
            .unwrap()
            .read_to_end(&mut back)
            .unwrap();
        assert_eq!(back, bytes, "{size}");
        let entry = provider
            .stat(&child(&folder, &name), &OpContext::new())
            .unwrap();
        assert_eq!(entry.size, Some(size as u64));
    }
}

#[test]
fn a_cancelled_context_starts_no_write() {
    let folder = scratch();
    let ctx = OpContext::new();
    ctx.cancel.cancel();
    assert!(matches!(
        LocalProvider::new().write(&child(&folder, "x.txt"), WriteMode::CreateNew, &ctx),
        Err(VfsError::Cancelled)
    ));
    assert!(everything_in(&folder).is_empty());
}

#[test]
fn a_fast_copy_the_volume_cannot_make_touches_nothing() {
    let folder = scratch();
    fs::write(folder.path().join("source.txt"), b"s").unwrap();
    let provider = LocalProvider::new();
    let from = child(&folder, "source.txt");
    let to = child(&folder, "copy.txt");
    let hint = provider.copy_hint(&from, &to);
    let capabilities = provider.capabilities(&from);
    if hint == CopyHint::Stream {
        assert!(matches!(
            provider.copy_within(&from, &to, &OpContext::new()),
            Err(VfsError::Unsupported { .. })
        ));
        assert_eq!(everything_in(&folder), vec!["source.txt".to_owned()]);
    } else {
        // A cloning volume (Btrfs, XFS, APFS, ReFS): the copy is whole,
        // with the source's modified time.
        assert!(capabilities.clone_copy);
        provider.copy_within(&from, &to, &OpContext::new()).unwrap();
        assert_eq!(fs::read(folder.path().join("copy.txt")).unwrap(), b"s");
        let a = provider.stat(&from, &OpContext::new()).unwrap();
        let b = provider.stat(&to, &OpContext::new()).unwrap();
        assert_eq!(a.modified, b.modified);
        assert_eq!(
            provider.copy_within(&from, &to, &OpContext::new()),
            Err(VfsError::AlreadyExists)
        );
    }
}
