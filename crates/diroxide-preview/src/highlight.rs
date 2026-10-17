//! Highlighting as a tokenizer (research/syntax-highlighting.md, R17):
//! syntect with its pure-Rust regex engine and bat's grammars (two-face)
//! reads a file's lines into token classes, words such as "keyword" and
//! "function name", never colors. Each frontend's theme turns a class
//! into a style with an attribute; the viewer's words-first commands say
//! the class ("Function name: parse_args, line 40"). Color is decoration,
//! never the only way to learn anything.
//!
//! The guardrails are part of the feature (R17): only the nine languages
//! of the allow-list ([`Language`]); only a file of at most 512 KiB,
//! 10,000 lines and no line over 2,000 characters ([`Caps`]), said in
//! words when it is over ("Highlighting off: over 512 KiB"); the parse on
//! a worker thread in chunks of [`CHUNK_LINES`] lines, with the cancel
//! and a deadline checked between chunks ([`Session::start`]); at most
//! one highlighter thread at a time; and a parse that overruns its
//! deadline pauses highlighting for the session ("Highlighting stopped:
//! took too long"), since a hang inside one line cannot be cancelled in
//! Rust. The syntax set loads on first use, on the worker.
//!
//! The syntax set is the `syntax-set` feature of this crate, on by
//! default; without it, highlighting says "Highlighting off: not in this
//! build" and the classes are never filled.

#[cfg(feature = "syntax-set")]
mod engine;
mod language;
mod scopes;
mod worker;

use std::time::{Duration, Instant};

pub use language::Language;
pub use scopes::{class_of_scopes, rule_of};
pub use worker::{Job, Session};

use crate::classes::ClassRun;

/// The lines one chunk of the parse takes, between two checks of the
/// cancel and the deadline.
pub const CHUNK_LINES: usize = 200;

/// The limits highlighting keeps (the `[viewer]` settings `highlight_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// The largest file, in bytes.
    pub max_bytes: u64,
    /// The most lines.
    pub max_lines: u64,
    /// The longest line, in characters.
    pub max_line_chars: u64,
    /// How long the parse may take before highlighting stops for the
    /// session.
    pub deadline: Duration,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            max_bytes: 512 * 1024,
            max_lines: 10_000,
            max_line_chars: 2_000,
            deadline: Duration::from_secs(2),
        }
    }
}

impl Caps {
    /// Whether lines of a file `bytes` long may be highlighted; the
    /// refusal says why not.
    pub fn check(&self, bytes: u64, lines: &[String]) -> Result<(), Refusal> {
        if bytes > self.max_bytes {
            return Err(Refusal::TooLarge {
                max_bytes: self.max_bytes,
            });
        }
        if lines.len() as u64 > self.max_lines {
            return Err(Refusal::TooManyLines {
                max: self.max_lines,
            });
        }
        if lines.iter().any(|l| {
            l.len() as u64 > self.max_line_chars && l.chars().count() as u64 > self.max_line_chars
        }) {
            return Err(Refusal::LineTooLong {
                max: self.max_line_chars,
            });
        }
        Ok(())
    }
}

/// Why a file is not highlighted. Each says so once, in words, inside 40
/// cells ([`Refusal::line`]); a language off the allow-list is plain text
/// and says nothing.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The file is larger than the cap.
    TooLarge {
        /// The cap, in bytes.
        max_bytes: u64,
    },
    /// The file has more lines than the cap.
    TooManyLines {
        /// The cap.
        max: u64,
    },
    /// A line is longer than the cap.
    LineTooLong {
        /// The cap, in characters.
        max: u64,
    },
    /// An earlier parse overran its deadline; highlighting waits for the
    /// next session.
    Paused,
    /// The parse overran its deadline now.
    TookTooLong,
    /// The build has no syntax set.
    NotInBuild,
    /// The syntax set has no grammar for the language.
    NoGrammar,
}

impl Refusal {
    /// The line to say, once, meaning first: "Highlighting off: over 512
    /// KiB", "Highlighting stopped: took too long".
    pub fn line(&self) -> String {
        match self {
            Refusal::TooLarge { max_bytes } => {
                format!("Highlighting off: over {}", kib_words(*max_bytes))
            }
            Refusal::TooManyLines { max } => format!(
                "Highlighting off: over {} lines",
                diroxide_core::format::grouped(*max)
            ),
            Refusal::LineTooLong { .. } => "Highlighting off: a line is too long".to_owned(),
            Refusal::Paused => "Highlighting paused for this session".to_owned(),
            Refusal::TookTooLong => "Highlighting stopped: took too long".to_owned(),
            Refusal::NotInBuild => "Highlighting off: not in this build".to_owned(),
            Refusal::NoGrammar => "Highlighting off: no grammar".to_owned(),
        }
    }
}

/// A cap in the units the settings use: "512 KiB", "2 MiB".
fn kib_words(bytes: u64) -> String {
    let kib = bytes / 1024;
    if kib >= 1024 && kib.is_multiple_of(1024) {
        format!("{} MiB", kib / 1024)
    } else {
        format!("{} KiB", diroxide_core::format::grouped(kib))
    }
}

/// A file's lines in token classes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlighted {
    /// For each line, its runs, in order; empty for a plain line.
    pub runs: Vec<Vec<ClassRun>>,
}

/// Whether this build has the syntax set.
pub const fn is_built() -> bool {
    cfg!(feature = "syntax-set")
}

/// Loads the syntax set now, if it is not loaded, and says how long the
/// load took (`None` without the syntax set). For the bench and spike
/// H2; the worker loads it on first use by itself.
pub fn load_syntax_set() -> Option<Duration> {
    #[cfg(feature = "syntax-set")]
    {
        engine::syntax_set();
        engine::load_time()
    }
    #[cfg(not(feature = "syntax-set"))]
    {
        None
    }
}

/// The names of the grammars in the syntax set: for the test that the
/// nine languages are all there.
pub fn grammar_names() -> Vec<String> {
    #[cfg(feature = "syntax-set")]
    {
        engine::syntax_set()
            .syntaxes()
            .iter()
            .map(|s| s.name.clone())
            .collect()
    }
    #[cfg(not(feature = "syntax-set"))]
    {
        Vec::new()
    }
}

/// Highlights lines now, on this thread, in chunks of [`CHUNK_LINES`],
/// stopping between chunks once `deadline` has passed. For the worker,
/// the tests and the fuzz targets; a frontend uses [`Session::start`].
pub fn highlight_lines(
    language: Language,
    lines: &[String],
    deadline: Option<Instant>,
) -> Result<Highlighted, Refusal> {
    highlight_with(language, lines, deadline, &|| false)
}

/// The same with a cancel check between chunks; `Ok` with fewer lines of
/// runs than lines when cancelled.
pub(crate) fn highlight_with(
    language: Language,
    lines: &[String],
    deadline: Option<Instant>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Highlighted, Refusal> {
    #[cfg(feature = "syntax-set")]
    {
        let mut parser = engine::LineParser::new(language, false).ok_or(Refusal::NoGrammar)?;
        let mut out = Highlighted {
            runs: Vec::with_capacity(lines.len()),
        };
        let mut failed = false;
        for chunk in lines.chunks(CHUNK_LINES) {
            if cancelled() {
                return Ok(out);
            }
            if deadline.is_some_and(|d| Instant::now() > d) {
                return Err(Refusal::TookTooLong);
            }
            for line in chunk {
                let parsed = if failed { None } else { parser.line(line) };
                match parsed {
                    Some(parsed) => out.runs.push(parsed.runs),
                    None => {
                        failed = true;
                        out.runs.push(Vec::new());
                    }
                }
            }
        }
        Ok(out)
    }
    #[cfg(not(feature = "syntax-set"))]
    {
        let _ = (language, lines, deadline, cancelled);
        Err(Refusal::NotInBuild)
    }
}

/// Every scope the grammar pushes on these lines, by name: for the tests
/// that check the scopes the mapper depends on.
pub fn scopes_of(language: Language, lines: &[String]) -> Vec<String> {
    #[cfg(feature = "syntax-set")]
    {
        let Some(mut parser) = engine::LineParser::new(language, true) else {
            return Vec::new();
        };
        let mut scopes: Vec<String> = lines
            .iter()
            .filter_map(|line| parser.line(line))
            .flat_map(|out| out.scopes)
            .collect();
        scopes.sort();
        scopes.dedup();
        scopes
    }
    #[cfg(not(feature = "syntax-set"))]
    {
        let _ = (language, lines);
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
