//! The highlighter's guardrails, and spike H2 (research/alpha2-plan.md
//! 4.7): the nine samples under the worker with a two-second deadline,
//! arbitrary text per language with no hang past the deadline, the
//! pause-for-the-session path, and the syntax set's first-use load time
//! (printed with `--no-capture`). `DIRO_FUZZ_SECONDS=60` runs the fuzz
//! pass for that long in all; without it, a short pass.

// Without the syntax set most of these tests are compiled out.
#![cfg_attr(not(feature = "syntax-set"), allow(unused_imports, dead_code))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

use super::*;
use crate::classes::runs_fit;

/// The sample of each language in `fixtures/viewer/code/`.
pub(super) const SAMPLES: [(Language, &str); 9] = [
    (Language::Markdown, "report.md"),
    (Language::Json, "data.json"),
    (Language::Toml, "config.toml"),
    (Language::Yaml, "ci.yaml"),
    (Language::Shell, "build.sh"),
    (Language::Rust, "main.rs"),
    (Language::Python, "sample.py"),
    (Language::Html, "page.html"),
    (Language::Css, "site.css"),
];

pub(super) fn sample_lines(name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/viewer/code")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines().map(str::to_owned).collect()
}

#[test]
fn the_caps_say_why_a_file_is_plain_in_40_cells() {
    let caps = Caps::default();
    let small = vec!["fn main() {}".to_owned()];
    assert_eq!(caps.check(12, &small), Ok(()));
    let refusal = caps.check(600 * 1024, &small).unwrap_err();
    assert_eq!(refusal.line(), "Highlighting off: over 512 KiB");
    let many = vec![String::new(); 10_001];
    assert_eq!(
        caps.check(10_001, &many).unwrap_err().line(),
        "Highlighting off: over 10,000 lines"
    );
    let long = vec!["x".repeat(2_001)];
    assert_eq!(
        caps.check(2_001, &long).unwrap_err().line(),
        "Highlighting off: a line is too long"
    );
    let accents = vec!["\u{e9}".repeat(1_500)];
    assert_eq!(caps.check(3_000, &accents), Ok(()), "characters, not bytes");
    for refusal in [
        Refusal::TooLarge { max_bytes: 2 << 20 },
        Refusal::TooManyLines { max: 10_000 },
        Refusal::LineTooLong { max: 2_000 },
        Refusal::Paused,
        Refusal::TookTooLong,
        Refusal::NotInBuild,
        Refusal::NoGrammar,
    ] {
        let line = refusal.line();
        assert!(line.chars().count() <= 40, "{line}");
        assert!(line.starts_with("Highlighting "), "{line}");
    }
    assert_eq!(
        Refusal::TooLarge { max_bytes: 2 << 20 }.line(),
        "Highlighting off: over 2 MiB"
    );
}

#[cfg(feature = "syntax-set")]
#[test]
fn the_syntax_set_has_all_nine_grammars() {
    let names = grammar_names();
    for language in Language::ALL {
        assert!(
            names.iter().any(|n| n == language.syntax_name()),
            "{} ({}) is missing",
            language.name(),
            language.syntax_name()
        );
    }
}

#[cfg(feature = "syntax-set")]
#[test]
fn spike_h2_each_sample_highlights_inside_the_deadline() {
    let load = load_syntax_set().unwrap_or_default();
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    eprintln!("H2: syntax set first-use load {load:?} ({profile})");
    let session = Session::new();
    for (language, name) in SAMPLES {
        let lines = Arc::new(sample_lines(name));
        let bytes: u64 = lines.iter().map(|l| l.len() as u64 + 1).sum();
        let started = Instant::now();
        let mut job = session
            .start(language, Arc::clone(&lines), bytes, Caps::default())
            .unwrap();
        let done = job
            .wait()
            .unwrap_or_else(|r| panic!("{name}: {}", r.line()));
        eprintln!("H2: {name} in {:?}", started.elapsed());
        assert_eq!(done.runs.len(), lines.len(), "{name}");
        for (line, runs) in lines.iter().zip(&done.runs) {
            assert!(runs_fit(line, runs), "{name}: {line:?} {runs:?}");
        }
        assert!(
            done.runs.iter().any(|r| !r.is_empty()),
            "{name} has some classes"
        );
    }
    assert!(!session.is_paused());
}

#[cfg(feature = "syntax-set")]
#[test]
fn a_parse_past_its_deadline_pauses_the_session() {
    let session = Session::new();
    let lines: Arc<Vec<String>> = Arc::new(
        (0..3_000)
            .map(|i| format!("fn f{i}() {{ let x = \"{i}\"; }}"))
            .collect(),
    );
    let caps = Caps {
        deadline: Duration::from_nanos(1),
        ..Caps::default()
    };
    let mut job = session
        .start(Language::Rust, Arc::clone(&lines), 100_000, caps)
        .unwrap();
    let refusal = job.wait().unwrap_err();
    assert_eq!(refusal.line(), "Highlighting stopped: took too long");
    assert!(session.is_paused());
    let again = session
        .start(Language::Rust, lines, 100_000, Caps::default())
        .unwrap_err();
    assert_eq!(again.line(), "Highlighting paused for this session");
    assert!(!Session::new().is_paused(), "a new session starts fresh");
}

#[cfg(feature = "syntax-set")]
#[test]
fn a_cancelled_job_ends_without_runs() {
    let session = Session::new();
    let lines: Arc<Vec<String>> = Arc::new(vec!["x = 1".to_owned(); 5_000]);
    let job = session
        .start(Language::Python, lines, 30_000, Caps::default())
        .unwrap();
    job.cancel();
    drop(job);
    let mut next = session
        .start(
            Language::Toml,
            Arc::new(vec!["a = 1".to_owned()]),
            6,
            Caps::default(),
        )
        .unwrap();
    assert_eq!(next.wait().unwrap().runs.len(), 1, "the next one runs");
}

#[cfg(not(feature = "syntax-set"))]
#[test]
fn without_the_syntax_set_it_says_so() {
    let refusal = Session::new()
        .start(Language::Rust, Arc::new(Vec::new()), 0, Caps::default())
        .unwrap_err();
    assert_eq!(refusal.line(), "Highlighting off: not in this build");
}

/// The text the fuzz pass feeds: what grammars react to, in any order.
fn code_like() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "(", ")", "[", "]", "{", "}", "<", ">", "\"", "'", "`", "\\", "/", "*", "#", "?", ":", ";",
        ",", ".", "=", "!", "-", "+", "|", "&", "$", "%", "@", "~", "^", " ", "\t", "\n", "a",
        "fn", "def", "let", "i", "0", "1", "x", "//", "/*", "*/", "<!--", "-->", "```", "#!",
        "\u{e9}", "\u{65e5}", "\r",
    ]);
    prop::collection::vec(pieces, 0..120).prop_map(|parts| parts.concat())
}

#[cfg(feature = "syntax-set")]
#[test]
fn spike_h2_arbitrary_text_never_hangs_past_the_deadline() {
    let seconds: u64 = std::env::var("DIRO_FUZZ_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let budget = Duration::from_secs(seconds);
    let started = Instant::now();
    let session = Session::new();
    let mut cases = 0u64;
    for (round, language) in Language::ALL.into_iter().enumerate() {
        let mut runner = TestRunner::new_with_rng(
            Config {
                failure_persistence: None,
                ..Config::default()
            },
            TestRng::from_seed(RngAlgorithm::ChaCha, &[round as u8 + 1; 32]),
        );
        let share = budget / Language::ALL.len() as u32;
        let until = started + share * (round as u32 + 1);
        let mut here = 0u32;
        loop {
            let text = code_like().new_tree(&mut runner).unwrap().current();
            let lines: Arc<Vec<String>> = Arc::new(text.lines().map(str::to_owned).collect());
            let bytes = text.len() as u64;
            let mut job = session
                .start(language, lines, bytes, Caps::default())
                .unwrap();
            match job.wait() {
                Ok(_) => {}
                Err(refusal) => panic!("{}: {} on {text:?}", language.name(), refusal.line()),
            }
            cases += 1;
            here += 1;
            if Instant::now() >= until && here >= 8 {
                break;
            }
        }
    }
    eprintln!(
        "H2: fuzz pass, {cases} inputs over the nine languages in {:?}, no hang",
        started.elapsed()
    );
    assert!(!session.is_paused());
}

#[test]
fn the_default_build_brings_no_c_regex_engine_and_no_yaml_or_plist_loader() {
    let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let text = std::fs::read_to_string(&lock).unwrap();
    for name in ["onig", "onig_sys", "yaml-rust", "plist"] {
        let line = format!("name = \"{name}\"");
        assert!(
            !text.lines().any(|l| l.trim() == line),
            "{name} is in the lock file"
        );
    }
}

#[cfg(feature = "syntax-set")]
#[test]
fn each_sample_has_the_scopes_the_mapper_depends_on() {
    let expected: [(&str, &[&str]); 9] = [
        (
            "report.md",
            &[
                "markup.heading",
                "entity.name.section",
                "markup.bold",
                "markup.raw",
                "markup.underline.link",
            ],
        ),
        (
            "data.json",
            &[
                "meta.mapping.key",
                "string.quoted",
                "constant.numeric",
                "constant.language",
            ],
        ),
        (
            "config.toml",
            &[
                "comment",
                "entity.name.table",
                "entity.name.tag.toml",
                "string.quoted",
                "constant.numeric",
            ],
        ),
        (
            "ci.yaml",
            &["comment", "entity.name.tag.yaml", "string.unquoted"],
        ),
        (
            "build.sh",
            &[
                "comment",
                "entity.name.function",
                "keyword.control",
                "string.quoted",
                "variable.function",
            ],
        ),
        (
            "main.rs",
            &[
                "comment",
                "entity.name.function",
                "entity.name.struct",
                "entity.name.enum",
                "entity.name.impl",
                "entity.name.module",
                "storage.type",
                "keyword",
                "string.quoted",
                "constant.numeric",
                "punctuation.section.block",
            ],
        ),
        (
            "sample.py",
            &[
                "comment",
                "entity.name.function",
                "entity.name.class",
                "keyword",
                "string.quoted",
            ],
        ),
        (
            "page.html",
            &[
                "comment",
                "entity.name.tag",
                "entity.other.attribute-name",
                "string.quoted",
            ],
        ),
        (
            "site.css",
            &[
                "comment",
                "entity.name.tag",
                "entity.other.attribute-name",
                "support.type.property-name",
                "constant.numeric",
            ],
        ),
    ];
    for ((language, name), (sample, wanted)) in SAMPLES.into_iter().zip(expected) {
        assert_eq!(name, sample);
        let scopes = scopes_of(language, &sample_lines(name));
        for prefix in wanted {
            assert!(
                scopes
                    .iter()
                    .any(|s| s == prefix || s.starts_with(&format!("{prefix}."))),
                "{name} has no {prefix} scope"
            );
        }
    }
}

#[cfg(feature = "syntax-set")]
#[test]
fn each_sample_yields_the_kinds_it_holds() {
    use crate::classes::TokenClass as C;
    let expected: [&[C]; 9] = [
        &[C::Heading, C::Emphasis, C::Code, C::Link, C::Comment],
        &[
            C::Attribute,
            C::String,
            C::Number,
            C::Constant,
            C::Punctuation,
        ],
        &[C::Comment, C::Attribute, C::Heading, C::String, C::Number],
        &[C::Comment, C::Attribute, C::String],
        &[
            C::Comment,
            C::FunctionName,
            C::Keyword,
            C::String,
            C::Variable,
        ],
        &[
            C::Comment,
            C::Keyword,
            C::FunctionName,
            C::TypeName,
            C::String,
            C::Number,
            C::Operator,
            C::Punctuation,
        ],
        &[
            C::Comment,
            C::Keyword,
            C::FunctionName,
            C::TypeName,
            C::String,
            C::Number,
        ],
        &[C::Comment, C::Tag, C::Attribute, C::String],
        &[C::Comment, C::Tag, C::Attribute, C::Number],
    ];
    for ((language, name), kinds) in SAMPLES.into_iter().zip(expected) {
        let lines = sample_lines(name);
        let done = highlight_lines(language, &lines, None).unwrap();
        let seen: Vec<C> = done.runs.iter().flatten().map(|r| r.class).collect();
        for kind in kinds {
            assert!(seen.contains(kind), "{name} has no {}", kind.word());
        }
    }
}

#[cfg(feature = "syntax-set")]
#[test]
fn code_symbols_come_from_the_grammar_in_file_order() {
    let rows = |language, name| -> Vec<String> {
        highlight_lines(language, &sample_lines(name), None)
            .unwrap()
            .symbols
            .iter()
            .map(Symbol::row)
            .collect()
    };
    assert_eq!(
        rows(Language::Rust, "main.rs"),
        [
            "type Config, line 8",
            "type Problem, line 14",
            "impl Config, line 19",
            "function parse_args, line 21",
            "module greeting, line 37",
            "function hello, line 39",
            "function main, line 47",
        ]
    );
    assert_eq!(
        rows(Language::Python, "sample.py"),
        [
            "type Counter, line 9",
            "function __init__, line 12",
            "function add, line 15",
            "function main, line 21",
        ]
    );
    assert_eq!(
        rows(Language::Shell, "build.sh"),
        ["function greet, line 5", "function clean_up, line 10"]
    );
    assert_eq!(
        rows(Language::Markdown, "report.md").len(),
        5,
        "from pulldown-cmark"
    );
}
