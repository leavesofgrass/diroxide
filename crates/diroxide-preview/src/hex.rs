//! One hex row function for every place that shows bytes: the viewer's
//! hex mode, the hex previewer, the item information list and, later,
//! the details view (research/alpha2-plan.md, the quick win "one hex-row
//! function with a width").
//!
//! A row is the offset, a colon, the bytes as pairs, then the bytes as
//! text: "000010: 48 65 6c 6c 6f 20 77 6f Hello wo", 40 characters with
//! eight bytes, so a screen reader's row fits the owner's 40-cell Braille
//! line. Six offset digits cover 16 MiB; past that the offset needs more
//! digits, and an eight-byte row drops to six bytes to stay inside 40.

use std::fmt::Write as _;

/// The bytes 16 MiB of six hex digits can address.
pub const SIX_DIGITS: u64 = 16 * 1024 * 1024;

/// How a row is laid out: its bytes and its offset's digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HexLayout {
    /// The bytes a row shows.
    pub bytes_per_row: usize,
    /// The hex digits of the offset.
    pub offset_digits: usize,
}

impl HexLayout {
    /// The layout for `len` bytes with `bytes_per_row` wanted (eight in
    /// screen reader mode, sixteen otherwise; the `hex_row` setting): six
    /// offset digits up to [`SIX_DIGITS`], as many as the last offset needs
    /// beyond, and six bytes a row instead of eight beyond it, so a row
    /// stays inside 40 cells.
    pub fn for_len(len: u64, bytes_per_row: usize) -> Self {
        let last = len.saturating_sub(1);
        let needed = (64 - last.leading_zeros() as usize).div_ceil(4);
        let offset_digits = needed.max(6);
        let bytes_per_row = if bytes_per_row <= 8 && len > SIX_DIGITS {
            6
        } else {
            bytes_per_row.max(1)
        };
        Self {
            bytes_per_row,
            offset_digits,
        }
    }

    /// The characters a full row takes: the offset, ": ", three for each
    /// byte's pair and its space, one for each byte's text.
    pub const fn row_chars(&self) -> usize {
        self.offset_digits + 2 + self.bytes_per_row * 4
    }

    /// Where the row that holds `offset` starts.
    pub const fn row_start(&self, offset: u64) -> u64 {
        offset - offset % self.bytes_per_row as u64
    }
}

/// One row: "000010: 48 65 6c 6c 6f 20 77 6f Hello wo". A short last row
/// keeps its text where a full row's is, so the columns line up.
pub fn hex_row(offset: u64, bytes: &[u8], layout: HexLayout) -> String {
    let n = layout.bytes_per_row;
    let mut row = String::with_capacity(layout.row_chars());
    let _ = write!(row, "{offset:0width$x}: ", width = layout.offset_digits);
    for i in 0..n {
        match bytes.get(i) {
            Some(b) => {
                let _ = write!(row, "{b:02x} ");
            }
            None => row.push_str("   "),
        }
    }
    row.extend(bytes.iter().take(n).map(|&b| shown_char(b)));
    row
}

/// Rows for bytes that start at `start_offset`.
pub fn hex_rows(bytes: &[u8], start_offset: u64, layout: HexLayout) -> Vec<String> {
    bytes
        .chunks(layout.bytes_per_row)
        .enumerate()
        .map(|(i, chunk)| {
            hex_row(
                start_offset + (i * layout.bytes_per_row) as u64,
                chunk,
                layout,
            )
        })
        .collect()
}

/// A byte as the text column shows it: printable ASCII as itself, every
/// other byte as a period.
pub const fn shown_char(byte: u8) -> char {
    if byte.is_ascii_graphic() || byte == b' ' {
        byte as char
    } else {
        '.'
    }
}

/// A byte in words, for Say Byte: "48 hex, the letter H", "0a hex, a line
/// feed", "e9 hex, not ASCII".
pub fn byte_words(byte: u8) -> String {
    format!("{byte:02x} hex, {}", byte_name(byte))
}

/// What a byte is as a character, in words.
pub fn byte_name(byte: u8) -> String {
    let c = byte as char;
    if c.is_ascii_alphabetic() {
        return format!("the letter {c}");
    }
    if c.is_ascii_digit() {
        return format!("the digit {c}");
    }
    let name = match byte {
        0 => "a zero byte",
        b'\t' => "a tab",
        b'\n' => "a line feed",
        b'\r' => "a carriage return",
        0x1b => "an escape",
        b' ' => "a space",
        b'!' => "an exclamation mark",
        b'"' => "a double quote",
        b'#' => "a number sign",
        b'$' => "a dollar sign",
        b'%' => "a percent sign",
        b'&' => "an ampersand",
        b'\'' => "an apostrophe",
        b'(' => "an opening parenthesis",
        b')' => "a closing parenthesis",
        b'*' => "an asterisk",
        b'+' => "a plus sign",
        b',' => "a comma",
        b'-' => "a hyphen",
        b'.' => "a period",
        b'/' => "a slash",
        b':' => "a colon",
        b';' => "a semicolon",
        b'<' => "a less-than sign",
        b'=' => "an equals sign",
        b'>' => "a greater-than sign",
        b'?' => "a question mark",
        b'@' => "an at sign",
        b'[' => "an opening bracket",
        b'\\' => "a backslash",
        b']' => "a closing bracket",
        b'^' => "a caret",
        b'_' => "an underscore",
        b'`' => "a backtick",
        b'{' => "an opening brace",
        b'|' => "a vertical bar",
        b'}' => "a closing brace",
        b'~' => "a tilde",
        0x01..=0x1f | 0x7f => "a control character",
        _ => "not ASCII",
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_eight_byte_row_is_forty_characters() {
        let layout = HexLayout::for_len(1000, 8);
        assert_eq!(
            layout,
            HexLayout {
                bytes_per_row: 8,
                offset_digits: 6
            }
        );
        let row = hex_row(16, b"Hello wo", layout);
        assert_eq!(row, "000010: 48 65 6c 6c 6f 20 77 6f Hello wo");
        assert_eq!(row.chars().count(), 40);
        assert_eq!(layout.row_chars(), 40);
        let last = hex_row(24, b"rld", layout);
        assert_eq!(last, "000018: 72 6c 64                rld");
    }

    #[test]
    fn past_16_mib_the_offset_grows_and_the_row_drops_to_six_bytes() {
        let at_limit = HexLayout::for_len(SIX_DIGITS, 8);
        assert_eq!(
            at_limit,
            HexLayout {
                bytes_per_row: 8,
                offset_digits: 6
            }
        );
        let past = HexLayout::for_len(SIX_DIGITS + 1, 8);
        assert_eq!(
            past,
            HexLayout {
                bytes_per_row: 6,
                offset_digits: 7
            }
        );
        let huge = HexLayout::for_len(1 << 40, 8);
        assert_eq!(huge.offset_digits, 10);
        for layout in [past, huge] {
            assert!(layout.row_chars() <= 40, "{layout:?}");
            let row = hex_row(SIX_DIGITS, &[0x41; 6], layout);
            assert!(row.chars().count() <= 40, "{row}");
        }
        let wide = HexLayout::for_len(SIX_DIGITS * 2, 16);
        assert_eq!(wide.bytes_per_row, 16, "sixteen stays sixteen");
        assert_eq!(wide.row_start(35), 32);
    }

    #[test]
    fn bytes_are_said_in_words() {
        assert_eq!(byte_words(0x48), "48 hex, the letter H");
        assert_eq!(byte_words(b'h'), "68 hex, the letter h");
        assert_eq!(byte_words(b'4'), "34 hex, the digit 4");
        assert_eq!(byte_words(b'\n'), "0a hex, a line feed");
        assert_eq!(byte_words(0), "00 hex, a zero byte");
        assert_eq!(byte_words(0x07), "07 hex, a control character");
        assert_eq!(byte_words(0xe9), "e9 hex, not ASCII");
        assert_eq!(byte_words(b','), "2c hex, a comma");
        for b in 0..=255u8 {
            assert!(byte_words(b).chars().count() <= 31, "{b}");
        }
    }

    #[test]
    fn rows_cover_the_bytes_from_their_offset() {
        let rows = hex_rows(b"PK\x03\x04hello", 0x20, HexLayout::for_len(13, 16));
        assert_eq!(
            rows,
            ["000020: 50 4b 03 04 68 65 6c 6c 6f                      PK..hello"]
        );
    }
}
