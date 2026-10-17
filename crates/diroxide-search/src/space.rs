//! The Space list: disk use in words, never a drawn treemap (R13). First
//! one sentence, "Total 66.4 GB. Largest 3 hold 81 percent" (at most 40
//! cells), then the largest items with their share, "Videos, folder,
//! 41.2 GB, 62 percent", at most 30 of them, then "And 340 smaller
//! items, 2.1 GB, 3 percent". Two more groupings walk everything below:
//! by kind (the built-in groups, by extension until the full type model)
//! and by age. "Largest items below here" is the same walk, the files
//! sorted by size with a limit.
//!
//! Every count is dirsize's: no filters, hidden files counted, links not
//! followed, hard links once on Unix; what could not be read is said
//! after the sentence.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use diroxide_core::filter::group_of;
use diroxide_core::format::{BRAILLE_CELLS, human_size};
use diroxide_core::{Clock, Entry, EntryKind, ListModel, ListRow, Place};
use diroxide_vfs::{Registry, facts};

use crate::dirsize::walk_files;
use crate::summary::counted;
use crate::{DirSize, SearchControl, SearchError, SearchHit, size_every_folder};

/// How many rows a Space list shows before "And N smaller items".
pub const SPACE_ROWS: usize = 30;

/// How a Space list groups what it counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Grouping {
    /// The folder's own items, each folder with everything inside it.
    #[default]
    Items,
    /// Every file below, by its group (images, music, video and the
    /// rest), by extension.
    Kind,
    /// Every file below, by when it was last changed.
    Age,
}

impl Grouping {
    /// The grouping in words, for the list's title.
    pub const fn words(self) -> &'static str {
        match self {
            Grouping::Items => "largest first",
            Grouping::Kind => "by kind",
            Grouping::Age => "by age",
        }
    }
}

/// One row of a Space list.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpaceRow {
    /// A stable identity: the item's place, or the group's name.
    pub id: String,
    /// What it is called: an item's name, or a group's.
    pub name: String,
    /// What it is: "folder", "file", or for a group how many files.
    pub what: String,
    /// The bytes it holds.
    pub bytes: u64,
    /// The item's place, to drill into; `None` for a group.
    pub place: Option<Place>,
}

/// A Space list.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpaceList {
    /// The folder counted.
    pub place: Place,
    /// How the rows group what was counted.
    pub grouping: Grouping,
    /// Every byte counted.
    pub total: u64,
    /// The rows shown, largest first.
    pub rows: Vec<SpaceRow>,
    /// Items not shown: how many, and their bytes.
    pub rest: (u64, u64),
    /// What the count could and could not read.
    pub size: DirSize,
}

/// A share in words: "62 percent", "under 1 percent" for a share that
/// rounds to none, "0 percent" of nothing.
pub fn percent_words(bytes: u64, total: u64) -> String {
    if total == 0 || bytes == 0 {
        return "0 percent".to_owned();
    }
    let percent = (u128::from(bytes) * 100 + u128::from(total) / 2) / u128::from(total);
    if percent == 0 {
        "under 1 percent".to_owned()
    } else {
        format!("{percent} percent")
    }
}

/// Text with its first letter capitalized.
fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl SpaceList {
    fn from_rows(place: Place, grouping: Grouping, mut rows: Vec<SpaceRow>, size: DirSize) -> Self {
        rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        let total: u64 = rows.iter().map(|row| row.bytes).sum();
        let rest: Vec<SpaceRow> = rows.split_off(rows.len().min(SPACE_ROWS));
        let rest = (rest.len() as u64, rest.iter().map(|row| row.bytes).sum());
        Self {
            place,
            grouping,
            total,
            rows,
            rest,
            size,
        }
    }

    /// The sentence said first, at most 40 cells, meaning first: "Total
    /// 66.4 GB. Largest 3 hold 81 percent".
    pub fn summary(&self) -> String {
        let total = human_size(self.total);
        if self.rows.is_empty() || self.total == 0 {
            return format!("Total {total}. Nothing takes space");
        }
        let n = self.rows.len().min(3);
        let top: u64 = self.rows.iter().take(n).map(|row| row.bytes).sum();
        let hold = if top >= self.total {
            "all".to_owned()
        } else {
            let total = u128::from(self.total);
            let percent = ((u128::from(top) * 100 + total / 2) / total).clamp(1, 99);
            format!("{percent} percent")
        };
        let (largest, top_words, verb) = if n == 1 {
            ("Largest".to_owned(), "Top 1".to_owned(), "holds")
        } else {
            (format!("Largest {n}"), format!("Top {n}"), "hold")
        };
        let long = format!("Total {total}. {largest} {verb} {hold}");
        if long.chars().count() <= BRAILLE_CELLS {
            long
        } else {
            format!("Total {total}. {top_words} {verb} {hold}")
        }
    }

    /// What follows the sentence when it applies: what could not be read,
    /// how hard links were counted where they may repeat, and for cloud
    /// files what is on this computer.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if !self.size.is_complete() {
            notes.push(format!("{}.", self.size.sentence()));
        }
        if !self.size.hard_links_once && self.place.local_path().is_some() {
            notes.push(self.size.hard_link_words().to_owned());
        }
        if let Some(cloud) = self.size.cloud_words() {
            notes.push(format!("{cloud}."));
        }
        notes
    }

    /// The rows as a list shows them: "Videos, folder, 41.2 GB, 62
    /// percent", then "And 340 smaller items, 2.1 GB, 3 percent".
    pub fn list_rows(&self) -> Vec<ListRow> {
        let mut rows: Vec<ListRow> = self
            .rows
            .iter()
            .map(|row| {
                ListRow::new(row.id.clone(), row.name.clone()).with_details([
                    row.what.clone(),
                    human_size(row.bytes),
                    percent_words(row.bytes, self.total),
                ])
            })
            .collect();
        let (count, bytes) = self.rest;
        if count > 0 {
            let label = format!("And {}", counted(count, "smaller item", "smaller items"));
            rows.push(
                ListRow::new("rest", label)
                    .with_details([human_size(bytes), percent_words(bytes, self.total)]),
            );
        }
        rows
    }

    /// The list: its title says the place and the grouping, its rows are
    /// [`SpaceList::list_rows`].
    pub fn list(&self) -> ListModel {
        let name = self
            .place
            .name()
            .map_or_else(|| self.place.to_string(), str::to_owned);
        let mut list = ListModel::new(format!("Space in {name}, {}", self.grouping.words()))
            .with_rows(self.list_rows());
        list.set_empty_text("Nothing takes space here");
        list
    }
}

/// The Space list of a folder's own items: each folder counted with
/// everything in it (up to four at a time), each file by its size.
pub fn space_list(
    place: &Place,
    registry: &Registry,
    control: &SearchControl,
) -> Result<SpaceList, SearchError> {
    let provider = registry.resolve(place)?;
    let children = provider.list(place, &control.op_context())?;
    let mut size = DirSize {
        hard_links_once: cfg!(unix) && place.local_path().is_some(),
        ..DirSize::default()
    };
    let mut rows = Vec::new();
    let mut folders = Vec::new();
    for entry in children {
        let Ok(at) = place.join(&entry.name) else {
            continue;
        };
        match entry.kind {
            EntryKind::Folder => folders.push((at, entry.name)),
            EntryKind::File | EntryKind::Archive => {
                let bytes = entry.size.unwrap_or(0);
                size.files += 1;
                size.bytes += bytes;
                if facts::is_online_only(&entry) {
                    size.online_only += 1;
                } else {
                    size.on_this_computer += bytes;
                }
                rows.push(SpaceRow {
                    id: at.to_string(),
                    name: entry.display_name().into_owned(),
                    what: entry.kind.word().to_owned(),
                    bytes,
                    place: Some(at),
                });
            }
            EntryKind::Link => size.links += 1,
            _ => {}
        }
    }
    let places: Vec<Place> = folders.iter().map(|(at, _)| at.clone()).collect();
    let sized = size_every_folder(&places, registry, control, &mut |_, _| {});
    for ((at, name), (_, result)) in folders.into_iter().zip(sized) {
        let Ok(inner) = result else {
            size.unreadable += 1;
            continue;
        };
        size.folders += 1 + inner.folders;
        size.files += inner.files;
        size.bytes += inner.bytes;
        size.on_this_computer += inner.on_this_computer;
        size.online_only += inner.online_only;
        size.links += inner.links;
        size.unreadable += inner.unreadable;
        size.skipped += inner.skipped;
        size.repeated_links += inner.repeated_links;
        size.cancelled |= inner.cancelled;
        rows.push(SpaceRow {
            id: at.to_string(),
            name: diroxide_core::names::display_name(&name).into_owned(),
            what: "folder".to_owned(),
            bytes: inner.bytes,
            place: Some(at),
        });
    }
    size.cancelled |= control.is_cancelled();
    Ok(SpaceList::from_rows(
        place.clone(),
        Grouping::Items,
        rows,
        size,
    ))
}

/// Sums files into named groups, for the two walking groupings.
fn grouped_list(
    place: &Place,
    registry: &Registry,
    control: &SearchControl,
    grouping: Grouping,
    group: &dyn Fn(&Entry) -> &'static str,
) -> Result<SpaceList, SearchError> {
    let mut groups: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    let size = walk_files(place, registry, control, &mut |_, entry, _| {
        let slot = groups.entry(group(entry)).or_default();
        slot.0 += 1;
        slot.1 += entry.size.unwrap_or(0);
    })?;
    let rows = groups
        .into_iter()
        .map(|(name, (files, bytes))| SpaceRow {
            id: format!("group:{name}"),
            name: capital(name),
            what: counted(files, "file", "files"),
            bytes,
            place: None,
        })
        .collect();
    Ok(SpaceList::from_rows(place.clone(), grouping, rows, size))
}

/// The Space list by kind: every file below, by its built-in group
/// (images, music, video, documents, archives, code, text), the rest as
/// other files.
pub fn space_by_kind(
    place: &Place,
    registry: &Registry,
    control: &SearchControl,
) -> Result<SpaceList, SearchError> {
    grouped_list(place, registry, control, Grouping::Kind, &|entry| {
        group_of(entry).map_or("other files", |group| group.name)
    })
}

/// The Space list by age: every file below, by when it was last changed
/// against `clock`: in the last year, not in a year, not in two years.
pub fn space_by_age(
    place: &Place,
    registry: &Registry,
    clock: &dyn Clock,
    control: &SearchControl,
) -> Result<SpaceList, SearchError> {
    const YEAR: i64 = 365 * 86_400;
    let now = clock.now();
    let one = now.plus_seconds(-YEAR);
    let two = now.plus_seconds(-2 * YEAR);
    grouped_list(
        place,
        registry,
        control,
        Grouping::Age,
        &move |entry| match entry.modified {
            None => "date not known",
            Some(time) if time >= one => "changed in the last year",
            Some(time) if time >= two => "not changed in a year",
            Some(_) => "not changed in two years",
        },
    )
}

/// The largest files below a place, largest first.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Largest {
    /// Where the walk started.
    pub root: Place,
    /// The files, largest first.
    pub hits: Vec<SearchHit>,
    /// What the walk counted, and could not.
    pub size: DirSize,
}

impl Largest {
    /// The rows, size first after the name: "movie.mkv, 4.2 GB, in
    /// Videos/2024".
    pub fn rows(&self) -> Vec<ListRow> {
        self.hits
            .iter()
            .map(|hit| {
                ListRow::new(hit.id(), hit.entry.display_name()).with_details([
                    human_size(hit.entry.size.unwrap_or(0)),
                    hit.folder_words(Some(&self.root)),
                ])
            })
            .collect()
    }

    /// The list, titled "Largest items in Documents".
    pub fn list(&self) -> ListModel {
        let name = self
            .root
            .name()
            .map_or_else(|| self.root.to_string(), str::to_owned);
        let mut list = ListModel::new(format!("Largest items in {name}")).with_rows(self.rows());
        list.set_empty_text("No files here");
        list
    }
}

/// Largest items below here: the `limit` largest files anywhere below a
/// place, by the same walk as dirsize.
pub fn largest_below(
    place: &Place,
    registry: &Registry,
    control: &SearchControl,
    limit: usize,
) -> Result<Largest, SearchError> {
    let mut heap: BinaryHeap<Reverse<(u64, String, usize)>> = BinaryHeap::new();
    let mut kept: BTreeMap<String, SearchHit> = BTreeMap::new();
    let size = walk_files(place, registry, control, &mut |at, entry, depth| {
        let key = (entry.size.unwrap_or(0), at.to_string(), depth);
        if heap.len() < limit {
            heap.push(Reverse(key.clone()));
        } else if heap.peek().is_some_and(|Reverse(least)| key > *least) {
            if let Some(Reverse(gone)) = heap.pop() {
                kept.remove(&gone.1);
            }
            heap.push(Reverse(key.clone()));
        } else {
            return;
        }
        let depth = u32::try_from(depth).unwrap_or(u32::MAX);
        kept.insert(
            key.1,
            SearchHit::new(at.clone(), entry.clone()).with_depth(depth),
        );
    })?;
    let mut hits: Vec<SearchHit> = kept.into_values().collect();
    hits.sort_by(|a, b| {
        b.entry
            .size
            .cmp(&a.entry.size)
            .then_with(|| a.place.to_string().cmp(&b.place.to_string()))
    });
    Ok(Largest {
        root: place.clone(),
        hits,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(rows: &[(&str, u64)], rest: (u64, u64)) -> SpaceList {
        SpaceList {
            place: Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap(),
            grouping: Grouping::Items,
            total: rows.iter().map(|r| r.1).sum::<u64>() + rest.1,
            rows: rows
                .iter()
                .map(|(name, bytes)| SpaceRow {
                    id: (*name).to_owned(),
                    name: (*name).to_owned(),
                    what: "folder".to_owned(),
                    bytes: *bytes,
                    place: None,
                })
                .collect(),
            rest,
            size: DirSize {
                hard_links_once: true,
                ..DirSize::default()
            },
        }
    }

    #[test]
    fn the_sentence_is_the_plans_and_never_longer_than_40_cells() {
        let example = list(
            &[
                ("Videos", 41_200_000_000),
                ("Photos", 8_000_000_000),
                ("Music", 4_400_000_000),
                ("Documents", 10_700_000_000),
            ],
            (340, 2_100_000_000),
        );
        // The plan's example: 66.4 GB in all, the largest three 81 percent.
        let mut sorted = example.clone();
        sorted
            .rows
            .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        assert_eq!(sorted.summary(), "Total 66.4 GB. Largest 3 hold 90 percent");
        let plan = list(
            &[
                ("Videos", 41_200_000_000),
                ("Documents", 7_700_000_000),
                ("Photos", 4_900_000_000),
            ],
            (340, 12_600_000_000),
        );
        assert_eq!(plan.summary(), "Total 66.4 GB. Largest 3 hold 81 percent");
        assert_eq!(plan.summary().chars().count(), 40);
        assert_eq!(
            plan.list_rows()[0].spoken(),
            "Videos, folder, 41.2 GB, 62 percent"
        );
        assert_eq!(
            plan.list_rows().last().unwrap().spoken(),
            "And 340 smaller items, 12.6 GB, 19 percent"
        );
        for (rows, rest) in [
            (vec![("a", 999), ("b", 1), ("c", 1)], (1, 997)),
            (vec![("a", 99_900_000_000), ("b", 10), ("c", 10)], (1, 10)),
            (vec![("a", 999_000), ("b", 900)], (0, 0)),
            (vec![("a", 5)], (0, 0)),
            (vec![("a", 5)], (3, 5)),
            (vec![], (0, 0)),
        ] {
            let summary = list(&rows, rest).summary();
            assert!(summary.chars().count() <= 40, "{summary}");
            assert!(summary.starts_with("Total "), "{summary}");
        }
        assert_eq!(
            list(&[("a", 5)], (0, 0)).summary(),
            "Total 5 bytes. Largest holds all"
        );
        assert_eq!(
            list(&[], (0, 0)).summary(),
            "Total 0 bytes. Nothing takes space"
        );
        assert_eq!(
            list(&[("a", 999), ("b", 1), ("c", 1)], (1, 997)).summary(),
            "Total 2.0 KB. Largest 3 hold 50 percent"
        );
        assert_eq!(
            list(&[("a", 500), ("b", 300), ("c", 100)], (2, 99)).summary(),
            "Total 999 bytes. Top 3 hold 90 percent"
        );
    }

    #[test]
    fn shares_in_words() {
        assert_eq!(percent_words(62, 100), "62 percent");
        assert_eq!(percent_words(1, 1000), "under 1 percent");
        assert_eq!(percent_words(0, 0), "0 percent");
        assert_eq!(percent_words(5, 10), "50 percent");
    }
}
