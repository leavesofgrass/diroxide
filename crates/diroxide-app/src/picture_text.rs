//! Read text in picture (alpha 4, R30 commit 44; R28 2.2 and question
//! 2.7; spike OCR1): Tesseract reads the text in the focused picture,
//! through the tools registry's `tesseract` entry, as an argument list
//! with no shell, and the viewer shows it under the title "Text in
//! picture (machine read)", one line of the picture a line, a blank line
//! between its paragraphs. The details list says how many words it read
//! ("Text in picture (machine read): 14 words", 40 cells).
//!
//! - **The run** is `tesseract INPUT stdout -l LANGS --psm 3 tsv`, at
//!   most 120 seconds and 32 MiB of output, stopped with its children on
//!   a cancel. A picture on a local disk is named by its path; one
//!   elsewhere (in an archive, on a site) goes in on standard input, its
//!   first 64 MiB, as `stdin`.
//! - **The languages** come from `tesseract --list-langs`: English when
//!   it is installed, else the first language listed that is not the
//!   orientation data. Spike OCR1 found 161 languages in this computer's
//!   Tesseract 5.4.0, where asking for ten took 3.8 seconds against 0.6
//!   for English alone, so one language is asked for; nothing is
//!   downloaded.
//! - **Only a picture goes in.** Tesseract reads any file that is not a
//!   picture as a list of picture names, one a line, and opens each
//!   (spike OCR1: a text file naming another picture was read, on Linux
//!   and on Windows, from a path and from standard input alike). So the
//!   file's first bytes must be a picture Tesseract reads (PNG, JPEG,
//!   GIF, TIFF, BMP, WebP, JPEG 2000, PNM) before it runs:
//!   "No text: not a picture Tesseract reads".
//! - **The TSV** has twelve columns (`level`, `page_num`, `block_num`,
//!   `par_num`, `line_num`, `word_num`, `left`, `top`, `width`,
//!   `height`, `conf`, `text`), a header row first; a word is level 5
//!   with a confidence of 0 or more and its text, and the lines and
//!   paragraphs come from the numbers before it.

use std::ffi::OsString;
use std::io::Read;
use std::time::Duration;

use diroxide_core::Place;
use diroxide_core::format::count;
use diroxide_vfs::{
    CancelToken, OpContext, Registry, RunLimits, ToolInput, ToolRequest, ToolRunner,
};

use crate::request::SharedTools;

/// The tools registry's id for Tesseract.
pub const TESSERACT: &str = "tesseract";

/// The title over the text, said first.
pub const TITLE: &str = "Text in picture (machine read)";

/// What is said when Tesseract is not installed (28 cells).
pub const NOT_FOUND: &str = "No text: tesseract not found";

/// What is said when the file is not a picture Tesseract reads.
pub const NOT_A_PICTURE: &str = "No text: not a picture Tesseract reads";

/// The run's limits: two minutes and 32 MiB of output.
pub const LIMITS: RunLimits = RunLimits::new(Duration::from_secs(120), 32 * 1024 * 1024);

/// The limits of `--list-langs`: ten seconds and 64 KiB.
pub const LIST_LIMITS: RunLimits = RunLimits::new(Duration::from_secs(10), 64 * 1024);

/// The most bytes of a picture that is not on a local disk sent on
/// standard input.
pub const MAX_PIPED: u64 = 64 * 1024 * 1024;

/// What Tesseract read in a picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureText {
    /// The lines, a blank one between paragraphs.
    pub lines: Vec<String>,
    /// How many words it read.
    pub words: usize,
    /// The languages asked for: `eng`.
    pub languages: String,
}

impl PictureText {
    /// The count in words: "14 words", "1 word", "no words".
    pub fn words_line(&self) -> String {
        count(self.words as u64, "word", "words")
    }

    /// The details row's words: "Text in picture (machine read): 14
    /// words".
    pub fn detail_line(&self) -> String {
        format!("{TITLE}: {}", self.words_line())
    }

    /// The viewer's text: the title, a blank line, then the lines.
    pub fn text(&self) -> String {
        let mut text = format!("{TITLE}\n\n");
        if self.lines.is_empty() {
            text.push_str("No text was found.\n");
        }
        for line in &self.lines {
            text.push_str(line);
            text.push('\n');
        }
        text
    }
}

/// Whether the first bytes are a picture Tesseract reads: PNG, JPEG,
/// GIF, TIFF, BMP, WebP, JPEG 2000 or PNM.
pub fn is_picture(head: &[u8]) -> bool {
    let starts = |sig: &[u8]| head.starts_with(sig);
    starts(b"\x89PNG\r\n\x1a\n")
        || starts(&[0xff, 0xd8, 0xff])
        || starts(b"GIF87a")
        || starts(b"GIF89a")
        || starts(b"II*\0")
        || starts(b"MM\0*")
        || starts(b"BM")
        || (starts(b"RIFF") && head.get(8..12) == Some(b"WEBP"))
        || starts(&[0, 0, 0, 0x0c, b'j', b'P', b' ', b' '])
        || starts(&[0xff, 0x4f, 0xff, 0x51])
        || (head.len() > 2
            && head[0] == b'P'
            && (b'1'..=b'6').contains(&head[1])
            && head[2].is_ascii_whitespace())
}

/// The languages `tesseract --list-langs` names, after its first line
/// ("List of available languages in ... (161):").
pub fn languages(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("List of available languages"))
        .filter(|line| {
            line.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .map(str::to_owned)
        .collect()
}

/// The language asked for: English when it is installed, else the first
/// listed that is not the orientation or equation data; `None` when there
/// is none.
pub fn choose_languages(available: &[String]) -> Option<String> {
    if available.iter().any(|l| l == "eng") {
        return Some("eng".to_owned());
    }
    available
        .iter()
        .find(|l| !matches!(l.as_str(), "osd" | "equ"))
        .cloned()
}

/// The run's arguments: `INPUT stdout -l LANGS --psm 3 tsv`.
pub fn arguments(input: impl Into<OsString>, languages: &str) -> Vec<OsString> {
    vec![
        input.into(),
        "stdout".into(),
        "-l".into(),
        languages.into(),
        "--psm".into(),
        "3".into(),
        "tsv".into(),
    ]
}

/// A word's text cleaned: control characters become spaces.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The words of Tesseract's TSV as lines: the words of one line joined by
/// spaces, a blank line where a new paragraph or block starts.
pub fn parse_tsv(stdout: &[u8]) -> (Vec<String>, usize) {
    let text = String::from_utf8_lossy(stdout);
    let mut lines: Vec<String> = Vec::new();
    let mut words = 0;
    let mut current: Option<(u32, u32, u32, u32)> = None;
    for row in text.lines().skip(1) {
        let cells: Vec<&str> = row.trim_end_matches('\r').split('\t').collect();
        if cells.len() < 12 || cells[0] != "5" {
            continue;
        }
        let conf: f32 = cells[10].trim().parse().unwrap_or(-1.0);
        let word = clean(&cells[11..].join(" "));
        if conf < 0.0 || word.is_empty() {
            continue;
        }
        let number = |i: usize| cells[i].trim().parse::<u32>().unwrap_or(0);
        let key = (number(1), number(2), number(3), number(4));
        match current {
            Some(at) if at == key => {
                if let Some(last) = lines.last_mut() {
                    last.push(' ');
                    last.push_str(&word);
                }
            }
            Some(at) => {
                if (at.0, at.1, at.2) != (key.0, key.1, key.2) {
                    lines.push(String::new());
                }
                lines.push(word);
            }
            None => lines.push(word),
        }
        current = Some(key);
        words += 1;
    }
    (lines, words)
}

/// Reads the text in a picture: its first bytes checked, the language
/// chosen from `--list-langs`, then one run of Tesseract through the
/// runner. The error is the line to say.
pub fn read(
    registry: &Registry,
    place: &Place,
    tools: &SharedTools,
    cancel: &CancelToken,
) -> Result<PictureText, String> {
    let runner: &dyn ToolRunner = tools.0.as_ref();
    if !runner.has(TESSERACT) {
        return Err(NOT_FOUND.to_owned());
    }
    let name = place
        .name()
        .map(diroxide_core::names::display_name)
        .unwrap_or_default()
        .into_owned();
    let could_not =
        |cause: &str| diroxide_core::Message::could_not("read text in", &name, cause, "").text;
    let provider = registry
        .resolve(place)
        .map_err(|e| could_not(&e.to_string()))?;
    let ctx = OpContext::new().with_cancel(cancel.clone());
    let local = place.local_path().map(std::path::PathBuf::from);
    let limit = if local.is_some() { 16 } else { MAX_PIPED };
    let mut bytes = Vec::new();
    provider
        .read(place, &ctx)
        .and_then(|reader| {
            reader
                .take(limit)
                .read_to_end(&mut bytes)
                .map_err(diroxide_vfs::VfsError::from)
        })
        .map_err(|e| could_not(&e.to_string()))?;
    if !is_picture(&bytes) {
        return Err(NOT_A_PICTURE.to_owned());
    }
    let listed = runner
        .run(
            &ToolRequest::new(TESSERACT, ["--list-langs"]).with_limits(LIST_LIMITS),
            cancel,
        )
        .map_err(|e| could_not(&e.to_string()))?;
    let available = languages(&listed.stdout);
    let Some(chosen) = choose_languages(&available) else {
        return Err(could_not("Tesseract has no languages installed"));
    };
    let request = match local {
        Some(path) => ToolRequest::new(TESSERACT, arguments(path, &chosen)),
        None => ToolRequest::new(TESSERACT, arguments("stdin", &chosen))
            .with_input(ToolInput::Bytes(bytes)),
    }
    .with_limits(LIMITS);
    let output = runner
        .run(&request, cancel)
        .map_err(|e| could_not(&e.to_string()))?;
    if !output.succeeded() && output.stdout.is_empty() {
        return Err(could_not("Tesseract could not read the picture"));
    }
    let (lines, words) = parse_tsv(&output.stdout);
    Ok(PictureText {
        lines,
        words,
        languages: chosen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spike OCR1's TSV from Tesseract 5.5.0 in the tools container, on a
    /// picture ImageMagick drew with two lines of text.
    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t940\t240\t-1\t
2\t1\t1\t0\t0\t0\t21\t27\t524\t77\t-1\t
3\t1\t1\t1\t0\t0\t21\t27\t524\t77\t-1\t
4\t1\t1\t1\t1\t0\t25\t27\t374\t28\t-1\t
5\t1\t1\t1\t1\t1\t25\t28\t105\t27\t96.754356\tCrows
5\t1\t1\t1\t1\t2\t146\t29\t89\t26\t96.676384\tmeet
5\t1\t1\t1\t1\t3\t249\t29\t33\t26\t94.787781\tat
5\t1\t1\t1\t1\t4\t296\t27\t103\t28\t96.568184\tdawn.
4\t1\t1\t1\t2\t0\t21\t70\t524\t34\t-1\t
5\t1\t1\t1\t2\t1\t21\t70\t137\t34\t92.843300\tJimothy
5\t1\t1\t1\t2\t2\t173\t70\t95\t28\t96.464149\treads
5\t1\t1\t1\t2\t3\t285\t71\t40\t26\t96.624184\t14
5\t1\t1\t1\t2\t4\t340\t70\t103\t28\t96.438271\twords
5\t1\t1\t1\t2\t5\t459\t70\t86\t28\t96.074409\there.
";

    #[test]
    fn the_tsv_reads_as_lines_and_counts_its_words() {
        let (lines, words) = parse_tsv(TSV.as_bytes());
        assert_eq!(
            lines,
            ["Crows meet at dawn.", "Jimothy reads 14 words here."]
        );
        assert_eq!(words, 9);
        // Windows' line ends and a second paragraph.
        let windows = TSV
            .replace('\n', "\r\n")
            .replace("5\t1\t1\t1\t2\t1\t", "5\t1\t1\t2\t1\t1\t");
        let (lines, _) = parse_tsv(windows.as_bytes());
        assert_eq!(lines[1], "", "a blank line between paragraphs: {lines:?}");
        let text = PictureText {
            lines: vec!["Crows meet at dawn.".to_owned()],
            words: 4,
            languages: "eng".to_owned(),
        };
        assert_eq!(
            text.text(),
            "Text in picture (machine read)\n\nCrows meet at dawn.\n"
        );
        let fourteen = PictureText {
            words: 14,
            ..text.clone()
        };
        assert_eq!(
            fourteen.detail_line(),
            "Text in picture (machine read): 14 words"
        );
        for line in [
            fourteen.detail_line(),
            NOT_FOUND.to_owned(),
            NOT_A_PICTURE.to_owned(),
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn english_is_asked_for_first_and_the_orientation_data_never() {
        let listed = b"List of available languages in \"/usr/share/tesseract-ocr/5/tessdata/\" (2):\neng\nosd\n";
        assert_eq!(languages(listed), ["eng", "osd"]);
        assert_eq!(choose_languages(&languages(listed)).as_deref(), Some("eng"));
        let others = vec!["osd".to_owned(), "deu".to_owned(), "fra".to_owned()];
        assert_eq!(choose_languages(&others).as_deref(), Some("deu"));
        assert_eq!(choose_languages(&["osd".to_owned()]), None);
        let windows = b"List of available languages in \"C:\\Program Files\\Tesseract-OCR/tessdata/\" (3):\r\nafr\r\neng\r\nosd\r\n";
        assert_eq!(languages(windows), ["afr", "eng", "osd"]);
        assert_eq!(
            arguments("photo.png", "eng"),
            ["photo.png", "stdout", "-l", "eng", "--psm", "3", "tsv"]
        );
    }

    #[test]
    fn only_a_picture_goes_in() {
        for head in [
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice(),
            &[0xff, 0xd8, 0xff, 0xe0],
            b"GIF89a",
            b"II*\0",
            b"MM\0*",
            b"BM\0\0",
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"P6\n4 3\n255\n",
        ] {
            assert!(is_picture(head), "{head:?}");
        }
        // A list of names, which Tesseract would open one by one.
        assert!(!is_picture(b"/home/someone/secret.png\n"));
        assert!(!is_picture(b"not a picture at all"));
        assert!(!is_picture(b""));
    }
}
