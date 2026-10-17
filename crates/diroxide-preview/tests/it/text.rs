//! Plain text and code in the document view, on three samples
//! (`fixtures/docs/text/`): a Rust file, a long text file, and bytes.

use diroxide_preview::doc::text::{self, HEX_ROW, HEX_ROW_SCREEN_READER};
use diroxide_preview::doc::{DocKind, DocRow, RowKind};
use diroxide_preview::highlight::Language;

use super::markdown::fixture;

fn sample(name: &str) -> Vec<u8> {
    std::fs::read(fixture(&format!("text/{name}"))).unwrap()
}

#[test]
fn code_is_rows_with_its_language_and_tabs_as_spaces() {
    let view = text::read("main.rs", &sample("main.rs"), HEX_ROW_SCREEN_READER);
    assert_eq!(view.kind, DocKind::Code(Language::Rust));
    assert_eq!(view.len(), 18);
    assert_eq!(
        view.preview_line("main.rs"),
        "Preview: main.rs, Rust, 18 lines"
    );
    assert_eq!(view.rows()[6].text, "    format!(\"Hello, {name}!\")");
    assert!(view.rows().iter().all(|r| r.kind == RowKind::Line));
    assert_eq!(
        view.label(0, true),
        "1: //! A small program for the quick view's code sample."
    );
    assert_eq!(
        view.label(0, false),
        "//! A small program for the quick view's code sample."
    );
    assert_eq!(
        view.rows()[3].text,
        "",
        "a text file's empty lines are its own"
    );
}

#[test]
fn plain_text_is_a_row_a_line() {
    let view = text::read("notes.txt", &sample("notes.txt"), HEX_ROW_SCREEN_READER);
    assert_eq!(view.kind, DocKind::Text);
    assert_eq!(
        view.preview_line("notes.txt"),
        "Preview: notes.txt, 1,204 lines"
    );
    assert_eq!(
        view.label(1203, true),
        "1,204: Note 1204: the crows came back at dusk."
    );
    assert!(view.outline().is_empty() && view.links().is_empty());
}

#[test]
fn binary_is_refused_to_hex_eight_bytes_a_row_for_a_screen_reader() {
    let view = text::read("data.bin", &sample("data.bin"), HEX_ROW_SCREEN_READER);
    assert_eq!(view.kind, DocKind::Hex);
    assert_eq!(
        view.preview_line("data.bin"),
        "Preview: data.bin, binary, hex"
    );
    assert_eq!(view.len(), 32);
    let first = &view.rows()[0];
    assert_eq!(first.kind, RowKind::Hex);
    assert!(
        first.text.starts_with("000000: 00 01 02 03 04 05 06 07"),
        "{}",
        first.text
    );
    assert!(first.text.chars().count() <= 40, "{}", first.text);
    let wide = text::read("data.bin", &sample("data.bin"), HEX_ROW);
    assert_eq!(wide.len(), 16);
}

#[test]
fn line_ends_and_hidden_characters_are_shown_safely() {
    let view = text::read("log.txt", b"one\r\ntwo \x1b[31mred\r\n\tthree\n", 8);
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert_eq!(shown, ["one", "two ?[31mred", "    three"]);
    let latin = text::read("old.txt", b"caf\xe9\n", 8);
    assert_eq!(latin.rows()[0].text, "caf\u{e9}", "decoded first");
    assert!(text::read("empty.txt", b"", 8).is_empty());
    assert_eq!(
        text::read("empty.txt", b"", 8).preview_line("empty.txt"),
        "Preview: empty.txt, empty"
    );
}

#[cfg(feature = "syntax-set")]
#[test]
fn code_gets_its_runs_from_the_viewers_highlighter() {
    use diroxide_preview::classes::{TokenClass, runs_fit};
    use diroxide_preview::highlight::{Caps, Session};

    let view = text::read("main.rs", &sample("main.rs"), HEX_ROW_SCREEN_READER);
    let session = Session::new();
    let mut job = text::highlight(&view, &session, Caps::default(), 380)
        .unwrap()
        .unwrap();
    let done = job.wait().unwrap();
    let view = view.with_runs(done.runs);
    assert!(view.rows().iter().all(|r| runs_fit(&r.text, &r.runs)));
    let fn_line = &view.rows()[5];
    assert!(
        fn_line
            .runs
            .iter()
            .any(|r| r.class == TokenClass::FunctionName),
        "{fn_line:?}"
    );
    let plain = text::read("notes.txt", &sample("notes.txt"), 8);
    assert!(text::highlight(&plain, &session, Caps::default(), 1).is_none());
}
