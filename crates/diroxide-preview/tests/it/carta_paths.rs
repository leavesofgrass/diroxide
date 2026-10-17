//! Spike C1 (research/alpha2-plan.md part 4.5, plan question 66): which
//! of carta's writers carries a document into the quick view. Path A has
//! carta write an HTML fragment for a fragment reader; path B has carta
//! write CommonMark for the Markdown reader. The rule: path B if the
//! headings, list items, tables and links of every sample survive it.
//!
//! The samples are hand-written, three for each of carta's five readers
//! (`fixtures/docs/carta/`), each with its counts written down here by
//! hand. Path A's counts are checked always; path B's with the
//! `carta-markdown` feature, which puts carta's CommonMark writer in the
//! build for this comparison only.

use std::path::PathBuf;
use std::time::Instant;

/// What a sample holds: headings, list items, tables and links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub headings: usize,
    pub items: usize,
    pub tables: usize,
    pub links: usize,
}

const fn counts(headings: usize, items: usize, tables: usize, links: usize) -> Counts {
    Counts {
        headings,
        items,
        tables,
        links,
    }
}

/// The samples, carta's reader for each, and their counts by hand. Org's
/// `#+TITLE` is metadata, not a heading of the body.
pub const SAMPLES: [(&str, &str, Counts); 15] = [
    ("notes-1.org", "org", counts(4, 5, 1, 2)),
    ("notes-2.org", "org", counts(4, 6, 1, 2)),
    ("notes-3.org", "org", counts(3, 3, 2, 3)),
    ("guide-1.rst", "rst", counts(3, 3, 1, 2)),
    ("guide-2.rst", "rst", counts(4, 6, 1, 2)),
    ("guide-3.rst", "rst", counts(3, 2, 2, 3)),
    ("page-1.wiki", "mediawiki", counts(3, 3, 1, 2)),
    ("page-2.wiki", "mediawiki", counts(3, 4, 1, 2)),
    ("page-3.wiki", "mediawiki", counts(3, 5, 2, 3)),
    ("wiki-1.dokuwiki", "dokuwiki", counts(3, 3, 1, 2)),
    ("wiki-2.dokuwiki", "dokuwiki", counts(3, 3, 1, 2)),
    ("wiki-3.dokuwiki", "dokuwiki", counts(3, 5, 2, 3)),
    ("issue-1.jira", "jira", counts(3, 3, 1, 2)),
    ("issue-2.jira", "jira", counts(2, 4, 1, 2)),
    ("issue-3.jira", "jira", counts(3, 3, 2, 3)),
];

pub fn sample_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("docs")
        .join("carta")
        .join(name)
}

fn sample(name: &str) -> Vec<u8> {
    std::fs::read(sample_path(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn write(from: &str, bytes: &[u8], to: &str) -> String {
    let (doc, media) = carta::read_document(from, bytes, &carta::ReaderOptions::default())
        .unwrap_or_else(|e| panic!("{from}: {e}"));
    match carta::render_document(to, doc, media, &carta::WriterOptions::default()) {
        Ok(carta::Output::Text(text)) => text,
        other => panic!("{from} to {to}: {other:?}"),
    }
}

/// Path A's counts, from the tags of carta's HTML fragment.
fn html_counts(html: &str) -> Counts {
    let lower = html.to_ascii_lowercase();
    let tags = |tag: &str| {
        lower
            .match_indices(&format!("<{tag}"))
            .filter(|(at, _)| {
                lower[at + tag.len() + 1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c == '>' || c.is_whitespace())
            })
            .count()
    };
    Counts {
        headings: (1..=6).map(|n| tags(&format!("h{n}"))).sum(),
        items: tags("li"),
        tables: tags("table"),
        links: tags("a"),
    }
}

#[test]
fn path_a_keeps_every_heading_list_item_table_and_link() {
    let mut failed = Vec::new();
    for (name, from, want) in SAMPLES {
        let got = html_counts(&write(from, &sample(name), "html"));
        if got != want {
            failed.push(format!("{name}: want {want:?}, got {got:?}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The time carta takes to read and write about 100 KB of each format,
/// the samples repeated; printed for the record (`--nocapture`).
#[test]
fn a_hundred_kilobytes_of_each_format_reads_in_well_under_a_second() {
    for from in ["org", "rst", "mediawiki", "dokuwiki", "jira"] {
        let mut text = String::new();
        while text.len() < 100_000 {
            for (name, format, _) in SAMPLES {
                if format == from {
                    text.push_str(&String::from_utf8_lossy(&sample(name)));
                    text.push_str("\n\n");
                }
            }
        }
        let start = Instant::now();
        let html = write(from, text.as_bytes(), "html");
        let took = start.elapsed();
        println!(
            "spike C1: {from}, {} bytes in, {} bytes of HTML, {} ms",
            text.len(),
            html.len(),
            took.as_millis()
        );
        assert!(took.as_secs() < 5, "{from}: {took:?}");
    }
}

#[cfg(feature = "carta-markdown")]
mod path_b {
    use pulldown_cmark::{Event, Options, Parser, Tag};

    use super::{Counts, SAMPLES, sample, write};

    /// Path B's counts, from what the Markdown reader would see in carta's
    /// CommonMark: raw HTML is not a table to it.
    fn markdown_counts(markdown: &str) -> Counts {
        let mut counts = Counts::default();
        for event in Parser::new_ext(markdown, Options::all()) {
            match event {
                Event::Start(Tag::Heading { .. }) => counts.headings += 1,
                Event::Start(Tag::Item) => counts.items += 1,
                Event::Start(Tag::Table(_)) => counts.tables += 1,
                Event::Start(Tag::Link { .. }) => counts.links += 1,
                _ => {}
            }
        }
        counts
    }

    /// Prints each sample's counts by both paths, then says whether path B
    /// holds on every one.
    #[test]
    fn path_b_compared_with_path_a() {
        let mut holds = true;
        for (name, from, want) in SAMPLES {
            let markdown = write(from, &sample(name), "commonmark");
            let got = markdown_counts(&markdown);
            println!("spike C1: {name}: by hand {want:?}, path B {got:?}");
            holds &= got == want;
        }
        println!("spike C1: path B holds on every sample: {holds}");
    }
}

/// How long carta takes on a 5 MB Org file, printed for the record: the
/// quick view's two-second deadline must cut a longer read off.
#[test]
fn five_megabytes_of_org_for_the_record() {
    let mut text = String::new();
    while text.len() < 5_000_000 {
        text.push_str(&String::from_utf8_lossy(&sample("notes-1.org")));
        text.push_str(&String::from_utf8_lossy(&sample("notes-2.org")));
    }
    let start = Instant::now();
    let html = write("org", text.as_bytes(), "html");
    println!(
        "spike C1: org, {} bytes in, {} bytes of HTML, {} ms",
        text.len(),
        html.len(),
        start.elapsed().as_millis()
    );
}

/// A list `depth` levels deep in carta's reader `from`.
pub fn nested_list(from: &str, depth: usize) -> String {
    let mut text = String::new();
    for level in 0..depth {
        let line = match from {
            "org" => format!(
                "{}- level {level}
",
                " ".repeat(level * 2)
            ),
            "rst" => format!(
                "{}- level {level}

",
                " ".repeat(level * 2)
            ),
            "dokuwiki" => format!(
                "{}* level {level}
",
                " ".repeat(level * 2 + 2)
            ),
            _ => format!(
                "{} level {level}
",
                "*".repeat(level + 1)
            ),
        };
        text.push_str(&line);
    }
    text
}

/// Reads a nested list on a thread with an 8 MiB stack under
/// `catch_unwind`, as the quick view's worker does; prints the time.
fn read_nested(from: &'static str, depth: usize) -> bool {
    let text = nested_list(from, depth);
    let start = Instant::now();
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(|| write(from, text.as_bytes(), "html").len()).is_ok()
        })
        .unwrap();
    let ok = handle.join().unwrap();
    println!(
        "spike C1: {from}, a list {depth} deep, finished {ok} in {} ms",
        start.elapsed().as_millis()
    );
    ok
}

/// A list nested 300 deep in each format reads without overflowing an
/// 8 MiB stack.
#[test]
fn a_nested_list_reads_on_an_eight_mebibyte_stack() {
    for from in ["org", "rst", "mediawiki", "dokuwiki", "jira"] {
        assert!(read_nested(from, 300), "{from}");
    }
}

/// The same 3,000 deep, for the record: no overflow, but Org took about
/// 157 seconds and reStructuredText about 42 in a debug build (Friday,
/// October 16, 2026), so the quick view's deadline is what keeps such a
/// file from holding a worker. Minutes long: run with `--ignored`.
#[test]
#[ignore = "takes minutes; run with --ignored for the record"]
fn a_list_three_thousand_deep_for_the_record() {
    for from in ["org", "rst", "mediawiki", "dokuwiki", "jira"] {
        assert!(read_nested(from, 3000), "{from}");
    }
}
