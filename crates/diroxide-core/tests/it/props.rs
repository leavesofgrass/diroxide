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

    #[test]
    fn places_never_panic(text in "\\PC{0,40}") {
        for style in [PathStyle::Windows, PathStyle::Unix] {
            if let Ok(place) = Place::parse_with(&text, style) {
                let again = Place::parse_with(&place.to_string(), style);
                prop_assert_eq!(again.as_ref(), Ok(&place));
            }
        }
    }
}
