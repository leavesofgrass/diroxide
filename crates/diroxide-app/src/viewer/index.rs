//! The lazy, sparse line index: one byte offset for every thousandth
//! line, built as far as the viewer has needed and no further, in slices
//! the caller sizes so the input thread never waits on a whole file. A
//! 1 GiB log of ten million lines needs ten thousand offsets, 80 KB.

use std::io;

use super::source::ViewerSource;
use super::text::Newline;

/// One checkpoint every this many lines.
pub(crate) const EVERY: u64 = 1000;

/// How much one read of the scan takes.
pub(crate) const SCAN_CHUNK: usize = 1024 * 1024;

/// Where a line starts, as far as the index can say within its budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// The line starts at this byte.
    At(u64),
    /// The file has fewer lines; `last` is its last line, 0-based.
    PastEnd {
        /// The last line.
        last: u64,
    },
    /// The budget ran out before the index reached the line.
    Pending,
}

/// The index. Line numbers are 0-based inside; the viewer says them
/// 1-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineIndex {
    newline: Newline,
    start: u64,
    /// `checkpoints[i]` is where line `i * EVERY` starts.
    checkpoints: Vec<u64>,
    /// Bytes before this offset have been scanned.
    scanned: u64,
    /// Line ends seen before `scanned`.
    ends: u64,
    /// Where the line after the last line end seen starts.
    last_start: u64,
    /// The scan reached the end of what the source holds.
    done: bool,
}

impl LineIndex {
    /// An index of a file whose first line starts at `start`.
    pub(crate) fn new(start: u64, newline: Newline) -> Self {
        Self {
            newline,
            start,
            checkpoints: vec![start],
            scanned: start,
            ends: 0,
            last_start: start,
            done: false,
        }
    }

    /// Whether the scan has reached the end of the source.
    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    /// The lines known so far: every line end seen, and the line after the
    /// last one when it has text.
    pub(crate) fn lines_known(&self) -> u64 {
        self.ends + u64::from(self.scanned > self.last_start)
    }

    /// The number of lines, once the scan is done.
    pub(crate) fn total(&self) -> Option<u64> {
        self.done.then(|| self.lines_known())
    }

    /// The memory the checkpoints take, in bytes.
    pub(crate) fn memory(&self) -> usize {
        self.checkpoints.capacity() * size_of::<u64>() + size_of::<Self>()
    }

    /// The source grew (follow, read more): scan on from where it stopped.
    pub(crate) fn reopen(&mut self) {
        self.done = false;
    }

    /// Scans on for at most `budget` bytes; whether the scan is done.
    pub(crate) fn scan(&mut self, source: &mut ViewerSource, budget: u64) -> io::Result<bool> {
        let unit = self.newline.unit();
        let mut spent = 0u64;
        while !self.done && spent < budget {
            let chunk = source.chunk(self.scanned, SCAN_CHUNK)?;
            let usable = chunk.len() - chunk.len() % unit;
            if usable == 0 {
                self.done = true;
                break;
            }
            let bytes = &chunk[..usable];
            let base = self.scanned;
            let count = self.newline.count(bytes);
            if count > 0 {
                let crosses = self.ends % EVERY + count >= EVERY;
                if crosses || self.newline != Newline::Byte {
                    let mut ends = self.ends;
                    let mut last = self.last_start;
                    let checkpoints = &mut self.checkpoints;
                    self.newline.ends(bytes, |after| {
                        ends += 1;
                        last = base + after as u64;
                        if ends.is_multiple_of(EVERY) {
                            checkpoints.push(last);
                        }
                    });
                    self.ends = ends;
                    self.last_start = last;
                } else {
                    self.ends += count;
                    if let Some(at) = bytes.iter().rposition(|b| *b == b'\n') {
                        self.last_start = base + at as u64 + 1;
                    }
                }
            }
            self.scanned += usable as u64;
            spent += usable as u64;
        }
        Ok(self.done)
    }

    /// Where line `n` starts, scanning at most `budget` bytes more.
    pub(crate) fn line_start(
        &mut self,
        source: &mut ViewerSource,
        n: u64,
        budget: u64,
    ) -> io::Result<Found> {
        if n == 0 {
            return Ok(Found::At(self.start));
        }
        if n > self.ends && !self.done {
            self.scan_until(source, budget, |index| n <= index.ends)?;
        }
        if n <= self.ends && (n < self.lines_known() || !self.done) {
            let checkpoint = (n / EVERY) as usize;
            let from = self.checkpoints[checkpoint];
            let at = self.skip(source, from, n % EVERY)?;
            return Ok(Found::At(at));
        }
        if self.done {
            return Ok(Found::PastEnd {
                last: self.lines_known().saturating_sub(1),
            });
        }
        Ok(Found::Pending)
    }

    /// The line that holds byte `offset`, scanning at most `budget` bytes
    /// more; `None` when the budget ran out first.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "jump by offset, the next commit")
    )]
    pub(crate) fn line_of(
        &mut self,
        source: &mut ViewerSource,
        offset: u64,
        budget: u64,
    ) -> io::Result<Option<u64>> {
        let offset = offset.max(self.start);
        if offset > self.scanned && !self.done {
            self.scan_until(source, budget, |index| offset <= index.scanned)?;
        }
        if offset > self.scanned && !self.done {
            return Ok(None);
        }
        let offset = offset.min(self.scanned);
        let checkpoint = self
            .checkpoints
            .partition_point(|c| *c <= offset)
            .saturating_sub(1);
        let from = self.checkpoints[checkpoint];
        let ends = self.count_between(source, from, offset)?;
        let line = checkpoint as u64 * EVERY + ends;
        Ok(Some(line.min(self.lines_known().saturating_sub(1))))
    }

    fn scan_until(
        &mut self,
        source: &mut ViewerSource,
        budget: u64,
        reached: impl Fn(&Self) -> bool,
    ) -> io::Result<()> {
        let mut left = budget;
        while !reached(self) && !self.done && left > 0 {
            let before = self.scanned;
            let step = left.min(SCAN_CHUNK as u64);
            self.scan(source, step)?;
            left = left.saturating_sub((self.scanned - before).max(1));
        }
        Ok(())
    }

    /// The offset after `lines` more line ends from `from`.
    fn skip(&self, source: &mut ViewerSource, from: u64, lines: u64) -> io::Result<u64> {
        let unit = self.newline.unit();
        let mut at = from;
        let mut left = lines;
        while left > 0 {
            let chunk = source.chunk(at, super::source::BLOCK)?;
            let usable = chunk.len() - chunk.len() % unit;
            if usable == 0 {
                break;
            }
            let mut found = None;
            let mut seen = 0;
            self.newline.ends(&chunk[..usable], |after| {
                if found.is_none() {
                    seen += 1;
                    if seen == left {
                        found = Some(after);
                    }
                }
            });
            match found {
                Some(after) => return Ok(at + after as u64),
                None => {
                    left -= seen;
                    at += usable as u64;
                }
            }
        }
        Ok(at)
    }

    /// The line ends between two offsets.
    fn count_between(&self, source: &mut ViewerSource, from: u64, to: u64) -> io::Result<u64> {
        let unit = self.newline.unit();
        let mut at = from;
        let mut count = 0;
        while at < to {
            let want = ((to - at) as usize).min(SCAN_CHUNK);
            let chunk = source.chunk(at, want)?;
            let usable = chunk.len() - chunk.len() % unit;
            if usable == 0 {
                break;
            }
            count += self.newline.count(&chunk[..usable]);
            at += usable as u64;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(lines: u64) -> Vec<u8> {
        (0..lines)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect()
    }

    #[test]
    fn the_index_finds_any_line_from_its_checkpoints() {
        let bytes = numbered(5_500);
        let mut source = ViewerSource::bytes(bytes.clone());
        let mut index = LineIndex::new(0, Newline::Byte);
        assert_eq!(index.line_start(&mut source, 0, 0).unwrap(), Found::At(0));
        let mut long = ViewerSource::bytes(numbered(200_000));
        let mut sliced = LineIndex::new(0, Newline::Byte);
        let found = sliced.line_start(&mut long, 190_000, 64).unwrap();
        assert_eq!(found, Found::Pending, "a slice of a megabyte is not enough");
        let found = index.line_start(&mut source, 4_321, u64::MAX).unwrap();
        let Found::At(at) = found else {
            panic!("{found:?}")
        };
        assert!(bytes[at as usize..].starts_with(b"line 4321\n"));
        assert!(index.scan(&mut source, u64::MAX).unwrap());
        assert_eq!(index.total(), Some(5_500));
        assert_eq!(index.checkpoints.len(), 6, "lines 0, 1000, ..., 5000");
        assert_eq!(
            index.line_start(&mut source, 9_000, u64::MAX).unwrap(),
            Found::PastEnd { last: 5_499 }
        );
        let offset = bytes.len() as u64 - 3;
        assert_eq!(
            index.line_of(&mut source, offset, u64::MAX).unwrap(),
            Some(5_499)
        );
        assert_eq!(index.line_of(&mut source, 6, u64::MAX).unwrap(), Some(0));
        assert_eq!(index.line_of(&mut source, 7, u64::MAX).unwrap(), Some(1));
    }

    #[test]
    fn a_last_line_without_an_end_counts_and_an_empty_file_has_none() {
        let mut source = ViewerSource::bytes(b"a\nb".to_vec());
        let mut index = LineIndex::new(0, Newline::Byte);
        index.scan(&mut source, u64::MAX).unwrap();
        assert_eq!(index.total(), Some(2));
        assert_eq!(index.line_start(&mut source, 1, 0).unwrap(), Found::At(2));
        let mut source = ViewerSource::bytes(b"a\n".to_vec());
        let mut index = LineIndex::new(0, Newline::Byte);
        index.scan(&mut source, u64::MAX).unwrap();
        assert_eq!(index.total(), Some(1));
        assert_eq!(
            index.line_start(&mut source, 1, 0).unwrap(),
            Found::PastEnd { last: 0 }
        );
        let mut source = ViewerSource::bytes(Vec::new());
        let mut index = LineIndex::new(0, Newline::Byte);
        index.scan(&mut source, u64::MAX).unwrap();
        assert_eq!(index.total(), Some(0));
    }

    #[test]
    fn utf16_lines_are_counted_by_code_unit() {
        let mut bytes = vec![0xff, 0xfe];
        let text: String = (0..2_100).map(|i| format!("{i}\n")).collect();
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let mut source = ViewerSource::bytes(bytes.clone());
        let mut index = LineIndex::new(2, Newline::Utf16Le);
        index.scan(&mut source, u64::MAX).unwrap();
        assert_eq!(index.total(), Some(2_100));
        let Found::At(at) = index.line_start(&mut source, 2_050, 0).unwrap() else {
            panic!("line 2050")
        };
        let expected: Vec<u8> = "2050\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(bytes[at as usize..].starts_with(&expected));
    }

    #[test]
    fn a_large_index_stays_small() {
        let bytes = numbered(200_000);
        let mut source = ViewerSource::bytes(bytes);
        let mut index = LineIndex::new(0, Newline::Byte);
        index.scan(&mut source, u64::MAX).unwrap();
        assert_eq!(index.total(), Some(200_000));
        assert!(index.memory() < 4_096, "{}", index.memory());
    }
}
