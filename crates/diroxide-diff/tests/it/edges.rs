//! The edge cases of research/diffing.md section 9, for two files,
//! through the local provider (folder compare is alpha.8's, and its cases
//! wait for it). Files are written to a temporary folder under the test
//! run's own temporary root.

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use diroxide_core::{Entry, Message, Place};
use diroxide_diff::{
    CompareOptions, DiffError, FileComparison, FileOptions, FileSide, Ignore, LargeFiles, Outcome,
    Sides, Verdict, compare_files, verdict_line,
};
use diroxide_vfs::{
    AttributeChange, CancelToken, Capabilities, FileWriter, LocalProvider, OpContext, Provider,
    VfsError, WriteMode,
};

struct Folder {
    dir: tempfile::TempDir,
}

impl Folder {
    fn new() -> Folder {
        Folder {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn place(&self, name: &str) -> Place {
        Place::parse(self.dir.path().join(name).to_str().unwrap()).unwrap()
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Place {
        std::fs::write(self.dir.path().join(name), bytes).unwrap();
        self.place(name)
    }
}

fn compare_with(
    provider: &dyn Provider,
    before: &Place,
    after: &Place,
    options: &FileOptions,
    ctx: &OpContext,
) -> Result<FileComparison, DiffError> {
    compare_files(
        FileSide {
            provider,
            place: before,
        },
        FileSide {
            provider,
            place: after,
        },
        options,
        ctx,
    )
}

fn compare(before: &Place, after: &Place) -> FileComparison {
    compare_with(
        &LocalProvider::new(),
        before,
        after,
        &FileOptions::default(),
        &OpContext::new(),
    )
    .unwrap()
}

/// The report as the command line says it, before and after named.
fn said(result: &FileComparison) -> Vec<String> {
    let mut lines = result.summary(
        "a.txt",
        "b.txt",
        &Sides::files("a.txt", "b.txt"),
        &FileOptions::default(),
    );
    lines.extend(result.rows(&FileOptions::default()));
    lines
}

#[test]
fn identical_files_are_the_same() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"one\ntwo\n");
    let b = folder.write("b.txt", b"one\ntwo\n");
    let result = compare(&a, &b);
    assert_eq!(result.verdict(), Verdict::Same);
    assert_eq!(said(&result), ["Same: a.txt and b.txt"]);
}

#[test]
fn a_file_against_itself_is_the_same_file() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"one\n");
    let result = compare(&a, &a);
    assert_eq!(result.outcome, Outcome::SameFile);
    assert_eq!(said(&result), ["Same file: a.txt"]);
}

#[test]
fn two_names_for_one_file_are_the_same_file() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"one\n");
    std::fs::hard_link(
        folder.dir.path().join("a.txt"),
        folder.dir.path().join("b.txt"),
    )
    .unwrap();
    let b = folder.place("b.txt");
    assert_eq!(compare(&a, &b).outcome, Outcome::SameFile);
}

#[test]
fn empty_files_and_single_lines() {
    let folder = Folder::new();
    let empty = folder.write("empty.txt", b"");
    let other_empty = folder.write("other.txt", b"");
    assert_eq!(compare(&empty, &other_empty).verdict(), Verdict::Same);
    let full = folder.write("full.txt", b"one\ntwo\n");
    assert_eq!(
        said(&compare(&empty, &full)),
        [
            "1 change between a.txt and b.txt",
            "Added 1.",
            "From a.txt to b.txt.",
            "Added lines 1 to 2, 2 lines"
        ]
    );
    let one = folder.write("one.txt", b"total due");
    let two = folder.write("two.txt", b"total owed");
    assert_eq!(
        said(&compare(&one, &two))[3],
        "Changed line 1, was: total due, now: total owed"
    );
    let with_break = folder.write("break.txt", b"total due\n");
    assert_eq!(
        said(&compare(&with_break, &one)),
        [
            "Same text: a.txt and b.txt",
            "Last line has no line break in b.txt."
        ]
    );
}

#[test]
fn line_endings_and_byte_order_marks_are_facts() {
    let folder = Folder::new();
    let lf = folder.write("lf.txt", b"a\nb\nc\n");
    let crlf = folder.write("crlf.txt", b"a\r\nb\r\nc\r\n");
    let mixed = folder.write("mixed.txt", b"a\nb\r\nc\n");
    let bom = folder.write("bom.txt", b"\xef\xbb\xbfa\nb\nc\n");
    assert_eq!(
        said(&compare(&lf, &crlf))[1],
        "Line endings differ: LF in a.txt, CRLF in b.txt. Compared ignoring them."
    );
    assert_eq!(
        said(&compare(&lf, &mixed))[1],
        "Line endings differ: LF in a.txt, mixed in b.txt. Compared ignoring them."
    );
    assert_eq!(
        said(&compare(&lf, &bom))[1],
        "Byte-order mark in b.txt, none in a.txt."
    );
}

#[test]
fn encodings_are_facts_and_latin_1_is_no_crash() {
    let folder = Folder::new();
    let utf8 = folder.write("utf8.txt", "caf\u{e9}\n".as_bytes());
    let mut utf16 = vec![0xff, 0xfe];
    for unit in "caf\u{e9}\n".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let utf16 = folder.write("utf16.txt", &utf16);
    let latin = folder.write("latin.txt", b"caf\xe9\n");
    let result = compare(&utf8, &utf16);
    assert_eq!(result.verdict(), Verdict::SameText);
    assert_eq!(
        said(&result)[1],
        "Encodings differ: UTF-8 in a.txt, UTF-16 in b.txt."
    );
    let latin_result = compare(&utf8, &latin);
    assert_eq!(latin_result.verdict(), Verdict::SameText);
    assert_eq!(
        said(&latin_result)[1],
        "Encodings differ: UTF-8 in a.txt, Windows-1252 in b.txt."
    );
}

#[test]
fn a_nul_past_the_first_8_kib_is_text_and_said_by_its_code_point() {
    let folder = Folder::new();
    let line = "x".repeat(99);
    let mut text = String::new();
    while text.len() < 9_000 {
        text.push_str(&line);
        text.push('\n');
    }
    let before = folder.write("before.txt", text.as_bytes());
    let mut nul = text.clone().into_bytes();
    nul.extend_from_slice(b"tail\x00end\n");
    text.push_str("tailend\n");
    let clean = folder.write("clean.txt", text.as_bytes());
    let with_nul = folder.write("nul.txt", &nul);
    let result = compare(&clean, &with_nul);
    assert!(matches!(result.outcome, Outcome::Text { .. }));
    assert!(
        said(&result)
            .last()
            .unwrap()
            .ends_with("hidden character U+0000 added"),
        "{:?}",
        said(&result)
    );
    // UTF-16 text has NULs and is still text.
    let mut utf16 = vec![0xff, 0xfe];
    for unit in "one\ntwo\n".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let utf16 = folder.write("utf16.txt", &utf16);
    assert!(matches!(
        compare(&before, &utf16).outcome,
        Outcome::Text { .. }
    ));
}

#[test]
fn hard_inputs_finish_and_say_so() {
    let folder = Folder::new();
    // Every line different.
    let many: String = (0..2_000).map(|i| format!("a{i}\n")).collect();
    let others: String = (0..2_000).map(|i| format!("b{i}\n")).collect();
    let a = folder.write("a.txt", many.as_bytes());
    let b = folder.write("b.txt", others.as_bytes());
    let all = compare(&a, &b);
    assert_eq!(all.verdict(), Verdict::Changes(1));
    assert_eq!(said(&all)[3], "Changed lines 1 to 2,000, 2,000 lines");
    // One huge line with one word changed.
    let huge = format!("{} due {}", "word ".repeat(30_000), "word ".repeat(30_000));
    let huge_after = huge.replacen(" due ", " owed ", 1);
    let h1 = folder.write("h1.txt", huge.as_bytes());
    let h2 = folder.write("h2.txt", huge_after.as_bytes());
    assert_eq!(
        said(&compare(&h1, &h2))[3],
        "Changed line 1, line too long to compare by word"
    );
    // A hundred thousand tiny lines and one change.
    let tiny: String = (0..100_000).map(|i| format!("{}\n", i % 10)).collect();
    let mut tiny_after = tiny.clone();
    tiny_after.insert_str(50_000, "x\n");
    let t1 = folder.write("t1.txt", tiny.as_bytes());
    let t2 = folder.write("t2.txt", tiny_after.as_bytes());
    assert_eq!(compare(&t1, &t2).verdict(), Verdict::Changes(1));
    // A deadline that passes mid-run says so, and the list still holds.
    let options = FileOptions {
        compare: CompareOptions {
            deadline: Duration::ZERO,
            ..CompareOptions::default()
        },
        ..FileOptions::default()
    };
    let late = compare_with(&LocalProvider::new(), &a, &b, &options, &OpContext::new()).unwrap();
    let lines = late.summary("a.txt", "b.txt", &Sides::files("a.txt", "b.txt"), &options);
    assert!(
        lines
            .last()
            .unwrap()
            .starts_with("Stopped after 0 seconds;"),
        "{lines:?}"
    );
}

#[test]
fn binary_files_are_compared_by_bytes() {
    let folder = Folder::new();
    let a = folder.write("a.bin", b"\x00\x01\x02\x03\x04\x05");
    let b = folder.write("b.bin", b"\x00\x01\xff\x03\x04\x05");
    let result = compare(&a, &b);
    assert_eq!(result.verdict(), Verdict::BinaryBytes(1));
    assert_eq!(
        said(&result),
        [
            "Binary, 1 byte differs: a.txt and b.txt",
            "Both 6 bytes; 1 byte differs in 1 run.",
            "First difference at byte 2.",
            "Byte 2 differs"
        ]
    );
}

#[test]
fn large_text_files_follow_the_policy() {
    let folder = Folder::new();
    let text: String = (0..400).map(|i| format!("line {i}\n")).collect();
    let mut changed = text.clone();
    changed.push_str("one more\n");
    let a = folder.write("a.txt", text.as_bytes());
    let b = folder.write("b.txt", changed.as_bytes());
    let policy = |large| FileOptions {
        max_text_bytes: 1024,
        large,
        ..FileOptions::default()
    };
    let run = |options: &FileOptions| {
        compare_with(&LocalProvider::new(), &a, &b, options, &OpContext::new()).unwrap()
    };
    let ask = run(&policy(LargeFiles::Ask));
    assert_eq!(ask.outcome, Outcome::TooLarge);
    let asked = ask.summary(
        "a.txt",
        "b.txt",
        &Sides::files("a.txt", "b.txt"),
        &policy(LargeFiles::Ask),
    );
    // Thirty cells of verdict leave too little for two names: the line
    // stands alone, and the full sentence keeps them.
    assert_eq!(asked[0], "Too large to compare as text");
    assert_eq!(
        asked[1],
        "Over the limit of 1,024 bytes. Compare the first 1,024 bytes as text, compare byte by byte, or cancel."
    );
    let head = run(&policy(LargeFiles::Head));
    assert_eq!(head.verdict(), Verdict::Same);
    let headed = head.summary(
        "a.txt",
        "b.txt",
        &Sides::files("a.txt", "b.txt"),
        &policy(LargeFiles::Head),
    );
    assert!(headed[1].starts_with("Compared the first"), "{headed:?}");
    let bytes = run(&policy(LargeFiles::Bytes));
    assert_eq!(bytes.verdict(), Verdict::BinarySizes);
    let by_bytes = bytes.summary(
        "a.txt",
        "b.txt",
        &Sides::files("a.txt", "b.txt"),
        &policy(LargeFiles::Bytes),
    );
    assert!(
        by_bytes[1].starts_with("Too large to compare as text"),
        "{by_bytes:?}"
    );
}

#[test]
fn a_missing_file_or_a_folder_is_not_compared() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"one\n");
    let missing = folder.place("missing.txt");
    let error = compare_with(
        &LocalProvider::new(),
        &a,
        &missing,
        &FileOptions::default(),
        &OpContext::new(),
    )
    .unwrap_err();
    assert_eq!(error, DiffError::Read(VfsError::NotFound));
    let sentence = Message::from_error("compare", "missing.txt", &error).text;
    assert!(sentence.starts_with("Could not compare missing.txt: it does not exist."));
    std::fs::create_dir(folder.dir.path().join("sub")).unwrap();
    let sub = folder.place("sub");
    let error = compare_with(
        &LocalProvider::new(),
        &a,
        &sub,
        &FileOptions::default(),
        &OpContext::new(),
    )
    .unwrap_err();
    assert!(matches!(error, DiffError::NotAFile(_)));
}

/// The local provider, with a hook on every look and read: it can say a
/// file grew after the first look, or cancel the run at its nth call.
#[derive(Debug)]
struct Watched {
    inner: LocalProvider,
    calls: AtomicUsize,
    grow_after: Option<usize>,
    cancel_at: Option<(usize, CancelToken)>,
}

impl Watched {
    fn new() -> Watched {
        Watched {
            inner: LocalProvider::new(),
            calls: AtomicUsize::new(0),
            grow_after: None,
            cancel_at: None,
        }
    }

    fn call(&self) -> usize {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some((at, cancel)) = &self.cancel_at
            && *at == n
        {
            cancel.cancel();
        }
        n
    }
}

impl Provider for Watched {
    fn scheme(&self) -> &str {
        self.inner.scheme()
    }
    fn capabilities(&self, place: &Place) -> Capabilities {
        self.inner.capabilities(place)
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.inner.list(place, ctx)
    }
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        let n = self.call();
        let mut entry = self.inner.stat(place, ctx)?;
        if self.grow_after.is_some_and(|after| n >= after) {
            entry.size = entry.size.map(|size| size + 1);
        }
        Ok(entry)
    }
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.call();
        self.inner.read(place, ctx)
    }
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.inner.write(place, mode, ctx)
    }
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.create_dir(place, ctx)
    }
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.rename(from, to, ctx)
    }
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.remove(place, ctx)
    }
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.inner.set_attributes(place, change, ctx)
    }
}

#[test]
fn a_file_that_changes_while_compared_leaves_no_result() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"one\n");
    let b = folder.write("b.txt", b"two\n");
    // Two first looks, two reads, then the last looks find b larger.
    let provider = Watched {
        grow_after: Some(5),
        ..Watched::new()
    };
    let result = compare_with(
        &provider,
        &a,
        &b,
        &FileOptions::default(),
        &OpContext::new(),
    )
    .unwrap();
    assert_eq!(
        result.outcome,
        Outcome::ChangedWhileComparing {
            before: false,
            after: true
        }
    );
    assert_eq!(
        said(&result),
        [
            "Changed while comparing: b.txt",
            "Compare again when they stop changing."
        ]
    );
}

#[test]
fn a_cancel_at_every_step_is_not_compared() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"one\ntwo\n");
    let b = folder.write("b.txt", b"one\nthree\n");
    // The steps: two first looks, two reads, two last looks.
    for step in 0..6 {
        let cancel = CancelToken::new();
        let provider = Watched {
            cancel_at: Some((step, cancel.clone())),
            ..Watched::new()
        };
        let ctx = OpContext::new().with_cancel(cancel);
        let got = compare_with(&provider, &a, &b, &FileOptions::default(), &ctx);
        assert_eq!(got, Err(DiffError::Cancelled), "step {step}");
    }
    assert_eq!(
        verdict_line(Verdict::NotCompared, "a.txt", "b.txt"),
        "Not compared: a.txt and b.txt"
    );
}

#[test]
fn ignore_options_reach_the_file_compare() {
    let folder = Folder::new();
    let a = folder.write("a.txt", b"Total Due\n");
    let b = folder.write("b.txt", b"total due\n");
    let options = FileOptions {
        compare: CompareOptions {
            ignore: Ignore {
                case: true,
                ..Ignore::default()
            },
            ..CompareOptions::default()
        },
        ..FileOptions::default()
    };
    let result = compare_with(&LocalProvider::new(), &a, &b, &options, &OpContext::new()).unwrap();
    let lines = result.summary("a.txt", "b.txt", &Sides::files("a.txt", "b.txt"), &options);
    assert_eq!(
        lines,
        [
            "Same text: a.txt and b.txt",
            "Ignoring changes in case.",
            "1 line differs only in what was ignored."
        ]
    );
}

/// Collects progress updates.
#[derive(Default)]
struct Heard(std::sync::Mutex<Vec<diroxide_vfs::ProgressUpdate>>);

impl diroxide_vfs::ProgressSink for Heard {
    fn report(&self, update: diroxide_vfs::ProgressUpdate) {
        self.0.lock().unwrap().push(update);
    }
}

#[test]
fn reading_reports_progress_in_bytes() {
    let folder = Folder::new();
    let size = 3 * 1024 * 1024;
    let text: String = "word\n".repeat(size / 5);
    let a = folder.write("a.txt", text.as_bytes());
    let b = folder.write("b.txt", text.as_bytes());
    let heard = std::sync::Arc::new(Heard::default());
    let ctx = OpContext::new().with_progress(heard.clone());
    let result =
        compare_with(&LocalProvider::new(), &a, &b, &FileOptions::default(), &ctx).unwrap();
    assert_eq!(result.verdict(), Verdict::Same);
    let updates = heard.0.lock().unwrap();
    assert!(updates.len() >= 5, "{} updates", updates.len());
    let total = (text.len() * 2) as u64;
    assert!(updates.iter().all(|u| u.bytes_total == Some(total)));
    assert!(
        updates
            .windows(2)
            .all(|w| w[0].bytes_done < w[1].bytes_done)
    );
}
