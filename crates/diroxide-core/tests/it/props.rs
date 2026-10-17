//! Property tests: the sort is a total, stable order; names, patterns and
//! places never panic on any text.

use std::cmp::Ordering;

use diroxide_core::names::{display_name, split_extension, split_part};
use diroxide_core::place::PathStyle;
use diroxide_core::sort::{compare, compare_names, sort_entries};
use diroxide_core::{
    Entry, EntryKind, FolderOrder, NameCase, Pattern, Place, SortDirection, SortKey, SortSpec,
};
use proptest::prelude::*;

fn entry() -> impl Strategy<Value = Entry> {
    (
        "[a-cA-C0-9. ]{0,6}",
        prop::option::of(0u64..5),
        prop::bool::ANY,
        prop::option::of("[a-b]{0,2}"),
    )
        .prop_map(|(name, size, folder, owner)| {
            let kind = if folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            };
            let mut entry = Entry::new(name, kind);
            entry.size = size;
            entry.owner = owner;
            entry
        })
}

fn spec() -> impl Strategy<Value = SortSpec> {
    (
        prop::sample::select(vec![
            SortKey::Name,
            SortKey::Extension,
            SortKey::Size,
            SortKey::Owner,
            SortKey::Kind,
        ]),
        prop::bool::ANY,
        prop::sample::select(vec![
            FolderOrder::First,
            FolderOrder::Mixed,
            FolderOrder::Last,
        ]),
        prop::bool::ANY,
        prop::bool::ANY,
    )
        .prop_map(|(key, descending, folders, respect, natural)| SortSpec {
            key,
            direction: if descending {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            },
            folders,
            case: if respect {
                NameCase::Respect
            } else {
                NameCase::Ignore
            },
            natural,
        })
}

proptest! {
    #[test]
    fn names_compare_as_a_total_order(a in "\\PC{0,8}", b in "\\PC{0,8}", c in "\\PC{0,8}") {
        for case in [NameCase::Ignore, NameCase::Respect] {
            for natural in [true, false] {
                let cmp = |x: &str, y: &str| compare_names(x, y, case, natural);
                prop_assert_eq!(cmp(&a, &b), cmp(&b, &a).reverse());
                prop_assert_eq!(cmp(&a, &b) == Ordering::Equal, a == b);
                if cmp(&a, &b) != Ordering::Greater && cmp(&b, &c) != Ordering::Greater {
                    prop_assert_ne!(cmp(&a, &c), Ordering::Greater);
                }
            }
        }
    }

    #[test]
    fn entries_sort_in_a_total_order(a in entry(), b in entry(), c in entry(), spec in spec()) {
        prop_assert_eq!(compare(&a, &b, &spec), compare(&b, &a, &spec).reverse());
        if compare(&a, &b, &spec) != Ordering::Greater && compare(&b, &c, &spec) != Ordering::Greater {
            prop_assert_ne!(compare(&a, &c, &spec), Ordering::Greater);
        }
        if a.name != b.name {
            prop_assert_ne!(compare(&a, &b, &spec), Ordering::Equal);
        }
    }

    #[test]
    fn sorting_twice_changes_nothing(mut entries in prop::collection::vec(entry(), 0..20), spec in spec()) {
        sort_entries(&mut entries, &spec);
        let once: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        sort_entries(&mut entries, &spec);
        let twice: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        prop_assert_eq!(once, twice);
    }

    #[test]
    fn names_never_panic_and_stay_whole(name in "\\PC{0,24}") {
        let (stem, extension) = split_extension(&name);
        prop_assert_eq!(format!("{stem}{extension}"), name.clone());
        prop_assert_eq!(display_name(&name).chars().count(), name.chars().count());
        let _ = split_part(&name);
    }

    #[test]
    fn patterns_never_panic(text in "\\PC{0,40}", name in "\\PC{0,16}", folder in prop::bool::ANY) {
        if let Ok(pattern) = Pattern::parse(&text) {
            let _ = pattern.matches(&name, folder);
        }
    }

    /// A numbered part of an archive (`backup.7z.001`) followed by `!` is
    /// an archive place that prints back equal, in both path styles, with
    /// any number of three digits or more (R20, commit 5).
    #[test]
    fn split_archive_places_round_trip(
        stem in "[a-z][a-z0-9_-]{0,7}",
        extension in prop::sample::select(vec![
            ".zip", ".7z", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".xz",
            ".ZIP", ".7Z", ".Tar.Gz", ".docx", ".gz", ".GZ",
        ]),
        number in 0u32..100_000,
        width in 3usize..6,
        folder in "[a-z]{1,6}",
        member in prop::collection::vec("[a-z0-9.]{0,2}[a-z0-9]", 0..3),
    ) {
        let name = format!("{stem}{extension}.{number:0width$}");
        let member = member.join("/");
        for (text, style) in [
            (format!("/{folder}/{name}!{member}"), PathStyle::Unix),
            (format!("D:\\{folder}\\{name}!{member}"), PathStyle::Windows),
        ] {
            let place = Place::parse_with(&text, style).unwrap();
            let (container, inside) = place.archive().expect("an archive place");
            prop_assert_eq!(container.name(), Some(name.as_str()));
            prop_assert_eq!(inside, member.as_str());
            prop_assert_eq!(Place::parse_with(&place.to_string(), style), Ok(place.clone()));
            prop_assert_eq!(
                place.archive().map(|(c, _)| c.parent()),
                Some(Place::parse_with(&text[..text.find(&name).unwrap() - 1], style).ok())
            );
        }
    }

    /// Look-alikes are plain local places: a numbered part of a file that
    /// is not an archive (`photo.jpg.001`), a part number of fewer than
    /// three digits (`a.7z.01`), a number with a letter in it, and a
    /// name that only resembles an archive's.
    #[test]
    fn look_alike_parts_are_not_archive_places(
        stem in "[a-z]{1,6}",
        tail in prop::sample::select(vec![
            ".jpg.001", ".pdf.0042", ".txt.1000", ".jpg.000", ".7z.01", ".zip.1",
            ".zip.0a1", ".zipx.001", ".7zz.001", ".tar.gz.bak.001", ".001", ".zip.",
        ]),
        member in "[a-z]{1,4}",
    ) {
        let text = format!("/srv/{stem}{tail}!{member}");
        let place = Place::parse_with(&text, PathStyle::Unix).unwrap();
        prop_assert_eq!(place.local_path(), Some(text.as_str()));
        prop_assert!(place.archive().is_none());
        let name = format!("{stem}{tail}");
        prop_assert!(!diroxide_core::place::opens_as_archive(&name), "{}", name);
    }

    #[test]
    fn places_never_panic(text in "\\PC{0,40}") {
        for style in [PathStyle::Windows, PathStyle::Unix] {
            if let Ok(place) = Place::parse_with(&text, style) {
                let again = Place::parse_with(&place.to_string(), style);
                prop_assert_eq!(again.as_ref(), Ok(&place));
            }
        }
    }

    /// W2b's fuzzing finding, held for every name: a name joined into a
    /// local, archive or remote folder reads back from the place's text as
    /// the same place, or the join refuses it. Names come from an
    /// alphabet heavy in archive extensions, markers and dots.
    #[test]
    fn joined_names_read_back_as_themselves(
        parts in prop::collection::vec(
            prop::sample::select(vec![
                "a", "b", ".", "!", ".zip", ".zip!", ".xz", ".gz", ".7z", ".001", "wow", " ",
                ".docx", "!x", ".tar",
            ]),
            1..7,
        ),
    ) {
        let name = parts.concat();
        let folders = [
            ("/fuzz", PathStyle::Unix),
            ("D:\\fuzz", PathStyle::Windows),
            ("/fuzz/a.zip!week1", PathStyle::Unix),
            ("D:\\fuzz\\a.7z!", PathStyle::Windows),
            ("sftp://host/srv", PathStyle::Unix),
        ];
        for (folder, style) in folders {
            let folder = Place::parse_with(folder, style).unwrap();
            if let Ok(place) = folder.join(&name) {
                let again = Place::parse_with(&place.to_string(), style);
                prop_assert_eq!(again.as_ref(), Ok(&place), "{} in {}", name, folder);
                prop_assert_eq!(place.name(), Some(name.as_str()));
            } else {
                prop_assert!(
                    diroxide_core::place::holds_archive_marker(&name)
                        || matches!(name.as_str(), "." | ".."),
                    "{} refused in {}", name, folder
                );
            }
        }
    }
}
