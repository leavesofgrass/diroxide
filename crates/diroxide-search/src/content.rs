//! Content search: the lines of a file that hold a filter's `content`
//! test, read by ripgrep's own searcher (`grep-searcher` with
//! `grep-regex`), set the way ripgrep reads the files it walks.
//!
//! - **Binary detection is set:** a NUL byte stops the read and the file
//!   is skipped as binary and counted (`BinaryDetection::quit`); a match
//!   before the NUL is not shown, as ripgrep shows none. A UTF-16 file
//!   without a byte-order mark has NUL bytes, so it is one.
//! - **Byte-order marks are sniffed:** a UTF-16 file with its mark (either
//!   order) is read as text; a UTF-8 mark is dropped.
//! - **The byte limit** (`first 64 KB`) stops the read after that many
//!   bytes of the file.
//! - **Lines** end at a line feed; a carriage return before it is not part
//!   of the line, and `$` matches before it. Line numbers start at one.
//!   `^` and `$` are the line's ends. A match never runs past its line: a
//!   class such as `\s` does not take the line feed.
//! - **Case** follows the filter: smart unless the test says.
//! - **No memory maps**, the searcher's default (ripgrep's own figures show
//!   them slower on many small files).

use std::io::{self, Read};
use std::ops::Range;

use diroxide_core::filter::ContentQuery;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use regex::bytes::{Regex, RegexBuilder};

use crate::hit::{ContentMatch, excerpt};

/// How many characters of a line a row shows, around the match.
pub(crate) const LINE_WIDTH: usize = 120;

/// How many matching lines one file gives in one-row-per-line results.
pub(crate) const MOST_LINES: usize = 10_000;

/// The size limit of a compiled pattern.
const PATTERN_LIMIT: usize = 10 * (1 << 20);

/// What reading a file found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// Matching lines: the first (or every one, up to the limit asked
    /// for) and how many there are.
    Lines {
        /// The lines kept.
        lines: Vec<ContentMatch>,
        /// Every matching line, kept or not.
        count: u64,
    },
    /// No line matches.
    Nothing,
    /// A NUL byte: skipped as binary.
    Binary,
}

/// One compiled content test, with its own searcher (one per walking
/// thread).
#[derive(Debug)]
pub(crate) struct Reader {
    matcher: RegexMatcher,
    /// The same pattern, to place the match inside a matching line.
    regex: Regex,
    searcher: Searcher,
    limit: Option<u64>,
}

impl Reader {
    /// The reader for a filter's content test.
    pub(crate) fn new(query: &ContentQuery) -> Result<Self, String> {
        let pattern = query.regex().as_str();
        let ignore_case = query.ignores_case();
        let mut builder = RegexMatcherBuilder::new();
        builder
            .case_insensitive(ignore_case)
            .multi_line(true)
            .crlf(true)
            .line_terminator(Some(b'\n'))
            .size_limit(PATTERN_LIMIT)
            .dfa_size_limit(PATTERN_LIMIT);
        // A pattern that names a line feed itself can never match inside a
        // line; without the line terminator the searcher reads it line by
        // line, where it finds nothing, as before.
        let matcher = match builder.build(pattern) {
            Ok(matcher) => matcher,
            Err(_) => builder
                .line_terminator(None)
                .build(pattern)
                .map_err(|error| error.to_string())?,
        };
        let regex = RegexBuilder::new(pattern)
            .case_insensitive(ignore_case)
            .multi_line(true)
            .crlf(true)
            .size_limit(PATTERN_LIMIT)
            .build()
            .map_err(|error| error.to_string())?;
        let searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .bom_sniffing(true)
            .line_number(true)
            .build();
        Ok(Self {
            matcher,
            regex,
            searcher,
            limit: query.limit(),
        })
    }

    /// Reads a file and says what it found, keeping at most `keep`
    /// matching lines, with the bytes read. `stop` is asked before each
    /// read, so a cancel (or a pause, which `stop` waits out) reaches
    /// inside a large file; a stopped read finds nothing.
    pub(crate) fn search(
        &mut self,
        reader: impl Read,
        keep: usize,
        stop: &dyn Fn() -> bool,
    ) -> io::Result<(Found, u64)> {
        let mut counted = Counted {
            inner: reader.take(self.limit.unwrap_or(u64::MAX)),
            read: 0,
            stop,
            stopped: false,
        };
        let mut lines = Lines {
            regex: &self.regex,
            keep: keep.max(1),
            kept: Vec::new(),
            count: 0,
            binary: false,
        };
        self.searcher
            .search_reader(&self.matcher, &mut counted, &mut lines)?;
        let found = if counted.stopped || (lines.count == 0 && !lines.binary) {
            Found::Nothing
        } else if lines.binary {
            Found::Binary
        } else {
            Found::Lines {
                lines: lines.kept,
                count: lines.count,
            }
        };
        Ok((found, counted.read))
    }
}

/// A reader that counts its bytes and ends early when asked to stop.
struct Counted<'a, R> {
    inner: R,
    read: u64,
    stop: &'a dyn Fn() -> bool,
    stopped: bool,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.stopped || (self.stop)() {
            self.stopped = true;
            return Ok(0);
        }
        let n = self.inner.read(buffer)?;
        self.read += n as u64;
        Ok(n)
    }
}

/// The searcher's sink: the matching lines kept, all of them counted,
/// and whether the file was binary.
struct Lines<'a> {
    regex: &'a Regex,
    keep: usize,
    kept: Vec<ContentMatch>,
    count: u64,
    binary: bool,
}

impl Sink for Lines<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        self.count += 1;
        if self.kept.len() < self.keep {
            let line = trim_line(found.bytes());
            let range = self.regex.find(line).map_or(0..0, |m| m.range());
            let number = found.line_number().unwrap_or(0);
            self.kept.push(matched_line(number, line, range));
        }
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> Result<bool, io::Error> {
        self.binary = true;
        Ok(false)
    }
}

/// A line without its line feed and the carriage return before it.
fn trim_line(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// A matching line for a row: its number, its text cut around the match,
/// where the match is in that text and in the whole line.
fn matched_line(number: u64, line: &[u8], range: Range<usize>) -> ContentMatch {
    let text = String::from_utf8_lossy(line);
    let before = String::from_utf8_lossy(&line[..range.start.min(line.len())]);
    let inside =
        String::from_utf8_lossy(&line[range.start.min(line.len())..range.end.min(line.len())]);
    let start = before.len();
    let (shown, matched) = excerpt(&text, start..start + inside.len(), LINE_WIDTH);
    let mut found = ContentMatch::new(number, shown);
    found.matched = matched;
    found.column = before.chars().count() as u64 + 1;
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A piece of a read, as the old reader read them.
    const CHUNK: usize = 64 * 1024;
    use diroxide_core::filter::{Filter, Test};

    fn reader(expression: &str) -> Reader {
        let filter = Filter::parse(expression).unwrap();
        let test = filter
            .expr()
            .tests()
            .into_iter()
            .find_map(|(test, _)| match test {
                Test::Content(_) => Some(()),
                _ => None,
            });
        assert!(test.is_some(), "{expression}");
        let query = content_query(&filter);
        Reader::new(&query).unwrap()
    }

    /// The content query the evaluator would pass, caught through a lookup.
    fn content_query(filter: &Filter) -> ContentQuery {
        use diroxide_core::filter::{Lookup, Truth, Unknown};
        use diroxide_core::{Entry, EntryKind, FixedClock, Timestamp, UtcOffset};
        use std::cell::RefCell;
        struct Catch(RefCell<Option<ContentQuery>>);
        impl Lookup for Catch {
            fn content(&self, _: &Entry, query: &ContentQuery) -> Truth {
                *self.0.borrow_mut() = Some(query.clone());
                Truth::Unknown(Unknown::ContentNotRead)
            }
        }
        let clock = FixedClock::new(Timestamp::from_unix(0, 0), UtcOffset::from_hours(0));
        let catch = Catch(RefCell::new(None));
        filter
            .evaluator(&clock)
            .test(&Entry::new("x", EntryKind::File), &catch);
        catch.0.into_inner().unwrap()
    }

    fn lines(found: &Found) -> Vec<(u64, String)> {
        match found {
            Found::Lines { lines, .. } => lines
                .iter()
                .map(|l| (l.line_number, l.text.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn utf16(text: &str, little: bool, mark: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if mark {
            bytes.extend(if little { [0xFF, 0xFE] } else { [0xFE, 0xFF] });
        }
        for unit in text.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        bytes
    }

    #[test]
    fn lines_are_numbered_and_case_is_smart() {
        let text = "first line\ntotal due here\nnothing\nTOTAL DUE caps\n";
        let (found, read) = reader("content \"total due\"")
            .search(text.as_bytes(), 100, &|| false)
            .unwrap();
        assert_eq!(read, text.len() as u64);
        assert_eq!(
            lines(&found),
            [(2, "total due here".into()), (4, "TOTAL DUE caps".into())]
        );
        let (found, _) = reader("content \"TOTAL DUE\"")
            .search(text.as_bytes(), 100, &|| false)
            .unwrap();
        assert_eq!(lines(&found), [(4, "TOTAL DUE caps".into())]);
        let (found, _) = reader("content \"total due\"")
            .search(text.as_bytes(), 1, &|| false)
            .unwrap();
        assert_eq!(
            found,
            Found::Lines {
                lines: vec![{
                    let mut m = ContentMatch::new(2, "total due here");
                    m.matched = 0..9;
                    m
                }],
                count: 2
            }
        );
    }

    #[test]
    fn utf16_with_a_mark_is_text_and_a_nul_is_binary() {
        for little in [true, false] {
            let bytes = utf16("alpha\ntotal due in utf16\n", little, true);
            let (found, _) = reader("content \"total due\"")
                .search(bytes.as_slice(), 10, &|| false)
                .unwrap();
            assert_eq!(lines(&found), [(2, "total due in utf16".into())]);
        }
        let bytes = utf16("total due without a mark\n", true, false);
        let (found, _) = reader("content \"total due\"")
            .search(bytes.as_slice(), 10, &|| false)
            .unwrap();
        assert_eq!(found, Found::Binary);
        let (found, _) = reader("content \"total due\"")
            .search(&b"total due\0\x01\x02"[..], 10, &|| false)
            .unwrap();
        assert_eq!(found, Found::Binary);
        let (found, _) = reader("content \"x\"")
            .search(&b""[..], 10, &|| false)
            .unwrap();
        assert_eq!(found, Found::Nothing);
        let mut marked = vec![0xEF, 0xBB, 0xBF];
        marked.extend(b"total due\n");
        let (found, _) = reader("content regex \"^total\"")
            .search(marked.as_slice(), 10, &|| false)
            .unwrap();
        assert_eq!(lines(&found), [(1, "total due".into())]);
    }

    #[test]
    fn crlf_lines_long_lines_and_the_byte_limit() {
        let crlf = b"one\r\ntotal due\r\nthree\r\n";
        let (found, _) = reader("content regex \"due$\"")
            .search(&crlf[..], 10, &|| false)
            .unwrap();
        assert_eq!(lines(&found), [(2, "total due".into())]);
        let long = format!(
            "{} total due {}\nshort\n",
            "x".repeat(300_000),
            "y".repeat(300_000)
        );
        let (found, _) = reader("content \"total due\"")
            .search(long.as_bytes(), 10, &|| false)
            .unwrap();
        let Found::Lines { lines: kept, count } = found else {
            panic!("no match in the long line");
        };
        assert_eq!(count, 1);
        assert_eq!(kept[0].line_number, 1);
        assert_eq!(&kept[0].text[kept[0].matched.clone()], "total due");
        assert!(kept[0].text.chars().count() <= LINE_WIDTH);
        assert_eq!(kept[0].column, 300_002);
        let text = b"first line\ntotal due\n";
        let (found, read) = reader("content \"total due\" first 10 B")
            .search(&text[..], 10, &|| false)
            .unwrap();
        assert_eq!((found, read), (Found::Nothing, 10));
        let (found, _) = reader("content \"total due\" first 20 B")
            .search(&text[..], 10, &|| false)
            .unwrap();
        assert_eq!(lines(&found), [(2, "total due".into())]);
    }

    #[test]
    fn a_stop_reaches_inside_a_large_file() {
        let big = "x".repeat(CHUNK * 8);
        let asked = std::cell::Cell::new(0);
        let (found, read) = reader("content \"y\"")
            .search(big.as_bytes(), 10, &|| {
                asked.set(asked.get() + 1);
                asked.get() > 1
            })
            .unwrap();
        assert_eq!(found, Found::Nothing);
        assert!(read <= (CHUNK * 2) as u64, "{read}");
    }

    #[test]
    fn a_match_across_a_line_feed_needs_its_line() {
        let text = b"total\ndue\ntotal due\n";
        let (found, _) = reader("content regex \"total\\sdue\"")
            .search(&text[..], 10, &|| false)
            .unwrap();
        assert_eq!(lines(&found), [(3, "total due".into())]);
        let many: String = (1..=5000).map(|n| format!("line {n}\n")).collect();
        let (found, _) = reader("content regex \"^line 4999$\"")
            .search(many.as_bytes(), 10, &|| false)
            .unwrap();
        assert_eq!(lines(&found), [(4999, "line 4999".into())]);
    }
}
