//! Saved searches and collections against the golden files in
//! `fixtures/search/saved`, saving into a folder under the target folder,
//! the stale check and newest files on the fake place.

use std::path::{Path, PathBuf};

use diroxide_core::filter::FilterOptions;
use diroxide_core::{CaseRule, FixedClock, UtcOffset};
use diroxide_search::{
    Day, FnSink, Freshness, OnlineOnly, RowShape, Saved, SavedError, SavedItem, SavedKind,
    SearchControl, SearchHit, SearchQuery, file_name_for, newest_files, run_search, stale_check,
};
use diroxide_vfs::OpContext;

use crate::fake::{Fake, WHEN};

const DAY: i64 = 86_400;

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/search/saved")
        .join(name)
}

fn clock_at(seconds: i64) -> FixedClock {
    FixedClock::new(WHEN.plus_seconds(seconds), UtcOffset::from_hours(0))
}

#[test]
fn every_golden_file_reads_and_writes_back_the_same() {
    for name in [
        "large-videos.toml",
        "taxes.toml",
        "from-a-later-version.toml",
    ] {
        let text = std::fs::read_to_string(golden(name)).unwrap();
        let saved = Saved::from_toml(&text).unwrap();
        assert_eq!(saved.to_toml().unwrap(), text, "{name}");
    }
}

#[test]
fn a_saved_search_holds_its_scope_and_runs_on_open() {
    let saved = Saved::load(&golden("large-videos.toml")).unwrap();
    assert_eq!(saved.kind, SavedKind::Search);
    assert_eq!(
        saved.row_words(),
        "Large videos, saved search, runs on open"
    );
    let query = saved.to_query(&FilterOptions::default()).unwrap();
    assert_eq!(query.scope.places.len(), 2);
    assert_eq!(query.scope.max_depth, Some(4));
    assert!(query.reads_content());
    assert_eq!(
        query.typed(),
        "(group video and size over 1 GB and modified within 2 years) and case content regex \"Ada Example\""
    );
    let later = Saved::load(&golden("from-a-later-version.toml")).unwrap();
    assert_eq!(
        later.extra.get("pinned").and_then(toml::Value::as_bool),
        Some(true)
    );
}

#[test]
fn a_collection_parses_and_is_not_a_search() {
    let saved = Saved::load(&golden("taxes.toml")).unwrap();
    assert_eq!(saved.kind, SavedKind::Collection);
    assert_eq!(saved.items.len(), 2);
    assert_eq!(saved.items[0].label.as_deref(), Some("urgent"));
    assert_eq!(saved.items[1].label, None);
    assert_eq!(saved.row_words(), "Taxes 2025, collection, 2 items");
    assert!(matches!(
        saved.to_query(&FilterOptions::default()),
        Err(SavedError::NotASearch)
    ));
}

#[test]
fn unknown_keys_survive_beside_tables() {
    let text = "format = 1
name = \"x\"
kind = \"search\"
pinned = true

[sort]
column = \"size\"
direction = \"descending\"

[later]
shade = \"blue\"
";
    let saved = Saved::from_toml(text).unwrap();
    let written = saved.to_toml().unwrap();
    assert_eq!(Saved::from_toml(&written).unwrap(), saved, "{written}");
}

#[test]
fn a_newer_format_and_a_wrong_shape_are_refused_in_words() {
    let newer = Saved::from_toml("format = 2\nname = \"x\"\nkind = \"search\"\n").unwrap_err();
    assert_eq!(
        newer.to_string(),
        "it was saved by a newer version (format 2)"
    );
    let wrong = Saved::from_toml("name = 3").unwrap_err();
    assert!(wrong.to_string().starts_with("it is not a saved search: "));
}

#[test]
fn saving_is_atomic_and_never_replaces_another_search() {
    let dir = tempfile::Builder::new()
        .prefix("search-saved-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let folder = dir.path().join("saved");
    let query = SearchQuery::parse(Fake::place("/home"), "ext pdf and modified this week")
        .unwrap()
        .with_rows(RowShape::PerLine);
    let saved = Saved::search("Large videos", &query, &clock_at(0));
    let path = saved.save(&folder).unwrap();
    assert_eq!(path.file_name().unwrap(), "large-videos.toml");
    assert_eq!(Saved::load(&path).unwrap(), saved);
    // Saving the same search again writes the same file.
    assert_eq!(saved.save(&folder).unwrap(), path);
    // Another search whose name makes the same file name gets a number.
    let other = Saved::search("Large Videos!", &query, &clock_at(0));
    let second = other.save(&folder).unwrap();
    assert_eq!(second.file_name().unwrap(), "large-videos-2.toml");
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "nothing unfinished is left: {names:?}");
    let listed = Saved::list(&folder);
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|(_, saved)| saved.is_ok()));
    let back = Saved::load(&path)
        .unwrap()
        .to_query(&FilterOptions::default())
        .unwrap();
    assert_eq!(back.rows, RowShape::PerLine);
    assert_eq!(back.typed(), "ext pdf and modified this week");
    assert_eq!(file_name_for("Large videos"), "large-videos.toml");
    let collection = Saved::collection(
        "Taxes",
        vec![SavedItem {
            place: Fake::place("/a.pdf"),
            label: None,
        }],
        &clock_at(0),
    );
    let at = collection.save(&folder).unwrap();
    assert_eq!(Saved::load(&at).unwrap().items.len(), 1);
}

#[test]
fn date_words_are_read_again_each_time_a_saved_search_runs() {
    let fake = Fake::new();
    fake.file("/home/today.txt", b"x");
    let query = SearchQuery::parse(Fake::place("/home"), "modified today").unwrap();
    let saved = Saved::search("Today", &query, &clock_at(0));
    assert_eq!(saved.case, None);
    let run = |seconds| {
        let query = saved.to_query(&FilterOptions::default()).unwrap();
        let mut found = 0;
        run_search(
            &query,
            &fake.registry(),
            &clock_at(seconds),
            &SearchControl::new(),
            &mut FnSink(|_: SearchHit| found += 1),
        )
        .unwrap();
        found
    };
    assert_eq!(run(3_600), 1);
    assert_eq!(run(3 * DAY), 0);
    let names = SearchQuery::names(Fake::place("/home"), "*.TXT");
    let saved = Saved::search("Text", &names, &clock_at(0));
    assert_eq!(saved.case.as_deref(), Some("ignore"));
    let back = saved.to_query(&FilterOptions::default()).unwrap();
    assert_eq!(back.options().case, CaseRule::Ignore);
    assert_eq!(back.scope.online_only, OnlineOnly::Skip);
}

#[test]
fn the_stale_check_says_fresh_changed_or_gone() {
    let fake = Fake::new();
    fake.file("/home/report.txt", b"first");
    let ctx = OpContext::new();
    let mut hits = Vec::new();
    run_search(
        &SearchQuery::parse(Fake::place("/home"), "report").unwrap(),
        &fake.registry(),
        &clock_at(0),
        &SearchControl::new(),
        &mut FnSink(|hit: SearchHit| hits.push(hit)),
    )
    .unwrap();
    let hit = &hits[0];
    assert_eq!(stale_check(hit, &fake.registry(), &ctx), Freshness::Fresh);
    fake.file("/home/report.txt", b"second, longer");
    assert_eq!(stale_check(hit, &fake.registry(), &ctx), Freshness::Changed);
    let mut gone = hit.clone();
    gone.place = Fake::place("/home/gone.txt");
    let freshness = stale_check(&gone, &fake.registry(), &ctx);
    assert_eq!(freshness, Freshness::Gone);
    assert_eq!(
        freshness.words("gone.txt").unwrap(),
        "gone.txt, gone since this list was made"
    );
}

#[test]
fn newest_files_and_the_day_before() {
    let fake = Fake::new();
    fake.file_at("/home/a.txt", b"a", WHEN.plus_seconds(-DAY));
    fake.file_at("/home/sub/b.txt", b"b", WHEN.plus_seconds(-DAY + 60));
    fake.file_at("/home/c.txt", b"c", WHEN.plus_seconds(-3 * DAY));
    fake.file_at("/home/d.txt", b"d", WHEN.plus_seconds(-10 * DAY));
    let scope = diroxide_search::Scope::new(Fake::place("/home"));
    let clock = clock_at(0);
    let newest = newest_files(
        &scope,
        &fake.registry(),
        &clock,
        &SearchControl::new(),
        None,
    )
    .unwrap();
    assert_eq!(newest.title(), "Newest files: 2 changed on October 2, 2026");
    assert_eq!(newest.hits[0].entry.name, "b.txt");
    let earlier = newest_files(
        &scope,
        &fake.registry(),
        &clock,
        &SearchControl::new(),
        newest.earlier(),
    )
    .unwrap();
    assert_eq!(
        earlier.title(),
        "Newest files: 1 changed on September 30, 2026"
    );
    let none = newest_files(
        &scope,
        &fake.registry(),
        &clock,
        &SearchControl::new(),
        Some(Day {
            year: 2026,
            month: 9,
            day: 1,
        }),
    )
    .unwrap();
    assert_eq!(none.title(), "Newest files: none before September 1, 2026");
    assert_eq!(
        newest
            .list(&Fake::place("/home"))
            .rows()
            .first()
            .unwrap()
            .spoken(),
        "b.txt, in home/sub, 1 byte"
    );
}
