//! The preview region driven through its methods (commit 53), on the
//! quarter's figures (`fixtures/docs/quarter.md`): twelve headings, two
//! tables, five links.

use std::time::Duration;

use diroxide_core::Move;

use super::tests::{Bench, fixture, texts};
use super::*;

fn quarter() -> Bench {
    let mut bench = Bench::new(QuickViewOptions::default());
    let place = bench.file("quarter.md", &fixture("quarter.md"));
    bench.on(&place);
    bench
}

#[test]
fn entering_says_the_title_then_the_row_with_its_place_last() {
    let mut bench = quarter();
    assert_eq!(bench.view.region_title(), "Preview, quarter.md");
    let said = texts(&bench.view.enter());
    assert_eq!(said[0], "Preview, quarter.md");
    let total = bench.view.region().len();
    assert_eq!(said[1], format!("Quarter, heading 1, 1 of {total}"));
    assert_eq!(
        texts(&bench.view.region_status()),
        [format!("Preview, quarter.md, 1 of {total}")]
    );
}

#[test]
fn headings_move_with_their_level_and_place_said() {
    let mut bench = quarter();
    assert_eq!(
        texts(&bench.view.next_heading()),
        ["Heading 2: Installation, 2 of 12"]
    );
    assert_eq!(
        texts(&bench.view.next_heading()),
        ["Heading 2: People, 3 of 12"]
    );
    assert_eq!(
        bench.view.region().focused().map(|r| r.label.as_str()),
        Some("People")
    );
    assert_eq!(
        texts(&bench.view.previous_heading()),
        ["Heading 2: Installation, 2 of 12"]
    );
    bench.view.region_move(Move::End);
    assert_eq!(texts(&bench.view.next_heading()), ["No heading below"]);
    assert_eq!(
        texts(&bench.view.previous_heading()),
        ["Heading 2: Appendix, 12 of 12"]
    );
}

#[test]
fn a_table_is_read_by_cell_with_its_header() {
    let mut bench = quarter();
    assert_eq!(
        texts(&bench.view.next_table()),
        ["Table 1 of 2, 5 rows, 3 columns"]
    );
    assert_eq!(
        texts(&bench.view.cell_down()),
        ["Name: Ada, row 2 of 5, column 1"]
    );
    assert_eq!(
        texts(&bench.view.cell_right()),
        ["Age: 36, row 2 of 5, column 2"]
    );
    assert_eq!(
        texts(&bench.view.cell_right()),
        ["City: Portland, row 2 of 5, column 3"]
    );
    assert_eq!(texts(&bench.view.cell_right()), ["End of the row"]);
    assert_eq!(
        texts(&bench.view.cell_up()),
        ["City, header, row 1 of 5, column 3"]
    );
    assert_eq!(texts(&bench.view.cell_up()), ["First row of the table"]);
    assert_eq!(
        texts(&bench.view.next_table()),
        ["Table 2 of 2, 3 rows, 2 columns"]
    );
    assert_eq!(texts(&bench.view.next_table()), ["No table below"]);
    bench.view.region_move(Move::Home);
    assert_eq!(texts(&bench.view.cell_right()), ["Not in a table"]);
}

#[test]
fn links_are_listed_and_followed_by_the_safe_opener_rule() {
    let mut bench = quarter();
    let links = bench.view.links();
    assert_eq!(links.title(), "Links: 5 in quarter.md");
    let rows: Vec<String> = links.rows().iter().map(|r| r.spoken()).collect();
    assert_eq!(rows[1], "docs, to docs/index.md");
    assert_eq!(
        texts(&bench.view.link_go("1")),
        ["Link 2 of 5: docs, to docs/index.md"]
    );
    // A link to a file is never opened.
    assert_eq!(
        texts(&bench.view.open_link("1")),
        ["Could not open docs/index.md: only web and mail links open from the preview."]
    );
    assert!(!bench.view.is_asking());
    // A web address is asked first, then opened by the app on yes.
    assert_eq!(
        texts(&bench.view.open_link("2")),
        ["Open https://example.org/portland in the browser? y or n"]
    );
    assert!(bench.view.is_asking());
    assert_eq!(
        bench.view.answer_link(true).request,
        Some(QuickViewRequest::OpenAddress(
            "https://example.org/portland".to_owned()
        ))
    );
    // Mail, asked, answered no.
    assert_eq!(
        texts(&bench.view.open_link("3")),
        ["Open mail to team@example.org? y or n"]
    );
    let no = bench.view.answer_link(false);
    assert_eq!(
        (texts(&no.said), no.request),
        (vec!["Not opened".to_owned()], None)
    );
    assert_eq!(
        bench.view.answer_link(true),
        Outcome::default(),
        "nothing asked"
    );
    assert_eq!(
        texts(&bench.view.next_link()),
        ["Link 3 of 5: Portland, to https://example.org/portland"]
    );
    assert_eq!(
        texts(&bench.view.previous_link()),
        ["Link 2 of 5: docs, to docs/index.md"]
    );
    assert_eq!(
        texts(&bench.view.previous_link()),
        ["Link 1 of 5: overview, to overview.md"]
    );
    assert_eq!(texts(&bench.view.previous_link()), ["No link above"]);
}

#[test]
fn the_opener_takes_web_and_mail_addresses_only() {
    for (target, want) in [
        ("https://example.org", "web"),
        ("HTTP://example.org/a?b=c", "web"),
        ("mailto:ada@example.org", "mail"),
        ("file:///home/owner/notes.md", "refused"),
        ("FILE:notes.md", "refused"),
        ("docs/index.md", "refused"),
        ("../secret.md", "refused"),
        ("javascript:alert(1)", "refused"),
        ("data:text/html,hello", "refused"),
        ("https://", "refused"),
        ("mailto:", "refused"),
        ("https://exa mple.org", "refused"),
        ("https://example.org/\u{202e}gpj.exe", "refused"),
        ("", "refused"),
    ] {
        let got = match LinkTarget::of(target) {
            LinkTarget::Web(_) => "web",
            LinkTarget::Mail(_) => "mail",
            LinkTarget::Refused(_) => "refused",
        };
        assert_eq!(got, want, "{target:?}");
    }
}

#[test]
fn find_says_the_count_and_wraps() {
    let mut bench = quarter();
    assert_eq!(
        texts(&bench.view.find("RAIN")),
        ["Match 1 of 2 in the preview"]
    );
    assert_eq!(
        bench.view.region().focused().map(|r| r.label.as_str()),
        Some("Rain")
    );
    assert_eq!(
        texts(&bench.view.next_match()),
        ["Match 2 of 2 in the preview"]
    );
    assert_eq!(
        texts(&bench.view.next_match()),
        ["Match 1 of 2 in the preview, wrapped"]
    );
    assert_eq!(
        texts(&bench.view.previous_match()),
        ["Match 2 of 2 in the preview, wrapped"]
    );
    assert_eq!(
        texts(&bench.view.find("zebra")),
        ["No match for zebra in the preview"]
    );
    assert_eq!(texts(&bench.view.next_match()), ["Nothing found yet"]);
}

#[test]
fn lines_pages_home_end_and_paragraphs_move_the_focus() {
    let mut bench = quarter();
    let total = bench.view.region().len();
    assert_eq!(
        texts(&bench.view.region_move(Move::Up)),
        ["Top of the preview"]
    );
    let down = texts(&bench.view.region_move(Move::Down));
    assert!(down[0].ends_with(&format!(", 2 of {total}")), "{down:?}");
    let end = texts(&bench.view.region_move(Move::End));
    assert_eq!(end, [format!("Nothing more., {total} of {total}")]);
    assert_eq!(
        texts(&bench.view.region_move(Move::Down)),
        ["Bottom of the preview"]
    );
    bench.view.region_move(Move::Home);
    bench.view.next_paragraph();
    assert_eq!(
        bench.view.region().focused().map(|r| r.label.as_str()),
        Some("The figures for the quarter. Read the overview and the docs first.")
    );
    assert_eq!(
        texts(&bench.view.previous_paragraph())[0],
        format!("Quarter, heading 1, 1 of {total}")
    );
    assert_eq!(
        texts(&bench.view.previous_paragraph()),
        ["Top of the preview"]
    );
}

#[test]
fn rows_wrap_at_the_width_each_part_after_the_first_said_continued() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let long = bench.file(
        "long.md",
        b"A paragraph long enough to wrap on a forty cell line of a Braille display.\n",
    );
    bench.on(&long);
    bench.view.set_width(40);
    let labels: Vec<&str> = bench
        .view
        .region()
        .rows()
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "A paragraph long enough to wrap on a",
            "continued: forty cell line of a Braille",
            "continued: display."
        ]
    );
    assert!(labels.iter().all(|l| l.chars().count() <= 40), "{labels:?}");
    bench.view.set_width(0);
    assert_eq!(bench.view.region().len(), 1);
}

#[test]
fn plain_text_lines_can_be_numbered() {
    let mut options = QuickViewOptions::default();
    options.settings.line_numbers = true;
    let mut bench = Bench::new(options);
    let notes = bench.file("notes.txt", b"first\nsecond\n");
    bench.on(&notes);
    let labels: Vec<&str> = bench
        .view
        .region()
        .rows()
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(labels, ["1: first", "2: second"]);
    assert_eq!(
        texts(&bench.view.find("second")),
        ["Match 1 of 1 in the preview"]
    );
}

#[test]
fn previous_and_next_file_show_at_once_and_move_the_panel() {
    let mut bench = Bench::new(QuickViewOptions::default());
    let a = bench.file("a.md", b"# A\n");
    let b = bench.file("b.md", b"# B\n\n## Two\n");
    let c = bench.file("c.md", b"# C\n");
    let folder = bench.folder.join("sub").unwrap();
    bench.log.fake.add_folder(&folder);
    bench.on(&a);
    let siblings = [&a, &folder, &b, &c]
        .into_iter()
        .map(|p| (p.clone(), bench.entry(p)))
        .collect();
    bench.view.set_siblings(siblings);
    let at = bench.at(10);
    let next = bench.view.next_file(at);
    assert_eq!(texts(&next.said)[0], "b.md, 2 of 3");
    assert_eq!(
        next.request,
        Some(QuickViewRequest::FocusEntry("b.md".to_owned()))
    );
    let said = texts(&bench.view.settle(at, Duration::from_secs(10)));
    assert_eq!(said, ["Preview: b.md, 2 headings"]);
    let at = bench.at(20);
    let back = bench.view.previous_file(at);
    assert_eq!(texts(&back.said)[0], "a.md, 1 of 3");
    bench.view.settle(at, Duration::from_secs(10));
    let none = bench.view.previous_file(bench.at(30));
    assert_eq!(
        (texts(&none.said), none.request),
        (vec!["No previous file".to_owned()], None)
    );
}

#[test]
fn a_file_changed_on_disk_keeps_the_focus_on_its_row() {
    let mut bench = quarter();
    bench.view.next_heading();
    bench.view.next_heading();
    let place = bench.view.shown().unwrap().place.clone();
    let mut changed = fixture("quarter.md");
    changed.extend_from_slice(b"\n## Later\n\nAdded.\n");
    bench.log.fake.add_file(&place, &changed);
    bench
        .view
        .watch_event(&WatchEvent::Changed(place.clone()), bench.at(100));
    assert_eq!(bench.settle(700), ["Preview updated: quarter.md changed"]);
    assert_eq!(
        bench.view.region().focused().map(|r| r.label.as_str()),
        Some("People")
    );
}

#[test]
fn no_document_says_why_on_entering() {
    let mut bench = Bench::new(QuickViewOptions::default());
    assert_eq!(texts(&bench.view.enter()), ["Preview", "Quick view off"]);
    let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    png.extend(std::iter::repeat_n(0u8, 32));
    let photo = bench.file("photo.png", &png);
    bench.on(&photo);
    assert_eq!(
        texts(&bench.view.enter()),
        [
            "Preview, photo.png",
            "No preview: photo.png, PNG image, 40 bytes"
        ]
    );
    assert!(bench.view.region().is_empty());
    assert_eq!(texts(&bench.view.next_heading()), ["Bottom of the preview"]);
}
