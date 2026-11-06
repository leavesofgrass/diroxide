//! A plugin's two documents (`ui/show_diff`, alpha 4's host-pieces lane,
//! R30's commit 22) compared for the compare screens.
//!
//! The road taken: the compare holds two documents beside its two places
//! (`app/comparing.rs`'s `Source`), rather than staging the texts as two
//! files in the host's spool. The texts are already in memory, the
//! comparison is the diff crate's own (`compare_contents` for text,
//! `compare_binary` for bytes), and the names the screens say are the
//! documents' titles ("HEAD", "working tree"), not spool files' names;
//! nothing is written to disk and nothing is left to clean up. The cost:
//! the viewer at a change opens only where a place exists, and a
//! document has none (the shape gives a side no place).
//!
//! The comparison runs here, on the plugin's own worker thread, never on
//! the input thread: the app hands over its compare options, this module
//! compares, and the app shows the result.

use std::io::Cursor;

use diroxide_core::{Entry, EntryKind};
use diroxide_diff::{FileComparison, FileOptions, LargeFiles, Outcome};
use diroxide_plugins::ask::DiffDocuments;
use diroxide_preview::encoding::{SNIFF_BYTES, binary_kind};
use diroxide_vfs::CancelToken;

/// Two documents compared, as the compare screens show them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownDiff {
    /// The view's title, the plugin's name last: "hello.txt (Git)".
    pub title: String,
    /// The old document's title: "HEAD".
    pub before: String,
    /// The new document's title: "working tree".
    pub after: String,
    /// What was found.
    pub result: FileComparison,
    /// The options it was compared under.
    pub options: FileOptions,
}

/// The first `most` bytes of a text, up to its last line break when it
/// has one.
fn head(bytes: &[u8], most: usize) -> &[u8] {
    if bytes.len() <= most {
        return bytes;
    }
    let cut = &bytes[..most];
    match cut.iter().rposition(|b| *b == b'\n') {
        Some(at) => &cut[..=at],
        None => cut,
    }
}

/// Whether a document's first bytes say it is binary: a NUL, many
/// control characters, or a known signature, as a file's are.
fn looks_binary(bytes: &[u8]) -> bool {
    binary_kind(&bytes[..bytes.len().min(SNIFF_BYTES)]).is_some()
}

/// Compares a plugin's two documents under the app's compare options: as
/// text, or byte by byte when either looks binary. A text over the limit
/// is compared in its first part as text (or byte by byte when the owner
/// chose that), since there is nothing to ask: the plugin sent it whole.
/// The error is the sentence the plugin receives.
pub fn compare_documents(
    documents: &DiffDocuments,
    options: &FileOptions,
) -> Result<ShownDiff, String> {
    let before = documents.old.text.as_bytes();
    let after = documents.new.text.as_bytes();
    let entry = |title: &str, bytes: &[u8]| {
        Entry::new(title, EntryKind::File).with_size(bytes.len() as u64)
    };
    let cancel = CancelToken::new();
    let most = usize::try_from(options.max_text_bytes).unwrap_or(usize::MAX);
    let over = before.len() > most || after.len() > most;
    let binary = looks_binary(before) || looks_binary(after);
    let failed = |e: diroxide_diff::DiffError| format!("the compare failed: {e}");
    let outcome = if binary || (over && options.large == LargeFiles::Bytes) {
        Outcome::Binary {
            binary: diroxide_diff::compare_binary(
                &mut Cursor::new(before),
                &mut Cursor::new(after),
                &cancel,
            )
            .map_err(failed)?,
            too_large: !binary,
        }
    } else {
        let content = diroxide_diff::compare_contents(
            head(before, most),
            head(after, most),
            &options.compare,
            &cancel,
        )
        .map_err(failed)?;
        Outcome::Text {
            content: Box::new(content),
            head: over.then_some(options.max_text_bytes),
        }
    };
    Ok(ShownDiff {
        title: documents.title.clone(),
        before: documents.old.title.clone(),
        after: documents.new.title.clone(),
        result: FileComparison {
            before: entry(&documents.old.title, before),
            after: entry(&documents.new.title, after),
            outcome,
        },
        options: *options,
    })
}

#[cfg(test)]
mod tests {
    use diroxide_diff::Verdict;
    use diroxide_plugins::ask::DiffSide;

    use super::*;

    fn documents(old: &str, new: &str) -> DiffDocuments {
        DiffDocuments {
            title: "hello.txt (Git)".to_owned(),
            old: DiffSide {
                title: "HEAD".to_owned(),
                text: old.to_owned(),
            },
            new: DiffSide {
                title: "working tree".to_owned(),
                text: new.to_owned(),
            },
        }
    }

    #[test]
    fn two_texts_compare_as_text_and_two_binaries_byte_by_byte() {
        let options = FileOptions::default();
        let shown = compare_documents(&documents("a\nb\n", "a\nc\n"), &options).unwrap();
        assert_eq!(shown.result.verdict(), Verdict::Changes(1));
        assert_eq!(shown.result.before.name, "HEAD");
        assert_eq!(shown.result.after.size, Some(4));
        let shown =
            compare_documents(&documents("PNG\0\0\0\u{1}x", "PNG\0\0\0\u{1}y"), &options).unwrap();
        assert!(matches!(shown.result.outcome, Outcome::Binary { .. }));
        assert_eq!(shown.result.verdict(), Verdict::BinaryBytes(1));
    }

    #[test]
    fn a_text_over_the_limit_is_compared_in_its_first_part() {
        let options = FileOptions {
            max_text_bytes: 8,
            ..FileOptions::default()
        };
        let shown = compare_documents(
            &documents("one\ntwo\nthree\n", "one\ntwo\nfour\n"),
            &options,
        )
        .unwrap();
        assert!(matches!(
            shown.result.outcome,
            Outcome::Text { head: Some(8), .. }
        ));
        assert_eq!(head(b"one\ntwo\nthree\n", 8), b"one\ntwo\n");
        assert_eq!(head(b"abcdefghij", 4), b"abcd");
    }
}
