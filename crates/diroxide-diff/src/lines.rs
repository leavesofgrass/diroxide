//! Lines and their endings. A text splits at LF, at CRLF and at a lone
//! CR, so a file with old Mac endings or a mix of endings still has lines;
//! each line keeps the ending it had, and the last line has none when the
//! text does not end with a line break.

use std::fmt;

/// How a line ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LineEnding {
    /// No line break: the last line of a text that does not end with one.
    None,
    /// A line feed, as Linux and macOS write.
    Lf,
    /// A carriage return and a line feed, as Windows writes.
    CrLf,
    /// A carriage return alone, as classic Mac OS wrote.
    Cr,
}

impl LineEnding {
    /// The characters of the ending: "", "\n", "\r\n" or "\r".
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::None => "",
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }

    /// The ending's name as said: "LF", "CRLF", "CR", or "no line break".
    pub const fn name(self) -> &'static str {
        match self {
            LineEnding::None => "no line break",
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }
}

impl fmt::Display for LineEnding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Where one line sits in its text: its characters run from `start` to
/// `end`, and its ending follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) ending: LineEnding,
}

impl LineSpan {
    /// Where the next line starts: after this line's ending.
    pub(crate) fn next(&self) -> usize {
        self.end + self.ending.as_str().len()
    }
}

/// The lines of a text. An empty text has none; "a\n" has one line, "a",
/// ending in LF; "a" has one line with no ending.
pub(crate) fn split_lines(text: &str) -> Vec<LineSpan> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\n' => {
                lines.push(LineSpan {
                    start,
                    end: at,
                    ending: LineEnding::Lf,
                });
                at += 1;
                start = at;
            }
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                lines.push(LineSpan {
                    start,
                    end: at,
                    ending: LineEnding::CrLf,
                });
                at += 2;
                start = at;
            }
            b'\r' => {
                lines.push(LineSpan {
                    start,
                    end: at,
                    ending: LineEnding::Cr,
                });
                at += 1;
                start = at;
            }
            _ => at += 1,
        }
    }
    if start < bytes.len() {
        lines.push(LineSpan {
            start,
            end: bytes.len(),
            ending: LineEnding::None,
        });
    }
    lines
}

/// A text with its lines found once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Text {
    text: String,
    lines: Vec<LineSpan>,
}

impl Text {
    pub(crate) fn new(text: String) -> Self {
        let lines = split_lines(&text);
        Text { text, lines }
    }

    /// How many lines.
    pub(crate) fn len(&self) -> usize {
        self.lines.len()
    }

    /// The whole text.
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    /// A line's characters, without its ending.
    pub(crate) fn line(&self, index: usize) -> Option<&str> {
        self.lines
            .get(index)
            .map(|span| &self.text[span.start..span.end])
    }

    /// A line's ending.
    pub(crate) fn ending(&self, index: usize) -> Option<LineEnding> {
        self.lines.get(index).map(|span| span.ending)
    }

    /// A line with its ending, as it stands in the text.
    pub(crate) fn raw(&self, index: usize) -> &str {
        self.lines
            .get(index)
            .map_or("", |span| &self.text[span.start..span.next()])
    }

    /// Every line, without its ending, with its ending.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, LineEnding)> + '_ {
        self.lines
            .iter()
            .map(|span| (&self.text[span.start..span.end], span.ending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<(&str, LineEnding)> {
        split_lines(text)
            .into_iter()
            .map(|span| (&text[span.start..span.end], span.ending))
            .collect()
    }

    #[test]
    fn texts_split_at_every_kind_of_line_break() {
        assert!(lines("").is_empty());
        assert_eq!(lines("a"), [("a", LineEnding::None)]);
        assert_eq!(lines("a\n"), [("a", LineEnding::Lf)]);
        assert_eq!(
            lines("a\r\nb\rc\nd"),
            [
                ("a", LineEnding::CrLf),
                ("b", LineEnding::Cr),
                ("c", LineEnding::Lf),
                ("d", LineEnding::None)
            ]
        );
        assert_eq!(lines("\n\n"), [("", LineEnding::Lf), ("", LineEnding::Lf)]);
        assert_eq!(lines("\r"), [("", LineEnding::Cr)]);
    }

    #[test]
    fn raw_lines_put_the_text_back_together() {
        for text in ["", "a", "a\n", "a\r\nb\rc\nd", "\n\r\n\r", "é\r\nü"] {
            let parsed = Text::new(text.to_owned());
            let joined: String = (0..parsed.len()).map(|i| parsed.raw(i)).collect();
            assert_eq!(joined, text);
            assert_eq!(parsed.as_str(), text);
        }
    }

    #[test]
    fn endings_have_names() {
        assert_eq!(LineEnding::CrLf.to_string(), "CRLF");
        assert_eq!(LineEnding::None.name(), "no line break");
        assert_eq!(LineEnding::Cr.as_str(), "\r");
    }
}
