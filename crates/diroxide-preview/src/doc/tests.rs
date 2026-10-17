//! The document view, built by hand: what each move finds and says.

use super::*;

/// A report with two headings, a table of three rows, a list, a code
/// block and links, built the way a reader builds one.
fn report() -> DocView {
    let mut b = DocView::builder(DocKind::Document);
    b.heading(1);
    b.text("Report");
    b.paragraph();
    b.text("  Read   the\n");
    b.start_link("docs/index.md");
    b.text("docs");
    b.end_link();
    b.text(" first.  ");
    b.heading(2);
    b.text("Installation");
    b.list_item(1, Some("1.".to_owned()));
    b.text("Download it.");
    b.list_item(2, None);
    b.text("From the ");
    b.start_link("https://example.org");
    b.text("site");
    b.end_link();
    b.text(".");
    b.start_table();
    for (n, row) in [["Name", "Age"], ["Ada", "36"], ["Grace", ""]]
        .into_iter()
        .enumerate()
    {
        b.start_table_row(n == 0);
        for cell in row {
            b.start_cell();
            b.text(cell);
            b.end_cell();
        }
        b.end_table_row();
    }
    b.end_table();
    b.code_block(Some("rust"), "\nfn main() {\n    run();\n}\n\n");
    b.heading(2);
    b.text("Usage");
    b.paragraph();
    b.text("One line");
    b.line_break();
    b.text("and the next.");
    b.separator();
    b.paragraph();
    b.start_link("mailto:ada@example.org");
    b.end_link();
    b.finish()
}

#[test]
fn rows_say_their_meaning_first_with_whitespace_collapsed() {
    let view = report();
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert_eq!(
        shown,
        [
            "Report",
            "Read the docs first.",
            "Installation",
            "1. Download it.",
            "From the site.",
            "Name | Age",
            "Ada | 36",
            "Grace | ",
            "fn main() {",
            "    run();",
            "}",
            "Usage",
            "One line",
            "and the next.",
            "Separator",
            "mailto:ada@example.org",
        ]
    );
    assert_eq!(view.row_details(0), ["heading 1"]);
    assert_eq!(view.row_details(4), ["list item", "level 2"]);
    assert_eq!(view.row_details(5), ["header row"]);
    assert_eq!(view.row_details(8), ["code", "Rust"]);
    assert!(view.row_details(9).is_empty(), "only the block's first row");
    assert!(view.rows()[13].continued && view.row_details(13).is_empty());
}

#[test]
fn headings_are_the_outline_with_their_levels_and_rows() {
    let view = report();
    let names: Vec<(SymbolKind, &str, usize)> = view
        .outline()
        .iter()
        .map(|s| (s.kind, s.name.as_str(), s.line))
        .collect();
    assert_eq!(
        names,
        [
            (SymbolKind::Heading(1), "Report", 0),
            (SymbolKind::Heading(2), "Installation", 2),
            (SymbolKind::Heading(2), "Usage", 11),
        ]
    );
    assert_eq!(view.next_heading(0), Some(1));
    assert_eq!(view.next_heading(2), Some(2));
    assert_eq!(view.next_heading(11), None);
    assert_eq!(view.previous_heading(11), Some(1));
    assert_eq!(view.heading_at(7), Some(1));
    assert_eq!(
        view.heading_words(1).as_deref(),
        Some("Heading 2: Installation, 2 of 3")
    );
    // The outline rows read as the viewer's do.
    assert_eq!(view.outline()[1].row(), "heading 2, Installation, line 3");
}

#[test]
fn tables_are_row_ranges_with_a_header_and_cells_said_with_it() {
    let view = report();
    let table = &view.tables()[0];
    assert_eq!((table.rows.clone(), table.header), (5..8, Some(0)));
    assert_eq!((table.row_count(), table.column_count()), (3, 2));
    assert_eq!(view.next_table(0), Some(0));
    assert_eq!(view.next_table(5), None);
    assert_eq!(view.table_at(6), Some(0));
    assert_eq!(view.previous_table(6), None, "not the table it is in");
    assert_eq!(view.previous_table(12), Some(0));
    assert_eq!(
        view.table_words(0).as_deref(),
        Some("Table 1 of 1, 3 rows, 2 columns")
    );
    assert_eq!(table.cell_words(1, 1), "Age: 36, row 2 of 3, column 2");
    assert_eq!(table.cell_words(2, 1), "Age: blank, row 3 of 3, column 2");
    assert_eq!(table.cell_words(0, 0), "Name, header, row 1 of 3, column 1");
    assert_eq!(table.cell_range(1, 1), Some(6..8));
    assert_eq!(&view.rows()[6].text[6..8], "36");
}

#[test]
fn links_keep_their_targets_and_where_their_text_is() {
    let view = report();
    let links: Vec<(usize, &str, &str)> = view
        .links()
        .iter()
        .map(|l| (l.row, l.text.as_str(), l.target.as_str()))
        .collect();
    assert_eq!(
        links,
        [
            (1, "docs", "docs/index.md"),
            (4, "site", "https://example.org"),
            (15, "mailto:ada@example.org", "mailto:ada@example.org"),
        ]
    );
    let first = &view.links()[0];
    assert_eq!(&view.rows()[1].text[first.range.clone()], "docs");
    assert_eq!(first.words(0, 3), "Link 1 of 3: docs, to docs/index.md");
    assert_eq!(
        view.links()[2].words(2, 3),
        "Link 3 of 3: mailto:ada@example.org"
    );
    assert_eq!(view.links_on(4).collect::<Vec<_>>(), [1]);
}

#[test]
fn code_blocks_keep_their_lines_and_language() {
    let view = report();
    let code = &view.code_blocks()[0];
    assert_eq!(
        (code.rows.clone(), code.language.as_deref()),
        (8..11, Some("rust"))
    );
    assert_eq!(code.language_words().as_deref(), Some("Rust"));
    let shell = CodeBlock {
        rows: 0..1,
        language: Some("sh".to_owned()),
    };
    assert_eq!(shell.language_words().as_deref(), Some("Shell"));
    let other = CodeBlock {
        rows: 0..1,
        language: Some("haskell".to_owned()),
    };
    assert_eq!(other.language_words().as_deref(), Some("haskell"));
}

#[test]
fn blocks_are_what_a_paragraph_move_skips_by() {
    let view = report();
    // The two rows of the hard-broken paragraph are one block.
    assert_eq!(view.next_block(11), Some(12));
    assert_eq!(view.next_block(12), Some(14));
    assert_eq!(view.block_start(13), 12);
    // A table and a code block are one block each.
    assert_eq!(view.next_block(5), Some(8));
    assert_eq!(view.next_block(8), Some(11));
    assert_eq!(view.previous_block(12), Some(11));
    assert_eq!(view.previous_block(9), Some(5));
    assert_eq!(view.previous_block(0), None);
    assert_eq!(view.next_block(15), None);
}

#[test]
fn find_counts_matches_ignoring_case() {
    let view = report();
    let found = view.find("THE");
    let rows: Vec<usize> = found.iter().map(|m| m.row).collect();
    assert_eq!(rows, [1, 4, 13]);
    assert!(view.find("").is_empty());
    assert_eq!(view.find("zebra").len(), 0);
}

#[test]
fn empty_rows_and_doubled_separators_are_dropped() {
    let mut b = DocView::builder(DocKind::Document);
    b.separator();
    b.paragraph();
    b.text("   \n\t ");
    b.paragraph();
    b.text("Kept");
    b.separator();
    b.separator();
    b.heading(3);
    b.code_block(None, "\n\n   \n");
    b.start_table();
    b.end_table();
    let view = b.finish();
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert_eq!(shown, ["Kept", "Separator"]);
    assert!(view.outline().is_empty() && view.tables().is_empty());
    assert!(view.code_blocks().is_empty());
}

#[test]
fn list_markers_say_tasks_in_words_first() {
    let mut b = DocView::builder(DocKind::Document);
    b.list_item(1, None);
    b.set_item_marker("not checked");
    b.text("Buy milk");
    b.list_item(1, Some("2.".to_owned()));
    b.set_item_marker("checked");
    b.text("Walk the dog");
    let view = b.finish();
    let shown: Vec<String> = view.rows().iter().map(DocRow::shown).collect();
    assert_eq!(shown, ["Not checked: Buy milk", "2. Checked: Walk the dog"]);
}

#[test]
fn the_summary_says_headings_or_lines() {
    assert_eq!(report().summary_words(), "3 headings");
    let mut b = DocView::builder(DocKind::Text);
    for n in 0..1204 {
        b.line(RowKind::Line, &format!("line {n}"));
    }
    assert_eq!(b.finish().summary_words(), "1,204 lines");
    assert_eq!(DocView::default().summary_words(), "empty");
    assert_eq!(counted(1, "heading", "headings"), "1 heading");
    assert_eq!(counted(0, "link", "links"), "no links");
}

#[test]
fn a_view_travels_as_json() {
    let view = report();
    let json = serde_json::to_string(&view).unwrap();
    let back: DocView = serde_json::from_str(&json).unwrap();
    assert_eq!(back, view);
}
