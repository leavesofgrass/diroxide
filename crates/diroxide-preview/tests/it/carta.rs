//! The carta bridge (commit 51, spike C1): the fifteen samples through
//! the quick view's reader with their counts, the include trap, the
//! deadline, the stack and the catch.

use std::time::{Duration, Instant};

use diroxide_core::Message;
use diroxide_preview::doc::carta::{
    self as bridge, CartaFormat, INCLUDE_WARNING, without_includes,
};
use diroxide_preview::doc::{
    self, DocError, DocKind, DocRow, DocView, Guard, ReadOptions, Reader, RowKind, SHOWING_THE_TEXT,
};

use super::carta_paths::{Counts, SAMPLES, nested_list, sample_path};
use super::markdown::fixture;

fn counts(view: &DocView) -> Counts {
    Counts {
        headings: view.outline().len(),
        items: view
            .rows()
            .iter()
            .filter(|r| matches!(r.kind, RowKind::ListItem { .. }) && !r.continued)
            .count(),
        tables: view.tables().len(),
        links: view.links().len(),
    }
}

fn read(name: &str) -> DocView {
    let bytes = std::fs::read(sample_path(name)).unwrap();
    doc::read(name, bytes, &ReadOptions::default()).unwrap()
}

#[test]
fn every_sample_keeps_its_headings_items_tables_and_links() {
    let mut failed = Vec::new();
    for (name, from, want) in SAMPLES {
        let view = read(name);
        let format = CartaFormat::from_name(name).unwrap();
        assert_eq!(format.reader(), from, "{name}");
        assert_eq!(view.kind, DocKind::Carta(format.name().to_owned()));
        let got = counts(&view);
        if got != want {
            failed.push(format!("{name}: want {want:?}, got {got:?}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn an_org_file_previews_with_headings_and_a_table() {
    let view = read("notes-1.org");
    assert_eq!(
        view.preview_line("notes-1.org"),
        "Preview: notes-1.org, Org, 4 headings"
    );
    assert_eq!(view.title.as_deref(), Some("Field Notes"));
    assert_eq!(
        view.heading_words(1).as_deref(),
        Some("Heading 2: What they eat, 2 of 4")
    );
    let table = &view.tables()[0];
    assert_eq!(table.cells[0], ["Day", "Crows", "Jays"]);
    assert_eq!(table.cell_words(1, 1), "Crows: 12, row 2 of 3, column 2");
    assert_eq!(
        view.links()[0].words(0, 2),
        "Link 1 of 2: the crow page, to https://example.org/crows"
    );
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert!(shown.contains(&"1. Jimothy".to_owned()), "{shown:?}");
}

#[test]
fn the_formats_are_known_by_extension_and_typst_is_not_built() {
    assert_eq!(Reader::for_name("a.rest"), Reader::Carta(CartaFormat::Rst));
    assert_eq!(
        Reader::for_name("a.WIKI"),
        Reader::Carta(CartaFormat::MediaWiki)
    );
    assert_eq!(Reader::for_name("a.md"), Reader::Markdown);
    assert_eq!(Reader::for_name("a.typ"), Reader::Text);
    assert_eq!(Reader::for_name("a.txt"), Reader::Text);
    assert!(
        bridge::EXTENSIONS
            .iter()
            .all(|e| CartaFormat::from_name(&format!("x.{e}")).is_some())
    );
    assert!(
        carta::read_document("typst", b"= Title", &carta::ReaderOptions::default()).is_err(),
        "carta's Typst reader reads any file a document names: not in this build"
    );
}

/// The include trap names a sibling that is not there; the include is a
/// comment before carta sees it, and the view says so once.
#[test]
fn an_include_is_never_followed() {
    let path = fixture("traps/include.rst");
    let bytes = std::fs::read(&path).unwrap();
    let view = doc::read("include.rst", bytes, &ReadOptions::default()).unwrap();
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert_eq!(shown, ["Notes with includes", "Before.", "After."]);
    assert_eq!(view.notes(), [INCLUDE_WARNING]);
    assert_eq!(
        without_includes("No directives.\n"),
        "No directives.\n",
        "nothing to leave out, nothing copied"
    );
}

/// What the neutralizer is for: left alone, carta reads an included file
/// from the disk. A harmless sibling in a temporary folder shows it, and
/// the bridge leaves it unread.
#[test]
fn carta_alone_would_follow_an_include_and_the_bridge_does_not() {
    let folder = tempfile::tempdir().unwrap();
    let sibling = folder.path().join("sibling-part.rst");
    std::fs::write(&sibling, "The sibling says pelican.\n").unwrap();
    let text = format!("Before.\n\n.. include:: {}\n\nAfter.\n", sibling.display());
    let (doc, media) =
        carta::read_document("rst", text.as_bytes(), &carta::ReaderOptions::default()).unwrap();
    let raw = match carta::render_document("html", doc, media, &carta::WriterOptions::default()) {
        Ok(carta::Output::Text(html)) => html,
        other => panic!("{other:?}"),
    };
    assert!(raw.contains("pelican"), "carta followed it: {raw}");
    let view = doc::read("guide.rst", text.into_bytes(), &ReadOptions::default()).unwrap();
    let all: String = view.rows().iter().map(DocRow::shown).collect();
    assert!(!all.contains("pelican"), "{all}");
    assert_eq!(view.notes(), [INCLUDE_WARNING]);
}

/// A 5 MB Org file that holds carta for minutes (a list nested deep at
/// its top, spike C1) is cut off after two seconds.
#[test]
fn a_five_megabyte_org_file_is_cut_off_after_two_seconds() {
    let mut text = nested_list("org", 2000);
    let filler = std::fs::read_to_string(sample_path("notes-2.org")).unwrap();
    while text.len() < 5_000_000 {
        text.push_str(&filler);
    }
    let options = ReadOptions::default();
    assert_eq!(options.guard.deadline(), Duration::from_secs(2));
    let started = Instant::now();
    let error = doc::read("big.org", text.into_bytes(), &options).unwrap_err();
    let took = started.elapsed();
    assert_eq!(error, DocError::TookTooLong);
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(4),
        "{took:?}"
    );
    let said = Message::from_error("read", "big.org", &error);
    assert_eq!(said.text, "Could not read big.org: took too long.");
    assert_eq!(options.guard.running(), 1, "left to finish on its own");
}

/// A list nested 300 deep in each format reads on the guard's 8 MiB
/// stack, its deepest item kept.
#[test]
fn deeply_nested_lists_read_on_the_guards_stack() {
    let options = ReadOptions {
        guard: Guard::new(Duration::from_secs(30), 3),
        ..ReadOptions::default()
    };
    for (format, ext) in [
        ("org", "org"),
        ("rst", "rst"),
        ("mediawiki", "wiki"),
        ("dokuwiki", "dokuwiki"),
        ("jira", "jira"),
    ] {
        let text = nested_list(format, 300);
        let view = doc::read(&format!("deep.{ext}"), text.into_bytes(), &options)
            .unwrap_or_else(|e| panic!("{format}: {e}"));
        // Past 256 elements of nesting the rest is read as text in one
        // row, without its structure. carta 0.0.11's DokuWiki reader
        // keeps 32 levels of a list and drops the rest itself.
        let all: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
        let deepest = if format == "dokuwiki" {
            "level 31"
        } else {
            "level 299"
        };
        assert!(
            all.iter().any(|r| r.contains(deepest)),
            "{format}: {} rows",
            view.len()
        );
    }
    assert_eq!(options.guard.running(), 0);
}

#[test]
fn a_reader_that_panics_fails_one_preview_and_the_guard_stays_flat() {
    let guard = Guard::new(Duration::from_millis(500), 2);
    let failed: Result<(), DocError> = guard.run(|| panic!("a reader's bug"));
    assert_eq!(failed, Err(DocError::Failed));
    assert_eq!(guard.run(|| 7), Ok(7));
    // Two readers left past a short deadline fill the guard; a third is
    // refused at once, and the guard empties when they finish.
    let short = Guard::new(Duration::from_millis(20), 2);
    for _ in 0..2 {
        let slow = short.run(|| std::thread::sleep(Duration::from_millis(300)));
        assert_eq!(slow, Err(DocError::TookTooLong));
    }
    assert_eq!(short.run(|| 1), Err(DocError::Busy));
    let until = Instant::now() + Duration::from_secs(5);
    while short.running() > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(short.running(), 0);
    let patient = short.clone().with_deadline(Duration::from_secs(5));
    assert_eq!(patient.run(|| 2), Ok(2));
}

#[test]
fn a_file_carta_cannot_read_falls_back_to_its_text() {
    // A guard that is full refuses at once: the bytes are shown as text.
    let full = Guard::new(Duration::from_millis(10), 1);
    let _ = full.run(|| std::thread::sleep(Duration::from_millis(500)));
    let options = ReadOptions {
        guard: full,
        ..ReadOptions::default()
    };
    let (view, error) = doc::read_or_text("notes.org", b"* Heading\nText.\n".to_vec(), &options);
    assert_eq!(error, Some(DocError::Busy));
    assert_eq!(view.kind, DocKind::Text);
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert_eq!(shown, ["* Heading", "Text."]);
    let words = Message::from_error("read", "notes.org", &DocError::NotValid("Org".to_owned()));
    assert_eq!(words.text, "Could not read notes.org: not valid Org.");
    assert!(words.text.chars().count() <= 40);
    assert_eq!(SHOWING_THE_TEXT, "Showing the text.");
}

#[test]
fn bytes_that_are_not_text_are_hex_whatever_the_name() {
    let mut bytes = b"PK".to_vec();
    bytes.extend([3, 4, 0, 0, 0, 0]);
    let view = doc::read("notes.org", bytes, &ReadOptions::default()).unwrap();
    assert_eq!(view.kind, DocKind::Hex);
}

#[test]
fn decoded_first_since_carta_reads_utf8_only() {
    let view = doc::read(
        "cafe.org",
        b"* Caf\xe9\nThe caf\xe9 is open.\n".to_vec(),
        &ReadOptions::default(),
    )
    .unwrap();
    assert_eq!(view.outline()[0].name, "Caf\u{e9}");
}
