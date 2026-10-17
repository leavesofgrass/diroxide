//! Content search on a local fixture (UTF-8, UTF-16 with and without a
//! byte-order mark, a binary file, CRLF lines, a long line, an empty
//! file), its parity with `rg` when `rg` is on PATH, and the online-only
//! guard on the fake place.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use diroxide_core::{Entry, FixedClock, Place, UtcOffset};
use diroxide_search::{
    FnSink, OnlineGuard, OnlineOnly, RowShape, Scope, SearchControl, SearchHit, SearchQuery,
    SearchSummary, run_search,
};
use diroxide_vfs::Registry;

use crate::fake::{Fake, WHEN};

fn utf16(text: &str, little: bool, mark: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if mark {
        bytes.extend(if little { [0xFF, 0xFE] } else { [0xFE, 0xFF] });
    }
    for unit in text.encode_utf16() {
        bytes.extend(if little {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    bytes
}

/// The fixture, written for each test under the target folder: the names
/// and bytes are fixed, so the expected lines are too.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("search-content-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = dir.path().join("content");
    std::fs::create_dir(&root).unwrap();
    let long = format!(
        "{} total due {}\nshort\n",
        "x".repeat(300_000),
        "y".repeat(300_000)
    );
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "utf8.txt",
            b"first line\ntotal due here\nnothing\nTOTAL DUE caps\n".to_vec(),
        ),
        (
            "utf16le.txt",
            utf16("alpha\ntotal due in utf16\n", true, true),
        ),
        (
            "utf16be.txt",
            utf16("alpha\ntotal due in utf16\n", false, true),
        ),
        (
            "nomark16.txt",
            utf16("total due without a mark\n", true, false),
        ),
        ("binary.bin", b"total due\0\x01\x02".to_vec()),
        ("crlf.txt", b"one\r\ntotal due\r\nthree\r\n".to_vec()),
        ("long.txt", long.into_bytes()),
        ("empty.txt", Vec::new()),
    ];
    for (name, bytes) in files {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    (dir, root)
}

fn clock() -> FixedClock {
    FixedClock::new(WHEN, UtcOffset::from_hours(0))
}

fn run(query: &SearchQuery, registry: &Registry) -> (Vec<SearchHit>, SearchSummary) {
    run_with(query, registry, &SearchControl::new())
}

fn run_with(
    query: &SearchQuery,
    registry: &Registry,
    control: &SearchControl,
) -> (Vec<SearchHit>, SearchSummary) {
    let mut hits = Vec::new();
    let summary = run_search(
        query,
        registry,
        &clock(),
        control,
        &mut FnSink(|hit: SearchHit| hits.push(hit)),
    )
    .unwrap();
    (hits, summary)
}

fn place(path: &Path) -> Place {
    Place::parse(&path.to_string_lossy()).unwrap()
}

/// Each hit as its file's name and line number.
fn lines(hits: &[SearchHit]) -> BTreeSet<(String, u64)> {
    hits.iter()
        .map(|hit| {
            (
                hit.entry.name.clone(),
                hit.line.as_ref().map_or(0, |l| l.line_number),
            )
        })
        .collect()
}

fn expected() -> BTreeSet<(String, u64)> {
    [
        ("crlf.txt", 2),
        ("long.txt", 1),
        ("utf16be.txt", 2),
        ("utf16le.txt", 2),
        ("utf8.txt", 2),
        ("utf8.txt", 4),
    ]
    .iter()
    .map(|(name, line)| ((*name).to_owned(), *line))
    .collect()
}

#[test]
fn utf16_with_a_mark_is_searched_and_nul_files_are_skipped_and_counted() {
    let (_dir, root) = fixture();
    let query = SearchQuery::parse(place(&root), "content \"total due\"").unwrap();
    let (hits, summary) = run(&query, &Registry::with_local());
    let mut names: Vec<&str> = hits.iter().map(|h| h.entry.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "crlf.txt",
            "long.txt",
            "utf16be.txt",
            "utf16le.txt",
            "utf8.txt"
        ]
    );
    assert_eq!(summary.skipped.binary, 2);
    assert_eq!(summary.sentence(), "5 found. 2 binary files skipped.");
    assert_eq!(summary.files_read, 8);
    let utf8 = hits.iter().find(|h| h.entry.name == "utf8.txt").unwrap();
    assert_eq!(
        utf8.row(Some(&place(&root))).spoken(),
        "utf8.txt, line 2: total due here, 1 more, in content"
    );
    let crlf = hits.iter().find(|h| h.entry.name == "crlf.txt").unwrap();
    assert_eq!(crlf.line.as_ref().unwrap().text, "total due");
}

#[test]
fn one_row_per_line_and_the_byte_limit() {
    let (_dir, root) = fixture();
    let query = SearchQuery::parse(place(&root), "content \"total due\"")
        .unwrap()
        .with_rows(RowShape::PerLine);
    let (hits, summary) = run(&query, &Registry::with_local());
    assert_eq!(lines(&hits), expected());
    assert_eq!(summary.hits, 6);
    let limited = SearchQuery::parse(
        place(&root),
        "name utf8.txt and content \"total due\" first 20 B",
    )
    .unwrap();
    let (hits, summary) = run(&limited, &Registry::with_local());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].more, 0);
    assert_eq!(summary.bytes_read, 20);
    let case = SearchQuery::parse(place(&root), "content \"TOTAL DUE\"").unwrap();
    let (hits, _) = run(&case, &Registry::with_local());
    assert_eq!(lines(&hits), [("utf8.txt".to_owned(), 4)].into());
    let regex = SearchQuery::parse(place(&root), "content regex \"^total due$\"").unwrap();
    let (hits, _) = run(&regex, &Registry::with_local());
    assert_eq!(lines(&hits), [("crlf.txt".to_owned(), 2)].into());
}

#[test]
fn parity_with_rg_on_the_fixture() {
    let available = Command::new("rg").arg("--version").output().is_ok();
    if !available {
        eprintln!("rg is not on PATH: the parity check with rg is skipped.");
        return;
    }
    let (_dir, root) = fixture();
    let output = Command::new("rg")
        .args(["-n", "--no-heading", "--with-filename", "-uu", "-S"])
        .arg("total due")
        .arg(&root)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    let theirs: BTreeSet<(String, u64)> = text
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix(&*root.to_string_lossy())?;
            let rest = rest.trim_start_matches(['/', '\\']);
            let mut parts = rest.splitn(3, ':');
            let name = parts.next()?.to_owned();
            let number = parts.next()?.parse().ok()?;
            Some((name, number))
        })
        .collect();
    let query = SearchQuery::parse(place(&root), "content \"total due\"")
        .unwrap()
        .with_rows(RowShape::PerLine)
        .with_scope(Scope::new(place(&root)).with_hidden(true));
    let (hits, _) = run(&query, &Registry::with_local());
    assert_eq!(lines(&hits), theirs, "rg said:\n{text}");
}

/// A guard that answers the same for every file, and counts the asks.
struct Answer(bool, std::sync::atomic::AtomicU64);

impl OnlineGuard for Answer {
    fn allow(&self, _: &Place, _: &Entry) -> bool {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0
    }
}

#[test]
fn online_only_files_are_skipped_unless_read_is_asked_and_allowed() {
    let fake = Fake::new();
    fake.file("/docs/here.txt", b"total due\n");
    fake.file("/docs/cloud.txt", b"total due\n");
    fake.online_only("/docs/cloud.txt");
    let registry = fake.registry();
    let query = SearchQuery::parse(Fake::place("/"), "content \"total due\"").unwrap();
    let (hits, summary) = run(&query, &registry);
    assert_eq!(hits.len(), 1);
    assert_eq!(summary.skipped.online_only, 1);
    assert_eq!(
        summary.sentence(),
        "1 found. 1 online-only file skipped, 10 bytes."
    );
    let reading = query
        .clone()
        .with_scope(Scope::new(Fake::place("/")).with_online_only(OnlineOnly::Read));
    let refuse = Arc::new(Answer(false, 0.into()));
    let (hits, summary) = run_with(
        &reading,
        &registry,
        &SearchControl::new().with_guard(Arc::clone(&refuse) as Arc<dyn OnlineGuard>),
    );
    assert_eq!((hits.len(), summary.skipped.online_only), (1, 1));
    assert_eq!(refuse.1.load(std::sync::atomic::Ordering::SeqCst), 1);
    let allow = Arc::new(Answer(true, 0.into()));
    let (hits, summary) = run_with(
        &reading,
        &registry,
        &SearchControl::new().with_guard(allow as Arc<dyn OnlineGuard>),
    );
    assert_eq!((hits.len(), summary.skipped.online_only), (2, 0));
}
