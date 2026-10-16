//! The safe primitives on temporary folders: a rename that never
//! replaces, names refused before they reach the disk, a remove that takes
//! away a link and never its target, attributes and times with their
//! inverse, and the three kinds of link (tags `temp`, `win`, `linux` in
//! research/file-operations.md; research/permissions.md section 12).

use std::fs;

use diroxide_core::{Attributes, EntryKind, Place, Timestamp};
use diroxide_vfs::{
    AttributeChange, LinkKind, LinkRequest, LocalProvider, OpContext, Provider, VfsError,
};

use crate::local::place_of;
use crate::scratch;

fn ctx() -> OpContext {
    OpContext::new()
}

fn child(folder: &tempfile::TempDir, name: &str) -> Place {
    place_of(folder).join(name).unwrap()
}

#[test]
fn a_rename_never_replaces() {
    let folder = scratch();
    fs::write(folder.path().join("a.txt"), b"A").unwrap();
    fs::write(folder.path().join("b.txt"), b"B").unwrap();
    let provider = LocalProvider::new();
    let result = provider.rename(&child(&folder, "a.txt"), &child(&folder, "b.txt"), &ctx());
    assert_eq!(result, Err(VfsError::AlreadyExists));
    assert_eq!(fs::read(folder.path().join("a.txt")).unwrap(), b"A");
    assert_eq!(fs::read(folder.path().join("b.txt")).unwrap(), b"B");
    provider
        .rename(&child(&folder, "a.txt"), &child(&folder, "c.txt"), &ctx())
        .unwrap();
    assert_eq!(fs::read(folder.path().join("c.txt")).unwrap(), b"A");
    // A rename onto itself does nothing.
    let same = child(&folder, "c.txt");
    provider.rename(&same, &same, &ctx()).unwrap();
}

#[test]
fn a_rename_racing_a_new_file_loses_nothing() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let source = folder.path().join("source.txt");
    let target = folder.path().join("target.txt");
    for _ in 0..100 {
        fs::write(&source, b"S").unwrap();
        let racer = {
            let target = target.clone();
            std::thread::spawn(move || {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .and_then(|mut file| std::io::Write::write_all(&mut file, b"T"))
                    .is_ok()
            })
        };
        let renamed = provider.rename(
            &child(&folder, "source.txt"),
            &child(&folder, "target.txt"),
            &ctx(),
        );
        let raced = racer.join().unwrap();
        match renamed {
            Ok(()) => {
                assert!(!raced, "the racer's file was replaced");
                assert_eq!(fs::read(&target).unwrap(), b"S");
            }
            Err(error) => {
                assert_eq!(error, VfsError::AlreadyExists);
                assert_eq!(fs::read(&source).unwrap(), b"S");
                assert_eq!(fs::read(&target).unwrap(), b"T");
                fs::remove_file(&source).unwrap();
            }
        }
        fs::remove_file(&target).unwrap();
    }
}

#[test]
fn a_change_of_case_renames_the_same_entry() {
    let folder = scratch();
    fs::write(folder.path().join("report.txt"), b"r").unwrap();
    let provider = LocalProvider::new();
    provider
        .rename(
            &child(&folder, "report.txt"),
            &child(&folder, "Report.txt"),
            &ctx(),
        )
        .unwrap();
    let names: Vec<String> = provider
        .list(&place_of(&folder), &ctx())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["Report.txt".to_owned()]);
}

#[test]
fn two_names_of_one_file_are_not_a_new_spelling() {
    let folder = scratch();
    fs::write(folder.path().join("one.txt"), b"1").unwrap();
    fs::hard_link(folder.path().join("one.txt"), folder.path().join("two.txt")).unwrap();
    let provider = LocalProvider::new();
    assert_eq!(
        provider.rename(
            &child(&folder, "one.txt"),
            &child(&folder, "two.txt"),
            &ctx()
        ),
        Err(VfsError::AlreadyExists)
    );
    assert!(folder.path().join("one.txt").exists());
}

#[test]
fn read_only_entries_rename() {
    let folder = scratch();
    let file = folder.path().join("locked.txt");
    fs::write(&file, b"x").unwrap();
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&file, permissions).unwrap();
    fs::create_dir(folder.path().join("box")).unwrap();
    let provider = LocalProvider::new();
    provider
        .rename(
            &child(&folder, "locked.txt"),
            &child(&folder, "box").join("locked.txt").unwrap(),
            &ctx(),
        )
        .unwrap();
    provider
        .rename(&child(&folder, "box"), &child(&folder, "crate"), &ctx())
        .unwrap();
    assert!(folder.path().join("crate").join("locked.txt").exists());
}

#[test]
fn unicode_names_make_rename_and_remove() {
    let folder = scratch();
    let provider = LocalProvider::new();
    for name in crate::local::UNICODE_NAMES {
        let place = child(&folder, name);
        provider.create_dir(&place, &ctx()).unwrap();
        let renamed = child(&folder, &format!("{name} 2"));
        provider.rename(&place, &renamed, &ctx()).unwrap();
        assert_eq!(
            provider.stat(&renamed, &ctx()).unwrap().kind,
            EntryKind::Folder
        );
        provider.remove(&renamed, &ctx()).unwrap();
    }
    assert!(
        provider
            .list(&place_of(&folder), &ctx())
            .unwrap()
            .is_empty()
    );
}

#[cfg(windows)]
#[test]
fn names_windows_would_change_never_reach_the_disk() {
    let folder = scratch();
    let provider = LocalProvider::new();
    for name in ["notes.", "notes ", "con", "aux.txt", "a:b", "what?"] {
        assert_eq!(
            provider.create_dir(&child(&folder, name), &ctx()),
            Err(VfsError::NameInvalid),
            "{name}"
        );
    }
    fs::write(folder.path().join("plain.txt"), b"x").unwrap();
    assert_eq!(
        provider.rename(
            &child(&folder, "plain.txt"),
            &child(&folder, "plain."),
            &ctx()
        ),
        Err(VfsError::NameInvalid)
    );
    let long = "x".repeat(256);
    assert_eq!(
        provider.create_dir(&child(&folder, &long), &ctx()),
        Err(VfsError::NameInvalid)
    );
    let names: Vec<String> = provider
        .list(&place_of(&folder), &ctx())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["plain.txt".to_owned()]);
}

#[test]
fn making_a_folder_says_why_it_cannot() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let place = child(&folder, "made");
    provider.create_dir(&place, &ctx()).unwrap();
    assert_eq!(
        provider.create_dir(&place, &ctx()),
        Err(VfsError::AlreadyExists)
    );
    let orphan = child(&folder, "missing").join("child").unwrap();
    assert_eq!(
        provider.create_dir(&orphan, &ctx()),
        Err(VfsError::NotFound)
    );
}

#[test]
fn a_remove_takes_one_entry_and_never_a_full_folder() {
    let folder = scratch();
    let provider = LocalProvider::new();
    fs::create_dir(folder.path().join("full")).unwrap();
    fs::write(folder.path().join("full").join("inside.txt"), b"x").unwrap();
    assert_eq!(
        provider.remove(&child(&folder, "full"), &ctx()),
        Err(VfsError::NotEmpty)
    );
    provider
        .remove(&child(&folder, "full").join("inside.txt").unwrap(), &ctx())
        .unwrap();
    provider.remove(&child(&folder, "full"), &ctx()).unwrap();
    assert!(
        provider
            .list(&place_of(&folder), &ctx())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_read_only_file_removes() {
    // Pins whether the platform removes a read-only file (Windows needs
    // the attribute cleared; the provider does that as part of the
    // remove).
    let folder = scratch();
    let file = folder.path().join("locked.txt");
    fs::write(&file, b"x").unwrap();
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&file, permissions).unwrap();
    LocalProvider::new()
        .remove(&child(&folder, "locked.txt"), &ctx())
        .unwrap();
    assert!(!file.exists());
}

#[test]
fn a_hard_link_is_a_second_name_for_one_file() {
    let folder = scratch();
    fs::write(folder.path().join("data.bin"), b"123").unwrap();
    let provider = LocalProvider::new();
    let link = child(&folder, "alias.bin");
    let request = LinkRequest {
        kind: LinkKind::Hard,
        target: "data.bin".to_owned(),
        target_is_folder: false,
    };
    provider.create_link(&link, &request, &ctx()).unwrap();
    let first = provider.stat(&child(&folder, "data.bin"), &ctx()).unwrap();
    let second = provider.stat(&link, &ctx()).unwrap();
    assert_eq!((first.inode, first.volume), (second.inode, second.volume));
    assert_eq!(second.hard_links, Some(2));
    assert_eq!(
        provider.create_link(&link, &request, &ctx()),
        Err(VfsError::AlreadyExists)
    );
}

#[test]
fn a_symbolic_link_is_made_or_its_privilege_is_named() {
    let folder = scratch();
    fs::write(folder.path().join("target.txt"), b"t").unwrap();
    let provider = LocalProvider::new();
    let link = child(&folder, "to-target");
    let request = LinkRequest {
        kind: LinkKind::Symbolic,
        target: "target.txt".to_owned(),
        target_is_folder: false,
    };
    match provider.create_link(&link, &request, &ctx()) {
        Ok(()) => {
            let entry = provider.stat(&link, &ctx()).unwrap();
            assert_eq!(entry.kind, EntryKind::Link);
            let target = entry.link.unwrap();
            assert_eq!(target.target, "target.txt");
            assert_eq!(target.points_to, Some(EntryKind::File));
            // Removing the link leaves the target.
            provider.remove(&link, &ctx()).unwrap();
            assert!(folder.path().join("target.txt").exists());
        }
        Err(VfsError::Io { message, .. }) if cfg!(windows) => {
            assert_eq!(
                message,
                "Windows needs Developer Mode to make symbolic links"
            );
        }
        Err(other) => panic!("{other}"),
    }
}

#[cfg(windows)]
#[test]
fn a_junction_is_a_link_and_removing_it_keeps_the_target() {
    let folder = scratch();
    fs::create_dir(folder.path().join("real")).unwrap();
    fs::write(folder.path().join("real").join("kept.txt"), b"k").unwrap();
    let provider = LocalProvider::new();
    let link = child(&folder, "jump");
    let request = LinkRequest {
        kind: LinkKind::Junction,
        target: "real".to_owned(),
        target_is_folder: true,
    };
    provider.create_link(&link, &request, &ctx()).unwrap();
    let entry = provider.stat(&link, &ctx()).unwrap();
    // Pins that the standard library reads a junction as a link.
    assert_eq!(entry.kind, EntryKind::Link);
    let target = entry.link.unwrap();
    assert!(target.target.ends_with("real"), "{}", target.target);
    assert!(!target.target.starts_with(r"\\?\"));
    assert_eq!(target.points_to, Some(EntryKind::Folder));
    let through: Vec<String> = provider
        .list(&link, &ctx())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(through, vec!["kept.txt".to_owned()]);
    provider.remove(&link, &ctx()).unwrap();
    assert!(folder.path().join("real").join("kept.txt").exists());
    assert!(!folder.path().join("jump").exists());
}

#[test]
fn times_change_and_the_inverse_puts_them_back() {
    let folder = scratch();
    fs::write(folder.path().join("dated.txt"), b"d").unwrap();
    fs::create_dir(folder.path().join("dated folder")).unwrap();
    let provider = LocalProvider::new();
    for name in ["dated.txt", "dated folder"] {
        let place = child(&folder, name);
        let before = provider.stat(&place, &ctx()).unwrap();
        let when = Timestamp::from_unix(1_600_000_000, 0);
        let change = AttributeChange {
            modified: Some(when),
            ..AttributeChange::default()
        };
        let inverse = provider.set_attributes(&place, &change, &ctx()).unwrap();
        assert_eq!(provider.stat(&place, &ctx()).unwrap().modified, Some(when));
        assert_eq!(inverse.modified, before.modified, "{name}");
        assert!(inverse.attributes.is_none() && inverse.accessed.is_none());
        provider.set_attributes(&place, &inverse, &ctx()).unwrap();
        assert_eq!(
            provider.stat(&place, &ctx()).unwrap().modified,
            before.modified
        );
    }
}

#[test]
fn the_created_time_is_set_where_the_platform_can() {
    let folder = scratch();
    fs::write(folder.path().join("born.txt"), b"b").unwrap();
    let place = child(&folder, "born.txt");
    let provider = LocalProvider::new();
    let when = Timestamp::from_unix(1_500_000_000, 0);
    let change = AttributeChange {
        created: Some(when),
        ..AttributeChange::default()
    };
    let result = provider.set_attributes(&place, &change, &ctx());
    if cfg!(any(windows, target_os = "macos")) {
        result.unwrap();
        assert_eq!(provider.stat(&place, &ctx()).unwrap().created, Some(when));
        assert!(
            provider
                .capabilities(&place)
                .access
                .created_time
                .is_possible()
        );
    } else {
        assert!(matches!(result, Err(VfsError::Unsupported { .. })));
        assert!(
            !provider
                .capabilities(&place)
                .access
                .created_time
                .is_possible()
        );
    }
}

#[test]
fn an_expected_value_that_moved_changes_nothing() {
    let folder = scratch();
    fs::write(folder.path().join("watched.txt"), b"w").unwrap();
    let place = child(&folder, "watched.txt");
    let provider = LocalProvider::new();
    let before = provider.stat(&place, &ctx()).unwrap();
    let stale = match before.attributes.unwrap() {
        Attributes::Unix(mode) => Attributes::Unix(mode ^ 0o4000),
        Attributes::Windows(flags) => Attributes::Windows(flags ^ 0x2),
    };
    let change = AttributeChange {
        modified: Some(Timestamp::from_unix(1_000_000_000, 0)),
        expect: Some(stale),
        ..AttributeChange::default()
    };
    assert!(matches!(
        provider.set_attributes(&place, &change, &ctx()),
        Err(VfsError::Unsafe { .. })
    ));
    assert_eq!(
        provider.stat(&place, &ctx()).unwrap().modified,
        before.modified
    );
}

#[cfg(windows)]
#[test]
fn windows_attributes_change_and_keep_what_they_cannot_set() {
    let folder = scratch();
    fs::write(folder.path().join(".gitignore"), b"target").unwrap();
    let place = child(&folder, ".gitignore");
    let provider = LocalProvider::new();
    let Some(Attributes::Windows(before)) = provider.stat(&place, &ctx()).unwrap().attributes
    else {
        panic!("Windows attributes");
    };
    let change = AttributeChange {
        attributes: Some(Attributes::Windows(before | 0x2 | 0x1)),
        expect: Some(Attributes::Windows(before)),
        ..AttributeChange::default()
    };
    let inverse = provider.set_attributes(&place, &change, &ctx()).unwrap();
    let after = provider.stat(&place, &ctx()).unwrap();
    assert!(after.hidden);
    assert_eq!(inverse.attributes, Some(Attributes::Windows(before)));
    provider.set_attributes(&place, &inverse, &ctx()).unwrap();
    let back = provider.stat(&place, &ctx()).unwrap();
    assert!(!back.hidden);
    assert_eq!(back.attributes, Some(Attributes::Windows(before)));
}

#[cfg(windows)]
#[test]
fn read_only_on_a_windows_folder_is_kept_and_said_as_a_mark() {
    let folder = scratch();
    fs::create_dir(folder.path().join("marked")).unwrap();
    let place = child(&folder, "marked");
    let provider = LocalProvider::new();
    let Some(Attributes::Windows(before)) = provider.stat(&place, &ctx()).unwrap().attributes
    else {
        panic!("Windows attributes");
    };
    let change = AttributeChange {
        attributes: Some(Attributes::Windows(before | 0x1)),
        ..AttributeChange::default()
    };
    let inverse = provider.set_attributes(&place, &change, &ctx()).unwrap();
    let after = provider.stat(&place, &ctx()).unwrap();
    let words = after.attributes.unwrap().words(after.kind);
    assert!(words.contains("a mark only"), "{words}");
    // Windows ignores it: a file can still be made inside.
    fs::write(folder.path().join("marked").join("inside.txt"), b"i").unwrap();
    provider.set_attributes(&place, &inverse, &ctx()).unwrap();
}

#[cfg(windows)]
#[test]
fn a_file_held_open_without_delete_sharing_is_in_use_until_it_closes() {
    use std::os::windows::fs::OpenOptionsExt;
    let folder = scratch();
    let path = folder.path().join("held.txt");
    fs::write(&path, b"h").unwrap();
    // Read and write sharing, but no delete sharing.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&path)
        .unwrap();
    let provider = LocalProvider::new();
    let place = child(&folder, "held.txt");
    let error = provider.remove(&place, &ctx()).unwrap_err();
    // Pins the class of a sharing violation: retry first.
    assert_eq!(error, VfsError::InUse);
    assert!(error.class().retry_first());
    drop(held);
    provider.remove(&place, &ctx()).unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn all_twelve_mode_bits_are_written_and_links_are_refused() {
    use std::os::unix::fs::symlink;
    let folder = scratch();
    fs::write(folder.path().join("tool"), b"#!/bin/sh\n").unwrap();
    symlink("tool", folder.path().join("to-tool")).unwrap();
    let provider = LocalProvider::new();
    let place = child(&folder, "tool");
    let before = provider.stat(&place, &ctx()).unwrap().attributes;
    for mode in [0o4755, 0o2775, 0o600] {
        let change = AttributeChange {
            attributes: Some(Attributes::Unix(mode)),
            ..AttributeChange::default()
        };
        let inverse = provider.set_attributes(&place, &change, &ctx()).unwrap();
        assert!(inverse.attributes.is_some());
        assert_eq!(
            provider.stat(&place, &ctx()).unwrap().attributes,
            Some(Attributes::Unix(mode))
        );
    }
    // The sticky bit, on a folder, where it means something.
    fs::create_dir(folder.path().join("shared")).unwrap();
    let shared = child(&folder, "shared");
    let sticky = AttributeChange {
        attributes: Some(Attributes::Unix(0o1777)),
        ..AttributeChange::default()
    };
    provider.set_attributes(&shared, &sticky, &ctx()).unwrap();
    assert_eq!(
        provider.stat(&shared, &ctx()).unwrap().attributes,
        Some(Attributes::Unix(0o1777))
    );
    let change = AttributeChange {
        attributes: before,
        ..AttributeChange::default()
    };
    provider.set_attributes(&place, &change, &ctx()).unwrap();
    let on_link = AttributeChange {
        attributes: Some(Attributes::Unix(0o777)),
        ..AttributeChange::default()
    };
    assert!(matches!(
        provider.set_attributes(&child(&folder, "to-tool"), &on_link, &ctx()),
        Err(VfsError::Unsafe { .. })
    ));
}
