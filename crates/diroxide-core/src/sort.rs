//! Sorting: the keys a lister sorts by, their directions, where folders go,
//! and one comparer every lister shares (plan section 5.2, the owner's
//! answer 33).
//!
//! The rules: folders first by default, with mixed and last as choices,
//! and folders sorted by the same key; each key has its sensible first
//! direction (names A to Z, sizes largest first, dates newest first) and
//! choosing the same key again reverses it; entries with no value for the
//! key (a folder's size before it is measured) come last in either
//! direction; name is always the last key, in natural order, so the order
//! is total and two entries that tie come in name order. Unsorted keeps
//! the provider's own order: [`compare`] then places only the folders, and
//! [`sort_entries`] is a stable sort.
//!
//! The status line says the result first: "Sorted by size, largest first".

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::entry::Entry;

/// What a lister sorts by.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortKey {
    /// The name, in natural order ("file2" before "file10").
    Name,
    /// The extension, then the name.
    Extension,
    /// The size in bytes.
    Size,
    /// When the contents last changed.
    Modified,
    /// When the entry was created.
    Created,
    /// When the entry was last read.
    Accessed,
    /// The kind: the file type's name, or the entry kind's word.
    Kind,
    /// The attributes or permissions, by their short words.
    Attributes,
    /// The owner's name.
    Owner,
    /// The first label.
    Label,
    /// A column a plugin adds, by its key in [`Entry::extra`].
    Extra(String),
    /// The provider's own order (the default inside an archive).
    Unsorted,
}

impl SortKey {
    /// The keys of a file panel, in the order the Sort list shows them.
    pub fn panel_keys() -> [SortKey; 11] {
        [
            SortKey::Name,
            SortKey::Extension,
            SortKey::Size,
            SortKey::Modified,
            SortKey::Created,
            SortKey::Accessed,
            SortKey::Kind,
            SortKey::Attributes,
            SortKey::Owner,
            SortKey::Label,
            SortKey::Unsorted,
        ]
    }

    /// The key in words, as the Sort list and the status line say it:
    /// "name", "size", "modified time".
    pub fn word(&self) -> String {
        match self {
            SortKey::Name => "name".to_owned(),
            SortKey::Extension => "extension".to_owned(),
            SortKey::Size => "size".to_owned(),
            SortKey::Modified => "modified time".to_owned(),
            SortKey::Created => "created time".to_owned(),
            SortKey::Accessed => "accessed time".to_owned(),
            SortKey::Kind => "kind".to_owned(),
            SortKey::Attributes => "attributes".to_owned(),
            SortKey::Owner => "owner".to_owned(),
            SortKey::Label => "label".to_owned(),
            SortKey::Extra(column) => column.clone(),
            SortKey::Unsorted => "the folder's own order".to_owned(),
        }
    }

    /// The direction a key starts in when it is first chosen: names and
    /// words A to Z, sizes largest first, dates newest first.
    pub fn first_direction(&self) -> SortDirection {
        match self {
            SortKey::Size | SortKey::Modified | SortKey::Created | SortKey::Accessed => {
                SortDirection::Descending
            }
            _ => SortDirection::Ascending,
        }
    }

    /// The direction in words for this key: "A to Z", "largest first",
    /// "newest first", or an empty string for unsorted.
    pub fn direction_words(&self, direction: SortDirection) -> &'static str {
        let ascending = direction == SortDirection::Ascending;
        match self {
            SortKey::Unsorted => "",
            SortKey::Size => {
                if ascending {
                    "smallest first"
                } else {
                    "largest first"
                }
            }
            SortKey::Modified | SortKey::Created | SortKey::Accessed => {
                if ascending {
                    "oldest first"
                } else {
                    "newest first"
                }
            }
            _ => {
                if ascending {
                    "A to Z"
                } else {
                    "Z to A"
                }
            }
        }
    }
}

/// Which way a key sorts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortDirection {
    /// Smallest, oldest or A first.
    #[default]
    Ascending,
    /// Largest, newest or Z first.
    Descending,
}

impl SortDirection {
    /// The other direction.
    pub const fn reversed(self) -> Self {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }
}

/// Where folders go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FolderOrder {
    /// Folders before files (the default).
    #[default]
    First,
    /// Folders among the files.
    Mixed,
    /// Folders after files.
    Last,
}

/// Whether names compare with regard to case.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NameCase {
    /// "a" and "A" sort together; case decides only a final tie (the
    /// default).
    #[default]
    Ignore,
    /// Uppercase before lowercase where the names otherwise match, and
    /// letters compared exactly.
    Respect,
}

/// A complete sort: the key, its direction, where folders go, and how
/// names compare.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SortSpec {
    /// The key.
    pub key: SortKey,
    /// Its direction.
    pub direction: SortDirection,
    /// Where folders go.
    pub folders: FolderOrder,
    /// How names compare.
    pub case: NameCase,
    /// Whether digits in names compare as numbers (on by default).
    pub natural: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
            folders: FolderOrder::First,
            case: NameCase::Ignore,
            natural: true,
        }
    }
}

impl SortSpec {
    /// A spec for a key in its first direction, with the other choices at
    /// their defaults.
    pub fn by(key: SortKey) -> Self {
        let direction = key.first_direction();
        Self {
            key,
            direction,
            ..Self::default()
        }
    }

    /// The spec after the user chooses `key`: the same key again reverses
    /// the direction; another key starts in its first direction. Where
    /// folders go and how names compare are kept.
    pub fn choose(&self, key: SortKey) -> Self {
        let direction = if key == self.key {
            self.direction.reversed()
        } else {
            key.first_direction()
        };
        Self {
            key,
            direction,
            ..self.clone()
        }
    }

    /// The result said first: "Sorted by size, largest first", "Sorted by
    /// name, A to Z", "Sorted in the folder's own order".
    pub fn describe(&self) -> String {
        match self.key {
            SortKey::Unsorted => "Sorted in the folder's own order".to_owned(),
            _ => format!(
                "Sorted by {}, {}",
                self.key.word(),
                self.key.direction_words(self.direction)
            ),
        }
    }
}

/// Compares two names: in natural order when `natural` is set (runs of
/// digits compare as numbers, so "file2" comes before "file10"), letters
/// without regard to case when `case` is [`NameCase::Ignore`]. The order is
/// total: names that compare equal by those rules are ordered by their
/// exact characters, so only identical names are equal.
pub fn compare_names(a: &str, b: &str, case: NameCase, natural: bool) -> Ordering {
    let loose = if natural {
        natural_order(a, b, case == NameCase::Ignore)
    } else {
        plain_order(a, b, case == NameCase::Ignore)
    };
    loose.then_with(|| a.cmp(b))
}

fn fold(c: char, ignore_case: bool) -> char {
    if ignore_case {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

fn plain_order(a: &str, b: &str, ignore_case: bool) -> Ordering {
    a.chars()
        .map(|c| fold(c, ignore_case))
        .cmp(b.chars().map(|c| fold(c, ignore_case)))
}

/// Natural order without allocating: digit runs compare by value (leading
/// zeros ignored, then fewer zeros first), other characters one by one.
fn natural_order(a: &str, b: &str, ignore_case: bool) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a.chars().next(), b.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let a_len = a.bytes().take_while(u8::is_ascii_digit).count();
                let b_len = b.bytes().take_while(u8::is_ascii_digit).count();
                let (a_run, b_run) = (&a[..a_len], &b[..b_len]);
                let a_value = a_run.trim_start_matches('0');
                let b_value = b_run.trim_start_matches('0');
                let order = a_value
                    .len()
                    .cmp(&b_value.len())
                    .then_with(|| a_value.cmp(b_value))
                    .then_with(|| a_len.cmp(&b_len));
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[a_len..];
                b = &b[b_len..];
            }
            (Some(x), Some(y)) => {
                let order = fold(x, ignore_case).cmp(&fold(y, ignore_case));
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[x.len_utf8()..];
                b = &b[y.len_utf8()..];
            }
        }
    }
}

fn folder_rank(entry: &Entry, order: FolderOrder) -> u8 {
    let folder = entry.is_folder_like();
    match (order, folder) {
        (FolderOrder::Mixed, _) => 0,
        (FolderOrder::First, true) | (FolderOrder::Last, false) => 0,
        (FolderOrder::First, false) | (FolderOrder::Last, true) => 1,
    }
}

/// Compares two optional values with missing values last, whatever the
/// direction.
fn by_value<T: Ord>(a: Option<T>, b: Option<T>, direction: SortDirection) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => match direction {
            SortDirection::Ascending => a.cmp(&b),
            SortDirection::Descending => b.cmp(&a),
        },
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Compares text values with natural order, missing or empty last.
fn by_text(a: Option<&str>, b: Option<&str>, spec: &SortSpec) -> Ordering {
    let a = a.filter(|t| !t.is_empty());
    let b = b.filter(|t| !t.is_empty());
    match (a, b) {
        (Some(a), Some(b)) => {
            let order = if spec.natural {
                natural_order(a, b, spec.case == NameCase::Ignore)
            } else {
                plain_order(a, b, spec.case == NameCase::Ignore)
            };
            match spec.direction {
                SortDirection::Ascending => order,
                SortDirection::Descending => order.reverse(),
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn kind_text(entry: &Entry) -> &str {
    entry.type_name.as_deref().unwrap_or(entry.kind.word())
}

/// Compares two entries by a spec: where folders go, then the key in its
/// direction, then the name in natural order (A to Z, whatever the
/// direction), then the exact name. A total order for every key but
/// [`SortKey::Unsorted`], which compares only where folders go so a stable
/// sort keeps the provider's order.
pub fn compare(a: &Entry, b: &Entry, spec: &SortSpec) -> Ordering {
    let placed = folder_rank(a, spec.folders).cmp(&folder_rank(b, spec.folders));
    if placed != Ordering::Equal {
        return placed;
    }
    let by_key = match &spec.key {
        SortKey::Unsorted => return Ordering::Equal,
        SortKey::Name => {
            let order = compare_names(&a.name, &b.name, spec.case, spec.natural);
            return match spec.direction {
                SortDirection::Ascending => order,
                SortDirection::Descending => order.reverse(),
            };
        }
        SortKey::Extension => by_text(a.extension(), b.extension(), spec),
        SortKey::Size => by_value(a.size, b.size, spec.direction),
        SortKey::Modified => by_value(a.modified, b.modified, spec.direction),
        SortKey::Created => by_value(a.created, b.created, spec.direction),
        SortKey::Accessed => by_value(a.accessed, b.accessed, spec.direction),
        SortKey::Kind => by_text(Some(kind_text(a)), Some(kind_text(b)), spec),
        SortKey::Attributes => {
            let words = |e: &Entry| e.attributes.map(|attr| attr.short_words(e.kind));
            by_text(words(a).as_deref(), words(b).as_deref(), spec)
        }
        SortKey::Owner => by_text(a.owner.as_deref(), b.owner.as_deref(), spec),
        SortKey::Label => by_text(
            a.labels.first().map(String::as_str),
            b.labels.first().map(String::as_str),
            spec,
        ),
        SortKey::Extra(column) => by_text(
            a.extra.get(column).map(String::as_str),
            b.extra.get(column).map(String::as_str),
            spec,
        ),
    };
    by_key.then_with(|| compare_names(&a.name, &b.name, spec.case, spec.natural))
}

/// Sorts entries in place by a spec. The sort is stable, so
/// [`SortKey::Unsorted`] keeps the given order within the folder groups.
pub fn sort_entries(entries: &mut [Entry], spec: &SortSpec) {
    entries.sort_by(|a, b| compare(a, b, spec));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Timestamp;
    use crate::entry::EntryKind;
    use crate::format::within_cells;

    fn file(name: &str) -> Entry {
        Entry::new(name, EntryKind::File)
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn natural_order_puts_file2_before_file10() {
        let order = |a, b| compare_names(a, b, NameCase::Ignore, true);
        assert_eq!(order("file2", "file10"), Ordering::Less);
        assert_eq!(order("file010", "file9"), Ordering::Greater);
        assert_eq!(order("a", "A"), Ordering::Greater);
        assert_eq!(order("a", "b"), Ordering::Less);
        assert_eq!(order("B", "a"), Ordering::Greater);
        assert_eq!(order("x01", "x1"), Ordering::Greater);
        assert_eq!(order("same", "same"), Ordering::Equal);
        assert_eq!(
            compare_names("file10", "file2", NameCase::Ignore, false),
            Ordering::Less
        );
    }

    #[test]
    fn a_and_capital_a_differ_only_by_the_final_tie() {
        let order = |a, b| natural_order(a, b, true);
        assert_eq!(order("a", "A"), Ordering::Equal);
        assert_ne!(
            compare_names("a", "A", NameCase::Ignore, true),
            Ordering::Equal
        );
    }

    #[test]
    fn folders_first_mixed_and_last() {
        let mut entries = vec![
            file("b.txt"),
            Entry::new("zeta", EntryKind::Folder),
            file("a.txt"),
            Entry::new("alpha", EntryKind::Folder),
        ];
        sort_entries(&mut entries, &SortSpec::default());
        assert_eq!(names(&entries), ["alpha", "zeta", "a.txt", "b.txt"]);
        let last = SortSpec {
            folders: FolderOrder::Last,
            ..SortSpec::default()
        };
        sort_entries(&mut entries, &last);
        assert_eq!(names(&entries), ["a.txt", "b.txt", "alpha", "zeta"]);
        let mixed = SortSpec {
            folders: FolderOrder::Mixed,
            ..SortSpec::default()
        };
        sort_entries(&mut entries, &mixed);
        assert_eq!(names(&entries), ["a.txt", "alpha", "b.txt", "zeta"]);
    }

    #[test]
    fn sizes_largest_first_with_unknowns_last_and_name_ties() {
        let mut entries = vec![
            file("small").with_size(1),
            file("b-big").with_size(100),
            file("a-big").with_size(100),
            file("unknown"),
        ];
        sort_entries(&mut entries, &SortSpec::by(SortKey::Size));
        assert_eq!(names(&entries), ["a-big", "b-big", "small", "unknown"]);
        let reversed = SortSpec::by(SortKey::Size).choose(SortKey::Size);
        assert_eq!(reversed.direction, SortDirection::Ascending);
        sort_entries(&mut entries, &reversed);
        assert_eq!(names(&entries), ["small", "a-big", "b-big", "unknown"]);
    }

    #[test]
    fn dates_newest_first() {
        let t = |s| Timestamp::from_unix(s, 0);
        let mut entries = vec![
            file("old").with_modified(t(10)),
            file("new").with_modified(t(30)),
            file("mid").with_modified(t(20)),
        ];
        sort_entries(&mut entries, &SortSpec::by(SortKey::Modified));
        assert_eq!(names(&entries), ["new", "mid", "old"]);
    }

    #[test]
    fn unsorted_keeps_the_given_order() {
        let mut entries = vec![file("c"), file("a"), file("b")];
        sort_entries(&mut entries, &SortSpec::by(SortKey::Unsorted));
        assert_eq!(names(&entries), ["c", "a", "b"]);
    }

    #[test]
    fn other_keys_sort_by_their_text() {
        let mut entries = vec![
            file("b.txt"),
            file("a.pdf"),
            file("c").with_extra("git", "modified"),
            file("d").with_extra("git", "added"),
        ];
        sort_entries(&mut entries, &SortSpec::by(SortKey::Extension));
        assert_eq!(names(&entries), ["a.pdf", "b.txt", "c", "d"]);
        sort_entries(&mut entries, &SortSpec::by(SortKey::Extra("git".into())));
        assert_eq!(names(&entries), ["d", "c", "a.pdf", "b.txt"]);
    }

    #[test]
    fn the_status_says_the_result_first() {
        assert_eq!(
            SortSpec::by(SortKey::Size).describe(),
            "Sorted by size, largest first"
        );
        assert_eq!(SortSpec::default().describe(), "Sorted by name, A to Z");
        assert_eq!(
            SortSpec::by(SortKey::Unsorted).describe(),
            "Sorted in the folder's own order"
        );
        for key in SortKey::panel_keys() {
            for direction in [SortDirection::Ascending, SortDirection::Descending] {
                let spec = SortSpec {
                    key: key.clone(),
                    direction,
                    ..SortSpec::default()
                };
                let said = spec.describe();
                assert!(said.chars().count() <= 40, "{said}");
                assert!(within_cells(&said, "Sorted"));
            }
        }
    }

    #[test]
    fn choosing_another_key_starts_in_its_first_direction() {
        let spec = SortSpec::default().choose(SortKey::Modified);
        assert_eq!(spec.direction, SortDirection::Descending);
        assert_eq!(
            spec.choose(SortKey::Name).direction,
            SortDirection::Ascending
        );
        assert_eq!(
            SortSpec::default().choose(SortKey::Name).direction,
            SortDirection::Descending
        );
    }

    #[test]
    #[ignore = "timing, not a gate: run with --ignored to measure"]
    fn sorting_100000_names_is_timed() {
        let mut entries: Vec<Entry> = (0..100_000)
            .map(|i| file(&format!("file{}", (i * 7919) % 100_000)))
            .collect();
        let start = std::time::Instant::now();
        sort_entries(&mut entries, &SortSpec::default());
        println!("100000 names sorted in {:?}", start.elapsed());
        assert_eq!(entries[0].name, "file0");
        assert_eq!(entries[99_999].name, "file99999");
    }
}
