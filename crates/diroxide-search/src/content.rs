//! Content search: the lines of a file that hold a filter's `content`
//! test, read the way ripgrep reads them.
//!
//! - **Binary detection is set:** a NUL byte stops the read and the file
//!   is skipped as binary and counted, as ripgrep does for the files it
//!   walks. A UTF-16 file without a byte-order mark has NUL bytes, so it
//!   is one.
//! - **Byte-order marks are sniffed:** a UTF-16 file with its mark (either
//!   order) is read as text, through `encoding_rs`; a UTF-8 mark is
//!   dropped.
//! - **The byte limit** (`first 64 KB`) stops the read after that many
//!   bytes of the file.
//! - **Lines** end at a line feed; a carriage return before it is not part
//!   of the line, and `$` matches before it. Line numbers start at one.
//!   `^` and `$` are the line's ends.
//! - **Case** follows the filter: smart unless the test says.
//!
//! The plan names ripgrep's `grep-searcher` for this. Two of its own
//! dependencies are not in this machine's cache, and downloading is
//! outside this lane's brief, so the reader is built on the `regex`
//! crate's byte matcher and `encoding_rs` with the same rules; the swap is
//! one module (the hand-back's contract change request).

use std::io::{self, Read};
use std::ops::Range;

use diroxide_core::filter::ContentQuery;
use encoding_rs::{CoderResult, Decoder, UTF_16BE, UTF_16LE};
use regex::bytes::{Regex, RegexBuilder};

use crate::hit::{ContentMatch, excerpt};

/// How much is read at a time.
const CHUNK: usize = 64 * 1024;

/// The longest line held whole: a longer one is searched in pieces of
/// this size, so one huge line cannot hold all of memory.
const LONGEST_LINE: usize = 16 * 1024 * 1024;

/// How many characters of a line a row shows, around the match.
pub(crate) const LINE_WIDTH: usize = 120;

/// How many matching lines one file gives in one-row-per-line results.
pub(crate) const MOST_LINES: usize = 10_000;

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

/// One compiled content test.
#[derive(Debug)]
pub(crate) struct Reader {
    regex: Regex,
    limit: Option<u64>,
}

impl Reader {
    /// The reader for a filter's content test.
    pub(crate) fn new(query: &ContentQuery) -> Result<Self, regex::Error> {
        let regex = RegexBuilder::new(query.regex().as_str())
            .case_insensitive(query.ignores_case())
            .multi_line(true)
            .crlf(true)
            .size_limit(10 * (1 << 20))
            .build()?;
        Ok(Self {
            regex,
            limit: query.limit(),
        })
    }

    /// Reads a file and says what it found, keeping at most `keep`
    /// matching lines, with the bytes read. `stop` is asked before each
    /// piece, so a cancel (or a pause, which `stop` waits out) reaches
    /// inside a large file; a stopped read finds nothing.
    pub(crate) fn search(
        &self,
        reader: impl Read,
        keep: usize,
        stop: &dyn Fn() -> bool,
    ) -> io::Result<(Found, u64)> {
        let mut source = Source::new(reader.take(self.limit.unwrap_or(u64::MAX)))?;
        let mut lines = Lines::new(&self.regex, keep.max(1));
        let mut buffer = Vec::with_capacity(CHUNK * 2);
        loop {
            if stop() {
                return Ok((Found::Nothing, source.read));
            }
            let more = source.fill(&mut buffer)?;
            if buffer.contains(&0) {
                return Ok((Found::Binary, source.read));
            }
            let end = match buffer.iter().rposition(|b| *b == b'\n') {
                Some(last) => last + 1,
                None if !more || buffer.len() >= LONGEST_LINE => buffer.len(),
                None => continue,
            };
            let end = if more { end } else { buffer.len() };
            lines.scan(&buffer[..end]);
            buffer.drain(..end);
            if !more {
                break;
            }
        }
        Ok((lines.found(), source.read))
    }
}

/// The bytes of a file as UTF-8 where a byte-order mark says UTF-16, and
/// as they are otherwise.
struct Source<R> {
    inner: R,
    decoder: Option<Decoder>,
    pending: Vec<u8>,
    read: u64,
    done: bool,
}

impl<R: Read> Source<R> {
    /// Reads the first piece and sniffs the byte-order mark in it: one
    /// read for a small file, never a byte at a time.
    fn new(mut inner: R) -> io::Result<Self> {
        let mut head = vec![0u8; CHUNK];
        let mut filled = 0;
        while filled < 3 {
            match inner.read(&mut head[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        head.truncate(filled);
        let read = filled as u64;
        let (decoder, pending) = if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (None, head[3..].to_vec())
        } else if head.starts_with(&[0xFF, 0xFE]) {
            (
                Some(UTF_16LE.new_decoder_without_bom_handling()),
                head[2..].to_vec(),
            )
        } else if head.starts_with(&[0xFE, 0xFF]) {
            (
                Some(UTF_16BE.new_decoder_without_bom_handling()),
                head[2..].to_vec(),
            )
        } else {
            (None, head)
        };
        Ok(Self {
            inner,
            decoder,
            pending,
            read,
            done: false,
        })
    }

    /// Appends the next piece to `out`; returns whether more may follow.
    fn fill(&mut self, out: &mut Vec<u8>) -> io::Result<bool> {
        if self.done {
            return Ok(false);
        }
        let mut raw = std::mem::take(&mut self.pending);
        let start = raw.len();
        raw.resize(start + CHUNK, 0);
        let n = loop {
            match self.inner.read(&mut raw[start..]) {
                Ok(n) => break n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        };
        raw.truncate(start + n);
        self.read += n as u64;
        let last = n == 0;
        match &mut self.decoder {
            None => out.extend_from_slice(&raw),
            Some(decoder) => {
                let mut src = raw.as_slice();
                loop {
                    let room = decoder
                        .max_utf8_buffer_length(src.len())
                        .unwrap_or(src.len() * 3 + 16);
                    let at = out.len();
                    out.resize(at + room, 0);
                    let (result, used, written, _) =
                        decoder.decode_to_utf8(src, &mut out[at..], last);
                    out.truncate(at + written);
                    src = &src[used..];
                    if result == CoderResult::InputEmpty {
                        break;
                    }
                }
            }
        }
        if last {
            self.done = true;
        }
        Ok(!last)
    }
}

/// The matching lines found so far.
struct Lines<'a> {
    regex: &'a Regex,
    keep: usize,
    kept: Vec<ContentMatch>,
    count: u64,
    /// Lines before the piece being scanned.
    line: u64,
}

impl<'a> Lines<'a> {
    fn new(regex: &'a Regex, keep: usize) -> Self {
        Self {
            regex,
            keep,
            kept: Vec::new(),
            count: 0,
            line: 0,
        }
    }

    /// Scans a piece that holds whole lines (the last may lack its line
    /// feed at the end of the file).
    fn scan(&mut self, piece: &[u8]) {
        let mut at = 0;
        let mut counted = 0;
        while at < piece.len() {
            let Some(found) = self.regex.find_at(piece, at) else {
                break;
            };
            let start = piece[..found.start()]
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(0, |i| i + 1)
                .max(at);
            let end = piece[found.start()..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(piece.len(), |i| found.start() + i);
            let line = trim_cr(&piece[start..end]);
            // A match that runs past its line's end (a pattern that can
            // match a line feed) counts only if the line matches alone.
            let within = if found.end() <= start + line.len() {
                Some(found.start() - start..found.end() - start)
            } else {
                self.regex.find(line).map(|m| m.range())
            };
            if let Some(range) = within {
                self.line += 1 + count_lines(&piece[counted..start]);
                counted = (end + 1).min(piece.len());
                self.count += 1;
                if self.kept.len() < self.keep {
                    self.kept.push(matched_line(self.line, line, range));
                }
            }
            at = end + 1;
        }
        self.line += count_lines(&piece[counted.min(piece.len())..]);
    }

    fn found(self) -> Found {
        if self.count == 0 {
            Found::Nothing
        } else {
            Found::Lines {
                lines: self.kept,
                count: self.count,
            }
        }
    }
}

fn count_lines(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|b| **b == b'\n').count() as u64
}

fn trim_cr(line: &[u8]) -> &[u8] {
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
