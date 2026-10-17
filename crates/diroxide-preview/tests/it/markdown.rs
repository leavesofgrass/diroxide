//! The Markdown reader on textweaver's cases (`fixtures/docs/markdown/`,
//! the inputs of textweaver's own Markdown tests): the same headings and
//! tables, and the same words, in rows.

use std::path::PathBuf;

use diroxide_preview::doc::markdown::{self, EMBED_NOTE};
use diroxide_preview::doc::{DocRow, DocView, RowKind};
use diroxide_preview::highlight::SymbolKind;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("docs")
        .join(name)
}

fn read(name: &str) -> DocView {
    let path = fixture(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    markdown::read(&text)
}

fn shown(view: &DocView) -> Vec<String> {
    view.rows().iter().map(DocRow::shown).collect()
}

fn headings(view: &DocView) -> Vec<(u8, String)> {
    view.outline()
        .iter()
        .map(|s| match s.kind {
            SymbolKind::Heading(level) => (level, s.name.clone()),
            other => panic!("not a heading: {other:?}"),
        })
        .collect()
}

#[test]
fn headings_lists_and_paragraphs() {
    let view = read("markdown/headings-lists.md");
    assert_eq!(
        shown(&view),
        [
            "Title",
            "Some text and bold.",
            "Lists",
            "one",
            "two",
            "nested",
            "1. first",
            "2. second"
        ]
    );
    assert_eq!(
        headings(&view),
        [(1, "Title".to_owned()), (2, "Lists".to_owned())]
    );
    assert_eq!(view.title.as_deref(), Some("Title"));
    let depths: Vec<u8> = view
        .rows()
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::ListItem { depth, .. } => Some(depth),
            _ => None,
        })
        .collect();
    assert_eq!(depths, [1, 1, 2, 1, 1]);
}

#[test]
fn front_matter_is_metadata_not_text() {
    let yaml = read("markdown/front-matter-yaml.md");
    assert_eq!(shown(&yaml), ["Body."]);
    assert_eq!(yaml.title.as_deref(), Some("My Doc"));
    let toml = read("markdown/front-matter-toml.md");
    assert_eq!(shown(&toml), ["Corps."]);
    assert_eq!(toml.title.as_deref(), Some("T"));
}

#[test]
fn tables_are_rows_of_cells_with_a_header() {
    let view = read("markdown/tables.md");
    assert_eq!(shown(&view), ["A | B | C", "1 |  | 3"]);
    let table = &view.tables()[0];
    assert_eq!(table.cells, [["A", "B", "C"], ["1", "", "3"]]);
    assert_eq!(table.header, Some(0));
    assert_eq!(
        view.rows()[0].kind,
        RowKind::TableRow {
            table: 0,
            row: 0,
            header: true
        }
    );
}

#[test]
fn code_blocks_keep_their_lines_and_language() {
    let view = read("markdown/code.md");
    assert_eq!(
        shown(&view),
        ["Before.", "fn main() {", "    x();", "}", "After code."]
    );
    assert_eq!(view.code_blocks()[0].language.as_deref(), Some("rust"));
    assert_eq!(view.row_details(1), ["code", "Rust"]);
}

#[test]
fn footnotes_come_after_the_text_under_a_heading() {
    let view = read("markdown/footnotes.md");
    assert_eq!(
        shown(&view),
        ["Text.[1] More.", "Footnotes", "[1] The note."]
    );
    assert_eq!(headings(&view), [(2, "Footnotes".to_owned())]);
}

#[test]
fn alerts_wiki_links_and_heading_attributes() {
    let view = read("markdown/alerts.md");
    assert_eq!(
        shown(&view),
        [
            "Title",
            "Note: Remember to save.",
            "Warning: Hot.",
            "See Other page and that part."
        ]
    );
    let labels: Vec<Option<String>> = view
        .rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Quote { label } => Some(label.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, [Some("note".into()), Some("warning".into())]);
    let targets: Vec<&str> = view.links().iter().map(|l| l.target.as_str()).collect();
    assert_eq!(targets, ["Other page", "Page#Part"]);
    assert_eq!(view.links()[1].text, "that part");
    assert_eq!(view.title.as_deref(), Some("Title"));
}

#[test]
fn callouts_of_any_type_say_their_type_first() {
    let view = read("markdown/callouts.md");
    assert_eq!(
        shown(&view),
        [
            "Tip: Remember",
            "Drink water.",
            "Warning, collapsed: Hot stove",
            "Careful.",
            "My box: Custom.",
            "FAQ, expanded",
            "Important: GitHub kind."
        ]
    );
    let plain = markdown::read("> [not a callout]\n> text\n");
    assert_eq!(shown(&plain), ["[not a callout] text"]);
    assert_eq!(plain.row_details(0), ["quote"]);
}

#[test]
fn raw_html_keeps_its_text_and_drops_its_tags() {
    let view = read("markdown/html.md");
    let all = shown(&view).join("\n");
    for kept in [
        "Welcome to the project & friends",
        "More \u{e9}tails",
        "Hidden text.",
        "Inline The logo and Ctrl.",
        "End.",
    ] {
        assert!(all.contains(kept), "{kept:?} missing from {all:?}");
    }
    for dropped in ["comment", "across", "var x", "color", "<", "align"] {
        assert!(!all.contains(dropped), "{dropped:?} in {all:?}");
    }
}

#[test]
fn math_keeps_its_delimiters() {
    let view = read("markdown/math.md");
    assert_eq!(
        shown(&view),
        [
            "The area is $\\pi r^2$, not $5 and $10.",
            "$$\\frac{a}{b}$$",
            "A $x_1 * y_2$ product."
        ]
    );
}

#[test]
fn task_items_say_their_state_first() {
    let view = read("markdown/tasks.md");
    assert_eq!(
        shown(&view),
        [
            "Not checked: Buy milk",
            "Checked: Walk the dog",
            "plain",
            "1. Checked: Numbered done"
        ]
    );
}

#[test]
fn links_images_and_breaks() {
    let view = read("markdown/links.md");
    assert_eq!(
        shown(&view),
        ["See the site. Next line", "hard.", "Alt text", "A", "B"]
    );
    let link = &view.links()[0];
    assert_eq!(
        (link.text.as_str(), link.target.as_str()),
        ("the site", "https://example.org")
    );
    assert!(view.rows()[1].continued, "a hard break continues the block");
}

#[test]
fn rules_are_separators_between_blocks() {
    let view = read("markdown/rules.md");
    assert_eq!(
        shown(&view),
        ["Keep drop this here.", "Separator", "After the rule."]
    );
}

#[test]
fn the_quarter_has_twelve_headings_two_tables_and_five_links() {
    let view = read("quarter.md");
    assert_eq!(view.outline().len(), 12);
    assert_eq!(view.summary_words(), "12 headings");
    assert_eq!(view.title.as_deref(), Some("The quarter"));
    assert_eq!(
        view.table_words(0).as_deref(),
        Some("Table 1 of 2, 5 rows, 3 columns")
    );
    assert_eq!(
        view.tables()[0].cell_words(1, 1),
        "Age: 36, row 2 of 5, column 2"
    );
    assert_eq!(view.links().len(), 5);
    assert_eq!(
        view.links()[1].words(1, 5),
        "Link 2 of 5: docs, to docs/index.md"
    );
    assert_eq!(
        view.heading_words(2).as_deref(),
        Some("Heading 2: People, 3 of 12")
    );
}

/// Spike Q1's embed case: an embed names a sibling that is not there,
/// and the reader takes text only, so nothing else can be read.
#[test]
fn an_embed_is_never_read_and_says_so() {
    let view = read("traps/embed.md");
    let all = shown(&view);
    assert!(
        all.contains(&"sibling-note (embed not shown)".to_owned()),
        "{all:?}"
    );
    assert!(
        all.iter()
            .any(|r| r.contains("sibling-note#Part B (embed not shown)")),
        "{all:?}"
    );
    assert_eq!(view.notes(), [EMBED_NOTE]);
    let targets: Vec<&str> = view.links().iter().map(|l| l.target.as_str()).collect();
    assert_eq!(targets, ["sibling-note"], "a wiki link stays a link");
}

#[test]
fn markdown_is_known_by_its_extension() {
    assert!(markdown::is_markdown("README.md") && markdown::is_markdown("notes.MARKDOWN"));
    assert!(!markdown::is_markdown("notes.txt") && !markdown::is_markdown("md"));
}

mod properties {
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    use diroxide_preview::doc::markdown;
    use diroxide_preview::doc::{DocView, RowKind};
    use diroxide_preview::highlight::SymbolKind;

    /// Markdown-ish text: the pieces the reader cares about, mixed.
    fn markdownish() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just("# ".to_owned()),
            Just("## ".to_owned()),
            Just("\n".to_owned()),
            Just("\n\n".to_owned()),
            Just("- ".to_owned()),
            Just("1. ".to_owned()),
            Just("  ".to_owned()),
            Just("| a | b |\n|---|---|\n".to_owned()),
            Just("|".to_owned()),
            Just("> ".to_owned()),
            Just("> [!tip] ".to_owned()),
            Just("```".to_owned()),
            Just("[x](y)".to_owned()),
            Just("[[w]]".to_owned()),
            Just("![[e]]".to_owned()),
            Just("**".to_owned()),
            Just("<p>".to_owned()),
            Just("<!--".to_owned()),
            Just("&amp;".to_owned()),
            Just("$".to_owned()),
            Just("[^1]".to_owned()),
            Just("[^1]: n".to_owned()),
            Just("---\n".to_owned()),
            Just("- [ ] ".to_owned()),
            "[a-z \u{e9}\u{4e2d}]{0,8}",
        ];
        prop::collection::vec(piece, 0..60).prop_map(|p| p.concat())
    }

    fn holds(view: &DocView) {
        for row in view.rows() {
            match row.kind {
                RowKind::Code { .. } | RowKind::Line | RowKind::Hex => {}
                RowKind::TableRow { .. } => assert!(!row.text.starts_with(char::is_whitespace)),
                _ => {
                    assert!(!row.text.is_empty(), "{row:?}");
                    assert_eq!(row.text.trim(), row.text, "{row:?}");
                }
            }
        }
        for link in view.links() {
            let row = &view.rows()[link.row].text;
            assert!(row.get(link.range.clone()).is_some(), "{link:?} in {row:?}");
        }
        for symbol in view.outline() {
            assert!(matches!(
                (symbol.kind, &view.rows()[symbol.line].kind),
                (SymbolKind::Heading(a), RowKind::Heading(b)) if a == *b
            ));
        }
        for (n, table) in view.tables().iter().enumerate() {
            assert_eq!(table.rows.len(), table.cells.len());
            for (i, row) in table.rows.clone().enumerate() {
                assert!(matches!(
                    view.rows()[row].kind,
                    RowKind::TableRow { table, row, .. } if table == n && row == i
                ));
            }
        }
    }

    proptest! {
        #![proptest_config(Config {
            cases: 512,
            failure_persistence: None,
            ..Config::default()
        })]

        /// Whatever the text, the rows hold the view's rules: no empty or
        /// padded prose row, links inside their rows, the outline on its
        /// headings, tables on their rows.
        #[test]
        fn any_text_reads_into_rows_that_hold(text in markdownish()) {
            holds(&markdown::read(&text));
        }
    }
}
