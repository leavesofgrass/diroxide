//! Reading a tool's version (alpha.7, R28 1.3; spike T3).
//!
//! The version command is run with its version arguments only, never a
//! file: standard input closed, in the program's own folder (so nothing in
//! the owner's current folder, such as a planted configuration file, is
//! read), two seconds at most, the first 64 KiB of each stream kept.
//!
//! The **version line** is the first non-empty line of standard output,
//! else of standard error, cleaned to 80 cells: Poppler's `pdfinfo -v`
//! writes only to standard error, and 7-Zip's banner starts with an empty
//! line. The exit code does not matter: Xpdf's `pdftotext -v` exits with
//! 99 (spike T3).
//!
//! The **version** is the number in that line, as the Settings page and
//! `diro doctor` say it: "7.1.2-35" from "Version: ImageMagick 7.1.2-35
//! Q16-HDRI x64", "5.4.0.20240606" from "tesseract v5.4.0.20240606",
//! "9.0.2" from BtbN's "ffmpeg version n9.0.2-17-g2a571b6068-20260930".

use std::path::Path;
use std::time::Duration;

use diroxide_core::names::{CharClass, classify};

use crate::process::ProcessSpec;
use crate::run::{Caps, Limited, run_limited};

/// How long a version command may run.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest version line, in cells.
pub const VERSION_LINE_CELLS: usize = 80;

/// How much of each stream a version command may write before the rest
/// is ignored.
const VERSION_OUTPUT_CAP: u64 = 64 * 1024;

/// Runs a version command and gives its version line, or `None` when it
/// could not start, said nothing, or ran past [`VERSION_TIMEOUT`] (it and
/// its children are then stopped). `args` are the version arguments
/// alone, never a file.
pub fn version_line(program: &Path, args: &[&str]) -> Option<String> {
    version_line_within(program, args, VERSION_TIMEOUT)
}

/// The same, stopping the command after `timeout` rather than
/// [`VERSION_TIMEOUT`]: for a registry given a longer deadline
/// ([`crate::ToolRegistry::with_version_timeout`]), as the lookup tests are,
/// whose fake tools are scripts a loaded machine once took 2.8 seconds to
/// start (E's merge check, Saturday, November 7, 2026).
pub fn version_line_within(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut spec = ProcessSpec::new(program)
        .args(args.iter().copied())
        .diroxide_environment();
    if let Some(folder) = program.parent().filter(|folder| folder.is_absolute()) {
        spec = spec.cwd(folder);
    }
    let limited = Limited::new(
        Some(timeout),
        Caps::each(VERSION_OUTPUT_CAP, VERSION_OUTPUT_CAP),
    );
    let done = run_limited(spec, limited, &|| false).ok()?;
    first_line(&done.stdout, &done.stderr)
}

/// The version line of a command's output: the first non-empty line of
/// standard output, else of standard error, cleaned to
/// [`VERSION_LINE_CELLS`].
pub fn first_line(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    [stdout, stderr].into_iter().find_map(|bytes| {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|line| clean_line(line, VERSION_LINE_CELLS))
            .find(|line| !line.is_empty())
    })
}

/// One line of a program's words for the screen and the screen reader:
/// terminal escape sequences and control, format and direction characters
/// removed, tabs and line breaks made spaces, runs of spaces made one,
/// trimmed, and cut to `max` cells.
pub fn clean_line(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max * 4));
    let mut count = 0;
    let mut space = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1B}' || c == '\u{9B}' {
            skip_escape(c, &mut chars);
            continue;
        }
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if classify(c) != CharClass::Normal {
            continue;
        }
        if space {
            if count + 1 >= max {
                break;
            }
            out.push(' ');
            count += 1;
            space = false;
        }
        if count >= max {
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Skips one terminal escape sequence after its introducer: CSI (`ESC [`
/// or U+009B) up to its final byte, OSC (`ESC ]`) up to BEL or `ESC \`,
/// any other `ESC x` pair.
fn skip_escape(first: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let kind = if first == '\u{9B}' {
        '['
    } else {
        match chars.next() {
            Some(kind) => kind,
            None => return,
        }
    };
    match kind {
        '[' => {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7E}').contains(&c) {
                    break;
                }
            }
        }
        ']' | 'P' | '_' | '^' => {
            while let Some(c) = chars.next() {
                if c == '\u{07}' {
                    break;
                }
                if c == '\u{1B}' && chars.peek() == Some(&'\\') {
                    chars.next();
                    break;
                }
            }
        }
        _ => {}
    }
}

/// The version number in a version line: the first word that, past a
/// leading `v` or `n`, is numbers joined by dots ("7.1.2", "25.01"),
/// kept with a dash and a number after it when that is the whole word
/// (ImageMagick's "7.1.2-35"); anything else after the numbers is left
/// out ("2.54.0" from "2.54.0.windows.1", "7.1.5" from
/// "7.1.5-0+deb13u1"). `None` when no word has one.
pub fn version_number(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let word =
            word.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';' | ':' | '[' | ']'));
        let bare = word
            .strip_prefix(['v', 'V', 'n'])
            .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            .unwrap_or(word);
        dotted(bare)
    })
}

/// The leading `N.N[.N...]` of a word, with a `-N` patch level when that
/// ends the word.
fn dotted(word: &str) -> Option<String> {
    // `end` is just past the last digit of the run; a dot counts only
    // after a digit, so the run never ends in a dot.
    let mut end = 0;
    let mut last_digit = false;
    for (index, c) in word.char_indices() {
        if c.is_ascii_digit() {
            end = index + 1;
            last_digit = true;
        } else if c == '.' && last_digit {
            last_digit = false;
        } else {
            break;
        }
    }
    let numbers = &word[..end];
    if !numbers.contains('.') {
        return None;
    }
    let rest = &word[end..];
    match rest.strip_prefix('-') {
        Some(patch) if !patch.is_empty() && patch.chars().all(|c| c.is_ascii_digit()) => {
            Some(word.to_owned())
        }
        _ => Some(numbers.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spike T3's golden lines: the first line each version command wrote
    /// in the development image's tools stage (Debian trixie) and on the
    /// owner's Windows computer, Friday, November 6, 2026, and the number
    /// the line gives.
    const GOLDEN: &[(&str, &str)] = &[
        (
            "Version: ImageMagick 7.1.1-43 Q16 x86_64 22550 https://imagemagick.org",
            "7.1.1-43",
        ),
        (
            "ffmpeg version 7.1.5-0+deb13u1 Copyright (c) 2000-2026 the FFmpeg developers",
            "7.1.5",
        ),
        (
            "ffmpeg version n9.0.2-17-g2a571b6068-20260930 Copyright (c) 2000-2026 the FFmpeg developers",
            "9.0.2",
        ),
        (
            "ffprobe version 7.1.5-0+deb13u1 Copyright (c) 2007-2026 the FFmpeg developers",
            "7.1.5",
        ),
        ("pandoc 3.1.11.1", "3.1.11.1"),
        ("pandoc 3.9.0.2", "3.9.0.2"),
        (
            "7-Zip 25.01 (x64) : Copyright (c) 1999-2025 Igor Pavlov : 2025-08-03",
            "25.01",
        ),
        (
            "7-Zip (a) 25.01 (x64) : Copyright (c) 1999-2025 Igor Pavlov : 2025-08-03",
            "25.01",
        ),
        (
            "7-Zip (r) 25.01 (x64) : Igor Pavlov : Public domain : 2025-08-03",
            "25.01",
        ),
        (
            "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11",
            "24.08",
        ),
        ("pdftotext version 25.03.0", "25.03.0"),
        ("pdfinfo version 25.03.0", "25.03.0"),
        ("pdftotext version 4.00", "4.00"),
        ("13.25", "13.25"),
        ("tesseract 5.5.0", "5.5.0"),
        ("tesseract v5.4.0.20240606", "5.4.0.20240606"),
        (
            "qemu-img version 10.0.13 (Debian 1:10.0.13+ds-0+deb13u1)",
            "10.0.13",
        ),
        ("xorriso 1.5.6", "1.5.6"),
        ("gpg (GnuPG) 2.4.7", "2.4.7"),
        ("gpg (GnuPG) 2.4.9", "2.4.9"),
        ("1.2.1", "1.2.1"),
        ("v1.3.2", "1.3.2"),
        ("minisign 0.12", "0.12"),
        ("git version 2.47.3", "2.47.3"),
        ("git version 2.54.0.windows.1", "2.54.0"),
        ("gh version 2.95.0 (2026-06-17)", "2.95.0"),
        (
            "curl 8.14.1 (x86_64-pc-linux-gnu) libcurl/8.14.1 OpenSSL/3.5.6",
            "8.14.1",
        ),
        (
            "curl 8.21.0 (Windows) libcurl/8.21.0 Schannel zlib/1.3.2 WinIDN WinLDAP",
            "8.21.0",
        ),
        ("GNU Wget 1.25.0 built on linux-gnu.", "1.25.0"),
    ];

    #[test]
    fn the_golden_lines_give_their_numbers() {
        for (line, number) in GOLDEN {
            assert_eq!(version_number(line).as_deref(), Some(*number), "{line}");
        }
        assert_eq!(version_number("Invalid Parameter - -version"), None);
        assert_eq!(version_number("usage: tool [options]"), None);
        assert_eq!(version_number("released 2026"), None);
        assert_eq!(version_number("1. Introduction"), None);
        assert_eq!(version_number("v"), None);
    }

    /// Standard output first, then standard error; an empty first line
    /// (7-Zip's banner) is passed over; a long line is cut to 80 cells.
    #[test]
    fn the_line_comes_from_standard_output_then_standard_error() {
        assert_eq!(
            first_line(b"\r\n7-Zip 24.08 (x64) : Copyright\r\n", b"").as_deref(),
            Some("7-Zip 24.08 (x64) : Copyright")
        );
        assert_eq!(
            first_line(b"", b"pdfinfo version 25.03.0\nCopyright 2005-2025\n").as_deref(),
            Some("pdfinfo version 25.03.0")
        );
        assert_eq!(
            first_line(
                b"xorriso 1.5.6\n",
                b"xorriso 1.5.6 : RockRidge filesystem manipulator\n"
            )
            .as_deref(),
            Some("xorriso 1.5.6")
        );
        assert_eq!(first_line(b"  \n\t\n", b""), None);
        let long = format!("tool {}", "x".repeat(200));
        let line = first_line(long.as_bytes(), b"").unwrap();
        assert_eq!(line.chars().count(), VERSION_LINE_CELLS);
    }

    /// This test binary run again as a slow version command; an ordinary
    /// test run does nothing here.
    #[test]
    fn slow_helper() {
        if std::env::args().any(|arg| arg == "mode=sleep") {
            std::thread::sleep(Duration::from_secs(60));
            std::process::exit(0);
        }
    }

    /// A version command that does not end in two seconds is stopped, and
    /// gives no line.
    #[test]
    fn a_version_command_past_its_time_is_stopped() {
        let exe = std::env::current_exe().unwrap();
        let started = std::time::Instant::now();
        let line = version_line(
            &exe,
            &[
                "--exact",
                "version::tests::slow_helper",
                "--nocapture",
                "--test-threads",
                "1",
                "mode=sleep",
            ],
        );
        assert_eq!(line, None);
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn escapes_and_hidden_characters_are_cleaned() {
        assert_eq!(
            clean_line("\u{1b}[1mtool\u{1b}[0m 1.0\u{202e}\u{7}\tbeta", 80),
            "tool 1.0 beta"
        );
        assert_eq!(clean_line("\u{1b}]0;title\u{7}tool 2.0", 80), "tool 2.0");
        assert_eq!(clean_line("  a   b  ", 80), "a b");
        assert_eq!(clean_line("abcdef", 3), "abc");
    }
}
