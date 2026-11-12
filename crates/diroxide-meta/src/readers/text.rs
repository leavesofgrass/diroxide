//! Text facts, on demand (R28 3.2, "text (encoding, line count, on
//! demand)"): the encoding as far as the bytes tell (a byte-order mark,
//! valid UTF-8, plain ASCII, or neither), the number of lines and the
//! line endings. It reads the whole file, so it waits to be asked.

use diroxide_core::details::DetailGroup;
use diroxide_vfs::ReadSeek;

use crate::field::Field;
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
use crate::words::grouped;

/// The text facts reader: `text.encoding`, `text.lines`,
/// `text.line_endings`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextFacts;

/// Names read as text when the type is not checked yet.
const TEXT_NAMES: [&str; 12] = [
    ".txt", ".md", ".csv", ".tsv", ".log", ".ini", ".toml", ".json", ".xml", ".yaml", ".yml", ".rs",
];

impl Reader for TextFacts {
    fn id(&self) -> &'static str {
        "text"
    }

    fn cost(&self) -> Cost {
        Cost::OnDemand
    }

    fn keys(&self) -> &'static [&'static str] {
        &["text.encoding", "text.lines", "text.line_endings"]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        match request.group {
            Some(group) => matches!(group, "text" | "code"),
            None => super::takes(request, &[], &TEXT_NAMES),
        }
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let facts = TextScan::read(source)?;
        Ok(facts.fields())
    }
}

/// What a pass over the bytes found.
#[derive(Debug, Default)]
struct TextScan {
    total: u64,
    bom: Option<Bom>,
    utf8: bool,
    ascii: bool,
    nul: bool,
    crlf: u64,
    lf: u64,
    cr: u64,
    ends_with_break: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bom {
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl TextScan {
    fn read(source: &mut dyn ReadSeek) -> Result<Self, MetaError> {
        let mut scan = TextScan {
            utf8: true,
            ascii: true,
            ..TextScan::default()
        };
        let mut buffer = vec![0u8; 64 * 1024];
        let mut pending: Vec<u8> = Vec::new();
        let mut first = true;
        let mut last_cr = false;
        let mut odd: Option<u8> = None;
        loop {
            let n = source
                .read(&mut buffer)
                .map_err(|e| MetaError::from_io(&e))?;
            if n == 0 {
                break;
            }
            let mut chunk = &buffer[..n];
            scan.total += n as u64;
            if first {
                first = false;
                if chunk.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    scan.bom = Some(Bom::Utf8);
                    chunk = &chunk[3..];
                } else if chunk.starts_with(&[0xFF, 0xFE]) {
                    scan.bom = Some(Bom::Utf16Le);
                    chunk = &chunk[2..];
                } else if chunk.starts_with(&[0xFE, 0xFF]) {
                    scan.bom = Some(Bom::Utf16Be);
                    chunk = &chunk[2..];
                }
            }
            match scan.bom {
                Some(Bom::Utf16Le | Bom::Utf16Be) => {
                    let mut bytes: Vec<u8> = odd.take().into_iter().collect();
                    bytes.extend_from_slice(chunk);
                    if bytes.len() % 2 == 1 {
                        odd = bytes.pop();
                    }
                    let little = scan.bom == Some(Bom::Utf16Le);
                    for pair in bytes.chunks_exact(2) {
                        let unit = if little {
                            u16::from_le_bytes([pair[0], pair[1]])
                        } else {
                            u16::from_be_bytes([pair[0], pair[1]])
                        };
                        scan.unit(unit, &mut last_cr);
                    }
                }
                _ => {
                    for &byte in chunk {
                        if byte >= 0x80 {
                            scan.ascii = false;
                        }
                        if byte == 0 {
                            scan.nul = true;
                        }
                        scan.unit(u16::from(byte), &mut last_cr);
                    }
                    if scan.utf8 {
                        pending.extend_from_slice(chunk);
                        match std::str::from_utf8(&pending) {
                            Ok(_) => pending.clear(),
                            Err(error) if error.error_len().is_none() => {
                                pending.drain(..error.valid_up_to());
                            }
                            Err(_) => scan.utf8 = false,
                        }
                    }
                }
            }
        }
        if last_cr {
            scan.cr += 1;
        }
        if !pending.is_empty() || odd.is_some() {
            scan.utf8 = false;
        }
        Ok(scan)
    }

    /// One character's code (a byte, or a UTF-16 unit) for the line
    /// counts.
    fn unit(&mut self, unit: u16, last_cr: &mut bool) {
        match unit {
            0x0A => {
                if *last_cr {
                    self.crlf += 1;
                } else {
                    self.lf += 1;
                }
                *last_cr = false;
                self.ends_with_break = true;
            }
            0x0D => {
                if *last_cr {
                    self.cr += 1;
                }
                *last_cr = true;
                self.ends_with_break = true;
            }
            _ => {
                if *last_cr {
                    self.cr += 1;
                }
                *last_cr = false;
                self.ends_with_break = false;
            }
        }
    }

    fn encoding(&self) -> &'static str {
        match self.bom {
            Some(Bom::Utf16Le) => "UTF-16, little-endian",
            Some(Bom::Utf16Be) => "UTF-16, big-endian",
            Some(Bom::Utf8) if self.utf8 => "UTF-8 with a byte-order mark",
            _ if self.nul => "binary: holds NUL bytes",
            _ if self.ascii => "ASCII",
            _ if self.utf8 => "UTF-8",
            _ => "not UTF-8: a legacy code page",
        }
    }

    fn lines(&self) -> u64 {
        let breaks = self.crlf + self.lf + self.cr;
        let content = self.total > self.bom_len();
        breaks + u64::from(content && !self.ends_with_break)
    }

    fn bom_len(&self) -> u64 {
        match self.bom {
            Some(Bom::Utf8) => 3,
            Some(_) => 2,
            None => 0,
        }
    }

    fn line_endings(&self) -> String {
        let kinds = [(self.crlf, "CR LF"), (self.lf, "LF"), (self.cr, "CR")];
        let used: Vec<_> = kinds.iter().filter(|(n, _)| *n > 0).collect();
        match used.as_slice() {
            [] => "none".to_owned(),
            [(_, "CR LF")] => "Windows, CR LF".to_owned(),
            [(_, "LF")] => "Unix, LF".to_owned(),
            [(_, _)] => "old Mac, CR".to_owned(),
            many => format!(
                "mixed: {}",
                many.iter()
                    .map(|(n, kind)| format!("{} {kind}", grouped(*n)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn fields(&self) -> Vec<Field> {
        vec![
            Field::new(
                "text.encoding",
                "Encoding",
                self.encoding(),
                DetailGroup::Technical,
            ),
            Field::new(
                "text.lines",
                "Lines",
                grouped(self.lines()),
                DetailGroup::Content,
            )
            .sorted_by_count(self.lines()),
            Field::new(
                "text.line_endings",
                "Line endings",
                self.line_endings(),
                DetailGroup::Technical,
            ),
        ]
    }
}

/// The text facts of some bytes.
#[cfg(test)]
pub(crate) fn scan_bytes(bytes: &[u8]) -> Vec<Field> {
    let mut cursor = std::io::Cursor::new(bytes);
    TextScan::read(&mut cursor)
        .map(|scan| scan.fields())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(bytes: &[u8]) -> Vec<String> {
        scan_bytes(bytes).iter().map(Field::line).collect()
    }

    #[test]
    fn encodings_lines_and_endings_in_words() {
        assert_eq!(
            lines_of(b"one\ntwo\nthree"),
            ["Encoding: ASCII", "Lines: 3", "Line endings: Unix, LF"]
        );
        assert_eq!(
            lines_of(b"one\r\ntwo\r\n"),
            [
                "Encoding: ASCII",
                "Lines: 2",
                "Line endings: Windows, CR LF"
            ]
        );
        assert_eq!(
            lines_of("caf\u{e9}\rna\u{ef}ve\r".as_bytes()),
            ["Encoding: UTF-8", "Lines: 2", "Line endings: old Mac, CR"]
        );
        assert_eq!(
            lines_of(b"\xEF\xBB\xBFa\r\nb\nc"),
            [
                "Encoding: UTF-8 with a byte-order mark",
                "Lines: 3",
                "Line endings: mixed: 1 CR LF, 1 LF"
            ]
        );
        assert_eq!(
            lines_of(b"caf\xE9\n"),
            [
                "Encoding: not UTF-8: a legacy code page",
                "Lines: 1",
                "Line endings: Unix, LF"
            ]
        );
        assert_eq!(
            lines_of(b"\xFF\xFEa\0\n\0b\0"),
            [
                "Encoding: UTF-16, little-endian",
                "Lines: 2",
                "Line endings: Unix, LF"
            ]
        );
        assert_eq!(
            lines_of(b"\xFE\xFF\0a\0\r\0\n"),
            [
                "Encoding: UTF-16, big-endian",
                "Lines: 1",
                "Line endings: Windows, CR LF"
            ]
        );
        assert_eq!(
            lines_of(b""),
            ["Encoding: ASCII", "Lines: 0", "Line endings: none"]
        );
        assert_eq!(lines_of(b"a\0b")[0], "Encoding: binary: holds NUL bytes");
        for line in lines_of(b"\xEF\xBB\xBFa\r\nb\nc\rd") {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn a_character_split_between_reads_is_still_utf8() {
        // 70,000 bytes puts a two-byte character across the 64 KiB read.
        let mut text = "a".repeat(65_535);
        text.push('\u{e9}');
        text.push_str(&"b\n".repeat(2000));
        assert_eq!(lines_of(text.as_bytes())[0], "Encoding: UTF-8");
        assert_eq!(lines_of(text.as_bytes())[1], "Lines: 2,000");
    }
}
