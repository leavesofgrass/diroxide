//! Completion for the filter language (plan section 5.19): Tab offers
//! the words the parser expects at the cursor, from the same
//! expected-next list an error there names, so Tab and the error line
//! never disagree. After a field, its comparison words; after a
//! comparison, the units or date words; field names, yes-or-no words,
//! kinds, groups, types and labels where they fit.
//!
//! The shape is the app's entry model's: each candidate is the text
//! inserted and its kind said after it ("size, field"), and the result
//! says where the word being completed starts, so the field replaces only
//! that word. Candidates that start with what was typed come first, then
//! those that contain it, without regard to case.

use super::FilterOptions;
use super::dates::{DateWord, TimeUnit};
use super::error::{Expected, Keyword};
use super::groups::GROUPS;
use super::parse::expected_at_end;
use super::readback::{needs_quotes, quoted};
use super::tree::{Field, FieldShape, Flag, KIND_WORDS, SizeUnit};

/// One candidate: the text inserted when chosen, and what it is, said
/// after it ("over, comparison").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The text inserted.
    pub text: String,
    /// What it is, in a word or two: "field", "comparison", "unit".
    pub kind: String,
}

impl Candidate {
    fn new(text: impl Into<String>, kind: &str) -> Self {
        Self {
            text: text.into(),
            kind: kind.to_owned(),
        }
    }

    /// The candidate as said, the text first: "over, comparison".
    pub fn spoken(&self) -> String {
        format!("{}, {}", self.text, self.kind)
    }
}

/// What Tab can offer at the cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completions {
    /// Where the word being completed starts, in characters from the
    /// start of the text: the part from here to the cursor is replaced.
    pub start: usize,
    /// The word being completed, as typed (empty after a space).
    pub stem: String,
    /// The candidates, those that start with the stem first.
    pub candidates: Vec<Candidate>,
}

/// What completion offers beyond the language's own words: the labels in
/// use and the type names the type registry knows.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionContext<'a> {
    /// The labels in use.
    pub labels: &'a [String],
    /// The detected types' short names, such as PDF.
    pub types: &'a [String],
}

/// Characters that end the word being completed, besides spaces.
fn separates(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';' | '<' | '>' | '=' | ':' | '!' | '"')
}

/// The comparison words a field takes.
fn comparisons(field: Field) -> &'static [&'static str] {
    match field.shape() {
        FieldShape::Text => &[
            "is",
            "is not",
            "contains",
            "starts with",
            "ends with",
            "matches",
            "like",
            "regex",
        ],
        FieldShape::Kind => &["is", "is not"],
        FieldShape::Size | FieldShape::Count => &[
            "is", "is not", "over", "under", "at least", "at most", "between",
        ],
        FieldShape::Date => &[
            "on",
            "before",
            "since",
            "after",
            "between",
            "within",
            "older than",
            "on or before",
            "in",
        ],
        FieldShape::Content => &["contains", "regex"],
    }
}

/// A label or type as inserted: quoted when it has spaces or looks like
/// the language.
fn insertable(value: &str) -> String {
    if needs_quotes(value) {
        quoted(value)
    } else {
        value.to_owned()
    }
}

/// The candidates an expected-next list names.
fn candidates(expected: &[Expected], context: &CompletionContext<'_>) -> Vec<Candidate> {
    let mut out = Vec::new();
    for want in expected {
        match want {
            Expected::Term => {
                for field in Field::ALL {
                    out.push(Candidate::new(field.word(), "field"));
                    if field == Field::Extension {
                        out.push(Candidate::new("ext", "field"));
                    }
                }
                for flag in Flag::ALL {
                    out.push(Candidate::new(flag.word(), "yes or no word"));
                }
                out.push(Candidate::new("not", "word"));
            }
            Expected::Joiner(only) => {
                if *only != Some(Keyword::Or) {
                    out.push(Candidate::new("and", "word"));
                }
                if *only != Some(Keyword::And) {
                    out.push(Candidate::new("or", "word"));
                }
            }
            Expected::CloseBracket => out.push(Candidate::new(")", "closing bracket")),
            Expected::Comparison(field) => {
                for words in comparisons(*field) {
                    out.push(Candidate::new(*words, "comparison"));
                }
            }
            Expected::Value(field) => match field.shape() {
                FieldShape::Kind => {
                    for (word, _) in KIND_WORDS {
                        out.push(Candidate::new(word, "kind"));
                    }
                }
                FieldShape::Date => {
                    for word in DateWord::ALL {
                        out.push(Candidate::new(word.words(), "date word"));
                    }
                }
                _ => match field {
                    Field::Group => {
                        for group in GROUPS {
                            out.push(Candidate::new(group.name, "group"));
                        }
                    }
                    Field::Type => {
                        for name in context.types {
                            out.push(Candidate::new(insertable(name), "type"));
                        }
                    }
                    Field::Label => {
                        for label in context.labels {
                            out.push(Candidate::new(insertable(label), "label"));
                        }
                    }
                    _ => {}
                },
            },
            Expected::SizeUnit => {
                for unit in SizeUnit::ALL {
                    let word = if unit == SizeUnit::Bytes {
                        "bytes"
                    } else {
                        unit.symbol()
                    };
                    out.push(Candidate::new(word, "unit"));
                }
            }
            Expected::TimeUnit => {
                for unit in TimeUnit::ALL {
                    out.push(Candidate::new(unit.said(false), "time unit"));
                }
            }
            Expected::Period => {
                for period in ["week", "month", "year"] {
                    out.push(Candidate::new(period, "date word"));
                }
            }
            Expected::RangeJoiner => out.push(Candidate::new("and", "word")),
            Expected::CaseNote => {
                out.push(Candidate::new("match case", "case"));
                out.push(Candidate::new("any case", "case"));
            }
            Expected::Selected => out.push(Candidate::new("selected", "yes or no word")),
            Expected::Words(words) => {
                for word in *words {
                    out.push(Candidate::new(*word, "word"));
                }
            }
        }
    }
    out
}

/// The candidates that fit a stem: starting with it first, then
/// containing it, without regard to case or a label's quote.
fn fitting(all: Vec<Candidate>, stem: &str) -> Vec<Candidate> {
    let stem = stem.to_lowercase();
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for candidate in all {
        let text = candidate.text.trim_start_matches('"').to_lowercase();
        let seen = starting
            .iter()
            .chain(containing.iter())
            .any(|c: &Candidate| c.text == candidate.text);
        if seen {
            continue;
        }
        if text.starts_with(&stem) {
            starting.push(candidate);
        } else if !stem.is_empty() && text.contains(&stem) {
            containing.push(candidate);
        }
    }
    starting.extend(containing);
    starting
}

/// What Tab offers with the cursor at `cursor` characters into `text`.
/// Several-word candidates ("at least", "last week", "match case")
/// complete from their first word.
pub fn complete(
    text: &str,
    cursor: usize,
    options: &FilterOptions,
    context: &CompletionContext<'_>,
) -> Completions {
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(i, _)| i);
    let before = &text[..end];
    // Inside quotes the text is free; nothing is offered.
    if before.matches('"').count() % 2 == 1 {
        return Completions::default();
    }
    let stem_start = before
        .char_indices()
        .rev()
        .find(|(_, c)| separates(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    // Try the last word alone, then with the one or two words before it,
    // for the phrases of several words.
    let mut starts = vec![stem_start];
    let mut at = stem_start;
    // After a space the next word starts fresh.
    let reach = if stem_start == before.len() { 0 } else { 2 };
    for _ in 0..reach {
        let head = before[..at].trim_end();
        if head.len() == at || head.is_empty() {
            break;
        }
        let word_start = head
            .char_indices()
            .rev()
            .find(|(_, c)| separates(*c))
            .map_or(0, |(i, c)| i + c.len_utf8());
        if word_start == head.len() {
            break;
        }
        at = word_start;
        starts.push(at);
    }
    let mut best = Completions::default();
    for (n, &start) in starts.iter().enumerate() {
        let stem = &before[start..];
        let Some(expected) = expected_at_end(&before[..start], options) else {
            continue;
        };
        let found = fitting(candidates(&expected, context), stem);
        // A longer stem counts only when a phrase of several words
        // starts with it.
        let phrase = found
            .iter()
            .any(|c| c.text.to_lowercase().starts_with(&stem.to_lowercase()));
        if n == 0 || (phrase && stem.contains(' ')) {
            let chosen = Completions {
                start: before[..start].chars().count(),
                stem: stem.to_owned(),
                candidates: if n == 0 {
                    found
                } else {
                    found
                        .into_iter()
                        .filter(|c| c.text.to_lowercase().starts_with(&stem.to_lowercase()))
                        .collect()
                },
            };
            if n == 0 || !chosen.candidates.is_empty() {
                best = chosen;
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starters() -> Vec<String> {
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

    fn texts(found: &Completions) -> Vec<&str> {
        found.candidates.iter().map(|c| c.text.as_str()).collect()
    }

    fn offer(text: &str) -> Completions {
        let labels = starters();
        let types = ["PDF", "PNG", "Text"].map(str::to_owned).to_vec();
        let context = CompletionContext {
            labels: &labels,
            types: &types,
        };
        complete(
            text,
            text.chars().count(),
            &FilterOptions::default(),
            &context,
        )
    }

    #[test]
    fn partial_input_gives_the_words_the_error_would_name() {
        // (typed, the first candidates in order, where the replaced word
        // starts)
        let table: [(&str, &[&str], usize); 25] = [
            ("si", &["size"], 0),
            (
                "size ",
                &[
                    "is", "is not", "over", "under", "at least", "at most", "between",
                ],
                5,
            ),
            ("size ov", &["over"], 5),
            ("size at l", &["at least"], 5),
            (
                "size over 10 ",
                &["bytes", "KB", "MB", "GB", "TB", "KiB", "MiB", "GiB", "TiB"],
                13,
            ),
            ("size over 10 M", &["MB", "MiB", "modified"], 13),
            ("modified la", &["last week", "last month", "last year"], 9),
            ("modified last w", &["last week"], 9),
            ("modified last ", &["week", "month", "year"], 14),
            (
                "modified within 3 ",
                &[
                    "seconds", "minutes", "hours", "days", "weeks", "months", "years",
                ],
                18,
            ),
            ("modified o", &["on", "older than", "on or before"], 9),
            ("modified older t", &["older than"], 9),
            ("kind f", &["folder", "file"], 5),
            ("group i", &["is", "is not", "images"], 6),
            ("group im", &["images"], 6),
            ("label u", &["Urgent"], 6),
            ("label sk", &["\"Skip in searches\""], 6),
            ("type p", &["PDF", "PNG"], 5),
            ("ext:", &["is", "is not"], 4),
            ("content ", &["contains", "regex"], 8),
            ("*.TXT, match c", &["match case"], 7),
            ("(a or b ", &["or", ")"], 8),
            ("a and ", &["name"], 6),
            ("hid", &["hidden"], 0),
            ("state ", &["selected"], 6),
        ];
        for (text, first, start) in table {
            let found = offer(text);
            let got = texts(&found);
            assert!(
                got.len() >= first.len() && got[..first.len()] == *first,
                "{text:?} offered {got:?}"
            );
            assert_eq!(found.start, start, "{text:?}");
        }
        // After an or, only or: and would mix them without brackets.
        let found = offer("a or b ");
        assert_eq!(texts(&found), ["or"]);
        let found = offer("a b ");
        assert_eq!(texts(&found)[0], "and");
        assert!(!texts(&found).contains(&"or"));
        // The empty field offers every field and yes-or-no word.
        let found = offer("");
        assert_eq!(
            found.candidates.len(),
            Field::ALL.len() + 1 + Flag::ALL.len() + 1
        );
        assert_eq!(found.candidates[0].spoken(), "name, field");
        // A problem before the cursor offers nothing; so does a quote.
        assert!(offer("a | b ").candidates.is_empty());
        assert!(offer("name \"rep").candidates.is_empty());
        assert!(offer("*.txt, ").candidates.iter().all(|c| c.kind == "case"));
    }

    #[test]
    fn the_cursor_may_be_inside_the_text() {
        let labels = starters();
        let context = CompletionContext {
            labels: &labels,
            types: &[],
        };
        let found = complete("siz and hidden", 3, &FilterOptions::default(), &context);
        assert_eq!(texts(&found), ["size"]);
        assert_eq!((found.start, found.stem.as_str()), (0, "siz"));
        let found = complete("é siz", 5, &FilterOptions::default(), &context);
        assert_eq!(found.start, 2);
    }

    proptest::proptest! {
        #![proptest_config(super::super::tests::quiet())]

        /// Whatever Tab offers reads: inserting any candidate never makes
        /// the text wrong before its end.
        #[test]
        fn every_candidate_reads(
            words in proptest::collection::vec(
                proptest::sample::select(vec![
                    "and", "or", "not", "(", ")", ",", "size", "over", "10", "modified", "last",
                    "today", "ext", "pdf", "*.txt", "hidden", "kind", "folder", "content",
                    "between", "1", "and", "within", "3", "label", "group", "type", "state",
                    "at", "older", "is", "Report",
                ]),
                0..8,
            )
        ) {
            let typed = format!("{} ", words.join(" "));
            let found = offer(&typed);
            for candidate in &found.candidates {
                let text = format!("{typed}{} ", candidate.text);
                proptest::prop_assert!(
                    expected_at_end(&text, &FilterOptions::default()).is_some(),
                    "{:?} then {:?} does not read",
                    typed,
                    candidate.text
                );
            }
        }
    }

    #[test]
    fn the_error_and_tab_name_the_same_words() {
        use super::super::Filter;
        for (typed, wanted) in [
            ("size over", Expected::Value(Field::Size)),
            ("modified within 3", Expected::TimeUnit),
            ("modified last", Expected::Period),
            ("state", Expected::Selected),
        ] {
            let error = Filter::parse(typed).unwrap_err();
            assert!(
                error.expected().contains(&wanted),
                "{typed}: {:?}",
                error.expected()
            );
            let tab = expected_at_end(typed, &FilterOptions::default()).unwrap();
            assert!(tab.contains(&wanted), "{typed}: {tab:?}");
        }
    }
}
