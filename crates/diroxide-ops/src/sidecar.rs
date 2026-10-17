//! The `.crc` sidecar of a split (plan section 5.4; research/alpha2-plan.md
//! 5.4 and commit 21): Total Commander's and Krusader's checksum file, so
//! another tool can check a join, and diroxide can check one made
//! elsewhere.
//!
//! **Written** in three lines, in this order, each ending in a carriage
//! return and a line feed, in UTF-8: `filename=` and the original file's
//! name (no path), `size=` and its byte count in decimal, `crc32=` and its
//! CRC-32 in eight uppercase hexadecimal digits. A split writes it last,
//! after every part is final, so a sidecar means the split finished.
//!
//! **Read** tolerantly: keys in any letter case; the three fields on
//! separate lines or on one line with spaces between them (Krusader's
//! example is one line); LF or CR LF; the CRC in either letter case; the
//! text as UTF-8, else as Windows-1252; unknown keys ignored and said; a
//! missing key is a check that is not made, and said.
//!
//! The CRC-32 is the IEEE one zip uses, streamed with `crc32fast`. The
//! writer leaves room for the SHA-256 line alpha.7's checksum format will
//! add: a reader ignores keys it does not know.

use std::io::{self, Read};

use diroxide_core::names::split_extension;
use diroxide_vfs::CancelToken;

/// The sidecar's extension: `report.pdf.crc`.
pub const SIDECAR_EXTENSION: &str = "crc";

/// The keys a sidecar holds.
const KNOWN: [&str; 3] = ["filename", "size", "crc32"];

/// What a sidecar says: the original file's name, size and CRC-32.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sidecar {
    /// The original file's name, without a path.
    pub filename: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its CRC-32.
    pub crc32: u32,
}

/// A CRC-32 in eight uppercase hexadecimal digits: "21EB719A".
pub fn crc_hex(crc32: u32) -> String {
    format!("{crc32:08X}")
}

impl Sidecar {
    /// A sidecar for a file.
    pub fn new(filename: impl Into<String>, size: u64, crc32: u32) -> Self {
        Self {
            filename: filename.into(),
            size,
            crc32,
        }
    }

    /// The file's text: three lines, CR LF after each.
    pub fn to_text(&self) -> String {
        format!(
            "filename={}\r\nsize={}\r\ncrc32={}\r\n",
            self.filename,
            self.size,
            crc_hex(self.crc32)
        )
    }

    /// The file's bytes, in UTF-8.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_text().into_bytes()
    }
}

/// The names a set's sidecar is looked for under, in order: the part name
/// without its number and with `.crc` (`report.pdf.crc`), then the same
/// with the original extension dropped (`report.crc`), as a joiner that
/// names parts without the extension writes it.
pub fn sidecar_names(base: &str) -> Vec<String> {
    let mut names = vec![format!("{base}.{SIDECAR_EXTENSION}")];
    let (stem, extension) = split_extension(base);
    if !extension.is_empty() && !stem.is_empty() {
        names.push(format!("{stem}.{SIDECAR_EXTENSION}"));
    }
    names
}

/// What a sidecar said, read tolerantly. A field that is missing is a
/// check that is not made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidecarRead {
    /// The original file's name, when given.
    pub filename: Option<String>,
    /// The size, when given as a number.
    pub size: Option<u64>,
    /// The CRC-32, when given as hexadecimal digits.
    pub crc32: Option<u32>,
    /// Keys this reader does not know, lowercase, in the order found.
    pub unknown: Vec<String>,
    /// Values that could not be read, by key: ("size", "12x").
    pub unreadable: Vec<(String, String)>,
    /// Lines that held no key at all.
    pub stray_lines: usize,
    /// Whether the text was not UTF-8 and was read as Windows-1252.
    pub windows_1252: bool,
}

/// Windows-1252's characters for the bytes 0x80 to 0x9F; the rest of the
/// code page is Latin-1. The five bytes the code page leaves undefined map
/// to the control characters of the same number, as browsers read them.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

/// Text in Windows-1252, as Unicode.
fn from_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| match byte {
            0x80..=0x9F => CP1252_HIGH[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect()
}

/// Whether a key starts at `at`: letters, digits, `_` or `-`, then `=`.
fn key_at(line: &str, at: usize) -> Option<(String, usize)> {
    let rest = &line[at..];
    let end = rest
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
        .map_or(rest.len(), |(index, _)| index);
    if end == 0 || !rest[end..].starts_with('=') {
        return None;
    }
    Some((rest[..end].to_ascii_lowercase(), at + end + 1))
}

/// The fields of one line: each key, where its value starts, and the
/// value's text. A key starts the line or follows a space. Inside a file
/// name only a known key ends the name, so a name may hold `x=y`.
fn fields(line: &str) -> (Vec<(String, String)>, bool) {
    let mut starts: Vec<(usize, String, usize)> = Vec::new();
    let mut previous_space = true;
    for (at, c) in line.char_indices() {
        if previous_space && let Some((key, value_at)) = key_at(line, at) {
            starts.push((at, key, value_at));
        }
        previous_space = c.is_whitespace();
    }
    let mut kept: Vec<(usize, String, usize)> = Vec::new();
    for start in starts {
        let inside_name = kept.last().is_some_and(|(_, key, _)| key == "filename");
        if inside_name && !KNOWN.contains(&start.1.as_str()) {
            continue;
        }
        kept.push(start);
    }
    let stray = kept.first().map_or(!line.trim().is_empty(), |(at, _, _)| {
        !line[..*at].trim().is_empty()
    });
    let mut out = Vec::with_capacity(kept.len());
    for (index, (_, key, value_at)) in kept.iter().enumerate() {
        let end = kept.get(index + 1).map_or(line.len(), |(at, _, _)| *at);
        out.push((key.clone(), line[*value_at..end].trim().to_owned()));
    }
    (out, stray)
}

impl SidecarRead {
    /// Reads a sidecar's bytes.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut read = SidecarRead::default();
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => {
                read.windows_1252 = true;
                from_windows_1252(bytes)
            }
        };
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            let (found, stray) = fields(line);
            if stray {
                read.stray_lines += 1;
            }
            for (key, value) in found {
                read.take(&key, value);
            }
        }
        read
    }

    fn take(&mut self, key: &str, value: String) {
        match key {
            "filename" if self.filename.is_none() && !value.is_empty() => {
                self.filename = Some(value);
            }
            "size" if self.size.is_none() => match value.parse::<u64>() {
                Ok(size) => self.size = Some(size),
                Err(_) => self.unreadable.push(("size".to_owned(), value)),
            },
            "crc32" if self.crc32.is_none() => {
                let digits = value.strip_prefix("0x").unwrap_or(&value);
                match (digits.len(), u32::from_str_radix(digits, 16)) {
                    (1..=8, Ok(crc)) => self.crc32 = Some(crc),
                    _ => self.unreadable.push(("crc32".to_owned(), value)),
                }
            }
            "filename" | "size" | "crc32" => {}
            other => {
                if !self.unknown.iter().any(|known| known == other) {
                    self.unknown.push(other.to_owned());
                }
            }
        }
    }

    /// The sidecar, when all three fields were read.
    pub fn complete(&self) -> Option<Sidecar> {
        Some(Sidecar {
            filename: self.filename.clone()?,
            size: self.size?,
            crc32: self.crc32?,
        })
    }

    /// Whether the sidecar is for a set of this base name: its file name is
    /// the base (`report.pdf` for `report.pdf.001`), or the base without
    /// the extension (`report.001` for `report.pdf`), in any letter case.
    /// One with no file name is taken to be the set's.
    pub fn matches(&self, base: &str) -> bool {
        let Some(filename) = &self.filename else {
            return true;
        };
        let filename = filename.to_lowercase();
        let base = base.to_lowercase();
        filename == base || split_extension(&filename).0 == base
    }

    /// What the reader would say about the file, each line meaning first:
    /// the encoding, the keys it did not know, the checks it cannot make.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.windows_1252 {
            notes.push("Checksum file read as Windows-1252.".to_owned());
        }
        if !self.unknown.is_empty() {
            let keys = self.unknown.join(", ");
            let noun = if self.unknown.len() == 1 {
                "key"
            } else {
                "keys"
            };
            notes.push(format!("Checksum file: unknown {noun} {keys}, ignored."));
        }
        for (key, value) in &self.unreadable {
            let what = if key == "size" { "size" } else { "CRC" };
            notes.push(format!("Checksum file: the {what} {value} cannot be read."));
        }
        if self.size.is_none() {
            notes.push("Checksum file has no size: size not checked.".to_owned());
        }
        if self.crc32.is_none() {
            notes.push("Checksum file has no CRC: bytes not checked.".to_owned());
        }
        if self.stray_lines > 0 {
            notes.push(format!(
                "Checksum file: {} not understood, ignored.",
                diroxide_core::format::count(self.stray_lines as u64, "line", "lines")
            ));
        }
        notes
    }
}

/// Streams a reader through a CRC-32 to its end, stopping when the token
/// is cancelled: the CRC and the bytes read.
pub fn crc32_of(reader: &mut dyn Read, cancel: &CancelToken) -> io::Result<(u32, u64)> {
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => return Ok((hasher.finalize(), total)),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn a_written_sidecar_is_three_lines_and_reads_back_equal() {
        let sidecar = Sidecar::new("report.pdf", 223_000_000, 0x21EB_719A);
        assert_eq!(
            sidecar.to_text(),
            "filename=report.pdf\r\nsize=223000000\r\ncrc32=21EB719A\r\n"
        );
        let read = SidecarRead::parse(&sidecar.to_bytes());
        assert_eq!(read.complete(), Some(sidecar));
        assert!(read.notes().is_empty(), "{:?}", read.notes());
        assert_eq!(crc_hex(0xA), "0000000A", "eight digits, padded");
    }

    #[test]
    fn plain_unicode_names_round_trip() {
        let sidecar = Sidecar::new("caf\u{e9} notes \u{6587}.txt", 5, 1);
        assert_eq!(
            SidecarRead::parse(&sidecar.to_bytes()).complete(),
            Some(sidecar)
        );
    }

    /// The tolerance table of research/alpha2-plan.md 5.4: each input with
    /// the name, size and CRC it gives.
    /// One row of the tolerance table: the bytes, then the name, size and
    /// CRC they give.
    type Row = (
        &'static [u8],
        Option<&'static str>,
        Option<u64>,
        Option<u32>,
    );

    #[test]
    fn the_reader_takes_every_form_the_joiners_write() {
        let table: [Row; 12] = [
            // Krusader's example, one line.
            (
                b"filename=foo size=432998467868 crc32=21EB719A",
                Some("foo"),
                Some(432_998_467_868),
                Some(0x21EB_719A),
            ),
            // Several lines, LF only.
            (
                b"filename=foo.zip\nsize=10\ncrc32=0000000a\n",
                Some("foo.zip"),
                Some(10),
                Some(0xA),
            ),
            // Keys in capitals, CR LF, a lowercase CRC.
            (
                b"FILENAME=Foo.Zip\r\nSIZE=7\r\nCRC32=deadbeef\r\n",
                Some("Foo.Zip"),
                Some(7),
                Some(0xDEAD_BEEF),
            ),
            // A name with spaces on the one-line form.
            (
                b"filename=my report.pdf size=3 crc32=FFFFFFFF",
                Some("my report.pdf"),
                Some(3),
                Some(0xFFFF_FFFF),
            ),
            // A name holding x=y: only a known key ends the name.
            (
                b"filename=a b=c.txt size=1 crc32=1",
                Some("a b=c.txt"),
                Some(1),
                Some(1),
            ),
            // A byte-order mark, and blank lines.
            (
                b"\xEF\xBB\xBFfilename=x\r\n\r\nsize=2\r\ncrc32=2\r\n",
                Some("x"),
                Some(2),
                Some(2),
            ),
            // An unknown key, kept out.
            (
                b"filename=x\nsize=2\ncrc32=2\nmd5=abc\n",
                Some("x"),
                Some(2),
                Some(2),
            ),
            // A missing CRC: a check not made.
            (b"filename=x\nsize=2\n", Some("x"), Some(2), None),
            // A missing size.
            (
                b"filename=x\ncrc32=12345678\n",
                Some("x"),
                None,
                Some(0x1234_5678),
            ),
            // Not UTF-8: Windows-1252, with an e acute and a euro sign.
            (
                b"filename=caf\xE9 \x80.txt\r\nsize=4\r\ncrc32=4\r\n",
                Some("caf\u{e9} \u{20ac}.txt"),
                Some(4),
                Some(4),
            ),
            // Values that cannot be read.
            (b"filename=x\nsize=12x\ncrc32=XYZ\n", Some("x"), None, None),
            // Old Mac line ends, spaces around the values.
            (
                b"filename= x.bin \rsize= 9 \rcrc32= 9 \r",
                Some("x.bin"),
                Some(9),
                Some(9),
            ),
        ];
        for (input, filename, size, crc32) in table {
            let read = SidecarRead::parse(input);
            let shown = String::from_utf8_lossy(input);
            assert_eq!(read.filename.as_deref(), filename, "{shown}");
            assert_eq!(read.size, size, "{shown}");
            assert_eq!(read.crc32, crc32, "{shown}");
        }
    }

    #[test]
    fn what_the_reader_could_not_do_is_said_meaning_first() {
        let unknown = SidecarRead::parse(b"filename=x\nsize=2\ncrc32=2\nmd5=abc\nsha1=d\n");
        assert_eq!(unknown.unknown, vec!["md5", "sha1"]);
        assert_eq!(
            unknown.notes(),
            vec!["Checksum file: unknown keys md5, sha1, ignored."]
        );
        let missing = SidecarRead::parse(b"filename=x\n");
        assert_eq!(missing.complete(), None);
        let notes = missing.notes();
        assert_eq!(notes.len(), 2);
        assert!(within_cells(&notes[0], "Checksum file has no size"));
        assert!(within_cells(&notes[1], "Checksum file has no CRC"));
        let old = SidecarRead::parse(b"filename=caf\xE9\nsize=1\ncrc32=1\n");
        assert_eq!(old.notes(), vec!["Checksum file read as Windows-1252."]);
        let bad = SidecarRead::parse(b"filename=x\nsize=1x\ncrc32=1\nnonsense\n");
        assert_eq!(
            bad.notes(),
            vec![
                "Checksum file: the size 1x cannot be read.",
                "Checksum file has no size: size not checked.",
                "Checksum file: 1 line not understood, ignored."
            ]
        );
        for note in unknown.notes().iter().chain(&notes).chain(&bad.notes()) {
            assert!(note.starts_with("Checksum file"), "{note}");
        }
    }

    #[test]
    fn a_sidecar_matches_its_set_with_or_without_the_extension() {
        let read = SidecarRead::parse(b"filename=Report.pdf\nsize=1\ncrc32=1\n");
        assert!(read.matches("report.pdf"), "the full name, any case");
        assert!(read.matches("report"), "parts named without the extension");
        assert!(!read.matches("other.pdf"));
        assert!(SidecarRead::parse(b"size=1\n").matches("anything"));
        assert_eq!(
            sidecar_names("report.pdf"),
            vec!["report.pdf.crc", "report.crc"]
        );
        assert_eq!(sidecar_names("Makefile"), vec!["Makefile.crc"]);
    }

    #[test]
    fn the_crc_is_the_zip_one_streamed() {
        let mut bytes: &[u8] = b"123456789";
        let (crc, read) = crc32_of(&mut bytes, &CancelToken::default()).unwrap();
        assert_eq!(crc, 0xCBF4_3926, "the IEEE check value");
        assert_eq!(read, 9);
        let cancel = CancelToken::default();
        cancel.cancel();
        let mut more: &[u8] = b"x";
        assert!(crc32_of(&mut more, &cancel).is_err());
    }
}
