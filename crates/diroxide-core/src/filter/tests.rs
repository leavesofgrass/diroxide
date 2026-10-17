//! Tests across the filter's parts: the twenty examples of the plan, the
//! errors with their words, and the round trip from text to tree to
//! sentence and back.

use proptest::prelude::*;

use super::*;
use crate::clock::{FixedClock, Timestamp, UtcOffset};
use crate::entry::EntryKind;
use crate::error::Explain;
use crate::sort::NameCase;

/// The twenty examples of `research/alpha3-plan.md`, as typed.
pub(super) const TWENTY: [&str; 20] = [
    "report",
    "*.log",
    "*.txt, *.md",
    "re:\"^IMG_\\d{4}\"",
    "size over 100 MB",
    "size between 1 KiB and 1 MiB",
    "modified today",
    "modified last week",
    "ext pdf and modified within 3 days",
    "modified older than 1 year and size over 1 GB",
    "kind folder and empty",
    "kind link and broken",
    "needs-care",
    "label urgent and not label done",
    "group images and size over 5 MB",
    "content:\"total due\"",
    "content:re:\"due \\d+\" and ext txt,md",
    "hidden and kind file",
    "depth at most 1 and name *.rs",
    "(ext jpg or ext png) and not name thumb*",
];

/// More forms the guide shows, and the first rung's patterns.
pub(super) const MORE: [&str; 16] = [
    "path \"/work/\" and draft",
    "name like \"rprt\"",
    "bookmarked or label urgent",
    "selected and size under 1 KB",
    "owner alice",
    "*.pdf, not draft*",
    "not *.tmp",
    "src/",
    "/",
    "*/",
    "RE:\\.(jpe?g|png)$",
    "not re:^\\.",
    "*(1)*",
    "report 2026",
    "*.pdf; *.docx",
    "[!a-c]*",
];

pub(super) fn parse(text: &str) -> Filter {
    Filter::parse(text).unwrap_or_else(|e| panic!("{text}: {}", e.sentences()))
}

pub(super) fn problem(text: &str) -> FilterError {
    match Filter::parse(text) {
        Ok(filter) => panic!("{text} read as {}", filter.sentence()),
        Err(error) => error,
    }
}

/// The moment the plan's examples were written: Sunday, October 11, 2026,
/// 2:09 PM, Pacific daylight time.
pub(super) fn written() -> FixedClock {
    let offset = UtcOffset::from_hours(-7);
    FixedClock::new(
        Timestamp::from_civil(2026, 10, 11, 14, 9, 0, offset).unwrap(),
        offset,
    )
}

fn round_trips(text: &str) {
    let filter = parse(text);
    for sentence in [
        filter.sentence(),
        filter.read_back(&written()).sentence().to_owned(),
    ] {
        let again = Filter::parse(&sentence)
            .unwrap_or_else(|e| panic!("{text} read back as {sentence}: {}", e.sentences()));
        assert_eq!(
            again.expr(),
            filter.expr(),
            "{text} read back as {sentence}"
        );
    }
    assert_eq!(
        parse(&filter.sentence()).sentence(),
        filter.sentence(),
        "{text}"
    );
}

#[test]
fn the_twenty_examples_parse_and_read_back_as_one_sentence() {
    let expected = [
        "name contains report",
        "name matches *.log",
        "name matches *.txt or *.md",
        "name regex ^IMG_\\d{4}, match case",
        "size over 100 MB",
        "size from 1 KiB to 1 MiB",
        "modified today",
        "modified last week",
        "extension is pdf and modified within 3 days",
        "modified older than 1 year and size over 1 GB",
        "kind is folder and empty",
        "kind is link and broken",
        "needs-care",
        "label is urgent and not label is done",
        "group is images and size over 5 MB",
        "content contains \"total due\"",
        "content regex \"due \\d+\" and (extension is txt or extension is md)",
        "hidden and kind is file, including hidden",
        "depth at most 1 and name matches *.rs",
        "(extension is jpg or extension is png) and not name matches thumb*",
    ];
    for (text, sentence) in TWENTY.iter().zip(expected) {
        assert_eq!(parse(text).sentence(), sentence, "{text}");
        round_trips(text);
    }
    for text in MORE {
        round_trips(text);
    }
}

/// Each example as typed, its read-back and its details line.
fn read_backs(texts: &[&str]) -> String {
    let mut out = String::new();
    for text in texts {
        let read_back = parse(text).read_back(&written());
        out.push_str(&format!("typed: {text}\nsaid: {}\n", read_back.sentence()));
        if !read_back.details().is_empty() {
            out.push_str(&format!("details: {}\n", read_back.details_line()));
        }
        out.push('\n');
    }
    out
}

#[test]
fn the_read_backs_snapshot() {
    insta::assert_snapshot!("twenty_read_backs", read_backs(&TWENTY));
    insta::assert_snapshot!("more_read_backs", read_backs(&MORE));
}

#[test]
fn every_first_clause_fits_one_braille_line() {
    for text in TWENTY.iter().chain(MORE.iter()) {
        let read_back = parse(text).read_back(&written());
        let clause = read_back.first_clause();
        assert!(
            clause.chars().count() <= crate::format::BRAILLE_CELLS,
            "{text}: \"{clause}\" has {} cells",
            clause.chars().count()
        );
        for detail in read_back.details() {
            assert!(
                detail.chars().count() <= crate::format::BRAILLE_CELLS,
                "{detail}"
            );
        }
    }
}

#[test]
fn byte_counts_go_on_the_details_line() {
    let clock = written();
    let read_back = parse("size over 100 MB").read_back(&clock);
    assert_eq!(read_back.sentence(), "size over 100 MB");
    assert_eq!(read_back.details(), ["100 MB is 100,000,000 bytes."]);
    let read_back = parse("size between 1 KiB and 1 MiB").read_back(&clock);
    assert_eq!(
        read_back.details_line(),
        "1 KiB is 1,024 bytes. 1 MiB is 1,048,576 bytes."
    );
    assert!(
        parse("size over 500")
            .read_back(&clock)
            .details()
            .is_empty()
    );
    assert_eq!(parse("size over 500").sentence(), "size over 500 bytes");
    let read_back = parse("size over 1.5 GiB or size under 1.5 GiB").read_back(&clock);
    assert_eq!(read_back.details(), ["1.5 GiB is 1,610,612,736 bytes."]);
    let read_back = parse("content:\"x\" first 64 KB").read_back(&clock);
    assert_eq!(read_back.details(), ["64 KB is 64,000 bytes."]);
}

#[test]
fn date_words_read_back_with_their_real_days() {
    let clock = written();
    let said = |text: &str| parse(text).read_back(&clock).sentence().to_owned();
    assert_eq!(said("modified today"), "modified today, October 11, 2026");
    assert_eq!(
        said("modified last week"),
        "modified last week, October 4 to 10"
    );
    assert_eq!(
        said("modified this week"),
        "modified this week, October 11 to 17"
    );
    assert_eq!(
        said("created yesterday"),
        "created yesterday, October 10, 2026"
    );
    assert_eq!(
        said("modified this month"),
        "modified this month, October 2026"
    );
    assert_eq!(
        said("modified last month"),
        "modified last month, September 2026"
    );
    assert_eq!(said("modified last year"), "modified last year, 2025");
    assert_eq!(
        said("modified since last week"),
        "modified since last week, October 4 to 10"
    );
    // A range of date words keeps the sentence short and puts the days on
    // the details line.
    let read_back = parse("modified from last week to today").read_back(&clock);
    assert_eq!(read_back.sentence(), "modified from last week to today");
    assert_eq!(
        read_back.details(),
        [
            "Last week is October 4 to 10.",
            "Today is October 11, 2026."
        ]
    );
    let read_back = parse("modified within 3 days").read_back(&clock);
    assert_eq!(
        read_back.details(),
        ["3 days ago: October 8, 2026, 2:09 PM."]
    );
    // A Monday start spells other days.
    let monday = FilterOptions {
        week_start: Weekday::Monday,
        ..FilterOptions::default()
    };
    let filter = Filter::parse_with("modified last week", &monday).unwrap();
    assert_eq!(
        filter.read_back(&clock).sentence(),
        "modified last week, September 28 to October 4"
    );
    // The days read back as nothing more, even the long forms.
    for text in [
        "modified last week, October 4 to 10",
        "modified last week, September 27 to October 3",
        "modified last week, December 27, 2026 to January 2, 2027",
        "modified last week, December 20 to 26, 2026",
        "modified from last week, October 4 to 10 to today, October 11, 2026",
        "modified this year, 2026 and size over 1 MB",
    ] {
        let filter = parse(text);
        assert!(
            !filter.sentence().contains("October"),
            "{text} kept its days in the tree"
        );
    }
}

#[test]
fn a_hidden_test_shows_hidden_entries_and_says_so() {
    assert!(parse("hidden and kind file").shows_hidden());
    assert!(parse("hidden or *.txt").shows_hidden());
    assert!(parse("not not hidden").shows_hidden());
    assert!(!parse("not hidden").shows_hidden());
    assert!(!parse("*.txt").shows_hidden());
    assert_eq!(parse("not hidden").sentence(), "not hidden");
    assert_eq!(parse("hidden").sentence(), "hidden, including hidden");
    // The note reads back as nothing more.
    assert_eq!(
        parse("hidden, including hidden").expr(),
        parse("hidden").expr()
    );
    assert_eq!(
        parse("*.txt, *.md and hidden, including hidden").expr(),
        parse("(*.txt or *.md) and hidden").expr()
    );
    // Only at the very end.
    assert_eq!(
        problem("hidden, including hidden and x").problem(),
        &Problem::CommaNotHere
    );
}

#[test]
fn trees_say_what_was_meant() {
    let Expr::Test {
        test: Test::Number(size),
        ..
    } = parse("size>100mb").expr().clone()
    else {
        panic!("not a size test");
    };
    assert_eq!(size.field, Field::Size);
    let NumberOp::Over(amount) = size.op else {
        panic!("not over");
    };
    assert_eq!(amount.value(), Some(100_000_000));
    assert_eq!(parse("size>100mb").expr(), parse("size over 100 MB").expr());
    assert_eq!(
        parse("size>=1mb").expr(),
        parse("size at least 1 MB").expr()
    );
    assert_eq!(parse("size!=0").expr(), parse("size is not 0 bytes").expr());
    assert_eq!(
        parse("size 1KB..2KB").expr(),
        parse("size between 1 KB and 2 KB").expr()
    );
    assert_eq!(
        parse("size between 1 and 5 MB").expr(),
        parse("size from 1 MB to 5 MB").expr()
    );
    assert_eq!(parse("ext:pdf").expr(), parse("ext is pdf").expr());
    assert_eq!(parse("ext .pdf").expr(), parse("extension pdf").expr());
    assert_eq!(
        parse("EXT pdf AND hidden").expr(),
        parse("ext pdf hidden").expr()
    );
    assert_eq!(parse("state=selected").expr(), parse("selected").expr());
    assert_eq!(
        parse("timemod today").expr(),
        parse("modified today").expr()
    );
    assert_eq!(parse("!hidden").expr(), parse("not hidden").expr());
    assert_eq!(parse("((a))").expr(), parse("a").expr());
    assert_eq!(parse("(a and b) and c").expr(), parse("a b c").expr());
    assert_eq!(
        parse("modified<3d").expr(),
        parse("modified within 3 days").expr()
    );
    assert_eq!(
        parse("modified>1y").expr(),
        parse("modified older than 1 year").expr()
    );
    assert_eq!(
        parse("modified 2026-01-01..2026-06-30").expr(),
        parse("modified between 2026-01-01 and 2026-06-30").expr()
    );
    assert_eq!(
        parse("modified since October 1, 2026").expr(),
        parse("modified>=2026-10-01").expr()
    );
    assert_eq!(
        parse("modified != today").expr(),
        parse("not modified today").expr()
    );
    // The first rung's not binds to one value of a list.
    assert_eq!(
        parse("*.pdf, not draft*").expr(),
        parse("*.pdf and not draft*").expr()
    );
    assert_eq!(parse("not a, b").expr(), parse("b and not a").expr());
    // Is not means none of the list.
    assert_eq!(
        parse("ext is not pdf, docx").sentence(),
        "extension is not pdf and extension is not docx"
    );
    let Expr::Test {
        test: Test::Kind(kind),
        ..
    } = parse("kind folders").expr().clone()
    else {
        panic!("not a kind test");
    };
    assert_eq!(kind.kind, EntryKind::Folder);
}

#[test]
fn groups_are_read_by_their_own_names() {
    assert_eq!(parse("group Image").sentence(), "group is images");
    assert_eq!(parse("group image").expr(), parse("group:images").expr());
    let error = problem("group imges");
    assert_eq!(
        error.problem(),
        &Problem::UnknownGroup {
            word: "imges".into(),
            nearest: Some("images"),
        }
    );
    assert_eq!(
        error.sentences(),
        "Filter problem: unknown group 'imges'. Word 2. Did you mean images?"
    );
    assert_eq!(
        problem("group pictures").next_step(),
        "Word 2. Groups: images, music, video, documents, archives, code, text."
    );
    // The error's group list is the table's.
    let names: Vec<&str> = GROUPS.iter().map(|g| g.name).collect();
    assert_eq!(
        names.join(", "),
        "images, music, video, documents, archives, code, text"
    );
}

#[test]
fn smart_case_is_decided_per_test_and_said() {
    let case_of = |text: &str| match parse(text).expr().clone() {
        Expr::Test {
            test: Test::Text(text),
            ..
        } => text.case,
        other => panic!("{other:?}"),
    };
    assert_eq!(case_of("report"), NameCase::Ignore);
    assert_eq!(case_of("Report"), NameCase::Respect);
    assert_eq!(case_of("case report"), NameCase::Respect);
    assert_eq!(case_of("anycase Report"), NameCase::Ignore);
    assert_eq!(case_of("ext PDF"), NameCase::Ignore);
    assert_eq!(
        parse("Report").sentence(),
        "name contains Report, match case"
    );
    assert_eq!(
        parse("anycase Report").sentence(),
        "name contains Report, any case"
    );
    assert_eq!(
        parse("case report").sentence(),
        "name contains report, match case"
    );
    assert_eq!(
        parse("*.TXT, *.md").sentence(),
        "name matches *.TXT, match case or *.md"
    );
    let ignore = FilterOptions {
        case: CaseRule::Ignore,
        ..FilterOptions::default()
    };
    let filter = Filter::parse_with("Report", &ignore).unwrap();
    assert_eq!(filter.sentence(), "name contains Report");
    let matching = FilterOptions {
        case: CaseRule::Match,
        ..FilterOptions::default()
    };
    let filter = Filter::parse_with("report", &matching).unwrap();
    assert_eq!(filter.sentence(), "name contains report, match case");
}

#[test]
fn values_that_look_like_the_language_are_quoted_back() {
    for text in [
        "\"and\"",
        "\"size\"",
        "\"a b\"",
        "\"x=1\"",
        "\"re:x\"",
        "\"(1).txt\"",
        "\"say \"\"hi\"\"\"",
        "name contains \"hidden\"",
        "\"ext:pdf\"",
        "name regex \"a b\"",
        "name regex a|b,c",
        "(name regex (a|b) or *.md) and size over 1 KB",
        "path \"D:\\work\\*\"",
        "content contains \"total due\" first 64 KB",
        "content:Total",
        "a or \"not\"",
        "modified on October 11, 2026, 2:30 PM",
        "modified 2026-10",
        "modified 2026",
        "modified on or before 2026-10-01",
        "modified from last week to today",
        "accessed after yesterday",
        "usergroup staff, wheel",
        "Urgent, any case",
        "a)",
    ] {
        round_trips(text);
    }
    assert_eq!(parse("\"and\"").sentence(), "name contains \"and\"");
    assert_eq!(parse("\"(1).txt\"").sentence(), "name contains \"(1).txt\"");
}

#[test]
fn errors_name_the_word_and_what_comes_next() {
    let table: [(&str, Problem, usize, &str); 12] = [
        (
            "size over MB",
            Problem::NeedsValue(Field::Size),
            3,
            "Filter problem: size needs a number",
        ),
        (
            "size over",
            Problem::NeedsValue(Field::Size),
            3,
            "Filter problem: size needs a number",
        ),
        (
            "sise over 10 MB",
            Problem::UnknownField {
                word: "sise".into(),
                nearest: Some(Field::Size),
            },
            1,
            "Filter problem: unknown field 'sise'",
        ),
        (
            "size over 10 mbb",
            Problem::UnknownUnit("mbb".into()),
            4,
            "Filter problem: unknown unit 'mbb'",
        ),
        (
            "a and",
            Problem::NothingAfter(Keyword::And),
            3,
            "Filter problem: nothing after and",
        ),
        (
            "a or b c",
            Problem::MixedAndOr,
            4,
            "Filter problem: and mixed with or",
        ),
        (
            "(a or b",
            Problem::UnclosedBracket,
            1,
            "Filter problem: unclosed bracket",
        ),
        (
            "modified sometime",
            Problem::DateNotUnderstood,
            2,
            "Filter problem: date not understood",
        ),
        (
            "re:(a",
            Problem::BadRegex("unclosed group".into()),
            1,
            "Filter problem: bad regular expression",
        ),
        (
            "re:x(?=y)",
            Problem::LookAround,
            1,
            "Filter problem: look-around not allowed",
        ),
        (
            "*.txt | *.md",
            Problem::Pipe,
            2,
            "Filter problem: write or, not |",
        ),
        (
            "content:",
            Problem::NeedsValue(Field::Content),
            2,
            "Filter problem: content needs text",
        ),
    ];
    for (text, expected, word, line) in table {
        let error = problem(text);
        assert_eq!(error.problem(), &expected, "{text}");
        assert_eq!(error.word(), word, "{text}");
        assert_eq!(error.line(), line, "{text}");
    }
    let error = problem("size over MB");
    assert_eq!(error.next_step(), "Word 3. Try size over 10 MB.");
    assert_eq!(error.expected(), [Expected::Value(Field::Size)]);
    let error = problem("sise over 10 MB");
    assert_eq!(error.next_step(), "Word 1. Did you mean size?");
    let error = problem("size over 10 mbb");
    assert_eq!(
        error.next_step(),
        "Word 4. Units: B, KB, MB, GB, TB, KiB, MiB, GiB, TiB."
    );
    let error = problem("a or b c");
    assert_eq!(
        error.next_step(),
        "Word 4. Put brackets around one of them."
    );
    let error = problem("size over");
    assert_eq!(
        error.expected(),
        [Expected::Value(Field::Size)],
        "at the end, the list is what could come there"
    );
    let error = problem("  size over");
    assert_eq!(error.span(), 11..11);
}

#[test]
fn more_errors_are_said_in_words() {
    let table: [(&str, Problem); 30] = [
        ("", Problem::Empty),
        ("size over 10 m", Problem::BareUnit('m')),
        ("size over 10k", Problem::BareUnit('k')),
        ("depth 1 KB", Problem::DepthHasNoUnit),
        ("size over 99999999999 TB", Problem::NumberTooLarge),
        ("size between 1 MB", Problem::RangeNeedsAnd),
        ("modified before 10/11/2026", Problem::YearFirst),
        ("modified 2026-02-29", Problem::NoSuchDay),
        ("modified within 3", Problem::NeedsTimeUnit),
        ("modified 3 days", Problem::WithinOrOlder),
        ("modified within today", Problem::NeedsDuration),
        ("modified last", Problem::DateNotUnderstood),
        ("re:(.)\\1", Problem::Backreference),
        ("[abc", Problem::GlobBracket),
        ("hidden, empty", Problem::CommaNotHere),
        ("case size over 1", Problem::CaseNeedsText),
        ("kind pipe", Problem::UnknownKind("pipe".into())),
        ("state is open", Problem::StateIsSelected),
        ("> 10", Problem::SignNeedsField(">")),
        ("a & b", Problem::Ampersand),
        ("\"open", Problem::UnclosedQuote),
        ("a )", Problem::ExtraBracket),
        ("()", Problem::EmptyBrackets),
        ("and a", Problem::NothingBefore(Keyword::And)),
        ("not", Problem::NothingAfter(Keyword::Not)),
        ("(a or)", Problem::NothingAfter(Keyword::Or)),
        (
            "photo:x",
            Problem::UnknownField {
                word: "photo".into(),
                nearest: None,
            },
        ),
        ("ext", Problem::NeedsValue(Field::Extension)),
        ("name \"\"", Problem::NeedsValue(Field::Name)),
        (
            &format!("{}a{}", "(".repeat(33), ")".repeat(33)),
            Problem::TooDeep,
        ),
    ];
    for (text, expected) in &table {
        let error = problem(text);
        assert_eq!(error.problem(), expected, "{text}");
        assert!(error.line().starts_with(PROBLEM_START));
        assert!(error::fits_one_line(&error.line()), "{}", error.line());
        assert!(error.next_step().starts_with("Word "), "{text}");
        assert!(!error.to_string().is_empty());
    }
    let error = problem("size over 10 m");
    assert_eq!(error.line(), "Filter problem: write MB or MiB");
    assert_eq!(
        error.next_step(),
        "Word 4. MB is 1,000,000 bytes; MiB is 1,048,576."
    );
    let error = problem("modified before 10/11/2026");
    assert_eq!(
        error.next_step(),
        "Word 3. Dates are written year, month, day, such as 2026-10-11."
    );
    let depth = format!("{}a{}", "(".repeat(32), ")".repeat(32));
    assert!(Filter::parse(&depth).is_ok());
}

/// Texts that each give one error, for the message snapshot.
const MISTAKES: [&str; 42] = [
    "group imges",
    "group pictures",
    "",
    "size over MB",
    "size over",
    "sise over 10 MB",
    "size over 10 mbb",
    "size over 10 m",
    "size over 10k",
    "depth 1 KB",
    "size over 99999999999 TB",
    "size between 1 MB",
    "a and",
    "and a",
    "not",
    "(a or)",
    "a or b c",
    "(a or b",
    "a )",
    "()",
    "modified sometime",
    "modified before 10/11/2026",
    "modified 2026-02-29",
    "modified within 3",
    "modified 3 days",
    "modified within today",
    "modified last",
    "re:(a",
    "re:x(?=y)",
    "re:(.)\\1",
    "[abc",
    "*.txt | *.md",
    "a & b",
    "\"open",
    "content:",
    "hidden, empty",
    "case size over 1",
    "kind pipe",
    "state is open",
    "> 10",
    "photo:x",
    "ext",
];

/// Texts that read, but carry a lint, for the message snapshot.
const LINTED: [&str; 12] = [
    "*.txt *.md",
    "*.txt and *.md",
    "sise 10 MB",
    "modifed today",
    "kind folder and kind file",
    "ext pdf ext txt",
    "size over 10 MB and size under 1 MB",
    "hidden and not hidden",
    "modified today and modified last week",
    "label urgnt",
    "label zzz",
    "size between 5 MB and 1 MB",
];

/// The starter labels (plan question 71).
fn starter_labels() -> Vec<String> {
    [
        "Urgent",
        "Waiting",
        "Review",
        "Done",
        "Keep",
        "Skip in searches",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn every_message_snapshot() {
    let mut errors = String::new();
    for text in MISTAKES {
        let error = problem(text);
        errors.push_str(&format!("typed: {text}\nsaid: {}\n\n", error.sentences()));
        assert!(error::fits_one_line(&error.line()), "{}", error.line());
    }
    insta::assert_snapshot!("error_messages", errors);
    let labels = starter_labels();
    let context = LintContext { labels: &labels };
    let mut lints = String::new();
    for text in LINTED {
        let found = parse(text).lints(&context, &written());
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        for lint in found {
            lints.push_str(&format!("typed: {text}\nsaid: {}\n\n", lint.sentences()));
            assert!(
                lint.line().chars().count() <= crate::format::BRAILLE_CELLS,
                "{}",
                lint.line()
            );
        }
    }
    insta::assert_snapshot!("lint_messages", lints);
}

#[test]
fn lints_say_what_was_probably_meant() {
    let labels = starter_labels();
    let context = LintContext { labels: &labels };
    let lints = |text: &str| parse(text).lints(&context, &written());
    let kinds = |text: &str| -> Vec<LintKind> {
        lints(text).into_iter().map(|l| l.kind().clone()).collect()
    };
    assert_eq!(
        lints("*.txt *.md")[0].sentences(),
        "Both patterns must match one name. Use a comma or or."
    );
    assert_eq!(kinds("*.txt, *.md"), []);
    assert_eq!(kinds("IMG_* *.jpg"), []);
    assert_eq!(kinds("*.TXT *.txt"), [], "case ignored by one of them");
    assert_eq!(kinds("src/ *.md"), []);
    assert_eq!(
        kinds("sise 10 MB"),
        [LintKind::MaybeField {
            word: "sise".into(),
            field: Field::Size
        }]
    );
    assert_eq!(kinds("nmae report"), [], "report is not a value");
    assert_eq!(kinds("kind file and kind archive"), []);
    assert_eq!(kinds("size over 1 MB and size under 10 MB"), []);
    assert_eq!(
        kinds("label urgent and label done"),
        [],
        "an entry has many labels"
    );
    assert_eq!(kinds("label Urgent"), []);
    assert_eq!(kinds("hidden or not hidden"), []);
    assert_eq!(
        kinds("modified before 2026-01-01 and modified since 2026-06"),
        [LintKind::Contradiction]
    );
    assert_eq!(
        kinds("(kind folder and kind file) or empty"),
        [LintKind::Contradiction]
    );
    assert_eq!(
        lints("label urgnt")[0].sentences(),
        "No label named urgnt. Did you mean Urgent?"
    );
    assert_eq!(
        parse("label urgnt").lints(&LintContext::default(), &written()),
        []
    );
    assert_eq!(lints("kind folder and kind file")[0].word(), 1);
    // A depth test means nothing in a one-folder panel.
    let error = parse("depth at most 1 and *.rs").panel_problem().unwrap();
    assert_eq!(error.line(), "Filter problem: depth works in searches");
    assert_eq!(error.word(), 1);
    assert!(parse("*.rs").panel_problem().is_none());
}

#[test]
fn every_problem_line_fits_one_braille_line() {
    let fields = Field::ALL.map(Problem::NeedsValue);
    let others = [
        Problem::Empty,
        Problem::TooLong,
        Problem::TooDeep,
        Problem::UnclosedQuote,
        Problem::UnclosedBracket,
        Problem::ExtraBracket,
        Problem::EmptyBrackets,
        Problem::Pipe,
        Problem::Ampersand,
        Problem::NothingAfter(Keyword::And),
        Problem::NothingAfter(Keyword::Not),
        Problem::NothingBefore(Keyword::Or),
        Problem::MixedAndOr,
        Problem::UnknownField {
            word: "sise".into(),
            nearest: Some(Field::Size),
        },
        Problem::UnknownUnit("mbb".into()),
        Problem::BareUnit('t'),
        Problem::DepthHasNoUnit,
        Problem::NumberTooLarge,
        Problem::RangeNeedsAnd,
        Problem::DateNotUnderstood,
        Problem::YearFirst,
        Problem::NoSuchDay,
        Problem::NeedsTimeUnit,
        Problem::WithinOrOlder,
        Problem::NeedsDuration,
        Problem::BadRegex("unclosed group".into()),
        Problem::LookAround,
        Problem::Backreference,
        Problem::PatternTooLong,
        Problem::PatternTooComplex,
        Problem::GlobBracket,
        Problem::CommaNotHere,
        Problem::CaseNeedsText,
        Problem::UnknownKind("pipe".into()),
        Problem::UnknownGroup {
            word: "imges".into(),
            nearest: Some("images"),
        },
        Problem::StateIsSelected,
        Problem::DepthInPanel,
        Problem::SignNeedsField(">="),
    ];
    for problem in fields.into_iter().chain(others) {
        let error = FilterError::new(problem.clone(), 1, 0..0, Vec::new());
        let line = error.line();
        assert!(
            line.chars().count() <= crate::format::BRAILLE_CELLS,
            "{line} has {} cells",
            line.chars().count()
        );
        assert!(!error.next_step().is_empty(), "{problem:?}");
    }
}

/// Tests whose texts combine freely in the property tests.
pub(super) const TERMS: [&str; 48] = [
    "report",
    "Report",
    "*.log",
    "\"a b\"",
    "*.txt, *.md",
    "re:\"^IMG_\\d{4}\"",
    "re:x|y",
    "size over 100 MB",
    "size between 1 KiB and 1 MiB",
    "size>=1mb",
    "size!=0",
    "size 1.5 GB",
    "depth at most 1",
    "modified today",
    "modified last week",
    "modified within 3 days",
    "modified older than 1 year",
    "modified before 2026-10-01",
    "modified since 2026-10",
    "created 2026",
    "accessed between 2026-01-01 and 2026-06-30",
    "changed on or before 2026-10-01T14:30",
    "modified<2w",
    "ext pdf",
    "ext txt,md",
    "ext is not tmp",
    "kind folder",
    "kind is not link",
    "type PDF",
    "group images",
    "label urgent",
    "owner alice",
    "usergroup staff",
    "empty",
    "broken",
    "needs-care",
    "hidden",
    "read-only",
    "selected",
    "content:\"total due\"",
    "content:re:\"due \\d+\"",
    "path \"/work/\"",
    "name like \"rprt\"",
    "name starts with draft",
    "name ends with .bak",
    "case report",
    "anycase Report",
    "*.pdf, not draft*",
];

/// A filter text built from the terms: and, or, not and brackets, never
/// mixing and with or at one level.
fn texts() -> impl Strategy<Value = String> {
    let leaf = prop::sample::select(TERMS.to_vec()).prop_map(str::to_owned);
    leaf.prop_recursive(4, 24, 3, |inner| {
        (
            prop::collection::vec(inner, 2..4),
            prop::bool::ANY,
            prop::bool::ANY,
        )
            .prop_map(|(parts, or, not)| {
                let joined = parts
                    .iter()
                    .map(|p| format!("({p})"))
                    .collect::<Vec<_>>()
                    .join(if or { " or " } else { " and " });
                if not {
                    format!("not ({joined})")
                } else {
                    joined
                }
            })
    })
}

/// Property tests write no regression files into the crate: a failure
/// prints its minimal input instead.
pub(super) fn quiet() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(quiet())]

    #[test]
    fn parsing_the_read_back_gives_the_same_tree(text in texts()) {
        let filter = Filter::parse(&text).map_err(|e| TestCaseError::fail(e.sentences()))?;
        let sentence = filter.sentence();
        let again = Filter::parse(&sentence)
            .map_err(|e| TestCaseError::fail(format!("{sentence}: {}", e.sentences())))?;
        prop_assert_eq!(again.expr(), filter.expr(), "{} read back as {}", text, sentence);
        prop_assert_eq!(again.sentence(), sentence);
        // Said against the clock, with the real days, it is the same tree.
        let spoken = filter.read_back(&written()).sentence().to_owned();
        let again = Filter::parse(&spoken)
            .map_err(|e| TestCaseError::fail(format!("{spoken}: {}", e.sentences())))?;
        prop_assert_eq!(again.expr(), filter.expr(), "{} said as {}", text, spoken);
    }

    #[test]
    fn any_text_reads_or_says_why(text in "\\PC{0,60}") {
        // The same checks as the filter_parse fuzz target.
        for case in CaseRule::ALL {
            let options = FilterOptions { case, ..FilterOptions::default() };
            match Filter::parse_with(&text, &options) {
                Ok(filter) => {
                    let sentence = filter.sentence();
                    let again = Filter::parse_with(&sentence, &options)
                        .map_err(|e| TestCaseError::fail(format!("{text:?} read back as {sentence}: {}", e.sentences())))?;
                    prop_assert_eq!(again.expr(), filter.expr(), "{:?} read back as {}", text, sentence);
                }
                Err(error) => {
                    prop_assert!(error.line().starts_with(PROBLEM_START));
                    prop_assert!(error.span().end <= text.len());
                    prop_assert!(error.word() >= 1);
                }
            }
        }
    }

    #[test]
    fn filter_words_in_any_order_read_or_say_why(
        words in prop::collection::vec(
            prop::sample::select(vec![
                "and", "or", "not", "(", ")", ",", "size", "over", "10", "MB", "modified",
                "last", "week", "today", "ext", "pdf", "*.txt", "hidden", "kind", "folder",
                "\"x y\"", "re:a|b", "content", "between", "1", "to", "match", "case", "!",
                ">", "=", "within", "3", "days", "label", "group", "images", "October", "2026",
            ]),
            0..10,
        )
    ) {
        let text = words.join(" ");
        if let Ok(filter) = Filter::parse(&text) {
            let sentence = filter.sentence();
            let again = Filter::parse(&sentence)
                .map_err(|e| TestCaseError::fail(format!("{text} read back as {sentence}: {}", e.sentences())))?;
            prop_assert_eq!(again.expr(), filter.expr(), "{} read back as {}", text, sentence);
        }
    }
}
