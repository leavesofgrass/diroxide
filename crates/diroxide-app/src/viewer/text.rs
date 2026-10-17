//! How the viewer reads text out of bytes: the encoding chosen from the
//! file's first bytes (the preview crate's `encoding` module), where its
//! lines end, and the line as shown, with every hidden or control
//! character made visible so nothing can move the terminal's cursor or
//! hide from a screen reader.

use std::io;

use diroxide_core::names::classify;
use diroxide_preview::encoding::{Choice, choose, choose_head};

use super::source::{BLOCK, ViewerSource};

/// How many columns a tab stop is.
pub(crate) const TAB_STOP: usize = 4;

/// How a line ends in the file's encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Newline {
    /// A line feed byte: UTF-8, Windows-1252 and every ASCII superset.
    Byte,
    /// A line feed code unit in UTF-16, little endian.
    Utf16Le,
    /// A line feed code unit in UTF-16, big endian.
    Utf16Be,
}

impl Newline {
    /// The bytes of one code unit: 1, or 2 in UTF-16.
    pub(crate) const fn unit(self) -> usize {
        match self {
            Newline::Byte => 1,
            Newline::Utf16Le | Newline::Utf16Be => 2,
        }
    }

    /// Whether the code unit at the start of `bytes` ends a line.
    pub(crate) fn is_at(self, bytes: &[u8]) -> bool {
        match self {
            Newline::Byte => bytes.first() == Some(&b'\n'),
            Newline::Utf16Le => bytes.starts_with(&[b'\n', 0]),
            Newline::Utf16Be => bytes.starts_with(&[0, b'\n']),
        }
    }

    /// The offsets in `bytes` just after each line end, `bytes` starting on
    /// a code unit.
    pub(crate) fn ends(self, bytes: &[u8], mut each: impl FnMut(usize)) {
        match self {
            Newline::Byte => {
                for (at, _) in bytes.iter().enumerate().filter(|(_, b)| **b == b'\n') {
                    each(at + 1);
                }
            }
            Newline::Utf16Le | Newline::Utf16Be => {
                for (i, pair) in bytes.chunks_exact(2).enumerate() {
                    if self.is_at(pair) {
                        each(i * 2 + 2);
                    }
                }
            }
        }
    }

    /// How many lines end in `bytes`, `bytes` starting on a code unit.
    pub(crate) fn count(self, bytes: &[u8]) -> u64 {
        match self {
            Newline::Byte => bytes.iter().filter(|b| **b == b'\n').count() as u64,
            Newline::Utf16Le | Newline::Utf16Be => bytes
                .chunks_exact(2)
                .filter(|pair| self.is_at(pair))
                .count() as u64,
        }
    }
}

/// The file's text form: its encoding, how it was chosen, and its line
/// ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextForm {
    pub choice: Choice,
    pub newline: Newline,
}

impl TextForm {
    /// The form for a file that starts with `head`; `more_follows` when
    /// `head` is only the start of it.
    pub(crate) fn from_head(head: &[u8], more_follows: bool) -> Self {
        let choice = if more_follows {
            choose_head(head, None)
        } else {
            choose(head, None)
        };
        let newline = match choice.encoding.name() {
            "UTF-16LE" => Newline::Utf16Le,
            "UTF-16BE" => Newline::Utf16Be,
            _ => Newline::Byte,
        };
        Self { choice, newline }
    }

    /// The first byte of the first line: after the byte-order mark.
    pub(crate) fn start(&self) -> u64 {
        self.choice.bom_len as u64
    }

    /// One line's bytes, without its line end, decoded; a carriage return
    /// before the line end goes with it.
    pub(crate) fn decode_line(&self, bytes: &[u8]) -> String {
        let (text, _) = self.choice.encoding.decode_without_bom_handling(bytes);
        let mut text = text.into_owned();
        if text.ends_with('\r') {
            text.pop();
        }
        text
    }
}

/// A line as shown: tabs to spaces at stops of [`TAB_STOP`], and every
/// hidden character (controls, escapes, format, direction and
/// private-use characters) as a question mark, so a file cannot move the
/// cursor, color the screen or hide text from a reader.
pub(crate) fn neutralize(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut column = 0usize;
    for c in line.chars() {
        if c == '\t' {
            let spaces = TAB_STOP - column % TAB_STOP;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else if classify(c).is_hidden() {
            out.push('?');
            column += 1;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}

/// A line read from the source: its bytes from `offset`, without the line
/// end, at most `max` bytes of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawLine {
    /// The line's bytes, without its line end.
    pub bytes: Vec<u8>,
    /// Where the next line starts; the source's end for the last line.
    pub next: u64,
    /// Whether the line was longer than `max` and was cut.
    pub cut: bool,
    /// Whether this was the source's last line (no line end after it).
    pub last: bool,
}

/// Reads the line that starts at `offset`.
pub(crate) fn read_line(
    source: &mut ViewerSource,
    newline: Newline,
    offset: u64,
    max: usize,
) -> io::Result<RawLine> {
    let unit = newline.unit();
    let mut bytes = Vec::new();
    let mut at = offset;
    let mut cut = false;
    loop {
        let chunk = source.chunk(at, BLOCK)?;
        if chunk.is_empty() {
            return Ok(RawLine {
                bytes,
                next: at,
                cut,
                last: true,
            });
        }
        // Whole code units only.
        let usable = chunk.len() - chunk.len() % unit;
        if usable == 0 {
            return Ok(RawLine {
                bytes,
                next: at + chunk.len() as u64,
                cut,
                last: true,
            });
        }
        let mut end = None;
        newline.ends(&chunk[..usable], |after| {
            if end.is_none() {
                end = Some(after);
            }
        });
        let (take, found) = match end {
            Some(after) => (after - unit, Some(after)),
            None => (usable, None),
        };
        let room = max.saturating_sub(bytes.len());
        if take > room {
            cut = true;
        }
        bytes.extend_from_slice(&chunk[..take.min(room)]);
        if let Some(after) = found {
            return Ok(RawLine {
                bytes,
                next: at + after as u64,
                cut,
                last: false,
            });
        }
        at += usable as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_and_tabs_become_visible_text() {
        assert_eq!(neutralize("a\tb"), "a   b");
        assert_eq!(neutralize("\tx"), "    x");
        assert_eq!(neutralize("abcd\tx"), "abcd    x");
        assert_eq!(neutralize("bad\u{1b}[2Jname"), "bad?[2Jname");
        assert_eq!(neutralize("v2\u{202e}gpj"), "v2?gpj");
        assert_eq!(neutralize("caf\u{e9}"), "caf\u{e9}");
    }

    #[test]
    fn lines_read_in_utf8_and_utf16_with_their_ends() {
        let mut source = ViewerSource::bytes(b"one\r\ntwo\nlast".to_vec());
        let form = TextForm::from_head(b"one\r\ntwo\nlast", false);
        let first = read_line(&mut source, form.newline, 0, 100).unwrap();
        assert_eq!(form.decode_line(&first.bytes), "one");
        assert_eq!(first.next, 5);
        let second = read_line(&mut source, form.newline, first.next, 100).unwrap();
        assert_eq!(
            (form.decode_line(&second.bytes).as_str(), second.next),
            ("two", 9)
        );
        let third = read_line(&mut source, form.newline, second.next, 100).unwrap();
        assert!(third.last && !third.cut);
        assert_eq!(form.decode_line(&third.bytes), "last");
        let cut = read_line(&mut source, form.newline, 0, 2).unwrap();
        assert!(cut.cut && cut.bytes == b"on" && cut.next == 5);

        let mut utf16: Vec<u8> = vec![0xff, 0xfe];
        utf16.extend("h\u{e9}\nworld".encode_utf16().flat_map(u16::to_le_bytes));
        let form = TextForm::from_head(&utf16, false);
        assert_eq!(form.newline, Newline::Utf16Le);
        assert_eq!(form.start(), 2);
        let mut source = ViewerSource::bytes(utf16);
        let line = read_line(&mut source, form.newline, form.start(), 100).unwrap();
        assert_eq!(form.decode_line(&line.bytes), "h\u{e9}");
        let next = read_line(&mut source, form.newline, line.next, 100).unwrap();
        assert_eq!(form.decode_line(&next.bytes), "world");
        assert_eq!(Newline::Utf16Le.count(&[b'\n', 0, b'a', 0, b'\n', 0]), 2);
        assert_eq!(Newline::Utf16Be.count(&[0, b'\n', b'\n', 0]), 1);
    }
}
