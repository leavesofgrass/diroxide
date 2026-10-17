//! Comparing two files through their providers: what Compare files and
//! `diro diff` run.
//!
//! The steps: both files are looked at; one file named twice is "Same
//! file" without a byte read; the first bytes of each say whether either
//! is binary (the viewer's test, after a byte-order mark, so UTF-16 is
//! text); binary files are compared byte by byte in blocks; text up to
//! the size limit (16 MiB, R18's starting value) is compared as text, and
//! a larger file follows the too-large policy. Last, both files are
//! looked at again: if either changed while it was read, the result does
//! not stand and the verdict says "Changed while comparing".
//!
//! Reading reports progress in bytes to the context's sink every MiB, so
//! the app can say "Comparing, 40 percent" at its interval. A cancel at
//! any step is [`DiffError::Cancelled`], which the caller says as "Not
//! compared", never a result that looks whole.

use std::cell::Cell;
use std::io::{Cursor, Read};

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_preview::encoding::{SNIFF_BYTES, binary_kind};
use diroxide_vfs::{OpContext, ProgressUpdate, Provider};

use crate::binary::{BinaryComparison, compare_binary};
use crate::content::{ContentComparison, compare_contents};
use crate::equal::fill;
use crate::error::DiffError;
use crate::facts::Sides;
use crate::grammar::{Verdict, verdict_line};
use crate::options::CompareOptions;

/// The largest file compared as text in full: 16 MiB (R18's starting
/// value for the bench gate to settle).
pub const MAX_TEXT_BYTES: u64 = 16 * 1024 * 1024;

/// What to do with a text file over the size limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LargeFiles {
    /// Compare nothing and say "Too large to compare as text", so the
    /// app can offer the choices: the first 16 MiB, byte by byte, or
    /// cancel. The default.
    #[default]
    Ask,
    /// Compare the first [`FileOptions::max_text_bytes`] of each as text,
    /// up to its last line break, and say so.
    Head,
    /// Compare byte by byte, and say so.
    Bytes,
}

/// Everything a file compare takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileOptions {
    /// The text compare's options.
    pub compare: CompareOptions,
    /// The largest file compared as text in full.
    pub max_text_bytes: u64,
    /// What to do with a larger one.
    pub large: LargeFiles,
}

impl Default for FileOptions {
    fn default() -> Self {
        FileOptions {
            compare: CompareOptions::default(),
            max_text_bytes: MAX_TEXT_BYTES,
            large: LargeFiles::default(),
        }
    }
}

/// One side of a compare: a file and the provider that serves it.
#[derive(Clone, Copy, Debug)]
pub struct FileSide<'a> {
    /// The provider.
    pub provider: &'a dyn Provider,
    /// The file.
    pub place: &'a Place,
}

/// What a file compare found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// One file named twice: the same place, or two names for one file.
    SameFile,
    /// Compared as text.
    Text {
        /// The comparison.
        content: Box<ContentComparison>,
        /// When only the first bytes were compared (the too-large policy
        /// [`LargeFiles::Head`]): how many.
        head: Option<u64>,
    },
    /// Compared byte by byte.
    Binary {
        /// The comparison.
        binary: BinaryComparison,
        /// Whether these are text files compared byte by byte because
        /// they were too large ([`LargeFiles::Bytes`]).
        too_large: bool,
    },
    /// Too large to compare as text, nothing compared
    /// ([`LargeFiles::Ask`]).
    TooLarge,
    /// A file changed while it was being compared, so no result stands.
    ChangedWhileComparing {
        /// Whether the before file changed.
        before: bool,
        /// Whether the after file changed.
        after: bool,
    },
}

/// A file compare's result: what each file was when it was looked at,
/// and what was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileComparison {
    /// The before file as first looked at.
    pub before: Entry,
    /// The after file as first looked at.
    pub after: Entry,
    /// What was found.
    pub outcome: Outcome,
}

fn file_entry(side: &FileSide<'_>, ctx: &OpContext) -> Result<Entry, DiffError> {
    let entry = side.provider.stat(side.place, ctx)?;
    let is_file = entry.kind == EntryKind::File
        || (entry.kind == EntryKind::Link
            && entry.link.as_ref().and_then(|l| l.points_to) == Some(EntryKind::File));
    if is_file {
        Ok(entry)
    } else {
        Err(DiffError::NotAFile(side.place.to_string()))
    }
}

/// Whether two entries are one file: the same place through the same
/// provider, or the same file identity on the same volume (hard links).
fn one_file(before: &FileSide<'_>, after: &FileSide<'_>, a: &Entry, b: &Entry) -> bool {
    if before.provider.scheme() == after.provider.scheme() && before.place == after.place {
        return true;
    }
    matches!(
        (a.inode, a.volume, b.inode, b.volume),
        (Some(i), Some(v), Some(j), Some(w)) if i == j && v == w
    ) && before.provider.scheme() == after.provider.scheme()
}

/// Whether an entry changed between two looks: its size or its modified
/// time.
fn changed(first: &Entry, second: &Entry) -> bool {
    first.size != second.size || first.modified != second.modified
}

/// Reads a stream to its end, or to `most` bytes and one more (so a
/// caller can tell it went past), checking `ctx` between blocks.
/// How often reading reports progress: every MiB read, across both files.
pub const PROGRESS_BYTES: u64 = 1024 * 1024;

/// What both files' reads have done so far, shared by their two readers.
struct Progress<'a> {
    ctx: &'a OpContext,
    done: Cell<u64>,
    reported: Cell<u64>,
    total: Option<u64>,
}

impl Progress<'_> {
    fn add(&self, bytes: usize) {
        let done = self.done.get() + bytes as u64;
        self.done.set(done);
        if done - self.reported.get() >= PROGRESS_BYTES {
            self.reported.set(done);
            self.ctx.progress.report(ProgressUpdate {
                items_done: 0,
                items_total: Some(2),
                bytes_done: done,
                bytes_total: self.total,
                current: None,
            });
        }
    }
}

/// A reader that tells [`Progress`] how much it read.
struct Counted<'a, R> {
    inner: R,
    progress: &'a Progress<'a>,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.progress.add(read);
        Ok(read)
    }
}

fn read_up_to(reader: &mut dyn Read, most: u64, ctx: &OpContext) -> Result<Vec<u8>, DiffError> {
    let mut out = Vec::new();
    let mut block = vec![0u8; crate::equal::BLOCK_BYTES];
    loop {
        ctx.check()?;
        let got = fill(reader, &mut block)?;
        out.extend_from_slice(&block[..got]);
        if got < block.len() || out.len() as u64 > most {
            return Ok(out);
        }
    }
}

/// The bytes up to the last line break within `most`.
fn head_of(bytes: &[u8], most: u64) -> &[u8] {
    let cut = bytes.len().min(usize::try_from(most).unwrap_or(usize::MAX));
    let head = &bytes[..cut];
    match head.iter().rposition(|&b| b == b'\n' || b == b'\r') {
        Some(at) if cut < bytes.len() => &head[..=at],
        _ => head,
    }
}

/// Compares two files under `options`, honoring the context's cancel
/// token at every step.
pub fn compare_files(
    before: FileSide<'_>,
    after: FileSide<'_>,
    options: &FileOptions,
    ctx: &OpContext,
) -> Result<FileComparison, DiffError> {
    let before_entry = file_entry(&before, ctx)?;
    let after_entry = file_entry(&after, ctx)?;
    let result = |outcome| FileComparison {
        before: before_entry.clone(),
        after: after_entry.clone(),
        outcome,
    };
    if one_file(&before, &after, &before_entry, &after_entry) {
        return Ok(result(Outcome::SameFile));
    }
    // Progress in bytes across both files, for "Comparing, 40 percent".
    let progress = Progress {
        ctx,
        done: Cell::new(0),
        reported: Cell::new(0),
        total: before_entry.size.zip(after_entry.size).map(|(b, a)| b + a),
    };
    let mut before_reader = Counted {
        inner: before.provider.read(before.place, ctx)?,
        progress: &progress,
    };
    let mut after_reader = Counted {
        inner: after.provider.read(after.place, ctx)?,
        progress: &progress,
    };
    let mut before_head = vec![0u8; SNIFF_BYTES];
    let mut after_head = vec![0u8; SNIFF_BYTES];
    let got = fill(&mut before_reader, &mut before_head)?;
    before_head.truncate(got);
    let got = fill(&mut after_reader, &mut after_head)?;
    after_head.truncate(got);
    ctx.check()?;
    let binary = binary_kind(&before_head).is_some() || binary_kind(&after_head).is_some();
    let too_large = [&before_entry, &after_entry]
        .iter()
        .any(|e| e.size.is_some_and(|size| size > options.max_text_bytes));
    let mut before_stream = Cursor::new(before_head).chain(before_reader);
    let mut after_stream = Cursor::new(after_head).chain(after_reader);
    let outcome = if binary || (too_large && options.large == LargeFiles::Bytes) {
        Outcome::Binary {
            binary: compare_binary(&mut before_stream, &mut after_stream, &ctx.cancel)?,
            too_large: !binary,
        }
    } else if too_large && options.large == LargeFiles::Ask {
        Outcome::TooLarge
    } else {
        let most = options.max_text_bytes;
        let before_bytes = read_up_to(&mut before_stream, most, ctx)?;
        let after_bytes = read_up_to(&mut after_stream, most, ctx)?;
        let over = before_bytes.len() as u64 > most || after_bytes.len() as u64 > most;
        match (over, options.large) {
            (true, LargeFiles::Ask) => Outcome::TooLarge,
            (true, LargeFiles::Bytes) => Outcome::Binary {
                binary: compare_binary(
                    &mut Cursor::new(before_bytes).chain(before_stream),
                    &mut Cursor::new(after_bytes).chain(after_stream),
                    &ctx.cancel,
                )?,
                too_large: true,
            },
            (true, LargeFiles::Head) => Outcome::Text {
                content: Box::new(compare_contents(
                    head_of(&before_bytes, most),
                    head_of(&after_bytes, most),
                    &options.compare,
                    &ctx.cancel,
                )?),
                head: Some(most),
            },
            (false, _) => Outcome::Text {
                content: Box::new(compare_contents(
                    &before_bytes,
                    &after_bytes,
                    &options.compare,
                    &ctx.cancel,
                )?),
                head: None,
            },
        }
    };
    // The last look: a file that changed while it was read leaves no
    // result standing.
    let before_now = before.provider.stat(before.place, ctx)?;
    let after_now = after.provider.stat(after.place, ctx)?;
    // A cancel at any step, this last look included, is "Not compared".
    ctx.check()?;
    let (before_changed, after_changed) = (
        changed(&before_entry, &before_now),
        changed(&after_entry, &after_now),
    );
    if before_changed || after_changed {
        return Ok(result(Outcome::ChangedWhileComparing {
            before: before_changed,
            after: after_changed,
        }));
    }
    Ok(result(outcome))
}

/// A size limit in words: "16 MiB", "1.5 MiB", or exact bytes below one
/// MiB ("1,024 bytes").
fn mebibytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes < MIB {
        format!("{} bytes", diroxide_core::format::grouped(bytes))
    } else if bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

impl FileComparison {
    /// The verdict.
    pub fn verdict(&self) -> Verdict {
        match &self.outcome {
            Outcome::SameFile => Verdict::SameFile,
            Outcome::Text { content, .. } => content.verdict(),
            Outcome::Binary { binary, .. } => binary.verdict(),
            Outcome::TooLarge => Verdict::TooLarge,
            Outcome::ChangedWhileComparing { .. } => Verdict::ChangedWhileComparing,
        }
    }

    /// The report before the change list, in the reading order, with
    /// what the outcome adds: the choices for a file too large, the note
    /// when only the first bytes or the bytes alone were compared, which
    /// file changed while comparing.
    pub fn summary(
        &self,
        before_name: &str,
        after_name: &str,
        sides: &Sides,
        options: &FileOptions,
    ) -> Vec<String> {
        let (first, second) = if sides.before_first() {
            (before_name, after_name)
        } else {
            (after_name, before_name)
        };
        match &self.outcome {
            Outcome::SameFile => vec![verdict_line(Verdict::SameFile, first, first)],
            Outcome::Text { content, head } => {
                let mut lines = content.summary(before_name, after_name, sides, &options.compare);
                if let Some(most) = head {
                    lines.insert(
                        1,
                        format!("Compared the first {} of each only.", mebibytes(*most)),
                    );
                }
                lines
            }
            Outcome::Binary { binary, too_large } => {
                let mut lines = binary.summary(before_name, after_name, sides);
                if *too_large {
                    lines.insert(
                        1,
                        format!(
                            "Too large to compare as text, over {}; compared byte by byte.",
                            mebibytes(options.max_text_bytes)
                        ),
                    );
                }
                lines
            }
            Outcome::TooLarge => vec![
                verdict_line(Verdict::TooLarge, first, second),
                format!(
                    "Over the limit of {}. Compare the first {} as text, compare byte by byte, or cancel.",
                    mebibytes(options.max_text_bytes),
                    mebibytes(options.max_text_bytes)
                ),
            ],
            Outcome::ChangedWhileComparing { before, after } => {
                let name = if *after { after_name } else { before_name };
                let mut lines = vec![verdict_line(Verdict::ChangedWhileComparing, name, name)];
                if *before && *after {
                    lines.push("Both files changed while comparing.".to_owned());
                }
                lines.push("Compare again when they stop changing.".to_owned());
                lines
            }
        }
    }

    /// The change list's rows: text changes, or binary runs.
    pub fn rows(&self, options: &FileOptions) -> Vec<String> {
        match &self.outcome {
            Outcome::Text { content, .. } => content.rows(&options.compare),
            Outcome::Binary { binary, .. } => binary.runs.iter().map(|run| run.row()).collect(),
            _ => Vec::new(),
        }
    }

    /// The same comparison read the other way (Swap).
    pub fn swapped(&self) -> FileComparison {
        FileComparison {
            before: self.after.clone(),
            after: self.before.clone(),
            outcome: match &self.outcome {
                Outcome::Text { content, head } => Outcome::Text {
                    content: Box::new(content.swapped()),
                    head: *head,
                },
                Outcome::Binary { binary, too_large } => Outcome::Binary {
                    binary: binary.swapped(),
                    too_large: *too_large,
                },
                Outcome::ChangedWhileComparing { before, after } => {
                    Outcome::ChangedWhileComparing {
                        before: *after,
                        after: *before,
                    }
                }
                other => other.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_end_at_a_line_break() {
        assert_eq!(head_of(b"ab\ncd\nef", 7), b"ab\ncd\n");
        assert_eq!(head_of(b"ab\ncd", 10), b"ab\ncd");
        assert_eq!(head_of(b"abcdef", 3), b"abc");
    }

    #[test]
    fn sizes_read_in_mebibytes() {
        assert_eq!(mebibytes(16 * 1024 * 1024), "16 MiB");
        assert_eq!(mebibytes(1024 * 1024 + 1024 * 512), "1.5 MiB");
    }
}
