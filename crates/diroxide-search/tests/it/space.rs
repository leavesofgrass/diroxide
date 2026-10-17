//! The Space list on the fake place: the items, the two groupings,
//! largest items below here, and the sentence at 40 cells; snapshots of
//! what is said.

use diroxide_core::{FixedClock, UtcOffset};
use diroxide_search::{
    SPACE_ROWS, SearchControl, SpaceList, largest_below, space_by_age, space_by_kind, space_list,
};

use crate::fake::{Fake, WHEN};

const DAY: i64 = 86_400;

/// A home folder: videos, photos (one old), documents, two loose files,
/// an unreadable folder.
fn home() -> std::sync::Arc<Fake> {
    let fake = Fake::new();
    fake.file("/home/Videos/trip.mkv", &[0; 40_000]);
    fake.file("/home/Videos/clip.mp4", &[0; 12_000]);
    fake.file("/home/Photos/beach.jpg", &[0; 9_000]);
    fake.file_at(
        "/home/Photos/2023/old.png",
        &[0; 3_000],
        WHEN.plus_seconds(-800 * DAY),
    );
    fake.file_at(
        "/home/Documents/report.pdf",
        &[0; 2_500],
        WHEN.plus_seconds(-400 * DAY),
    );
    fake.file("/home/notes.txt", &[0; 1_200]);
    fake.file("/home/.config", &[0; 300]);
    fake.refuse("/home/locked");
    fake
}

/// Everything a Space list says, one line each: the sentence, the notes,
/// the rows.
fn said(list: &SpaceList) -> String {
    let mut lines = vec![list.list().title().to_owned(), list.summary()];
    lines.extend(list.notes());
    lines.extend(list.list_rows().iter().map(|row| row.spoken()));
    lines.join("\n")
}

#[test]
fn the_items_largest_first() {
    let fake = home();
    let list = space_list(
        &Fake::place("/home"),
        &fake.registry(),
        &SearchControl::new(),
    )
    .unwrap();
    assert_eq!(list.total, 68_000);
    assert!(list.summary().chars().count() <= 40);
    assert_eq!(list.rows[0].name, "Videos");
    insta::assert_snapshot!("space_items", said(&list));
}

#[test]
fn by_kind_and_by_age() {
    let fake = home();
    let registry = fake.registry();
    let kind = space_by_kind(&Fake::place("/home"), &registry, &SearchControl::new()).unwrap();
    insta::assert_snapshot!("space_by_kind", said(&kind));
    let clock = FixedClock::new(WHEN, UtcOffset::from_hours(0));
    let age = space_by_age(
        &Fake::place("/home"),
        &registry,
        &clock,
        &SearchControl::new(),
    )
    .unwrap();
    insta::assert_snapshot!("space_by_age", said(&age));
}

#[test]
fn many_items_end_with_the_smaller_ones() {
    let fake = Fake::new();
    for n in 0..40u64 {
        let size = usize::try_from(1_000 + n * 10).unwrap();
        fake.file(&format!("/many/file-{n:02}.bin"), &vec![0; size]);
    }
    let list = space_list(
        &Fake::place("/many"),
        &fake.registry(),
        &SearchControl::new(),
    )
    .unwrap();
    assert_eq!(list.rows.len(), SPACE_ROWS);
    assert_eq!(list.rest.0, 10);
    let rows = list.list_rows();
    assert_eq!(rows[0].spoken(), "file-39.bin, file, 1.4 KB, 3 percent");
    assert_eq!(
        rows.last().unwrap().spoken(),
        "And 10 smaller items, 10.4 KB, 22 percent"
    );
    assert_eq!(list.summary(), "Total 47.8 KB. Largest 3 hold 9 percent");
}

#[test]
fn largest_items_below_here() {
    let fake = home();
    let largest = largest_below(
        &Fake::place("/home"),
        &fake.registry(),
        &SearchControl::new(),
        3,
    )
    .unwrap();
    let rows: Vec<String> = largest.rows().iter().map(|r| r.spoken()).collect();
    assert_eq!(
        rows,
        [
            "trip.mkv, 40.0 KB, in home/Videos",
            "clip.mp4, 12.0 KB, in home/Videos",
            "beach.jpg, 9.0 KB, in home/Photos",
        ]
    );
    assert_eq!(largest.list().title(), "Largest items in home");
    assert_eq!(largest.size.unreadable, 1);
}

#[test]
fn an_empty_folder_says_so() {
    let fake = Fake::new();
    fake.folder("/empty");
    let list = space_list(
        &Fake::place("/empty"),
        &fake.registry(),
        &SearchControl::new(),
    )
    .unwrap();
    assert_eq!(list.summary(), "Total 0 bytes. Nothing takes space");
    assert_eq!(list.list().announce_focus(), "Nothing takes space here");
}
