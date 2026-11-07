// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::doc::{DocRow, RowKind};

/// The `u` control word's backslash and name, built at run time so no
/// tool reads it as an escape in this source.
fn uni_word() -> String {
    format!("{}u", '\\')
}

/// An RTF Unicode character: `u` and its value.
fn uni(n: i32) -> String {
    format!("{}{n}", uni_word())
}

fn shown(view: &DocView) -> Vec<String> {
    view.rows().iter().map(DocRow::shown).collect()
}

fn rtf(text: &str) -> DocView {
    read(text.as_bytes()).unwrap()
}

const HEAD: &str = r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss\fcharset0 Arial;}{\f1\fcharset204 Times Cyr;}}{\stylesheet{\s0 Normal;}{\s1\sbasedon0 heading 1;}{\s2 Heading 2;}{\*\cs10 Default Paragraph Font;}}{\info{\title Handout}{\author Ada Example}{\operator Someone}}";

#[test]
fn headings_and_paragraphs_from_the_style_sheet() {
    let view = rtf(&format!(
        r"{HEAD}\pard\s1 Week one\par\pard\plain Some {{\b bold}} and {{\i italic}} text.\par\pard\s2 Part\par}}"
    ));
    assert_eq!(
        shown(&view),
        ["Week one", "Some bold and italic text.", "Part"]
    );
    assert_eq!(view.outline().len(), 2);
    assert_eq!(
        view.heading_words(1).as_deref(),
        Some("Heading 2: Part, 2 of 2")
    );
    assert_eq!(view.title.as_deref(), Some("Handout"));
    assert_eq!(view.kind, DocKind::Format("RTF".to_owned()));
}

#[test]
fn code_pages_unicode_and_their_fallbacks() {
    let view = rtf(&format!(
        r"{HEAD}\pard caf\'e9 {{\f1 \'cf\'f0\'e8\'e2\'e5\'f2}} {}? \uc2{} EU x\uc1{}?{}?\par}}",
        uni(8212),
        uni(8364),
        uni(-10179),
        uni(-8704)
    ));
    assert_eq!(
        shown(&view),
        ["caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{2014} \u{20ac} x\u{1f600}"]
    );
    let japanese = rtf(
        r"{\rtf1\ansi\ansicpg932\deff0{\fonttbl{\f0\fcharset128 MS Mincho;}}\pard \'93\'fa\'96\'7b\par}",
    );
    assert_eq!(shown(&japanese), ["\u{65e5}\u{672c}"]);
}

#[test]
fn destinations_are_skipped_and_symbols_read() {
    let view = rtf(&format!(
        r"{HEAD}{{\header Page header}}{{\*\unknowndest secret}}{{\colortbl;\red0\green0\blue0;}}\pard A\~B\emdash C \{{x\}} \\ \lquote q\rquote{{\v hidden}}\par}}"
    ));
    assert_eq!(shown(&view), ["A B\u{2014}C {x} \\ \u{2018}q\u{2019}"]);
}

#[test]
fn lists_take_the_labels_the_writer_rendered() {
    let view = rtf(&format!(
        r"{HEAD}\pard\ls1\ilvl0{{\listtext 1.\tab}}First\par{{\listtext 2.\tab}}Second\par\pard\ls1\ilvl1{{\listtext a.\tab}}Inner\par\pard\ls2{{\listtext \'b7\tab}}Bullet\par\pard After\par}}"
    ));
    assert_eq!(
        shown(&view),
        ["1. First", "2. Second", "a. Inner", "Bullet", "After"]
    );
    let depths: Vec<u8> = view
        .rows()
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::ListItem { depth, .. } => Some(depth),
            _ => None,
        })
        .collect();
    assert_eq!(depths, [1, 1, 2, 1]);
}

#[test]
fn tables_keep_their_rows_and_header() {
    let view = rtf(&format!(
        r"{HEAD}\pard Before\par\trowd\trhdr\cellx1000\cellx2000\pard\intbl Name\cell Mark\cell\row\trowd\cellx1000\cellx2000\pard\intbl Ada\cell A\par plus\cell\row\pard After\par}}"
    ));
    assert_eq!(
        shown(&view),
        ["Before", "Name | Mark", "Ada | A plus", "After"]
    );
    let table = &view.tables()[0];
    assert_eq!(table.header, Some(0));
    assert_eq!(table.cell_words(1, 1), "Mark: A plus, row 2 of 2, column 2");
    // A table whose writer marked no header row: its first row is.
    let plain = rtf(
        r"{\rtf1\trowd\cellx1000\cellx2000\intbl Bird\cell Seen\cell\row\trowd\intbl Crow\cell 12\cell\row}",
    );
    assert_eq!(plain.tables()[0].header, Some(0));
    assert_eq!(plain.tables()[0].row_count(), 2);
}

#[test]
fn footnotes_links_revisions_and_pictures() {
    let view = rtf(&format!(
        r#"{HEAD}\pard See{{\super\chftn}}{{\footnote\pard\plain{{\super\chftn}} The source.}} {{\field{{\*\fldinst {{HYPERLINK "https://example.org/a"}}}}{{\fldrslt {{\ul the page}}}}}}.\par}}"#
    ));
    assert_eq!(
        shown(&view),
        ["See[1] the page.", "Footnotes", "[1] The source."]
    );
    assert_eq!(
        view.links()[0].words(0, 1),
        "Link 1 of 1: the page, to https://example.org/a"
    );
    assert_eq!(
        hyperlink(r#"HYPERLINK \l "sec2""#).as_deref(),
        Some("#sec2")
    );
    assert_eq!(
        hyperlink("HYPERLINK https://x.org").as_deref(),
        Some("https://x.org")
    );
    assert_eq!(hyperlink("PAGE"), None);
    let revised = rtf(&format!(
        r"{HEAD}{{\*\revtbl {{Unknown;}}{{Ada Example;}}}}\pard The {{\deleted\revauthdel1 old }}{{\revised\revauth1 new }}plan.\par}}"
    ));
    assert_eq!(shown(&revised), ["The new plan."]);
    let picture = rtf(&format!(
        r"{HEAD}\pard A {{\*\shppict{{\pict{{\*\picprop{{\sp{{\sn wzDescription}}{{\sv A crow on a wire}}}}}}\pngblip 89504e47}}}}{{\nonshppict{{\pict\wmetafile8 0102}}}} B\par}}"
    ));
    assert_eq!(shown(&picture), ["A A crow on a wire B"]);
}

#[test]
fn hostile_input_stays_bounded() {
    let deep = format!(
        r"{{\rtf1 {}deep{} {}c99999999{} xyzabcdefghijk\bin999999999999 tail",
        "{".repeat(100_000),
        "}".repeat(100_000),
        uni_word(),
        uni(8364)
    );
    let view = rtf(&deep);
    assert!(shown(&view)[0].starts_with("deep"), "{:?}", shown(&view));
    assert_eq!(view.notes(), [NESTING_NOTE]);
    let word = format!(r"{{\rtf1 \{}5 text}}", "a".repeat(100_000));
    assert_eq!(shown(&rtf(&word)), ["text"]);
    let hidden = rtf(&format!(r"{{\rtf1 evil{}?name\par}}", uni(8238)));
    assert_eq!(shown(&hidden), ["evil?name"]);
    assert_eq!(
        read(b"not rtf").unwrap_err(),
        DocError::NotValid("RTF".to_owned())
    );
    assert!(is_rtf(b"\xef\xbb\xbf  {\\rtf1}"));
}
