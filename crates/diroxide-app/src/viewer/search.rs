//! Search in the viewer: what was typed read by one grammar, the whole
//! file scanned once in slices for every match (so the count is real),
//! then next and previous over the matches with wrap-around, each said
//! with its number and line: "Match 3 of 12, line 120".
//!
//! The grammar, after Midnight Commander's and Double Commander's:
//!
//! - plain text is a regular expression with smart case (case matters
//!   only when it has a capital), with `^` and `$` at each line's start
//!   and end;
//! - text in double quotes is matched as it is, with smart case;
//! - `hex:` and hex byte pairs (`hex: 48 65 6c`, `hex:48656c`) match
//!   bytes; in hex mode bare byte pairs do, and quoted text there adds
//!   its bytes (`48 65 "llo"`).

use std::io;
use std::ops::Range;

use diroxide_core::Message;
use diroxide_core::format::grouped;
use diroxide_core::pattern::{CaseRule, compile_regex};
use regex::Regex;

use super::source::ViewerSource;
use super::text::{Newline, TextForm, neutralize};

/// How much one read of a search takes.
pub(crate) const SEARCH_CHUNK: usize = 1024 * 1024;

/// The most matches kept; past it the count says "more than".
pub const MAX_HITS: usize = 100_000;

/// What to look for.
#[derive(Clone, Debug)]
pub enum Query {
    /// A regular expression over the decoded text.
    Pattern(Regex),
    /// Bytes, matched in the raw file.
    Bytes(Vec<u8>),
}

impl PartialEq for Query {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Query::Pattern(a), Query::Pattern(b)) => a.as_str() == b.as_str(),
            (Query::Bytes(a), Query::Bytes(b)) => a == b,
            _ => false,
        }
    }
}

impl Query {
    /// Reads what was typed; `hex_mode` when the viewer shows hex. The
    /// error is the line to say: "Pattern needs care: bad expression".
    pub fn parse(typed: &str, hex_mode: bool) -> Result<Self, String> {
        let text = typed.trim();
        if text.is_empty() {
            return Err("Type something to search for".to_owned());
        }
        if let Some(rest) = text
            .strip_prefix("hex:")
            .or_else(|| text.strip_prefix("HEX:"))
        {
            return hex_bytes(rest)
                .map(Query::Bytes)
                .ok_or_else(|| format!("Not hex bytes: {}", rest.trim()));
        }
        if hex_mode {
            return hex_bytes(text)
                .map(Query::Bytes)
                .ok_or_else(|| format!("Not hex bytes: {text}"));
        }
        let quoted = text.len() >= 2 && text.starts_with('"') && text.ends_with('"');
        let (source, plain) = if quoted {
            let inner = &text[1..text.len() - 1];
            (regex::escape(inner), inner.to_owned())
        } else {
            (text.to_owned(), text.to_owned())
        };
        let case = if quoted {
            CaseRule::Smart.resolve(&plain)
        } else {
            CaseRule::Smart.resolve_regex(&source)
        };
        compile_regex(&format!("(?mR){source}"), case)
            .map(Query::Pattern)
            .map_err(|error| error.line().to_owned())
    }

    /// The matches in one shown line: byte ranges of its text.
    pub fn ranges_in(&self, line: &str) -> Vec<Range<usize>> {
        match self {
            Query::Pattern(regex) => regex
                .find_iter(line)
                .filter(|m| !m.is_empty())
                .map(|m| m.range())
                .collect(),
            Query::Bytes(bytes) => {
                let hay = line.as_bytes();
                let mut found = Vec::new();
                let mut at = 0;
                while let Some(i) = find(&hay[at..], bytes) {
                    let start = at + i;
                    found.push(start..start + bytes.len());
                    at = start + bytes.len().max(1);
                }
                found
            }
        }
    }
}

/// Hex byte pairs, spaced or not, with quoted text adding its bytes.
fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut digits = String::new();
    let mut chars = text.trim().chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if !digits.is_empty() {
                    return None;
                }
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some(c) => inner.push(c),
                        None => return None,
                    }
                }
                bytes.extend(inner.as_bytes());
            }
            c if c.is_ascii_hexdigit() => {
                digits.push(c);
                if digits.len() == 2 {
                    bytes.push(u8::from_str_radix(&digits, 16).ok()?);
                    digits.clear();
                }
            }
            c if c.is_whitespace() || c == ',' => {
                if digits.len() == 1 {
                    bytes.push(u8::from_str_radix(&digits, 16).ok()?);
                    digits.clear();
                }
            }
            _ => return None,
        }
    }
    if digits.len() == 1 {
        bytes.push(u8::from_str_radix(&digits, 16).ok()?);
    }
    (!bytes.is_empty()).then_some(bytes)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One match: its line, where it starts in the line's shown text, and
/// the byte of the file it starts at (for a text search, where the chunk
/// that holds it starts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Hit {
    pub line: u64,
    pub column: usize,
    pub offset: u64,
}

/// A search: its query, the scan's progress, and the matches.
#[derive(Clone, Debug)]
pub(crate) struct Search {
    pub typed: String,
    pub query: Query,
    pub hits: Vec<Hit>,
    pub overflow: bool,
    pub done: bool,
    pub current: Option<usize>,
    /// Where the scan reads next, and the line there.
    at: u64,
    line: u64,
    /// Bytes kept from the last chunk of a bytes search.
    carry: Vec<u8>,
}

impl Search {
    pub(crate) fn new(typed: &str, query: Query, start: u64) -> Self {
        Self {
            typed: typed.trim().to_owned(),
            query,
            hits: Vec::new(),
            overflow: false,
            done: false,
            current: None,
            at: start,
            line: 0,
            carry: Vec::new(),
        }
    }

    /// Scans on for at most `budget` bytes; whether the scan is done.
    pub(crate) fn scan(
        &mut self,
        source: &mut ViewerSource,
        form: &TextForm,
        budget: u64,
    ) -> io::Result<bool> {
        let mut spent = 0u64;
        while !self.done && spent < budget {
            let read = match &self.query {
                Query::Pattern(regex) => {
                    let regex = regex.clone();
                    self.scan_text(source, form, &regex)?
                }
                Query::Bytes(bytes) => {
                    let bytes = bytes.clone();
                    self.scan_bytes(source, form.newline, &bytes)?
                }
            };
            if read == 0 {
                self.done = true;
            }
            spent += read.max(1) as u64;
        }
        Ok(self.done)
    }

    fn push(&mut self, hit: Hit) {
        if self.hits.len() < MAX_HITS {
            self.hits.push(hit);
        } else {
            self.overflow = true;
        }
    }

    /// One chunk of a text search, cut at its last line end.
    fn scan_text(
        &mut self,
        source: &mut ViewerSource,
        form: &TextForm,
        regex: &Regex,
    ) -> io::Result<usize> {
        let newline = form.newline;
        let unit = newline.unit();
        let chunk = source.chunk(self.at, SEARCH_CHUNK)?;
        let usable = chunk.len() - chunk.len() % unit;
        if usable == 0 {
            return Ok(0);
        }
        // A short chunk is the end of what the source holds: all of it.
        // A full one is cut after its last line end, so no line is split;
        // a line longer than a chunk is searched a chunk at a time.
        let mut cut = usable;
        if chunk.len() >= SEARCH_CHUNK {
            let mut last_end = None;
            newline.ends(&chunk[..usable], |after| last_end = Some(after));
            if let Some(after) = last_end {
                cut = after;
            }
        }
        let bytes = chunk[..cut].to_vec();
        let base = self.at;
        let (text, _) = form.choice.encoding.decode_without_bom_handling(&bytes);
        // Walk the matches in order, counting the lines between them.
        let mut line = self.line;
        let mut line_start = 0usize;
        let mut counted = 0usize;
        let mut found = Vec::new();
        for m in regex.find_iter(&text) {
            if m.is_empty() {
                continue;
            }
            for (i, b) in text.as_bytes()[counted..m.start()].iter().enumerate() {
                if *b == b'\n' {
                    line += 1;
                    line_start = counted + i + 1;
                }
            }
            counted = m.start();
            let column = neutralize(&text[line_start..m.start()]).len();
            found.push(Hit {
                line,
                column,
                offset: base,
            });
        }
        for hit in found {
            self.push(hit);
        }
        self.line += newline.count(&bytes);
        self.at += cut as u64;
        Ok(cut)
    }

    /// One chunk of a bytes search, with the end of the last chunk kept
    /// so a match across the boundary is found.
    fn scan_bytes(
        &mut self,
        source: &mut ViewerSource,
        newline: Newline,
        needle: &[u8],
    ) -> io::Result<usize> {
        let unit = newline.unit();
        let chunk = source.chunk(self.at, SEARCH_CHUNK)?;
        if chunk.is_empty() {
            return Ok(0);
        }
        let read = chunk.len();
        let start = self.at - self.carry.len() as u64;
        let mut hay = std::mem::take(&mut self.carry);
        let kept = hay.len();
        hay.extend_from_slice(chunk);
        let mut at = 0;
        let mut found = Vec::new();
        while let Some(i) = find(&hay[at..], needle) {
            let pos = at + i;
            // A match inside the kept bytes alone was found last time.
            if pos + needle.len() > kept {
                found.push(pos);
            }
            at = pos + 1;
        }
        for pos in found {
            let aligned = pos - pos % unit;
            let line = self.line + newline.count(&hay[..aligned]);
            self.push(Hit {
                line,
                column: 0,
                offset: start + pos as u64,
            });
        }
        // Keep the last bytes, whole code units, for the next chunk.
        let mut keep = needle.len().saturating_sub(1);
        keep += (unit - keep % unit) % unit;
        let keep = keep.min(hay.len() - hay.len() % unit);
        let counted = hay.len() - keep;
        self.line += newline.count(&hay[..counted - counted % unit]);
        self.carry = hay[counted..].to_vec();
        self.at += read as u64;
        Ok(read)
    }

    /// The count in words: "12", or "more than 100,000".
    pub(crate) fn count_words(&self) -> String {
        if self.overflow {
            format!("more than {}", grouped(MAX_HITS as u64))
        } else {
            grouped(self.hits.len() as u64)
        }
    }

    /// "Match 3 of 12, line 120"; in hex mode "Match 3 of 12, offset
    /// 4,096".
    pub(crate) fn match_line(&self, index: usize, hex: bool) -> String {
        let hit = self.hits.get(index);
        let place = if hex {
            format!("offset {}", grouped(hit.map_or(0, |h| h.offset)))
        } else {
            format!("line {}", grouped(hit.map_or(0, |h| h.line + 1)))
        };
        format!(
            "Match {} of {}, {place}",
            grouped(index as u64 + 1),
            self.count_words()
        )
    }

    /// "No match for hello."
    pub(crate) fn none_line(&self) -> Message {
        Message::info(format!("No match for {}.", self.typed))
    }
}

/// Which way a search moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Forward,
    Backward,
}

impl super::Viewer {
    /// Searches for what was typed (the grammar is in the module
    /// documentation), forward from the cursor's line, or backward: the
    /// whole file is scanned once, in slices on a large one, so the count
    /// is real. "Match 3 of 12, line 120", "No match for hello.", "Search
    /// wrapped to the top, match 1 of 12".
    pub fn search(&mut self, typed: &str, backward: bool) -> Vec<Message> {
        let hex = self.mode == super::ViewerMode::Hex;
        let query = match Query::parse(typed, hex) {
            Ok(query) => query,
            Err(line) => return vec![Message::warning(line)],
        };
        let start = match query {
            Query::Bytes(_) => 0,
            Query::Pattern(_) => self.form.start(),
        };
        self.search = Some(Search::new(typed, query, start));
        let direction = if backward {
            Direction::Backward
        } else {
            Direction::Forward
        };
        self.start_task(super::Task::Search {
            direction,
            fresh: true,
        })
    }

    /// The next match, wrapping to the top after the last.
    pub fn next_match(&mut self) -> Vec<Message> {
        self.step_match(Direction::Forward)
    }

    /// The previous match, wrapping to the bottom before the first.
    pub fn previous_match(&mut self) -> Vec<Message> {
        self.step_match(Direction::Backward)
    }

    fn step_match(&mut self, direction: Direction) -> Vec<Message> {
        if self.search.is_none() {
            return vec![Message::info("No search yet")];
        }
        self.start_task(super::Task::Search {
            direction,
            fresh: false,
        })
    }

    /// What was searched for last, as typed.
    pub fn search_text(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.typed.as_str())
    }

    /// The matches of the last search in a row's text, for a frontend to
    /// mark (with an attribute, never by color alone).
    pub fn match_ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.search
            .as_ref()
            .map_or_else(Vec::new, |s| s.query.ranges_in(text))
    }

    /// Carries a search on and, once its scan is done, lands on a match.
    pub(super) fn run_search(
        &mut self,
        direction: Direction,
        fresh: bool,
        budget: u64,
    ) -> io::Result<Option<Vec<Message>>> {
        let Some(search) = self.search.as_mut() else {
            return Ok(Some(Vec::new()));
        };
        if !search.done && !search.scan(&mut self.source, &self.form, budget)? {
            return Ok(None);
        }
        let search = match self.search.as_ref() {
            Some(search) => search,
            None => return Ok(Some(Vec::new())),
        };
        if search.hits.is_empty() {
            return Ok(Some(vec![search.none_line()]));
        }
        let last = search.hits.len() - 1;
        let hex = self.mode == super::ViewerMode::Hex;
        let here = self.cursor.line;
        let (index, wrapped) = match (fresh, search.current, direction) {
            (true, _, Direction::Forward) | (false, None, Direction::Forward) => {
                match search.hits.iter().position(|h| h.line >= here) {
                    Some(i) => (i, None),
                    None => (0, Some("top")),
                }
            }
            (true, _, Direction::Backward) | (false, None, Direction::Backward) => {
                match search.hits.iter().rposition(|h| h.line <= here) {
                    Some(i) => (i, None),
                    None => (last, Some("bottom")),
                }
            }
            (false, Some(i), Direction::Forward) if i < last => (i + 1, None),
            (false, Some(_), Direction::Forward) => (0, Some("top")),
            (false, Some(i), Direction::Backward) if i > 0 => (i - 1, None),
            (false, Some(_), Direction::Backward) => (last, Some("bottom")),
        };
        let hit = search.hits[index];
        let said = match wrapped {
            Some(end) => format!(
                "Search wrapped to the {end}, match {} of {}",
                grouped(index as u64 + 1),
                search.count_words()
            ),
            None => search.match_line(index, hex),
        };
        if let Some(search) = self.search.as_mut() {
            search.current = Some(index);
        }
        if hex {
            self.hex_go(hit.offset);
        } else {
            self.land_on_column(hit.line, hit.column);
        }
        Ok(Some(vec![Message::info(said)]))
    }

    /// Forgets the matches, keeping the query, when the file changed.
    pub(super) fn reset_search(&mut self) {
        let start = self.form.start();
        if let Some(search) = self.search.as_mut() {
            let begin = match search.query {
                Query::Bytes(_) => 0,
                Query::Pattern(_) => start,
            };
            *search = Search::new(&search.typed, search.query.clone(), begin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regex_of(typed: &str) -> String {
        match Query::parse(typed, false).unwrap() {
            Query::Pattern(regex) => regex.as_str().to_owned(),
            Query::Bytes(bytes) => format!("{bytes:?}"),
        }
    }

    #[test]
    fn the_search_grammar_reads_patterns_quotes_and_bytes() {
        // The typed text, then whether it matches each of three lines.
        let table: [(&str, bool, [bool; 3]); 9] = [
            ("hello", false, [true, true, false]),
            ("Hello", false, [true, false, false]),
            ("hel+o", false, [true, true, false]),
            ("^hello", false, [false, true, false]),
            ("world$", false, [true, false, false]),
            ("\"a.b\"", false, [false, false, true]),
            ("\"HELLO\"", false, [false, false, false]),
            ("hex: 68 65", false, [false, true, false]),
            ("48 65", true, [true, false, false]),
        ];
        let lines = ["say Hello world", "hello there", "a.b and axb"];
        for (typed, hex_mode, expected) in table {
            let query = Query::parse(typed, hex_mode).unwrap();
            let got: Vec<bool> = lines
                .iter()
                .map(|l| !query.ranges_in(l).is_empty())
                .collect();
            assert_eq!(got, expected, "{typed:?}");
        }
        assert_eq!(regex_of("hello"), "(?mR)hello");
        assert_eq!(
            Query::parse("hex:48656c", false).unwrap(),
            Query::Bytes(b"Hel".to_vec())
        );
        assert_eq!(
            Query::parse("48 65 \"llo\"", true).unwrap(),
            Query::Bytes(b"Hello".to_vec())
        );
        assert_eq!(
            Query::parse("(unclosed", false).unwrap_err(),
            "Pattern needs care: bad expression"
        );
        assert_eq!(Query::parse("zz", true).unwrap_err(), "Not hex bytes: zz");
        assert_eq!(
            Query::parse("  ", false).unwrap_err(),
            "Type something to search for"
        );
        assert_eq!(
            Query::parse("(?=x)", false).unwrap_err(),
            "Pattern needs care: look-around"
        );
    }

    #[test]
    fn a_hex_search_finds_bytes_across_a_chunk_boundary() {
        let mut bytes = vec![b'a'; SEARCH_CHUNK * 2 + 10];
        let at = SEARCH_CHUNK - 2;
        bytes[at..at + 4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        bytes[100] = b'\n';
        bytes[SEARCH_CHUNK + 50..SEARCH_CHUNK + 54].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let mut source = ViewerSource::bytes(bytes);
        let form = TextForm::from_head(b"aaaa", false);
        let query = Query::parse("hex: de ad be ef", false).unwrap();
        let mut search = Search::new("hex: de ad be ef", query, 0);
        while !search.scan(&mut source, &form, 1).unwrap() {}
        let offsets: Vec<u64> = search.hits.iter().map(|h| h.offset).collect();
        assert_eq!(offsets, [at as u64, SEARCH_CHUNK as u64 + 50]);
        assert!(search.hits.iter().all(|h| h.line == 1), "{:?}", search.hits);
    }

    #[test]
    fn a_text_search_counts_lines_and_columns_across_chunks() {
        let mut text = String::new();
        for i in 0..200_000 {
            if i % 50_000 == 7 {
                text.push_str("\tthe needle is here\n");
            } else {
                text.push_str("hay hay hay\n");
            }
        }
        let bytes = text.into_bytes();
        let mut source = ViewerSource::bytes(bytes);
        let form = TextForm::from_head(b"hay", false);
        let query = Query::parse("needle", false).unwrap();
        let mut search = Search::new("needle", query, 0);
        while !search.scan(&mut source, &form, 1).unwrap() {}
        let found: Vec<(u64, usize)> = search.hits.iter().map(|h| (h.line, h.column)).collect();
        assert_eq!(found, [(7, 8), (50_007, 8), (100_007, 8), (150_007, 8)]);
        assert_eq!(search.match_line(1, false), "Match 2 of 4, line 50,008");
    }
}
