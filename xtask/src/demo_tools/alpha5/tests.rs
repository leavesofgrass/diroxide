//! The alpha 5 fixtures read back by hand: each says what R34's steps
//! will hear from it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use diroxide_core::filter::{Filter, FilterOptions};

use super::*;
use crate::demo_tools::tests::make_set;

/// A file's modified time.
fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// Every entry below `root`, files and folders, by its path relative to
/// it with `/`, and whether it is a folder.
fn entries(root: &Path) -> BTreeMap<String, bool> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in fs::read_dir(&dir).unwrap().flatten() {
            let path = item.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|part| part.as_os_str().to_str().unwrap().to_owned())
                .collect::<Vec<_>>()
                .join("/");
            let folder = path.is_dir();
            if folder {
                stack.push(path);
            }
            found.insert(relative, folder);
        }
    }
    found
}

#[test]
fn rename_holds_fifty_pictures_and_two_names_to_swap() {
    let (_dir, demo, _) = make_set();
    let folder = demo.join("rename");
    let all = entries(&folder);
    assert_eq!(all.len(), 52);
    let pictures = all.keys().filter(|name| name.starts_with("IMG_")).count();
    assert_eq!(pictures, 50);
    assert!(all.contains_key("IMG_0001.jpg") && all.contains_key("IMG_0050.jpg"));
    assert!(!all.contains_key("IMG_0051.jpg"));
    assert_ne!(
        fs::read(folder.join("a.txt")).unwrap(),
        fs::read(folder.join("b.txt")).unwrap()
    );
}

#[test]
fn sync_has_ten_paths_five_same_and_five_differences() {
    let (_dir, demo, _) = make_set();
    let here = demo.join("sync/here");
    let there = demo.join("sync/there");
    let (h, t) = (entries(&here), entries(&there));
    // The folder docs holds guide.md on both sides and is not counted.
    let mut paths: Vec<&String> = h.keys().chain(t.keys()).filter(|p| *p != "docs").collect();
    paths.sort();
    paths.dedup();
    let mut expected: Vec<&str> = SYNC_PATHS.to_vec();
    expected.sort_unstable();
    assert_eq!(paths, expected);
    for same in ["a.txt", "b.txt", "c.txt", "docs/guide.md", "d.txt"] {
        assert_eq!(
            fs::read(here.join(same)).unwrap(),
            fs::read(there.join(same)).unwrap(),
            "{same}"
        );
        let gap = modified(&there.join(same))
            .duration_since(modified(&here.join(same)))
            .unwrap();
        let want = if same == "d.txt" { 1 } else { 0 };
        assert_eq!(gap.as_secs(), want, "{same}");
        assert_eq!(gap.subsec_nanos(), 0, "{same}");
    }
    let size = |path: PathBuf| fs::metadata(path).unwrap().len();
    assert!(modified(&here.join("notes.txt")) > modified(&there.join("notes.txt")));
    assert!(size(here.join("notes.txt")) > size(there.join("notes.txt")));
    assert!(modified(&there.join("todo.txt")) > modified(&here.join("todo.txt")));
    assert!(h.contains_key("plan.md") && !t.contains_key("plan.md"));
    assert!(t.contains_key("old.txt") && !h.contains_key("old.txt"));
    assert_eq!(h.get("thing"), Some(&false));
    assert_eq!(t.get("thing"), Some(&true));
    assert!(fs::read_dir(there.join("thing")).unwrap().next().is_none());
}

#[test]
fn dups_has_three_groups_seven_files_and_44_kb_to_free() {
    let (_dir, demo, made) = make_set();
    let folder = demo.join("dups");
    let all = entries(&folder);
    // Group every non-empty file by its bytes, the hard-linked pair as
    // one file, as a duplicate finder must.
    let mut groups: BTreeMap<Vec<u8>, Vec<String>> = BTreeMap::new();
    let mut empty = 0;
    for (name, is_folder) in &all {
        if *is_folder || name == "linked/two.txt" {
            continue;
        }
        let bytes = fs::read(folder.join(name)).unwrap();
        if bytes.is_empty() {
            empty += 1;
            continue;
        }
        groups.entry(bytes).or_default().push(name.clone());
    }
    let groups: Vec<(usize, Vec<String>)> = groups
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(bytes, names)| (bytes.len(), names))
        .collect();
    assert_eq!(groups.len(), 3, "{groups:?}");
    let files: usize = groups.iter().map(|(_, names)| names.len()).sum();
    assert_eq!(files, 7);
    let free: usize = groups
        .iter()
        .map(|(size, names)| size * (names.len() - 1))
        .sum();
    assert_eq!(free, 44_000);
    assert_eq!(empty, 2);
    // The crows: 16,000 bytes, three copies, the oldest in photos/a.
    let crows = groups.iter().find(|(size, _)| *size == CROW_BYTES).unwrap();
    assert_eq!(crows.1.len(), 3);
    let oldest = crows
        .1
        .iter()
        .min_by_key(|name| modified(&folder.join(name)))
        .unwrap();
    assert_eq!(oldest, "photos/a/crow.png");
    assert!(
        fs::read(folder.join(oldest))
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    assert!(
        groups
            .iter()
            .any(|(size, n)| *size == SONG_BYTES && n.len() == 2)
    );
    assert!(
        groups
            .iter()
            .any(|(size, n)| *size == REPORT_BYTES && n.len() == 2)
    );
    // The near pair: alike at both ends, one byte different inside.
    let a = fs::read(folder.join("near-a.bin")).unwrap();
    let b = fs::read(folder.join("near-b.bin")).unwrap();
    assert_eq!(a.len(), b.len());
    assert_eq!(a[..NEAR_BLOCK], b[..NEAR_BLOCK]);
    assert_eq!(a[a.len() - NEAR_BLOCK..], b[b.len() - NEAR_BLOCK..]);
    let differ: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    assert_eq!(differ, [NEAR_DIFFERS_AT]);
    // The hard link, where one was made, is one file under two names.
    let linked = folder.join("linked/two.txt").is_file();
    assert!(made.lines.iter().any(|line| line.starts_with("dups, ")
        && line.contains(if linked {
            "a hard link"
        } else {
            "no hard link"
        })));
    if linked {
        assert_eq!(
            fs::read(folder.join("linked/one.txt")).unwrap(),
            fs::read(folder.join("linked/two.txt")).unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let one = fs::metadata(folder.join("linked/one.txt")).unwrap();
            let two = fs::metadata(folder.join("linked/two.txt")).unwrap();
            assert_eq!((one.dev(), one.ino()), (two.dev(), two.ino()));
            assert_eq!(one.nlink(), 2);
        }
    }
}

#[test]
fn flat_work_has_14_files_in_4_folders_three_deep() {
    let (_dir, demo, _) = make_set();
    let all = entries(&demo.join("flat/work"));
    let files: Vec<&String> = all.iter().filter(|(_, f)| !**f).map(|(n, _)| n).collect();
    let folders: Vec<&String> = all.iter().filter(|(_, f)| **f).map(|(n, _)| n).collect();
    assert_eq!(files.len(), 14);
    assert_eq!(folders, ["a", "b", "b/deep", "b/deep/old"]);
    assert!(
        files.iter().all(|name| name.contains('/')),
        "none at the top"
    );
    let readmes: Vec<&&String> = files
        .iter()
        .filter(|n| n.ends_with("/readme.txt"))
        .collect();
    assert_eq!(readmes, [&"a/readme.txt", &"b/deep/readme.txt"]);
    let deepest = files.iter().map(|n| n.matches('/').count()).max();
    assert_eq!(deepest, Some(3));
}

#[test]
fn the_saved_files_read_as_a_collection_and_a_search() {
    let (_dir, demo, _) = make_set();
    let state = demo.join("state");
    let names: Vec<String> = entries(&state.join(SAVED_FOLDER)).into_keys().collect();
    assert_eq!(names, ["large-images.toml", "trip-photos.toml"]);
    let trip = Saved::load(&saved_file(&state, COLLECTION)).unwrap();
    assert_eq!(trip.kind, SavedKind::Collection);
    assert_eq!(trip.name, COLLECTION);
    assert_eq!(trip.items.len(), 3);
    let there: Vec<bool> = trip
        .items
        .iter()
        .map(|item| Path::new(item.place.local_path().unwrap()).is_file())
        .collect();
    assert_eq!(there, [true, true, false]);
    let gone = trip.items[2].place.local_path().unwrap();
    assert!(gone.ends_with("gone.txt"), "{gone}");
    assert!(!demo.join("old").exists());
    let large = Saved::load(&saved_file(&state, SAVED_SEARCH)).unwrap();
    assert_eq!(large.kind, SavedKind::Search);
    assert_eq!(large.expression.as_deref(), Some(SAVED_EXPRESSION));
    Filter::parse(SAVED_EXPRESSION).unwrap();
    let query = large.to_query(&FilterOptions::default()).unwrap();
    assert_eq!(query.scope.places.len(), 1);
    assert_eq!(
        query.scope.places[0].local_path().map(PathBuf::from),
        Some(demo.clone())
    );
}

#[test]
fn paths_nul_names_three_files_each_ended_by_a_nul() {
    let (_dir, demo, _) = make_set();
    let bytes = fs::read(demo.join("paths.nul")).unwrap();
    assert_eq!(bytes.last(), Some(&0));
    let paths: Vec<&[u8]> = bytes[..bytes.len() - 1].split(|b| *b == 0).collect();
    assert_eq!(paths.len(), 3);
    for path in paths {
        let path = PathBuf::from(std::str::from_utf8(path).unwrap());
        assert!(path.is_absolute() && path.is_file(), "{}", path.display());
        assert!(path.starts_with(&demo));
    }
    assert!(demo.join("open/report.md").is_file());
    assert!(demo.join("open/notes.txt").is_file());
}
