//! Long paths (spike four): a tree 40 levels deep of plain 30-character
//! names, about 1,300 characters, made, written, listed, renamed, walked
//! and removed through the provider, with the extended-length prefix never
//! in anything a person could see or hear.

use std::io::{Read, Write};

use diroxide_core::Place;
use diroxide_vfs::walk::longest_path_units;
use diroxide_vfs::{LocalProvider, OpContext, Provider, WriteMode};

use crate::local::place_of;
use crate::scratch;

const PREFIX: &str = r"\\?\";

fn level_name(level: usize) -> String {
    format!("level{level:02}-{}", "x".repeat(22))
}

#[test]
fn a_tree_of_1300_characters_works_end_to_end() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let ctx = OpContext::new();
    let top = place_of(&folder);
    // macOS caps a path at 1,024 bytes, and the provider refuses past it in
    // words rather than letting the system fail; the end-to-end tree of
    // 1,300 characters is for Windows (with the extended prefix) and Linux.
    if cfg!(target_os = "macos") {
        let mut place = top.clone();
        let mut refused = None;
        for level in 0..40 {
            let next = place.join(&level_name(level)).unwrap();
            match provider.create_dir(&next, &ctx) {
                Ok(_) => place = next,
                Err(error) => {
                    refused = Some((level, error));
                    break;
                }
            }
        }
        let (level, error) = refused.expect("macOS refuses a path past 1,024 bytes");
        assert!(level >= 30, "refused already at level {level}");
        assert!(
            matches!(error, diroxide_vfs::VfsError::NameInvalid),
            "{error:?}"
        );
        return;
    }
    let mut levels = vec![top.clone()];
    for level in 0..40 {
        let next = levels[level].join(&level_name(level)).unwrap();
        provider.create_dir(&next, &ctx).unwrap();
        levels.push(next);
    }
    let deepest = levels[40].clone();
    assert!(
        deepest.to_string().len() > 1_250,
        "{}",
        deepest.to_string().len()
    );

    // Write and read a file at depth.
    let file = deepest.join("deep.txt").unwrap();
    let mut writer = provider.write(&file, WriteMode::CreateNew, &ctx).unwrap();
    writer.write_all(b"at depth").unwrap();
    writer.commit(true).unwrap();
    let mut text = String::new();
    provider
        .read(&file, &ctx)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "at depth");

    // List and stat: no prefix in any name, and the identity is read.
    let entries = provider.list(&deepest, &ctx).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].name.contains(PREFIX));
    let entry = provider.stat(&file, &ctx).unwrap();
    assert_eq!(entry.size, Some(8));
    assert!(entry.inode.is_some());

    // The volume, the capabilities and the free space at depth.
    let volume = provider.volume(&deepest, &ctx).unwrap();
    assert!(!volume.root.to_string().contains(PREFIX));
    assert!(provider.capabilities(&deepest).writable);
    assert!(provider.space(&file, &ctx).unwrap().free_to_user.is_some());

    // Rename the deepest folder and back.
    let renamed = levels[39].join("renamed-at-depth").unwrap();
    provider.rename(&deepest, &renamed, &ctx).unwrap();
    provider.rename(&renamed, &deepest, &ctx).unwrap();

    // The walk measures the longest path, as the trash gate needs.
    let longest = longest_path_units(&provider, &top, &ctx).unwrap();
    assert!(longest > 1_250, "{longest}");

    // Remove from the leaf up, one entry at a time.
    provider.remove(&file, &ctx).unwrap();
    for level in (1..=40).rev() {
        provider.remove(&levels[level], &ctx).unwrap();
    }
    assert!(provider.list(&top, &ctx).unwrap().is_empty());
}

#[test]
fn a_canonicalized_path_never_shows_its_prefix() {
    let folder = scratch();
    let real = std::fs::canonicalize(folder.path()).unwrap();
    let text = real.to_str().unwrap();
    let place = Place::parse(text).unwrap();
    assert!(!place.to_string().contains(PREFIX));
    assert!(!place.title().contains(PREFIX));
    assert_eq!(place, place_of(&folder));
}
