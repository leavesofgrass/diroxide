//! The evaluator's tests: truth tables over fixture entries with known
//! and unknown facts, the cost order, the first rung's patterns through
//! the whole language, and the laws of three-valued logic on random
//! entries.

use std::cell::Cell;

use proptest::prelude::*;

use super::tests::{TERMS, quiet, written};
use super::*;
use crate::clock::{Timestamp, UtcOffset};
use crate::entry::{Attributes, Entry, EntryKind, Link};

/// A local time on the day the plan was written (Pacific daylight time).
fn at(y: i32, m: u8, d: u8, h: u8) -> Timestamp {
    Timestamp::from_civil(y, m, d, h, 0, 0, UtcOffset::from_hours(-7)).unwrap()
}

fn file(name: &str, size: u64, modified: Timestamp) -> Entry {
    let mut entry = Entry::new(name, EntryKind::File)
        .with_size(size)
        .with_modified(modified);
    entry.created = Some(modified);
    entry.accessed = Some(modified);
    entry.changed = Some(modified);
    entry.attributes = Some(Attributes::Unix(0o644));
    entry.owner = Some("ada".into());
    entry.group = Some("staff".into());
    entry
}

/// The fixture entries, by name.
fn fixture(name: &str) -> Entry {
    let mut entry = match name {
        "report.pdf" => {
            let mut e = file(name, 2_100_000, at(2026, 10, 9, 10));
            e.labels = vec!["urgent".into()];
            e.type_name = Some("PDF".into());
            e
        }
        "notes.txt" => {
            let mut e = file(name, 1_200, at(2026, 10, 11, 9));
            e.type_name = Some("Text".into());
            e
        }
        "week-old.txt" => file(name, 10, at(2026, 10, 6, 12)),
        "holiday.PNG" => file(name, 6_000_000, at(2026, 8, 1, 12)),
        "old.log" => file(name, 2_000_000_000, at(2024, 1, 1, 12)),
        "deep.txt" => file(name, 10, at(2026, 10, 1, 12)),
        ".profile" => {
            let mut e = file(name, 0, at(2026, 1, 1, 12));
            e.hidden = true;
            e
        }
        "locked.txt" => {
            let mut e = file(name, 10, at(2026, 1, 1, 12));
            e.attributes = Some(Attributes::Unix(0o444));
            e
        }
        "run.sh" => {
            let mut e = file(name, 10, at(2026, 1, 1, 12));
            e.attributes = Some(Attributes::Unix(0o755));
            e
        }
        "tool.exe" => {
            let mut e = file(name, 1_000, at(2026, 1, 1, 12));
            e.attributes = Some(Attributes::Windows(0x21));
            e
        }
        "pagefile.sys" => {
            let mut e = file(name, 1_000, at(2026, 1, 1, 12));
            e.attributes = Some(Attributes::Windows(0x4));
            e
        }
        "cloud.docx" => {
            let mut e = file(name, 1_000, at(2026, 1, 1, 12));
            e.attributes = Some(Attributes::Windows(0x400000));
            e
        }
        "archive.zip" => {
            let mut e = file(name, 5_000_000, at(2026, 1, 1, 12));
            e.kind = EntryKind::Archive;
            e
        }
        "untyped.bin" | "unreadable.bin" => Entry::new(name, EntryKind::File),
        "build" | "empty-dir" => {
            let mut e = Entry::new(name, EntryKind::Folder).with_modified(at(2026, 10, 1, 12));
            e.attributes = Some(Attributes::Unix(0o755));
            e
        }
        "link-to-gone" | "link-ok" | "link-unchecked" => {
            let mut e = Entry::new(name, EntryKind::Link);
            e.link = Some(Link {
                target: if name == "link-to-gone" {
                    "gone"
                } else {
                    "there"
                }
                .into(),
                points_to: (name == "link-ok").then_some(EntryKind::File),
            });
            e
        }
        other => Entry::new(other, EntryKind::File),
    };
    entry.name = name.to_owned();
    entry
}

/// A lister that knows every fact, and counts the files it reads.
#[derive(Default)]
struct Facts {
    reads: Cell<usize>,
}

impl Lookup for Facts {
    fn path(&self, entry: &Entry) -> Option<String> {
        Some(format!("D:\\work\\{}", entry.name))
    }

    fn depth(&self, entry: &Entry) -> Option<u32> {
        Some(if entry.name.starts_with("deep") { 3 } else { 1 })
    }

    fn selected(&self, entry: &Entry) -> Option<bool> {
        Some(entry.labels.iter().any(|l| l == "urgent"))
    }

    fn bookmarked(&self, entry: &Entry) -> Option<bool> {
        Some(entry.name == "build")
    }

    fn folder_empty(&self, entry: &Entry) -> Option<bool> {
        Some(entry.name == "empty-dir")
    }

    fn link_broken(&self, entry: &Entry) -> Option<bool> {
        entry.link.as_ref().map(|link| link.target == "gone")
    }

    fn content(&self, entry: &Entry, query: &ContentQuery) -> Truth {
        self.reads.set(self.reads.get() + 1);
        if entry.name == "unreadable.bin" {
            return Truth::Unknown(Unknown::Unreadable);
        }
        Truth::of(query.is_match(&format!("contents of {}: Total due 42", entry.name)))
    }
}

fn truth(text: &str, name: &str) -> Truth {
    let filter = Filter::parse(text).unwrap_or_else(|e| panic!("{text}: {}", e.sentences()));
    filter
        .evaluator(&written())
        .test(&fixture(name), &Facts::default())
}

fn truth_alone(text: &str, entry: &Entry) -> Truth {
    Filter::parse(text)
        .unwrap()
        .evaluator(&written())
        .test(entry, &NoLookup)
}

#[test]
fn truth_tables_over_known_and_unknown_facts() {
    use Truth::{No as N, Unknown as U, Yes as Y};
    let table: [(&str, &str, Truth); 76] = [
        ("report", "report.pdf", Y),
        ("report", "notes.txt", N),
        ("*.PDF", "report.pdf", N),
        ("anycase *.PDF", "report.pdf", Y),
        ("size over 2 MB", "report.pdf", Y),
        ("size over 2 MiB", "report.pdf", Y),
        ("size over 2.2 MB", "report.pdf", N),
        ("size between 2.1 MB and 3 MB", "report.pdf", Y),
        ("size is 2100000", "report.pdf", Y),
        ("size under 0", "report.pdf", N),
        ("size over 1 KB", "build", U(Unknown::SizeNotCounted)),
        ("not size over 1 KB", "build", U(Unknown::SizeNotCounted)),
        ("size over 1 KB or kind folder", "build", Y),
        ("size over 1 KB and kind file", "build", N),
        ("modified today", "notes.txt", Y),
        ("modified today", "report.pdf", N),
        ("modified last week", "week-old.txt", Y),
        ("modified this week", "week-old.txt", N),
        ("modified within 3 days", "report.pdf", Y),
        ("modified within 1 day", "report.pdf", N),
        ("modified older than 1 year", "old.log", Y),
        ("modified before 2026-10-01", "old.log", Y),
        ("modified since 2026-10", "report.pdf", Y),
        ("modified 2026-10-09", "report.pdf", Y),
        ("modified on or before 2026-10-09", "report.pdf", Y),
        ("modified after 2026-10-09", "report.pdf", N),
        ("created between 2026-10-01 and 2026-10-31", "report.pdf", Y),
        ("modified today", "untyped.bin", U(Unknown::TimeNotKnown)),
        ("ext pdf", "report.pdf", Y),
        ("ext PDF", "report.pdf", Y),
        ("ext is not pdf", "notes.txt", Y),
        ("ext pdf", ".profile", N),
        ("ext pdf, txt", "notes.txt", Y),
        ("type PDF", "report.pdf", Y),
        ("type pdf", "report.pdf", Y),
        ("type PDF", "untyped.bin", U(Unknown::TypeNotChecked)),
        ("kind file", "archive.zip", Y),
        ("kind archive", "report.pdf", N),
        ("kind is not folder", "build", N),
        ("label urgent", "report.pdf", Y),
        ("label done", "report.pdf", N),
        ("label is not urgent", "notes.txt", Y),
        ("labeled", "report.pdf", Y),
        ("not labeled", "notes.txt", Y),
        ("owner ada", "report.pdf", Y),
        ("owner ada", "untyped.bin", U(Unknown::OwnerNotRead)),
        ("usergroup staff", "report.pdf", Y),
        ("hidden", ".profile", Y),
        ("not hidden", ".profile", N),
        ("read-only", "locked.txt", Y),
        ("read-only", "report.pdf", N),
        ("read-only", "tool.exe", Y),
        ("read-only", "untyped.bin", U(Unknown::AttributesNotRead)),
        ("executable", "run.sh", Y),
        ("executable", "tool.exe", Y),
        ("executable", "build", N),
        ("system", "pagefile.sys", Y),
        ("system", "report.pdf", N),
        ("online-only", "cloud.docx", Y),
        ("empty", ".profile", Y),
        ("empty", "empty-dir", Y),
        ("kind folder and empty", "build", N),
        ("kind link and broken", "link-to-gone", Y),
        ("broken", "link-ok", N),
        ("broken", "report.pdf", N),
        ("selected", "report.pdf", Y),
        ("bookmarked", "build", Y),
        ("depth at most 1", "report.pdf", Y),
        ("depth at most 1", "deep.txt", N),
        ("path \"/work/\"", "report.pdf", Y),
        ("path D:\\work\\*.pdf", "report.pdf", Y),
        ("path D:\\*.pdf", "report.pdf", N),
        ("path D:\\**\\*.pdf", "report.pdf", Y),
        ("content:\"total due\"", "report.pdf", Y),
        ("content:\"Total Due\"", "report.pdf", N),
        ("content:re:\"due \\d+\"", "report.pdf", Y),
    ];
    for (text, name, expected) in table {
        assert_eq!(truth(text, name), expected, "{text} on {name}");
    }
    let more: [(&str, &str, Truth); 13] = [
        ("group images and size over 5 MB", "holiday.PNG", Y),
        ("content:x", "build", N),
        ("content:x", "unreadable.bin", U(Unknown::Unreadable)),
        ("needs-care", "plain.txt", N),
        ("not not report", "report.pdf", Y),
        ("name like \"rprt\"", "report.pdf", Y),
        // The group table, by extension.
        ("group images", "holiday.PNG", Y),
        ("group image", "holiday.PNG", Y),
        ("group images", "report.pdf", N),
        ("group documents", "report.pdf", Y),
        ("group images", "build", N),
        ("group is not images", "build", Y),
        ("group images and size over 5 MB", "notes.txt", N),
    ];
    for (text, name, expected) in more {
        assert_eq!(truth(text, name), expected, "{text} on {name}");
    }
    // What a lister did not give is unknown, never a guess.
    let alone: [(&str, &str, Truth); 7] = [
        ("broken", "link-unchecked", U(Unknown::LinkNotChecked)),
        ("broken", "link-ok", N),
        ("selected", "report.pdf", U(Unknown::SelectionNotKnown)),
        ("bookmarked", "build", U(Unknown::BookmarksNotKnown)),
        ("depth at most 1", "report.pdf", U(Unknown::DepthNotKnown)),
        ("path \"/work/\"", "report.pdf", U(Unknown::PathNotKnown)),
        (
            "content:\"total due\"",
            "report.pdf",
            U(Unknown::ContentNotRead),
        ),
    ];
    for (text, name, expected) in alone {
        assert_eq!(
            truth_alone(text, &fixture(name)),
            expected,
            "{text} on {name}"
        );
    }
    assert_eq!(
        truth_alone("needs-care", &Entry::new("C\u{F03A}", EntryKind::Folder)),
        Y
    );
    // A name that is not Unicode needs care, as its row says (P3b).
    let mut lossy = Entry::new("report\u{FFFD}.txt", EntryKind::File);
    lossy.needs_care = false;
    lossy.name_not_unicode = true;
    assert_eq!(truth_alone("needs-care", &lossy), Y);
    assert_eq!(truth_alone("not needs-care", &lossy), N);
    assert_eq!(
        truth_alone("empty", &fixture("empty-dir")),
        U(Unknown::FolderNotListed)
    );
}

#[test]
fn content_is_read_last_and_only_when_it_can_decide() {
    let clock = written();
    let reads = |text: &str, name: &str| {
        let facts = Facts::default();
        let answer = Filter::parse(text)
            .unwrap()
            .evaluator(&clock)
            .test(&fixture(name), &facts);
        (answer, facts.reads.get())
    };
    // Typed first, read last: the size answers before any file is read.
    assert_eq!(
        reads("content:\"total due\" and size over 1 GB", "report.pdf"),
        (Truth::No, 0)
    );
    assert_eq!(
        reads("content:\"total due\" and size over 1 MB", "report.pdf"),
        (Truth::Yes, 1)
    );
    // An unknown size cannot become a match, so the file is not read.
    assert_eq!(
        reads("size over 1 KB and content:x", "untyped.bin"),
        (Truth::Unknown(Unknown::SizeNotKnown), 0)
    );
    // Under not, the file must be read: no inside becomes yes outside.
    assert_eq!(
        reads("not (size over 1 KB and content:x)", "untyped.bin"),
        (Truth::Yes, 1)
    );
    assert_eq!(
        reads("content:x or report", "report.pdf"),
        (Truth::Yes, 0),
        "the name answers the or"
    );
    assert!(
        Filter::parse("ext pdf and content:x")
            .unwrap()
            .reads_content()
    );
    assert!(!Filter::parse("ext pdf").unwrap().reads_content());
}

#[test]
fn the_first_rungs_patterns_keep_their_meaning() {
    // The thirty patterns of the pattern module's own table.
    let table: [(&str, &str, bool, bool); 30] = [
        ("report", "Annual Report.pdf", false, true),
        ("report 2026", "2026 report.pdf", false, true),
        ("report 2026", "report 2025.pdf", false, false),
        ("*.pdf", "a.PDF", false, true),
        ("*.pdf", "a.pdf.bak", false, false),
        ("*.pdf, *.docx", "b.docx", false, true),
        ("*.pdf; *.docx", "c.txt", false, false),
        ("IMG_????.jpg", "IMG_0042.jpg", false, true),
        ("IMG_????.jpg", "IMG_42.jpg", false, false),
        ("[a-c]*", "beta", false, true),
        ("[a-c]*", "delta", false, false),
        ("[!a-c]*", "delta", false, true),
        ("[]]x", "]x", false, true),
        ("*.pdf, not draft*", "draft-1.pdf", false, false),
        ("*.pdf, not draft*", "final.pdf", false, true),
        ("not *.tmp", "notes.txt", false, true),
        ("not *.tmp", "x.tmp", false, false),
        ("src/", "src", true, true),
        ("src/", "src", false, false),
        ("*/", "anything", true, true),
        ("*/", "file.txt", false, false),
        ("/", "folder", true, true),
        ("re:^IMG_\\d{4}", "IMG_2026.jpg", false, true),
        ("re:^IMG_\\d{4}", "my IMG_2026.jpg", false, false),
        ("RE:\\.(jpe?g|png)$", "a.JPEG", false, true),
        ("not re:^\\.", ".profile", false, false),
        ("not re:^\\.", "profile", false, true),
        ("a+b (1).txt", "a+b (1).txt", false, true),
        ("*(1)*", "a (1).txt", false, true),
        ("dollar$", "dollar$sign", false, true),
    ];
    let clock = written();
    for (pattern, name, folder, expected) in table {
        let first = crate::pattern::Pattern::parse(pattern).unwrap();
        assert_eq!(first.matches(name, folder), expected, "{pattern}");
        let kind = if folder {
            EntryKind::Folder
        } else {
            EntryKind::File
        };
        match Filter::parse(pattern) {
            Ok(filter) => {
                let said = filter
                    .evaluator(&clock)
                    .matches(&Entry::new(name, kind), &NoLookup);
                assert_eq!(said, expected, "{pattern} against {name}");
            }
            // The one change: a word that starts with a bracket is a
            // group in the language, so a name like this is quoted.
            Err(error) => {
                assert_eq!(pattern, "a+b (1).txt");
                assert_eq!(error.problem(), &Problem::UnclosedBracket);
                let quoted = Filter::parse("\"a+b (1).txt\"").unwrap();
                assert!(
                    quoted
                        .evaluator(&clock)
                        .matches(&Entry::new(name, kind), &NoLookup)
                );
            }
        }
    }
}

#[test]
fn a_tally_says_how_many_were_unknown_and_why() {
    let mut tally = Tally::default();
    for truth in [
        Truth::Yes,
        Truth::No,
        Truth::Unknown(Unknown::SizeNotCounted),
        Truth::Unknown(Unknown::SizeNotCounted),
        Truth::Unknown(Unknown::TypeNotChecked),
    ] {
        tally.add(truth);
    }
    assert_eq!(tally.matched(), 1);
    assert_eq!(tally.unknown(), 3);
    assert_eq!(
        tally.unknown_sentence().as_deref(),
        Some("3 unknown, 2 folder size not counted, 1 type not checked")
    );
    let mut one = Tally::default();
    one.add(Truth::Unknown(Unknown::SizeNotCounted));
    assert_eq!(
        one.unknown_sentence().as_deref(),
        Some("1 unknown, folder size not counted")
    );
    assert_eq!(Tally::default().unknown_sentence(), None);
}

#[test]
fn content_queries_honor_their_limit_and_case() {
    let filter = Filter::parse("content:\"due\" first 10 B").unwrap();
    struct Grab;
    impl Lookup for Grab {
        fn content(&self, _: &Entry, query: &ContentQuery) -> Truth {
            assert!(query.ignores_case());
            assert_eq!(query.limit(), Some(10));
            Truth::of(query.is_match("Total due ") && !query.is_match("0123456789 due"))
        }
    }
    let evaluator = filter.evaluator(&written());
    assert!(evaluator.matches(&fixture("notes.txt"), &Grab));
    // A limit inside a character cuts back to its start.
    let wide = Filter::parse("content:\"é\" first 1 B").unwrap();
    struct Wide;
    impl Lookup for Wide {
        fn content(&self, _: &Entry, query: &ContentQuery) -> Truth {
            Truth::of(query.is_match("éé"))
        }
    }
    assert!(
        !wide
            .evaluator(&written())
            .matches(&fixture("notes.txt"), &Wide)
    );
}

/// Entries with every fact known.
fn entries() -> impl Strategy<Value = Entry> {
    let names = prop::sample::select(vec![
        "report.pdf",
        "notes.txt",
        "draft-1.md",
        "IMG_2026.jpg",
        "thumb.png",
        "a.log",
        ".hidden",
        "Makefile",
        "x.tmp",
        "rprt.doc",
        "Report.TXT",
    ]);
    let kinds = prop::sample::select(vec![
        EntryKind::File,
        EntryKind::Folder,
        EntryKind::Link,
        EntryKind::Archive,
    ]);
    (
        names,
        kinds,
        0u64..10_000_000_000,
        -800i64..10,
        prop::bool::ANY,
        0u32..0o1000,
        prop::sample::subsequence(vec!["urgent", "done"], 0..=2),
        prop::sample::select(vec!["PDF", "Text", "PNG"]),
        prop::bool::ANY,
    )
        .prop_map(
            |(name, kind, size, days, unix, bits, labels, type_name, hidden)| {
                let when = at(2026, 10, 11, 14).plus_seconds(days * 86_400);
                let mut entry = file(name, size, when);
                entry.kind = kind;
                entry.attributes = Some(if unix {
                    Attributes::Unix(bits)
                } else {
                    Attributes::Windows(bits & 0x40027)
                });
                entry.labels = labels.into_iter().map(str::to_owned).collect();
                entry.type_name = Some(type_name.to_owned());
                entry.hidden = hidden;
                if kind == EntryKind::Link {
                    entry.link = Some(Link {
                        target: if hidden { "gone" } else { "there" }.into(),
                        points_to: Some(EntryKind::File),
                    });
                }
                entry
            },
        )
}

/// The terms, every one of whose facts the fixture lister knows.
fn known_terms() -> impl Strategy<Value = &'static str> {
    prop::sample::select(TERMS.to_vec())
}

fn terms() -> impl Strategy<Value = &'static str> {
    prop::sample::select(TERMS.to_vec())
}

fn answer(text: &str, entry: &Entry) -> Truth {
    Filter::parse(text)
        .unwrap_or_else(|e| panic!("{text}: {}", e.sentences()))
        .evaluator(&written())
        .test(entry, &Facts::default())
}

/// Whether an entry matches with no lister at all, so path, depth,
/// selection, bookmarks and contents are unknown.
fn alone(text: &str, entry: &Entry) -> bool {
    Filter::parse(text)
        .unwrap_or_else(|e| panic!("{text}: {}", e.sentences()))
        .evaluator(&written())
        .matches(entry, &NoLookup)
}

proptest! {
    #![proptest_config(quiet())]

    #[test]
    fn double_negation_changes_nothing_on_known_facts(a in known_terms(), entry in entries()) {
        prop_assert_eq!(answer(&format!("not not ({a})"), &entry), answer(&format!("({a})"), &entry));
    }

    #[test]
    fn de_morgan_holds_on_known_facts(a in known_terms(), b in known_terms(), entry in entries()) {
        prop_assert_eq!(
            answer(&format!("not (({a}) and ({b}))"), &entry),
            answer(&format!("(not ({a})) or (not ({b}))"), &entry)
        );
        prop_assert_eq!(
            answer(&format!("not (({a}) or ({b}))"), &entry),
            answer(&format!("(not ({a})) and (not ({b}))"), &entry)
        );
    }

    /// With unknown facts the answers may differ between no and unknown,
    /// because a file is not read once an and cannot match; whether an
    /// entry matches never differs.
    #[test]
    fn de_morgan_matches_the_same_with_unknown_facts(a in terms(), b in terms(), entry in entries()) {
        prop_assert_eq!(
            alone(&format!("not (({a}) and ({b}))"), &entry),
            alone(&format!("(not ({a})) or (not ({b}))"), &entry)
        );
        prop_assert_eq!(
            alone(&format!("not (({a}) or ({b}))"), &entry),
            alone(&format!("(not ({a})) and (not ({b}))"), &entry)
        );
        prop_assert_eq!(alone(&format!("not not ({a})"), &entry), alone(&format!("({a})"), &entry));
        // Unknown matches neither a test nor its negation.
        let both = alone(&format!("({a})"), &entry) && alone(&format!("not ({a})"), &entry);
        prop_assert!(!both, "{} and its negation both matched", a);
    }

    #[test]
    fn known_facts_give_known_answers(a in known_terms(), entry in entries()) {
        let truth = answer(&format!("({a})"), &entry);
        prop_assert!(!matches!(truth, Truth::Unknown(_)), "{} on {:?} gave {:?}", a, entry.name, truth);
    }

    #[test]
    fn the_read_back_tests_the_same(a in terms(), b in terms(), entry in entries()) {
        let text = format!("({a}) or not ({b})");
        let filter = Filter::parse(&text).unwrap();
        let again = Filter::parse(filter.read_back(&written()).sentence()).unwrap();
        prop_assert_eq!(answer(&text, &entry), {
            again.evaluator(&written()).test(&entry, &Facts::default())
        });
    }
}

/// P3b (W3s's request 4): an undecided content answer names its reason,
/// binary or online only, and the reason reads in the summary.
#[test]
fn an_undecided_content_answer_names_its_reason() {
    struct Skipping;
    impl Lookup for Skipping {
        fn content(&self, entry: &Entry, _: &ContentQuery) -> Truth {
            match entry.name.as_str() {
                "photo.raw" => Truth::Unknown(Unknown::Binary),
                "cloud.txt" => Truth::Unknown(Unknown::OnlineOnly),
                _ => Truth::No,
            }
        }
    }
    let clock = written();
    let filter = Filter::parse("content:total").unwrap();
    let evaluator = filter.evaluator(&clock);
    let mut tally = Tally::default();
    for name in ["photo.raw", "cloud.txt", "notes.txt"] {
        tally.add(evaluator.test(&Entry::new(name, EntryKind::File), &Skipping));
    }
    assert_eq!(
        tally.unknown_sentence().as_deref(),
        Some("2 unknown, 1 binary file not searched, 1 online-only file not read")
    );
    // Neither the test nor its negation matches, and the reason stays.
    let negated = Filter::parse("not content:total").unwrap();
    assert_eq!(
        negated
            .evaluator(&clock)
            .test(&Entry::new("photo.raw", EntryKind::File), &Skipping),
        Truth::Unknown(Unknown::Binary)
    );
    for why in [Unknown::Binary, Unknown::OnlineOnly] {
        assert!(why.words().len() <= 40, "{why}");
    }
}
