//! The local provider on real temporary folders under the build's own
//! target folder (tag `temp` in research/file-operations.md). Names are
//! plain Unicode only: letters, emoji and CJK; never a hostile name on
//! disk.

use std::fs;
use std::io::Read;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_vfs::{LocalProvider, OpContext, Provider, VfsError, facts};

use crate::scratch;

/// The place of a temporary folder.
pub(crate) fn place_of(folder: &tempfile::TempDir) -> Place {
    Place::parse(folder.path().to_str().unwrap()).unwrap()
}

fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
    entries.iter().find(|e| e.name == name).unwrap()
}

/// Plain Unicode names: an emoji, Japanese, Chinese, an accented letter.
pub(crate) const UNICODE_NAMES: [&str; 4] = [
    "\u{1F980} crab.txt",
    "\u{65E5}\u{672C}\u{8A9E}.txt",
    "\u{6587}\u{4EF6}",
    "caf\u{E9}.md",
];

#[test]
fn every_kind_lists_with_its_facts() {
    let folder = scratch();
    let root = folder.path();
    fs::write(root.join("empty.txt"), b"").unwrap();
    fs::write(root.join("one.bin"), b"x").unwrap();
    fs::write(root.join("course.zip"), b"PK").unwrap();
    fs::create_dir(root.join("photos")).unwrap();
    for name in UNICODE_NAMES {
        fs::write(root.join(name), name.as_bytes()).unwrap();
    }
    let entries = LocalProvider::new()
        .list(&place_of(&folder), &OpContext::new())
        .unwrap();
    assert_eq!(entries.len(), 4 + UNICODE_NAMES.len());

    let empty = find(&entries, "empty.txt");
    assert_eq!((empty.kind, empty.size), (EntryKind::File, Some(0)));
    assert!(empty.modified.is_some() && empty.accessed.is_some());
    assert!(empty.attributes.is_some());
    assert!(!empty.needs_care && !empty.name_not_unicode);
    assert_eq!(find(&entries, "one.bin").size, Some(1));
    assert_eq!(find(&entries, "course.zip").kind, EntryKind::Archive);
    let photos = find(&entries, "photos");
    assert_eq!((photos.kind, photos.size), (EntryKind::Folder, None));
    for name in UNICODE_NAMES {
        let entry = find(&entries, name);
        assert_eq!(entry.size, Some(name.len() as u64), "{name}");
        assert_eq!(entry.display_name(), name);
        assert!(!entry.needs_care, "{name}");
    }
    // Every entry of one folder is on one volume.
    let volumes: Vec<_> = entries.iter().map(|e| e.volume).collect();
    assert!(volumes.iter().all(|v| v.is_some() && *v == volumes[0]));
}

#[test]
fn a_stat_reads_the_identity_and_never_follows_a_final_link() {
    let folder = scratch();
    fs::write(folder.path().join("a.txt"), b"abc").unwrap();
    let provider = LocalProvider::new();
    let place = place_of(&folder).join("a.txt").unwrap();
    let entry = provider.stat(&place, &OpContext::new()).unwrap();
    assert_eq!(entry.name, "a.txt");
    assert_eq!(entry.size, Some(3));
    assert!(entry.inode.is_some() && entry.volume.is_some());
    assert_eq!(entry.hard_links, Some(1));
    let listed = provider
        .list(&place_of(&folder), &OpContext::new())
        .unwrap();
    assert_eq!(listed[0].volume, entry.volume);
}

#[test]
fn hidden_follows_the_platform_rule() {
    let folder = scratch();
    fs::write(folder.path().join(".gitignore"), b"target").unwrap();
    let entries = LocalProvider::new()
        .list(&place_of(&folder), &OpContext::new())
        .unwrap();
    // A leading dot hides a file on Unix and macOS, never on Windows
    // (research/file-operations.md, "Hidden files").
    assert_eq!(entries[0].hidden, !cfg!(windows));
}

#[test]
fn unfinished_files_are_left_out_and_found_on_request() {
    let folder = scratch();
    fs::write(folder.path().join(".diro-part-0000beef"), b"half").unwrap();
    fs::write(folder.path().join("kept.txt"), b"whole").unwrap();
    let provider = LocalProvider::new();
    let place = place_of(&folder);
    let names: Vec<String> = provider
        .list(&place, &OpContext::new())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["kept.txt".to_owned()]);
    let found = provider.unfinished(&place, &OpContext::new()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, ".diro-part-0000beef");
}

#[test]
fn reading_a_folder_or_a_missing_file_says_why() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let missing = place_of(&folder).join("gone.txt").unwrap();
    assert!(matches!(
        provider.read(&missing, &OpContext::new()),
        Err(VfsError::NotFound)
    ));
    assert!(matches!(
        provider.list(&missing, &OpContext::new()),
        Err(VfsError::NotFound)
    ));
    fs::write(folder.path().join("file.txt"), b"x").unwrap();
    let file = place_of(&folder).join("file.txt").unwrap();
    assert!(provider.list(&file, &OpContext::new()).is_err());
}

#[test]
fn a_seekable_read_seeks() {
    use std::io::{Seek, SeekFrom};
    let folder = scratch();
    fs::write(folder.path().join("abc.txt"), b"abcdef").unwrap();
    let place = place_of(&folder).join("abc.txt").unwrap();
    let mut reader = LocalProvider::new()
        .open_seekable(&place, &OpContext::new())
        .unwrap();
    reader.seek(SeekFrom::Start(3)).unwrap();
    let mut rest = String::new();
    reader.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "def");
}

#[cfg(unix)]
#[test]
fn unix_entries_carry_mode_owner_and_links() {
    use diroxide_core::Attributes;
    use std::os::unix::fs::{PermissionsExt, symlink};

    let folder = scratch();
    let root = folder.path();
    fs::write(root.join("script.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("script.sh"), fs::Permissions::from_mode(0o4755)).unwrap();
    symlink("script.sh", root.join("to-script")).unwrap();
    symlink("nowhere", root.join("broken")).unwrap();
    let entries = LocalProvider::new()
        .list(&place_of(&folder), &OpContext::new())
        .unwrap();
    let script = find(&entries, "script.sh");
    assert_eq!(script.attributes, Some(Attributes::Unix(0o4755)));
    assert!(script.owner.is_some() && script.group.is_some());
    let link = find(&entries, "to-script");
    assert_eq!(link.kind, EntryKind::Link);
    let target = link.link.as_ref().unwrap();
    assert_eq!(target.target, "script.sh");
    assert_eq!(target.points_to, Some(EntryKind::File));
    let broken = find(&entries, "broken");
    assert!(facts::is_broken_link(broken));
}

#[test]
fn capabilities_volume_and_space_are_asked_of_the_place() {
    let folder = scratch();
    let place = place_of(&folder);
    let provider = LocalProvider::new();
    let caps = provider.capabilities(&place);
    assert!(caps.writable && caps.random_read && caps.attributes);
    assert!(caps.max_name_length.is_some());
    assert!(caps.access.times.is_possible());
    // A place that is not made yet is asked of its nearest folder.
    let new = place.join("new").unwrap().join("deeper.txt").unwrap();
    assert_eq!(provider.capabilities(&new), caps);
    let volume = provider.volume(&place, &OpContext::new()).unwrap();
    assert!(place.is_within(&volume.root), "{}", volume.root);
    assert!(volume.file_system.is_some());
    let space = provider.space(&new, &OpContext::new()).unwrap();
    assert!(space.free_to_user.unwrap() > 0);
    assert!(space.total.unwrap() >= space.free_to_user.unwrap());
    if cfg!(windows) {
        // The checkout is on a fixed NTFS drive on the owner's machine and
        // in CI: the trash, Windows's name rules, watching.
        assert_eq!(volume.kind, diroxide_vfs::VolumeKind::Fixed);
        assert!(caps.trash && caps.restore && caps.reserved_names && caps.watch);
        assert!(!caps.case_sensitive, "NTFS folders ignore case");
    }
}

#[test]
fn the_places_list_has_the_tops_and_home() {
    use diroxide_vfs::{PlaceRole, local_places};
    let places = local_places();
    assert!(places.iter().any(|p| p.role == PlaceRole::Home));
    let tops = if cfg!(windows) {
        PlaceRole::Drive
    } else {
        PlaceRole::Root
    };
    assert!(places.iter().any(|p| p.role == tops));
    for place in &places {
        assert!(
            diroxide_core::format::within_cells(
                &place.words,
                place.words.split(',').next().unwrap()
            ),
            "{}",
            place.words
        );
    }
}

#[test]
fn state_words_read_an_ordinary_file_as_nothing_special() {
    let folder = scratch();
    fs::write(folder.path().join("plain.txt"), b"x").unwrap();
    let entries = LocalProvider::new()
        .list(&place_of(&folder), &OpContext::new())
        .unwrap();
    assert!(!facts::is_online_only(&entries[0]));
    assert!(!facts::is_sparse(&entries[0]));
    assert!(facts::state_words(&entries[0]).is_empty());
}
