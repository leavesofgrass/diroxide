//! The viewer driven through its methods, on the samples in
//! `fixtures/viewer/` and on bytes made here; spike V1 behind
//! `DIRO_SPIKE_LARGE=1`.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;

use diroxide_core::Place;
use diroxide_core::format::within_cells;
use diroxide_vfs::Registry;

use super::*;

pub(super) fn fixture(rel: &str) -> Place {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/viewer")
        .join(rel);
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let text = path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    Place::parse(&text).unwrap()
}

pub(super) fn open(rel: &str) -> (Viewer, Vec<String>) {
    let (viewer, said) = Viewer::open(
        &Registry::with_local(),
        &fixture(rel),
        None,
        ViewerOptions::default(),
    )
    .unwrap();
    (viewer, texts(&said))
}

pub(super) fn texts(said: &[Message]) -> Vec<String> {
    said.iter().map(|m| m.text.clone()).collect()
}

pub(super) fn memory(bytes: &[u8]) -> Viewer {
    let place = Place::parse(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .join("made.txt")
        .unwrap();
    Viewer::from_source(
        place,
        ViewerSource::bytes(bytes.to_vec()),
        ViewerOptions::default(),
    )
    .unwrap()
    .0
}

fn numbered(lines: u64) -> Vec<u8> {
    (1..=lines)
        .flat_map(|i| format!("line {i}\n").into_bytes())
        .collect()
}

#[test]
fn a_text_file_opens_with_its_title_and_its_first_screen() {
    let (mut viewer, said) = open("text/notes.txt");
    assert_eq!(said.len(), 1, "plain UTF-8 says only the title: {said:?}");
    assert!(said[0].starts_with("notes.txt, in "), "{said:?}");
    assert_eq!(viewer.title(), said[0]);
    viewer.set_size(80, 5);
    let rows = viewer.rows();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].text, "Notes for the week, kept by Ada Example.");
    assert!(rows[0].focused && !rows[1].focused);
    assert_eq!(viewer.cursor_row(), 0);
    assert_eq!(viewer.status(), "Line 1 of 14, 7 percent");
    assert_eq!(viewer.line_count(), Some(14));
}

#[test]
fn moving_scrolls_only_as_far_as_the_cursor_needs() {
    let (mut viewer, _) = open("text/notes.txt");
    viewer.set_size(80, 3);
    for _ in 0..4 {
        assert!(
            viewer.down().is_empty(),
            "a move is read by the screen reader"
        );
    }
    assert_eq!(viewer.cursor().line, 4);
    let rows = viewer.rows();
    assert_eq!(rows[0].pos.line, 2);
    assert!(rows[2].focused);
    assert_eq!(viewer.cursor_row(), 2);
    viewer.page_down();
    viewer.page_down();
    viewer.page_down();
    viewer.page_down();
    viewer.page_down();
    viewer.page_down();
    assert_eq!(viewer.cursor().line, 13, "stops at the last line");
    viewer.down();
    assert_eq!(viewer.cursor().line, 13);
    assert_eq!(
        viewer.rows().last().unwrap().text,
        "The last line has no line end after it."
    );
    viewer.home();
    assert_eq!(viewer.cursor().line, 0);
    assert_eq!(viewer.rows()[0].pos.line, 0);
    viewer.page_up();
    assert_eq!(viewer.cursor().line, 0);
    let said = texts(&viewer.end());
    assert_eq!(said, ["Line 14 of 14, the last"]);
}

#[test]
fn tabs_and_hidden_characters_never_reach_the_screen() {
    let (mut viewer, _) = open("text/notes.txt");
    viewer.set_size(80, 20);
    let rows = viewer.rows();
    let tabs = rows.iter().find(|r| r.pos.line == 12).unwrap();
    assert_eq!(tabs.text, "Tabs    are turned  into    spaces.");
    let mut viewer = memory(b"bad\x1b[2Jname\nnext\x07\n");
    let rows = viewer.rows();
    assert_eq!(rows[0].text, "bad?[2Jname");
    assert_eq!(rows[1].text, "next?");
}

#[test]
fn utf16_text_reads_as_text_and_says_its_encoding() {
    let (mut viewer, said) = open("encoding/utf16le-bom.txt");
    assert_eq!(said[1], "Encoding: UTF-16 (byte-order mark)");
    assert!(
        said[1..].iter().all(|s| s.chars().count() <= 40),
        "{said:?}"
    );
    let rows = viewer.rows();
    assert_eq!(rows[0].text, "Caf\u{e9} notes.");
    assert_eq!(rows[1].text, "Second line, na\u{ef}ve.");
    assert_eq!(viewer.line_count(), Some(3));
    let (mut viewer, said) = open("encoding/windows-1252.txt");
    assert_eq!(said[1], "Encoding: Windows-1252 (not UTF-8)");
    assert_eq!(viewer.rows()[0].text, "Quote: \u{201c}caf\u{e9}\u{201d}.");
}

#[test]
fn a_binary_file_says_why_it_opens_in_hex() {
    let (viewer, said) = open("encoding/pixel.png");
    assert_eq!(said[1], "pixel.png looks binary; hex mode");
    assert!(within_cells(&said[1], "hex mode"));
    assert_eq!(viewer.mode(), ViewerMode::Hex);
}

#[test]
fn a_stream_over_its_cap_says_the_policy_and_reads_more() {
    let bytes = numbered(300_000);
    let total = bytes.len() as u64;
    let place = Place::parse(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .join("big.log")
        .unwrap();
    let mib = 1024 * 1024;
    let source = ViewerSource::stream(Box::new(Cursor::new(bytes)), Some(total), mib);
    let options = ViewerOptions {
        settings: ViewerSettings {
            member_cache_mib: 1,
            ..ViewerSettings::default()
        },
        ..ViewerOptions::default()
    };
    let (mut viewer, said) = Viewer::from_source(place, source, options).unwrap();
    let said = texts(&said);
    assert_eq!(said[1], "Showing the first 1.0 MB of 3.5 MB.");
    assert!(said[1].chars().count() <= 40);
    assert_eq!(viewer.rows()[0].text, "line 1");
    viewer.finish();
    viewer.end();
    viewer.finish();
    let known = viewer.lines_known();
    assert!(known < 300_000, "only the cap is read: {known}");
    let said = texts(&viewer.read_more());
    assert_eq!(said, ["Showing the first 2.1 MB of 3.5 MB."]);
    viewer.read_more();
    assert_eq!(texts(&viewer.read_more()), ["Showing all of big.log."]);
    while viewer.wants_work() {
        viewer.work(WORK_BUDGET);
    }
    assert_eq!(viewer.line_count(), Some(300_000));
}

#[test]
fn the_end_of_a_large_file_is_found_in_slices() {
    let bytes = numbered(2_000_000);
    let mut viewer = memory(&bytes);
    assert_eq!(viewer.line_count(), None, "too large to index at open");
    assert!(viewer.status().starts_with("Line 1, 0 percent"));
    let said = texts(&viewer.end());
    assert_eq!(
        said,
        ["Finding the last line"],
        "the first slice does not reach it"
    );
    assert!(viewer.is_busy());
    let said = texts(&viewer.finish());
    assert_eq!(said, ["Line 2,000,000 of 2,000,000, the last"]);
    assert_eq!(viewer.rows()[0].pos.line + 24, 2_000_000);
    assert_eq!(viewer.status(), "Line 2,000,000 of 2,000,000, 100 percent");
    viewer.end();
    assert!(texts(&viewer.cancel()).is_empty(), "nothing pending");
}

#[test]
fn an_empty_file_says_so() {
    let mut viewer = memory(b"");
    assert_eq!(viewer.status(), "Empty file");
    assert!(viewer.rows().len() <= 1);
}

/// Spike V1 (research/alpha2-plan.md 4.12): a 1 GiB text file; the time
/// to the first screen, to line 5,000,000 and to the end, and the memory
/// of the line index. Run with `DIRO_SPIKE_LARGE=1` and
/// `--no-capture`; the file is made in the temp folder and removed by
/// the test.
#[test]
fn spike_v1_a_gibibyte_log() {
    if std::env::var_os("DIRO_SPIKE_LARGE").is_none() {
        return;
    }
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    let made = Instant::now();
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
        let mut written = 0u64;
        let mut n = 0u64;
        while written < 1 << 30 {
            n += 1;
            let line = format!("2026-10-16 21:00:00 INFO request {n} served in 12 ms\n");
            out.write_all(line.as_bytes()).unwrap();
            written += line.len() as u64;
        }
        out.flush().unwrap();
        eprintln!(
            "V1: wrote {written} bytes, {n} lines in {:?}",
            made.elapsed()
        );
    }
    let place = Place::parse(&path.to_string_lossy()).unwrap();
    let started = Instant::now();
    let (mut viewer, _) = Viewer::open(
        &Registry::with_local(),
        &place,
        None,
        ViewerOptions::default(),
    )
    .unwrap();
    viewer.set_size(80, 24);
    let rows = viewer.rows();
    let first = started.elapsed();
    assert!(rows[0].text.contains("request 1 served"));
    let jump = Instant::now();
    viewer.start_task(Task::Line {
        line: 4_999_999,
        landing: Landing::Quiet,
    });
    viewer.finish();
    let rows = viewer.rows();
    let to_line = jump.elapsed();
    assert!(
        rows.iter()
            .any(|r| r.text.contains("request 5000000 served")),
        "{:?}",
        rows.first()
    );
    let end = Instant::now();
    viewer.end();
    viewer.finish();
    let to_end = end.elapsed();
    eprintln!(
        "V1: first screen {first:?}; line 5,000,000 {to_line:?}; the end {to_end:?}; {} lines; index {} bytes",
        viewer.line_count().unwrap_or_default(),
        viewer.index_memory()
    );
    drop(viewer);
    dir.close().unwrap();
}

#[test]
fn wrapped_rows_say_continued_first_and_fit_a_braille_line() {
    let (mut viewer, _) = open("text/notes.txt");
    viewer.set_size(80, 40);
    viewer.set_wrap_width(40);
    let rows = viewer.rows();
    let long: Vec<&ViewerRow> = rows.iter().filter(|r| r.pos.line == 9).collect();
    assert!(long.len() > 3, "{long:?}");
    assert!(long[0].text.starts_with("A long line follows"));
    for row in &rows {
        assert!(row.text.chars().count() <= 40, "{:?}", row.text);
    }
    for row in &long[1..] {
        assert!(row.text.starts_with(CONTINUED), "{:?}", row.text);
        let first = row.text[row.prefix..].chars().next().unwrap();
        assert!(
            first.is_alphanumeric(),
            "a word starts the row: {:?}",
            row.text
        );
    }
    let whole: String = long.iter().map(|r| &r.text[r.prefix..]).collect();
    assert_eq!(Some(whole), viewer.line_text(9));
    // The cursor walks the parts, and the status stays on the line.
    viewer.jump("10");
    viewer.down();
    assert_eq!(viewer.cursor(), RowPos { line: 9, part: 1 });
    assert_eq!(viewer.status(), "Line 10 of 14, 71 percent");
    viewer.up();
    viewer.up();
    assert_eq!(viewer.cursor(), RowPos { line: 8, part: 0 });
    assert_eq!(texts(&viewer.toggle_wrap()), ["Wrap off"]);
    assert_eq!(viewer.rows().iter().filter(|r| r.pos.line == 9).count(), 1);
    assert_eq!(
        texts(&viewer.set_wrap_width(0)),
        ["Wrap at the screen's width"]
    );
}

#[test]
fn line_numbers_start_each_line_and_not_its_continued_rows() {
    let (mut viewer, _) = open("text/notes.txt");
    viewer.set_size(40, 30);
    assert_eq!(texts(&viewer.toggle_line_numbers()), ["Line numbers on"]);
    let rows = viewer.rows();
    assert_eq!(rows[0].text, "1: Notes for the week, kept by Ada ");
    assert_eq!(rows[0].prefix, 3);
    assert!(rows[1].text.starts_with("continued: Example."));
    let tenth = rows.iter().find(|r| r.pos.line == 9).unwrap();
    assert!(tenth.text.starts_with("10: A long line"));
    assert_eq!(texts(&viewer.toggle_line_numbers()), ["Line numbers off"]);
}

#[test]
fn a_jump_says_where_it_landed() {
    let (mut viewer, _) = open("text/notes.txt");
    viewer.set_size(80, 5);
    assert_eq!(texts(&viewer.jump("5")), ["At line 5 of 14"]);
    assert_eq!(viewer.cursor().line, 4);
    assert_eq!(viewer.rows().iter().position(|r| r.focused), Some(4));
    assert_eq!(texts(&viewer.jump("50%")), ["At line 7 of 14, 50 percent"]);
    assert_eq!(
        texts(&viewer.jump("line 999")),
        ["Past the end: at line 14, the last"]
    );
    assert_eq!(texts(&viewer.jump("@41")), ["At line 2, offset 41"]);
    assert_eq!(texts(&viewer.jump("0x0")), ["At line 1, offset 0"]);
    assert_eq!(
        texts(&viewer.jump("offset 99999")),
        ["Past the end: at line 14, the last"]
    );
    assert_eq!(texts(&viewer.jump("top")), ["At line 1 of 14"]);
    assert_eq!(texts(&viewer.jump("end")), ["Line 14 of 14, the last"]);
    let said = viewer.jump("seven");
    assert_eq!(texts(&said), ["Not a line, offset or percent: seven"]);
    assert_eq!(said[0].kind, diroxide_core::MessageKind::Warning);
}

#[test]
fn a_far_jump_in_a_large_file_lands_after_the_scan() {
    let mut viewer = memory(&numbered(2_000_000));
    assert_eq!(texts(&viewer.jump("1,500,000")), ["Finding line 1,500,000"]);
    assert!(viewer.is_busy());
    let said = texts(&viewer.finish());
    assert_eq!(said, ["At line 1,500,000"], "the total is not known yet");
    assert_eq!(
        viewer.rows().iter().find(|r| r.focused).unwrap().text,
        "line 1500000"
    );
    assert_eq!(texts(&viewer.jump("50%")).len(), 1);
}

#[test]
fn reload_reads_the_file_again_and_keeps_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("growing.log");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let place = Place::parse(&path.to_string_lossy()).unwrap();
    let registry = Registry::with_local();
    let (mut viewer, _) = Viewer::open(&registry, &place, None, ViewerOptions::default()).unwrap();
    viewer.jump("2");
    std::fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();
    assert_eq!(
        texts(&viewer.reload(&registry)),
        ["Reloaded growing.log, 4 lines"]
    );
    assert_eq!(viewer.cursor().line, 1);
    assert_eq!(viewer.rows()[3].text, "four");
    std::fs::write(&path, "only\n").unwrap();
    viewer.reload(&registry);
    assert_eq!(
        viewer.cursor().line,
        0,
        "the line is gone: the last there is"
    );
}

#[test]
fn follow_says_how_many_lines_arrived() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.log");
    std::fs::write(&path, "start\n").unwrap();
    let place = Place::parse(&path.to_string_lossy()).unwrap();
    let registry = Registry::with_local();
    let (mut viewer, _) = Viewer::open(&registry, &place, None, ViewerOptions::default()).unwrap();
    assert_eq!(texts(&viewer.toggle_follow()), ["Follow on"]);
    assert!(viewer.follow_tick().is_empty(), "nothing new");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"a\nb\n").unwrap();
    drop(file);
    assert_eq!(texts(&viewer.follow_tick()), ["2 new lines"]);
    assert_eq!(viewer.cursor().line, 2);
    assert_eq!(texts(&viewer.toggle_follow()), ["Follow off"]);
    let mut stream = memory(b"x\n");
    assert_eq!(
        texts(&stream.toggle_follow()),
        ["Follow works on files on disk only"]
    );
}

#[test]
fn search_counts_every_match_and_wraps_around() {
    let (mut viewer, _) = open("text/notes.txt");
    viewer.set_size(80, 5);
    assert_eq!(
        texts(&viewer.search("hello", false)),
        ["Match 1 of 5, line 11"]
    );
    assert_eq!(viewer.cursor().line, 10);
    assert_eq!(texts(&viewer.next_match()), ["Match 2 of 5, line 11"]);
    viewer.next_match();
    viewer.next_match();
    assert_eq!(texts(&viewer.next_match()), ["Match 5 of 5, line 12"]);
    assert_eq!(
        texts(&viewer.next_match()),
        ["Search wrapped to the top, match 1 of 5"]
    );
    assert_eq!(
        texts(&viewer.previous_match()),
        ["Search wrapped to the bottom, match 5 of 5"]
    );
    let row = viewer.rows().into_iter().find(|r| r.focused).unwrap();
    assert_eq!(viewer.match_ranges(&row.text).len(), 3);
    assert_eq!(
        texts(&viewer.search("zebra", false)),
        ["No match for zebra."]
    );
    assert_eq!(
        texts(&viewer.search("Hello", false)),
        ["No match for Hello."]
    );
    viewer.jump("end");
    assert_eq!(
        texts(&viewer.search("hello", true)),
        ["Match 5 of 5, line 12"],
        "backward from the end"
    );
    viewer.home();
    assert_eq!(
        texts(&viewer.search("week|Sunday", false)),
        ["Match 1 of 2, line 1"]
    );
    let said = viewer.search("(", false);
    assert_eq!(texts(&said), ["Pattern needs care: bad expression"]);
    let mut fresh = memory(b"x\n");
    assert_eq!(texts(&fresh.next_match()), ["No search yet"]);
}

#[test]
fn a_search_of_a_large_file_says_it_is_searching_then_the_count() {
    let mut viewer = memory(&numbered(2_000_000));
    let said = texts(&viewer.search("^line 1999999$", false));
    assert_eq!(said, ["Searching for ^line 1999999$"]);
    assert!(viewer.is_busy());
    let said = texts(&viewer.finish());
    assert_eq!(said, ["Match 1 of 1, line 1,999,999"]);
    assert_eq!(
        viewer.rows().iter().find(|r| r.focused).unwrap().text,
        "line 1999999"
    );
}
