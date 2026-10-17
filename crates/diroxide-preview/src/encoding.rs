// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Carried whole from textweaver's formats crate under the owner's
//! relicensing (answer 41); diroxide's additions follow the carried code.
//!
//! Character encodings: decoding source bytes to text.
//!
//! star decoded everything as UTF-8 with replacement characters and never
//! set `Document.encoding` (the star parity reference, Part 1 §7, quirk 10), so a
//! Windows-1252 text file or a Latin-1 web page lost every accented letter.
//! textweaver decides the encoding in this order:
//!
//! 1. a byte order mark (UTF-8, UTF-16LE, UTF-16BE);
//! 2. a declaration the caller found in the bytes (an HTML `<meta charset>`
//!    or `http-equiv` content type, an XML declaration), resolved with the
//!    WHATWG label table, so `iso-8859-1` means Windows-1252 as browsers
//!    read it; a declared UTF-16 without a BOM is ignored, as browsers do;
//! 3. UTF-8, when the bytes are valid UTF-8;
//! 4. Windows-1252, the usual encoding of legacy text on the machines
//!    students use (every byte decodes, so nothing is lost silently).
//!
//! The chosen encoding is reported in [`Decoded::encoding`]; loaders record
//! anything other than UTF-8 in `DocumentMeta::properties["encoding"]`.
//!
//! UTF-16 without a byte order mark (a NUL after or before nearly every
//! ASCII letter) is recognized between steps 2 and 3. Bytes that are not
//! text at all (a PDF, a zip archive, an image, a program) are recognized
//! by [`binary_kind`], so the text loaders can refuse them instead of
//! reading them aloud as garbage (the September 2026 audit, finding D3).

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// Text decoded from bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// The text, without a byte order mark, line endings unchanged.
    pub text: String,
    /// WHATWG name of the encoding used ("UTF-8", "windows-1252", ...).
    pub encoding: &'static str,
    /// True when some bytes were malformed and replaced with U+FFFD.
    pub had_errors: bool,
}

impl Decoded {
    /// True when the text was not UTF-8 (worth recording in the metadata).
    pub fn is_legacy(&self) -> bool {
        self.encoding != UTF_8.name()
    }
}

/// Decodes `bytes` by BOM, then the `declared` label, then UTF-8 when valid,
/// then Windows-1252.
pub fn decode(bytes: &[u8], declared: Option<&str>) -> Decoded {
    if let Some((enc, bom)) = Encoding::for_bom(bytes) {
        return run(enc, &bytes[bom..]);
    }
    if let Some(enc) = declared.and_then(|l| Encoding::for_label(l.trim().as_bytes()))
        && enc != UTF_16LE
        && enc != UTF_16BE
    {
        return run(enc.output_encoding(), bytes);
    }
    if let Some(enc) = utf16_without_bom(bytes) {
        return run(enc, bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Decoded {
            text: s.to_owned(),
            encoding: UTF_8.name(),
            had_errors: false,
        },
        Err(_) => run(WINDOWS_1252, bytes),
    }
}

/// How much of the start of a file is examined to tell text from binary.
pub const SNIFF_BYTES: usize = 8192;

/// UTF-16 without a byte order mark: in the first [`SNIFF_BYTES`], most
/// code units have a NUL high byte (ASCII-range text), on the same side.
fn utf16_without_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES) & !1];
    let units = head.len() / 2;
    if units < 2 {
        return None;
    }
    let (mut even, mut odd) = (0usize, 0usize);
    for pair in head.chunks_exact(2) {
        even += usize::from(pair[0] == 0 && pair[1] != 0);
        odd += usize::from(pair[1] == 0 && pair[0] != 0);
    }
    // Nine in ten units ASCII-range, with the NUL on one side.
    if odd * 10 >= units * 9 && even * 20 <= units {
        Some(UTF_16LE)
    } else if even * 10 >= units * 9 && odd * 20 <= units {
        Some(UTF_16BE)
    } else {
        None
    }
}

/// What kind of non-text file `bytes` look like, or `None` for text.
///
/// Known signatures are named ("a PDF document", "a zip archive (such as
/// a Word document or an EPUB)", "an image", "a program"); other bytes
/// count as binary when the first [`SNIFF_BYTES`] hold a NUL byte (and are
/// not UTF-16) or more than one control character in twenty. Text with a
/// byte order mark is always text.
pub fn binary_kind(bytes: &[u8]) -> Option<&'static str> {
    if Encoding::for_bom(bytes).is_some() || utf16_without_bom(bytes).is_some() {
        return None;
    }
    const SIGNATURES: &[(&[u8], &str)] = &[
        (b"%PDF-", "a PDF document"),
        (
            b"PK\x03\x04",
            "a zip archive (such as a Word document or an EPUB)",
        ),
        (b"\x89PNG", "an image"),
        (b"\xff\xd8\xff", "an image"),
        (b"GIF8", "an image"),
        (b"RIFF", "an audio, video, or image file"),
        (b"ID3", "an audio file"),
        (b"OggS", "an audio file"),
        (b"fLaC", "an audio file"),
        (b"\x7fELF", "a program"),
        (b"MZ", "a program"),
        (b"\xd0\xcf\x11\xe0", "an old Microsoft Office document"),
        (b"{\\rtf", "a rich text (RTF) document"),
    ];
    for (magic, kind) in SIGNATURES {
        if bytes.starts_with(magic) {
            // "MZ" and "RIFF" are short: only when the rest is not text.
            if (*magic == b"MZ" || *magic == b"RIFF") && binary_by_content(bytes).is_none() {
                continue;
            }
            return Some(kind);
        }
    }
    binary_by_content(bytes)
}

fn binary_by_content(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if head.is_empty() {
        return None;
    }
    if head.contains(&0) {
        return Some("a binary file");
    }
    let controls = head
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b))
        .count();
    (controls * 20 > head.len()).then_some("a binary file")
}

fn run(enc: &'static Encoding, bytes: &[u8]) -> Decoded {
    let (text, had_errors) = enc.decode_without_bom_handling(bytes);
    Decoded {
        text: text.into_owned(),
        encoding: enc.name(),
        had_errors,
    }
}

/// The charset an HTML document declares in its first 1024 bytes:
/// `<meta charset="…">` or `<meta http-equiv="Content-Type"
/// content="text/html; charset=…">`, or an XML declaration (XHTML).
pub fn sniff_html_charset(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(1024)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let mut at = 0;
    while let Some(i) = find(&lower[at..], b"<meta") {
        let start = at + i;
        let end = lower[start..]
            .iter()
            .position(|&b| b == b'>')
            .map_or(lower.len(), |e| start + e);
        let tag = &lower[start..end];
        if let Some(c) = find(tag, b"charset") {
            let rest = &tag[c + b"charset".len()..];
            let rest = trim_start(rest);
            if let Some(rest) = rest.strip_prefix(b"=")
                && let Some(v) = attr_value(trim_start(rest))
            {
                return Some(v);
            }
        }
        at = end.max(start + 1);
    }
    sniff_xml_encoding(bytes)
}

/// The `encoding` of an XML declaration (`<?xml version="1.0"
/// encoding="…"?>`) at the start of `bytes`.
pub fn sniff_xml_encoding(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(256)];
    let head = trim_start(head);
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = find(head, b"?>")?;
    let decl = &head[..end];
    let e = find(decl, b"encoding")?;
    let rest = trim_start(&decl[e + b"encoding".len()..]);
    let rest = trim_start(rest.strip_prefix(b"=")?);
    attr_value(rest)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn trim_start(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|b| b.is_ascii_whitespace()).count();
    &s[n..]
}

/// A quoted or bare attribute value at the start of `s`.
fn attr_value(s: &[u8]) -> Option<String> {
    let (quote, body) = match s.first() {
        Some(&q @ (b'"' | b'\'')) => (Some(q), &s[1..]),
        _ => (None, s),
    };
    let len = body
        .iter()
        .position(|&b| match quote {
            Some(q) => b == q,
            None => b.is_ascii_whitespace() || matches!(b, b';' | b'"' | b'\'' | b'/' | b'>'),
        })
        .unwrap_or(body.len());
    let v = std::str::from_utf8(&body[..len]).ok()?.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

// diroxide's additions, not carried: the choice without the decoding, so
// the viewer can decode a large file line by line from its first bytes,
// and the choice said in words ("Encoding: UTF-16 (byte-order mark)").

/// How [`choose`] picked an encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chosen {
    /// A byte-order mark at the start.
    ByteOrderMark,
    /// A declaration the caller found in the bytes.
    Declared,
    /// UTF-16 recognized without a byte-order mark.
    Detected,
    /// Valid UTF-8 with no mark: the usual case, said only on request.
    Utf8,
    /// Not UTF-8 and nothing declared: Windows-1252, where every byte
    /// decodes.
    NotUtf8,
}

/// The encoding [`decode`] would use for a file that starts with given
/// bytes, how it was chosen, and the length of the byte-order mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The encoding.
    pub encoding: &'static Encoding,
    /// How it was chosen.
    pub chosen: Chosen,
    /// The bytes of the byte-order mark, 0 without one.
    pub bom_len: usize,
}

impl Choice {
    /// The encoding's name as said: "UTF-8", "UTF-16", "Windows-1252"; the
    /// byte order of UTF-16 is in [`Choice::detail`].
    pub fn name(&self) -> &'static str {
        spoken_name(self.encoding)
    }

    /// The encoding with its byte order where it has one: "UTF-16, little
    /// endian".
    pub fn detail(&self) -> String {
        if self.encoding == UTF_16LE {
            "UTF-16, little endian".to_owned()
        } else if self.encoding == UTF_16BE {
            "UTF-16, big endian".to_owned()
        } else {
            self.name().to_owned()
        }
    }

    /// The line the viewer says when the file opens, meaning first and
    /// inside 40 cells: "Encoding: UTF-16 (byte-order mark)". `None` for
    /// plain UTF-8, which is said only on request.
    pub fn line(&self) -> Option<String> {
        let how = match self.chosen {
            Chosen::Utf8 => return None,
            Chosen::ByteOrderMark => "byte-order mark",
            Chosen::Declared => "declared",
            Chosen::Detected => "no byte-order mark",
            Chosen::NotUtf8 => "not UTF-8",
        };
        Some(format!("Encoding: {} ({how})", self.name()))
    }
}

/// An encoding's name as said: "UTF-16" for both byte orders, the
/// capitalized form of "windows-1252", the WHATWG name otherwise.
pub fn spoken_name(encoding: &'static Encoding) -> &'static str {
    if encoding == UTF_16LE || encoding == UTF_16BE {
        "UTF-16"
    } else if encoding == WINDOWS_1252 {
        "Windows-1252"
    } else {
        encoding.name()
    }
}

/// The encoding [`decode`] uses for these bytes, the whole of a file, in
/// its order: a byte-order mark, then the `declared` label, then UTF-16
/// without a mark, then UTF-8 when the bytes are valid, then
/// Windows-1252.
pub fn choose(bytes: &[u8], declared: Option<&str>) -> Choice {
    choose_from(bytes, declared, false)
}

/// The same for the first bytes of a longer file (the viewer reads
/// [`SNIFF_BYTES`] of a large file): a character cut off at the end of
/// `head` does not count against UTF-8.
pub fn choose_head(head: &[u8], declared: Option<&str>) -> Choice {
    choose_from(head, declared, true)
}

fn choose_from(head: &[u8], declared: Option<&str>, more_follows: bool) -> Choice {
    if let Some((encoding, bom_len)) = Encoding::for_bom(head) {
        return Choice {
            encoding,
            chosen: Chosen::ByteOrderMark,
            bom_len,
        };
    }
    if let Some(enc) = declared.and_then(|l| Encoding::for_label(l.trim().as_bytes()))
        && enc != UTF_16LE
        && enc != UTF_16BE
    {
        return Choice {
            encoding: enc.output_encoding(),
            chosen: Chosen::Declared,
            bom_len: 0,
        };
    }
    if let Some(encoding) = utf16_without_bom(head) {
        return Choice {
            encoding,
            chosen: Chosen::Detected,
            bom_len: 0,
        };
    }
    let utf8 = match std::str::from_utf8(head) {
        Ok(_) => true,
        // A sequence cut off by the end of the sample, not a bad byte.
        Err(error) => more_follows && error.error_len().is_none(),
    };
    if utf8 {
        Choice {
            encoding: UTF_8,
            chosen: Chosen::Utf8,
            bom_len: 0,
        }
    } else {
        Choice {
            encoding: WINDOWS_1252,
            chosen: Chosen::NotUtf8,
            bom_len: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_without_a_bom_is_recognized() {
        let le: Vec<u8> = "Hello, world.\nSecond line."
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let d = decode(&le, None);
        assert_eq!(
            (d.text.as_str(), d.encoding),
            ("Hello, world.\nSecond line.", "UTF-16LE")
        );
        let be: Vec<u8> = "Hi there"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let d = decode(&be, None);
        assert_eq!((d.text.as_str(), d.encoding), ("Hi there", "UTF-16BE"));
        assert_eq!(binary_kind(&le), None);
    }

    #[test]
    fn binary_files_are_named_and_text_is_not_binary() {
        assert_eq!(binary_kind(b"%PDF-1.7\n%\xe2\xe3"), Some("a PDF document"));
        assert!(binary_kind(b"PK\x03\x04\x14\x00").unwrap().contains("zip"));
        assert_eq!(binary_kind(b"\x89PNG\r\n\x1a\n\0\0"), Some("an image"));
        assert_eq!(binary_kind(b"abc\0def"), Some("a binary file"));
        assert_eq!(binary_kind(b"\x01\x02\x03\x04 text"), Some("a binary file"));
        // Text, including text that happens to start like a signature.
        assert_eq!(binary_kind(b"MZ is a postcode area.\n"), None);
        assert_eq!(binary_kind(b"RIFF raff and friends.\n"), None);
        assert_eq!(
            binary_kind("caf\u{e9} \u{2014} na\u{ef}ve\n".as_bytes()),
            None
        );
        assert_eq!(binary_kind(b"caf\xe9 in Windows-1252\n"), None);
        assert_eq!(binary_kind(b"\x1b[1mbold\x1b[0m\tand\x0c\n"), None);
        assert_eq!(binary_kind(b"\xff\xfeh\x00\x00\x00"), None);
        assert_eq!(binary_kind(b""), None);
    }

    #[test]
    fn bom_declared_utf8_then_windows_1252() {
        let d = decode(b"\xef\xbb\xbfcaf\xc3\xa9", Some("iso-8859-1"));
        assert_eq!((d.text.as_str(), d.encoding), ("café", "UTF-8"));
        let d = decode(b"\xff\xfeh\x00i\x00", None);
        assert_eq!((d.text.as_str(), d.encoding), ("hi", "UTF-16LE"));
        let d = decode(b"caf\xe9", Some("latin1"));
        assert_eq!((d.text.as_str(), d.encoding), ("café", "windows-1252"));
        assert!(d.is_legacy());
        let d = decode("café".as_bytes(), None);
        assert_eq!((d.text.as_str(), d.encoding), ("café", "UTF-8"));
        assert!(!d.is_legacy());
        // Not UTF-8, nothing declared: Windows-1252, curly quotes included.
        let d = decode(b"\x93caf\xe9\x94", None);
        assert_eq!(d.text, "\u{201c}café\u{201d}");
        assert!(!d.had_errors);
        // A declared UTF-16 without a BOM is ignored.
        let d = decode(b"plain", Some("utf-16"));
        assert_eq!(d.encoding, "UTF-8");
        // An unknown label is ignored.
        assert_eq!(decode(b"x", Some("klingon")).encoding, "UTF-8");
    }

    #[test]
    fn sniffs_meta_and_xml_declarations() {
        assert_eq!(
            sniff_html_charset(b"<html><head><META CHARSET='Windows-1252'>").as_deref(),
            Some("windows-1252")
        );
        assert_eq!(
            sniff_html_charset(
                b"<meta name=x><meta http-equiv=\"Content-Type\" content=\"text/html; charset=ISO-8859-1\">"
            )
            .as_deref(),
            Some("iso-8859-1")
        );
        assert_eq!(
            sniff_html_charset(b"<?xml version=\"1.0\" encoding=\"koi8-r\"?><html>").as_deref(),
            Some("koi8-r")
        );
        assert_eq!(sniff_html_charset(b"<html><body>no charset</body>"), None);
        assert_eq!(sniff_xml_encoding(b"<?xml version='1.0'?><a/>"), None);
    }

    // diroxide's tests of its additions.

    #[test]
    fn the_choice_agrees_with_decode_and_says_how_in_40_cells() {
        let cases: [(&[u8], Chosen, Option<&str>); 6] = [
            (
                b"\xef\xbb\xbfcaf\xc3\xa9",
                Chosen::ByteOrderMark,
                Some("Encoding: UTF-8 (byte-order mark)"),
            ),
            (
                b"\xff\xfeh\x00i\x00",
                Chosen::ByteOrderMark,
                Some("Encoding: UTF-16 (byte-order mark)"),
            ),
            (
                b"\xfe\xff\x00h\x00i",
                Chosen::ByteOrderMark,
                Some("Encoding: UTF-16 (byte-order mark)"),
            ),
            (
                b"H\x00e\x00l\x00l\x00o\x00",
                Chosen::Detected,
                Some("Encoding: UTF-16 (no byte-order mark)"),
            ),
            (b"caf\xc3\xa9", Chosen::Utf8, None),
            (
                b"caf\xe9",
                Chosen::NotUtf8,
                Some("Encoding: Windows-1252 (not UTF-8)"),
            ),
        ];
        for (bytes, chosen, line) in cases {
            let choice = choose(bytes, None);
            assert_eq!(choice.chosen, chosen, "{bytes:?}");
            assert_eq!(choice.encoding.name(), decode(bytes, None).encoding);
            assert_eq!(choice.line().as_deref(), line);
            if let Some(line) = choice.line() {
                assert!(line.chars().count() <= 40, "{line}");
            }
        }
        assert_eq!(choose(b"\xff\xfeh\x00", None).bom_len, 2);
        assert_eq!(
            choose(b"\xff\xfeh\x00", None).detail(),
            "UTF-16, little endian"
        );
        let declared = choose(b"caf\xe9", Some("latin1"));
        assert_eq!(declared.chosen, Chosen::Declared);
        assert_eq!(
            declared.line().as_deref(),
            Some("Encoding: Windows-1252 (declared)")
        );
    }

    #[test]
    fn a_character_cut_off_by_the_sample_is_still_utf8() {
        let text = "na\u{ef}ve caf\u{e9}".as_bytes();
        let cut = &text[..text.len() - 1];
        assert_eq!(choose_head(cut, None).chosen, Chosen::Utf8);
        assert_eq!(choose(cut, None).chosen, Chosen::NotUtf8, "the whole file");
        assert_eq!(
            choose_head(b"caf\xe9 au lait", None).chosen,
            Chosen::NotUtf8
        );
    }
}
