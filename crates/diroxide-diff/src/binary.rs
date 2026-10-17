//! Binary compare (R18, "Binary files"): which bytes differ, in runs,
//! with their offsets, read in blocks side by side and capped, so a
//! listener hears "Bytes 1,266 to 1,269 differ, 4 bytes" and, on Enter,
//! "was 48 65 6c 6c, now 48 66 6c 6c".
//!
//! Offsets count from 0, as the viewer's hex mode numbers them. When the
//! files have different sizes, bytes after an insertion no longer line
//! up, so the report says the sizes and where the shorter file ends, and
//! never pretends a byte-for-byte comparison means anything past it.

use std::io::Read;

use diroxide_core::format::grouped;
use diroxide_vfs::CancelToken;

use crate::equal::{BLOCK_BYTES, fill};
use crate::error::DiffError;
use crate::facts::Sides;
use crate::grammar::{Verdict, verdict_line};

/// The most runs a compare lists; past it the count goes on and the
/// report says how many were left out (R18's starting value).
pub const MAX_RUNS: usize = 1_000;

/// The most bytes of a run spelled out on each side.
pub const SPELLED_BYTES: usize = 8;

/// One run of differing bytes at the same offsets in both files.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ByteRun {
    /// The first byte's offset, from 0.
    pub start: u64,
    /// How many bytes in a row differ.
    pub len: u64,
    /// The before file's first bytes of the run, at most [`SPELLED_BYTES`].
    pub before: Vec<u8>,
    /// The after file's first bytes of the run, at most [`SPELLED_BYTES`].
    pub after: Vec<u8>,
}

impl ByteRun {
    /// The run as its row: "Bytes 1,266 to 1,269 differ, 4 bytes", or
    /// "Byte 1,266 differs".
    pub fn row(&self) -> String {
        if self.len == 1 {
            format!("Byte {} differs", grouped(self.start))
        } else {
            format!(
                "Bytes {} to {} differ, {} bytes",
                grouped(self.start),
                grouped(self.start + self.len - 1),
                grouped(self.len)
            )
        }
    }

    /// The run's bytes in hex: "was 48 65 6c 6c, now 48 66 6c 6c", with
    /// "and 12 more" when the run is longer than the bytes spelled.
    pub fn bytes(&self) -> String {
        let hex = |bytes: &[u8]| -> String {
            bytes
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut text = format!("was {}, now {}", hex(&self.before), hex(&self.after));
        if self.len > SPELLED_BYTES as u64 {
            text.push_str(&format!(
                ", and {} more",
                grouped(self.len - SPELLED_BYTES as u64)
            ));
        }
        text
    }
}

/// Two files compared byte by byte.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BinaryComparison {
    /// The before file's size in bytes.
    pub before_size: u64,
    /// The after file's size in bytes.
    pub after_size: u64,
    /// How many bytes differ at the same offsets, up to the shorter
    /// file's end.
    pub differing_bytes: u64,
    /// The runs, at most [`MAX_RUNS`].
    pub runs: Vec<ByteRun>,
    /// How many runs there are in all, listed or not.
    pub run_count: u64,
}

impl BinaryComparison {
    /// The verdict: the same, so many bytes differing, or the sizes.
    pub fn verdict(&self) -> Verdict {
        if self.before_size != self.after_size {
            Verdict::BinarySizes
        } else if self.differing_bytes == 0 {
            Verdict::Same
        } else {
            Verdict::BinaryBytes(self.differing_bytes)
        }
    }

    /// The first offset that differs: the first run's, or where the
    /// shorter file ends.
    pub fn first_difference(&self) -> Option<u64> {
        self.runs.first().map(|run| run.start).or_else(|| {
            (self.before_size != self.after_size).then_some(self.before_size.min(self.after_size))
        })
    }

    /// The report before the runs, in the reading order: the verdict line,
    /// then the sizes and the first difference, and what was left out.
    pub fn summary(&self, before_name: &str, after_name: &str, sides: &Sides) -> Vec<String> {
        let verdict = self.verdict();
        let (first, second) = if sides.before_first() {
            (before_name, after_name)
        } else {
            (after_name, before_name)
        };
        let mut lines = vec![verdict_line(verdict, first, second)];
        if verdict == Verdict::Same {
            return lines;
        }
        let bytes = |n: u64| {
            if n == 1 {
                "1 byte".to_owned()
            } else {
                format!("{} bytes", grouped(n))
            }
        };
        let differ = if self.differing_bytes == 1 {
            "differs"
        } else {
            "differ"
        };
        if self.before_size == self.after_size {
            let runs = if self.run_count == 1 {
                "1 run".to_owned()
            } else {
                format!("{} runs", grouped(self.run_count))
            };
            lines.push(format!(
                "Both {}; {} {differ} in {runs}.",
                bytes(self.before_size),
                bytes(self.differing_bytes)
            ));
        } else {
            lines.push(format!(
                "Sizes: {}.",
                sides.pair(&bytes(self.before_size), &bytes(self.after_size))
            ));
            let shorter = self.before_size.min(self.after_size);
            if self.runs.is_empty() {
                lines.push(format!(
                    "The first {} are the same; one file goes on past them.",
                    bytes(shorter)
                ));
            } else {
                lines.push(format!(
                    "{} {differ} before the shorter file ends; past it bytes are not compared.",
                    bytes(self.differing_bytes)
                ));
            }
        }
        if let Some(at) = self.first_difference() {
            lines.push(format!("First difference at byte {}.", grouped(at)));
        }
        if self.run_count > self.runs.len() as u64 {
            lines.push(format!(
                "Listing the first {} runs of {}.",
                grouped(self.runs.len() as u64),
                grouped(self.run_count)
            ));
        }
        lines
    }

    /// The same comparison read the other way (Swap).
    pub fn swapped(&self) -> BinaryComparison {
        BinaryComparison {
            before_size: self.after_size,
            after_size: self.before_size,
            differing_bytes: self.differing_bytes,
            runs: self
                .runs
                .iter()
                .map(|run| ByteRun {
                    start: run.start,
                    len: run.len,
                    before: run.after.clone(),
                    after: run.before.clone(),
                })
                .collect(),
            run_count: self.run_count,
        }
    }
}

/// Compares two streams byte by byte, in blocks, honoring `cancel`
/// between blocks.
pub fn compare_binary(
    before: &mut dyn Read,
    after: &mut dyn Read,
    cancel: &CancelToken,
) -> Result<BinaryComparison, DiffError> {
    let mut one = vec![0u8; BLOCK_BYTES];
    let mut other = vec![0u8; BLOCK_BYTES];
    let mut result = BinaryComparison {
        before_size: 0,
        after_size: 0,
        differing_bytes: 0,
        runs: Vec::new(),
        run_count: 0,
    };
    // The run being read, while it lasts: its start, length and bytes.
    let mut open: Option<ByteRun> = None;
    let mut before_done = false;
    let mut after_done = false;
    while !(before_done && after_done) {
        if cancel.is_cancelled() {
            return Err(DiffError::Cancelled);
        }
        let got_one = if before_done {
            0
        } else {
            fill(before, &mut one)?
        };
        let got_other = if after_done {
            0
        } else {
            fill(after, &mut other)?
        };
        before_done |= got_one < BLOCK_BYTES;
        after_done |= got_other < BLOCK_BYTES;
        // Bytes are compared only while both files have them at the same
        // offsets: from the start, up to the shorter file's end.
        let comparing = result.before_size == result.after_size;
        let offset = result.before_size;
        result.before_size += got_one as u64;
        result.after_size += got_other as u64;
        if !comparing {
            continue;
        }
        let common = got_one.min(got_other);
        for at in 0..common {
            let (b, a) = (one[at], other[at]);
            if b == a {
                if let Some(run) = open.take() {
                    push_run(&mut result, run);
                }
                continue;
            }
            result.differing_bytes += 1;
            let run = open.get_or_insert_with(|| ByteRun {
                start: offset + at as u64,
                len: 0,
                before: Vec::new(),
                after: Vec::new(),
            });
            run.len += 1;
            if run.before.len() < SPELLED_BYTES {
                run.before.push(b);
                run.after.push(a);
            }
        }
        if got_one != got_other
            && let Some(run) = open.take()
        {
            push_run(&mut result, run);
        }
    }
    if let Some(run) = open.take() {
        push_run(&mut result, run);
    }
    Ok(result)
}

fn push_run(result: &mut BinaryComparison, run: ByteRun) {
    result.run_count += 1;
    if result.runs.len() < MAX_RUNS {
        result.runs.push(run);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn compare(a: &[u8], b: &[u8]) -> BinaryComparison {
        compare_binary(
            &mut Cursor::new(a.to_vec()),
            &mut Cursor::new(b.to_vec()),
            &CancelToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn the_same_bytes_are_the_same() {
        let same = compare(b"\x00\x01\x02", b"\x00\x01\x02");
        assert_eq!(same.verdict(), Verdict::Same);
        assert_eq!(same.first_difference(), None);
        assert_eq!(
            same.summary("a.bin", "b.bin", &Sides::panels()),
            ["Same: b.bin and a.bin"]
        );
    }

    #[test]
    fn runs_have_offsets_and_bytes() {
        let mut a = b"Hello, world. Hello, world.".to_vec();
        let mut b = a.clone();
        b[1] = b'f';
        b[2] = b'f';
        b[20] = b'W';
        a[26] = b'!';
        let result = compare(&a, &b);
        assert_eq!(result.verdict(), Verdict::BinaryBytes(4));
        assert_eq!(result.run_count, 3);
        assert_eq!(result.runs[0].row(), "Bytes 1 to 2 differ, 2 bytes");
        assert_eq!(result.runs[0].bytes(), "was 65 6c, now 66 66");
        assert_eq!(result.runs[1].row(), "Byte 20 differs");
        assert_eq!(result.first_difference(), Some(1));
        assert_eq!(
            result.summary("a.bin", "b.bin", &Sides::files("a.bin", "b.bin")),
            [
                "Binary, 4 bytes differ: a.bin and b.bin",
                "Both 27 bytes; 4 bytes differ in 3 runs.",
                "First difference at byte 1."
            ]
        );
    }

    #[test]
    fn long_runs_spell_eight_bytes() {
        let a = vec![0u8; 20];
        let b = vec![255u8; 20];
        let result = compare(&a, &b);
        assert_eq!(result.runs.len(), 1);
        assert_eq!(
            result.runs[0].bytes(),
            "was 00 00 00 00 00 00 00 00, now ff ff ff ff ff ff ff ff, and 12 more"
        );
    }

    #[test]
    fn runs_cross_block_edges_as_one() {
        let a = vec![0u8; BLOCK_BYTES * 2];
        let mut b = a.clone();
        for byte in &mut b[BLOCK_BYTES - 2..BLOCK_BYTES + 2] {
            *byte = 1;
        }
        let result = compare(&a, &b);
        assert_eq!(result.run_count, 1);
        assert_eq!(result.runs[0].start, BLOCK_BYTES as u64 - 2);
        assert_eq!(result.runs[0].len, 4);
    }

    #[test]
    fn different_sizes_say_the_sizes_and_stop_comparing_at_the_shorter_end() {
        let a = vec![7u8; 100];
        let mut b = vec![7u8; 150];
        let prefix = compare(&a, &b);
        assert_eq!(prefix.verdict(), Verdict::BinarySizes);
        assert_eq!(prefix.first_difference(), Some(100));
        assert_eq!(
            prefix.summary("a.bin", "b.bin", &Sides::panels()),
            [
                "Binary, sizes differ: b.bin and a.bin",
                "Sizes: 150 bytes here, 100 bytes there.",
                "The first 100 bytes are the same; one file goes on past them.",
                "First difference at byte 100."
            ]
        );
        b[3] = 9;
        let early = compare(&a, &b);
        assert_eq!(early.differing_bytes, 1);
        assert_eq!(early.first_difference(), Some(3));
        // Sizes past a block: the tail of the longer is only counted.
        let long = vec![1u8; BLOCK_BYTES * 3 + 7];
        let short = vec![1u8; 10];
        let result = compare(&short, &long);
        assert_eq!(result.after_size, long.len() as u64);
        assert_eq!(result.differing_bytes, 0);
    }

    #[test]
    fn the_list_of_runs_is_capped_and_the_count_goes_on() {
        let a = vec![0u8; MAX_RUNS * 4];
        let b: Vec<u8> = (0..a.len()).map(|i| u8::from(i % 2 == 0)).collect();
        let result = compare(&a, &b);
        assert_eq!(result.runs.len(), MAX_RUNS);
        assert_eq!(result.run_count, MAX_RUNS as u64 * 2);
        let said = result.summary("a", "b", &Sides::panels());
        assert_eq!(
            said.last().unwrap(),
            "Listing the first 1,000 runs of 2,000."
        );
        let swapped = result.swapped();
        assert_eq!(swapped.runs[0].before, result.runs[0].after);
    }

    #[test]
    fn a_cancel_is_honored() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            compare_binary(
                &mut Cursor::new(vec![1u8]),
                &mut Cursor::new(vec![2u8]),
                &cancel
            ),
            Err(DiffError::Cancelled)
        );
    }
}
