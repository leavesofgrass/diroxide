//! The quick view driven through its methods, on the fake provider with a
//! log of every call, the time handed in so no test waits for the clock
//! except spike Q1, which runs the owner's hundred moves in real time.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_core::{Entry, EntryKind, Message, Place};
use diroxide_ops::fake::{CallFault, FakeCall, FakeProvider, Latency};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, ReadSeek, Registry, VfsError,
    WatchEvent, WriteMode,
};

use super::*;

/// A local place written Unix-style, on `D:` on Windows: in the fake only,
/// never on a disk.
pub(super) fn p(text: &str) -> Place {
    Place::parse(&if cfg!(windows) {
        format!("D:{}", text.replace('/', "\\"))
    } else {
        text.to_owned()
    })
    .unwrap()
}

pub(super) fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/docs")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fake provider with a log of every list, stat and read, by place.
#[derive(Debug)]
pub(super) struct Logged {
    pub fake: FakeProvider,
    log: Mutex<Vec<(&'static str, String)>>,
}

impl Logged {
    pub fn new(fake: FakeProvider) -> Arc<Self> {
        Arc::new(Self {
            fake,
            log: Mutex::new(Vec::new()),
        })
    }

    fn note(&self, call: &'static str, place: &Place) {
        self.log.lock().unwrap().push((call, place.to_string()));
    }

    /// The places read, in order.
    pub fn reads(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(call, _)| matches!(*call, "read" | "open"))
            .map(|(_, place)| place.clone())
            .collect()
    }

    /// Every call, in order.
    pub fn calls(&self) -> Vec<(&'static str, String)> {
        self.log.lock().unwrap().clone()
    }
}

impl Provider for Logged {
    fn scheme(&self) -> &str {
        self.fake.scheme()
    }

    fn capabilities(&self, place: &Place) -> Capabilities {
        self.fake.capabilities(place)
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.note("list", place);
        self.fake.list(place, ctx)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.note("stat", place);
        self.fake.stat(place, ctx)
    }

    fn read(
        &self,
        place: &Place,
        ctx: &OpContext,
    ) -> Result<Box<dyn std::io::Read + Send>, VfsError> {
        self.note("read", place);
        self.fake.read(place, ctx)
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.note("open", place);
        self.fake.open_seekable(place, ctx)
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.fake.write(place, mode, ctx)
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.create_dir(place, ctx)
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.rename(from, to, ctx)
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.remove(place, ctx)
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.fake.set_attributes(place, change, ctx)
    }
}

/// A folder in the fake with the quick view's samples, and a quick view
/// reading through it.
pub(super) struct Bench {
    pub log: Arc<Logged>,
    pub folder: Place,
    pub view: QuickView,
    pub t0: Instant,
}

impl Bench {
    pub fn new(options: QuickViewOptions) -> Self {
        let fake = FakeProvider::new();
        let folder = p("/quick");
        fake.add_folder(&folder);
        let log = Logged::new(fake);
        let mut registry = Registry::new();
        registry.set_local(log.clone());
        Self {
            view: QuickView::new(registry, options),
            log,
            folder,
            t0: Instant::now(),
        }
    }

    /// Adds a file and returns its place.
    pub fn file(&self, name: &str, bytes: &[u8]) -> Place {
        let place = self.folder.join(name).unwrap();
        self.log.fake.add_file(&place, bytes);
        place
    }

    pub fn entry(&self, place: &Place) -> Entry {
        self.log.fake.entry(place).unwrap()
    }

    pub fn at(&self, ms: u64) -> Instant {
        self.t0 + Duration::from_millis(ms)
    }

    /// Turns the quick view on with the cursor on a file and waits for
    /// its preview.
    pub fn on(&mut self, place: &Place) -> Vec<String> {
        let entry = self.entry(place);
        let at = self.at(0);
        let mut said = self.view.turn_on(Some((place, &entry)), at);
        said.extend(self.view.settle(at, Duration::from_secs(10)));
        texts(&said)
    }

    /// Moves the cursor at a time.
    pub fn move_to(&mut self, place: &Place, ms: u64) {
        let entry = self.entry(place);
        let at = self.at(ms);
        assert!(self.view.cursor_moved(place, &entry, at).is_empty());
    }

    /// Ticks at a time, then waits for the render it starts.
    pub fn settle(&mut self, ms: u64) -> Vec<String> {
        let at = self.at(ms);
        texts(&self.view.settle(at, Duration::from_secs(10)))
    }
}

pub(super) fn texts(said: &[Message]) -> Vec<String> {
    said.iter().map(|m| m.text.clone()).collect()
}

#[test]
fn turning_on_previews_the_file_under_the_cursor_at_once_in_one_line() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let quarter = bench.file("quarter.md", &fixture("quarter.md"));
    assert!(!bench.view.is_on() && bench.view.worker_threads() == 0);
    assert_eq!(
        bench.on(&quarter),
        ["Quick view on", "Preview: quarter.md, 12 headings"]
    );
    assert!(bench.view.is_on());
    let view = bench.view.view().unwrap();
    assert_eq!(view.outline().len(), 12);
    assert_eq!(bench.view.shown().unwrap().place, quarter);
    assert_eq!(bench.log.reads(), [quarter.to_string()]);
    assert_eq!(bench.view.worker_threads(), 1);
    assert_eq!(
        texts(&bench.view.toggle(None, bench.at(10))),
        ["Quick view off"]
    );
    assert!(bench.view.shown().is_none() && !bench.view.wants_tick());
}

#[test]
fn the_preview_follows_the_cursor_only_after_the_wait() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let a = bench.file("a.txt", b"first\n");
    let b = bench.file("b.txt", b"second\nfile\n");
    bench.on(&a);
    bench.move_to(&b, 100);
    assert!(bench.view.wants_tick());
    assert_eq!(
        bench.view.due_in(bench.at(300)),
        Some(Duration::from_millis(300))
    );
    assert!(bench.settle(599).is_empty(), "still waiting");
    assert_eq!(bench.settle(600), ["Preview: b.txt, 2 lines"]);
    // Back on what is shown: nothing to do.
    bench.move_to(&b, 700);
    assert!(bench.settle(2000).is_empty());
    assert_eq!(bench.log.reads().len(), 2);
}

/// Spike Q1, in real time: a 200 ms read delay, 100 cursor moves in three
/// seconds, at most two renders (the one turning on, the one where the
/// cursor stops), one worker thread, no reader left running, nothing kept
/// but the file shown.
#[test]
fn a_hundred_moves_in_three_seconds_render_at_most_twice() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let places: Vec<Place> = (0..100)
        .map(|n| {
            bench.file(
                &format!("note-{n:03}.md"),
                format!("# Note {n}\n\nText.\n").as_bytes(),
            )
        })
        .collect();
    bench.log.fake.switches_mut().latency = Latency {
        per_call: Duration::from_millis(200),
        ..Latency::default()
    };
    let first = &places[0];
    let entry = bench.entry(first);
    let started = Instant::now();
    bench.view.turn_on(Some((first, &entry)), Instant::now());
    let mut most_threads = 0;
    for place in &places[1..] {
        let entry = bench.entry(place);
        bench.view.cursor_moved(place, &entry, Instant::now());
        bench.view.tick(Instant::now());
        most_threads = most_threads.max(bench.view.worker_threads());
        std::thread::sleep(Duration::from_millis(30));
    }
    let moving = started.elapsed();
    let mut said = Vec::new();
    let until = Instant::now() + Duration::from_secs(5);
    while (bench.view.wants_tick() || said.is_empty()) && Instant::now() < until {
        said.extend(texts(&bench.view.tick(Instant::now())));
        std::thread::sleep(Duration::from_millis(20));
    }
    let reads = bench.log.reads();
    println!(
        "spike Q1: {} moves in {} ms, {} renders started, {} reads: {:?}, said {said:?}",
        places.len() - 1,
        moving.as_millis(),
        bench.view.renders(),
        reads.len(),
        reads
    );
    assert!(moving <= Duration::from_millis(3500), "{moving:?}");
    assert!(reads.len() <= 2, "{reads:?}");
    assert!(bench.view.renders() <= 2);
    assert_eq!(reads.last(), Some(&places[99].to_string()));
    assert_eq!(
        said.last().map(String::as_str),
        Some("Preview: note-099.md, 1 heading")
    );
    assert!(most_threads <= 1 && bench.view.worker_threads() == 1);
    assert_eq!(bench.view.readers_running(), 0);
    assert_eq!(bench.view.shown().unwrap().place, places[99]);
}

/// Spike Q1's second half: a Markdown embed naming a sibling and a
/// reStructuredText include cause no read but the previewed file's, even
/// with the siblings there in the fake.
#[test]
fn an_embed_and_an_include_read_nothing_but_the_previewed_file() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let embed = bench.file("embed.md", &fixture("traps/embed.md"));
    let include = bench.file("include.rst", &fixture("traps/include.rst"));
    bench.file("sibling-note.md", b"# The sibling\n");
    bench.file("sibling-part.rst", b"The sibling part.\n");
    // The note is shown on the status line and said on Say Status.
    assert_eq!(
        bench.on(&embed),
        [
            "Quick view on",
            "Preview: embed.md, 1 heading",
            "An embed was left out, so the note it names is not read."
        ]
    );
    bench.move_to(&include, 10);
    let said = bench.settle(1000);
    assert_eq!(said[0], "Preview: include.rst, reStructuredText, 1 heading");
    assert_eq!(
        said[1..],
        ["An include directive was left out, so the file it names is not read."]
    );
    assert_eq!(bench.log.reads(), [embed.to_string(), include.to_string()]);
    let calls = bench.log.calls();
    assert!(
        calls.iter().all(|(_, place)| !place.contains("sibling")),
        "{calls:?}"
    );
}

#[test]
fn a_file_changed_on_disk_renders_once_and_says_so() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let quarter = bench.file("notes.md", b"# One\n");
    bench.on(&quarter);
    bench.log.fake.add_file(&quarter, b"# One\n\n## Two\n");
    for ms in [100, 110, 120] {
        let at = bench.at(ms);
        assert!(
            bench
                .view
                .watch_event(&WatchEvent::Changed(quarter.clone()), at)
                .is_empty()
        );
    }
    // A change elsewhere in the folder is not this file's.
    let other = bench.folder.join("other.md").unwrap();
    bench
        .view
        .watch_event(&WatchEvent::Changed(other), bench.at(130));
    assert!(
        bench.settle(500).is_empty(),
        "the wait runs from the last change"
    );
    assert_eq!(bench.settle(620), ["Preview updated: notes.md changed"]);
    assert_eq!(bench.view.view().unwrap().outline().len(), 2);
    assert_eq!(bench.view.shown().unwrap().size, Some(14), "asked again");
    assert!(bench.settle(5000).is_empty(), "once");
    assert_eq!(bench.log.reads().len(), 2);
    assert_eq!(
        texts(
            &bench
                .view
                .watch_event(&WatchEvent::Removed(quarter.clone()), bench.at(6000))
        ),
        ["Preview: notes.md was removed"]
    );
    assert!(bench.view.shown().is_none());
}

#[test]
fn changes_are_not_followed_when_the_setting_is_off() {
    let mut options = QuickViewOptions::default();
    options.settings.follow_changes = false;
    let mut bench = Bench::new(options);
    let notes = bench.file("notes.md", b"# One\n");
    bench.on(&notes);
    bench
        .view
        .watch_event(&WatchEvent::Changed(notes.clone()), bench.at(10));
    assert!(bench.settle(2000).is_empty());
    assert_eq!(bench.log.reads().len(), 1);
}

#[test]
fn a_large_file_is_read_to_the_cap_and_says_so() {
    let mut options = QuickViewOptions::default();
    options.settings.max_kib = 64;
    let mut bench = Bench::new(options);
    let line = "2026-10-16 the crows came back at dusk\n";
    let big: String = line.repeat(200_000 / line.len());
    let log = bench.file("big.log", big.as_bytes());
    bench.log.fake.set_listed_size(&log, 2_000_000_000);
    assert_eq!(
        bench.on(&log),
        ["Quick view on", "Preview: big.log, first 64 KiB of 2.0 GB"]
    );
    let shown = bench.view.shown().unwrap();
    assert!(shown.capped);
    let view = bench.view.view().unwrap();
    assert!(
        view.rows().iter().all(|r| r.text == line.trim_end()),
        "no cut line"
    );
    assert!(view.len() * line.len() <= 64 * 1024);
}

#[test]
fn each_kind_of_file_says_its_one_line() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let main = bench.file("main.rs", &fixture("text/main.rs"));
    let notes = bench.file("notes.txt", &fixture("text/notes.txt"));
    let data = bench.file("a.bin", &fixture("text/data.bin"));
    let org = bench.file("notes.org", &fixture("carta/notes-1.org"));
    let mut photo_bytes = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13];
    photo_bytes.extend(std::iter::repeat_n(0u8, 64));
    let photo = bench.file("photo.png", &photo_bytes);
    bench.log.fake.set_listed_size(&photo, 2_000_000);
    let folder = bench.folder.join("src").unwrap();
    bench.log.fake.add_folder(&folder);
    assert_eq!(bench.on(&main)[1], "Preview: main.rs, Rust, 18 lines");
    let cases = [
        (&notes, "Preview: notes.txt, 1,204 lines"),
        (&data, "Preview: a.bin, binary, hex"),
        (&org, "Preview: notes.org, Org, 4 headings"),
        (&photo, "No preview: photo.png, PNG image, 2.0 MB"),
        (&folder, "No preview: src, folder"),
    ];
    for (n, (place, want)) in cases.into_iter().enumerate() {
        let ms = 1000 * (n as u64 + 1);
        bench.move_to(place, ms);
        assert_eq!(bench.settle(ms + 500), [want], "{place}");
        assert!(want.chars().count() <= 40, "{want}");
    }
    // The picture and the folder were never read.
    let reads = bench.log.reads();
    assert!(!reads.contains(&folder.to_string()));
}

#[test]
fn a_document_its_reader_cannot_read_shows_its_text() {
    let mut options = QuickViewOptions::default();
    options.settings.deadline_ms = 50;
    let mut bench = Bench::new(options);
    let mut text = String::new();
    for level in 0..1500 {
        text.push_str(&" ".repeat(level * 2));
        text.push_str(&format!("- level {level}\n"));
    }
    let big = bench.file("big.org", text.as_bytes());
    let said = bench.on(&big);
    assert_eq!(
        said,
        [
            "Quick view on",
            "Could not read big.org: took too long.",
            "Showing the text."
        ]
    );
    let view = bench.view.view().unwrap();
    assert_eq!(view.kind, DocKind::Text);
    assert_eq!(view.rows()[0].text, "- level 0");
}

#[test]
fn a_file_that_cannot_be_read_says_why() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let locked = bench.file("locked.md", b"# Locked\n");
    bench
        .log
        .fake
        .switches_mut()
        .call_faults
        .push(CallFault::new(
            FakeCall::Read,
            locked.clone(),
            VfsError::PermissionDenied,
        ));
    let said = bench.on(&locked);
    assert_eq!(said[0], "Quick view on");
    assert!(
        said[1].starts_with("Could not read locked.md: permission was denied."),
        "{said:?}"
    );
    assert!(matches!(
        bench.view.shown().unwrap().showing,
        Showing::Unreadable(_)
    ));
}

#[test]
fn nothing_follows_the_cursor_while_it_is_off() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let a = bench.file("a.txt", b"a\n");
    bench.move_to(&a, 0);
    assert!(bench.settle(5000).is_empty());
    assert!(bench.log.reads().is_empty() && bench.view.worker_threads() == 0);
    assert!(texts(&bench.view.turn_off()).is_empty(), "already off");
}

#[test]
fn code_gets_its_colors_when_the_options_say_so() {
    let mut bench = Bench::new(QuickViewOptions {
        highlighting: Highlighting::On,
        ..QuickViewOptions::default()
    });
    bench.view.set_highlighter(Session::new());
    let main = bench.file("main.rs", &fixture("text/main.rs"));
    bench.on(&main);
    let until = Instant::now() + Duration::from_secs(20);
    while bench.view.wants_tick() && Instant::now() < until {
        bench.view.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    let view = bench.view.view().unwrap();
    if diroxide_preview::highlight::is_built() {
        assert!(view.rows().iter().any(|r| !r.runs.is_empty()));
    }
    // A screen reader in automatic mode gets no colors.
    assert!(!QuickViewOptions::default().colors_code());
}

#[test]
fn the_options_come_from_the_settings() {
    let mut settings = Settings::default();
    settings.quick_view.wait_ms = 250;
    settings.quick_view.max_kib = 2048;
    let options = QuickViewOptions::from_settings(&settings, false);
    assert_eq!(options.wait(), Duration::from_millis(250));
    assert_eq!(options.max_bytes(), 2048 * 1024);
    assert_eq!(options.hex_row(), 16);
    assert_eq!(QuickViewOptions::default().hex_row(), 8);
    assert_eq!(cap_words(1024 * 1024), "1 MiB");
    assert_eq!(cap_words(64 * 1024), "64 KiB");
    let _ = EntryKind::File;
}
