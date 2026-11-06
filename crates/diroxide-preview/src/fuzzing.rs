//! The bodies of the `preview_*` fuzz targets (alpha 4), as plain
//! functions, so the libFuzzer entry points in `fuzz/fuzz_targets/` and
//! the ordinary tests call the same code. Each body panics only when a
//! property fails; bad input is an error, never a panic, a hang or a read
//! past the limits.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::images::{self, HeaderLimits, PixelGuard, TypeHint};

/// A source that counts the bytes read from it, so a target can check
/// that a reader stayed within its limits (a seek reads nothing).
struct Counted<'a> {
    inner: Cursor<&'a [u8]>,
    read: u64,
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl Seek for Counted<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// `preview_image_header`: any bytes as a picture's file, through the
/// header readers, with small limits. The properties: no panic; a header
/// that reads gives words that start with the format and rows whose every
/// line is labeled; nothing past the limits is read.
pub fn image_header(data: &[u8]) {
    let limits = HeaderLimits {
        max_bytes: 1 << 20,
        max_parts: 4_096,
    };
    let mut source = Counted {
        inner: Cursor::new(data),
        read: 0,
    };
    if let Ok(facts) = images::read_header(&mut source, TypeHint::default(), limits) {
        let guard = PixelGuard::DEFAULT;
        let sentence = facts.sentence(Some(data.len() as u64), &guard);
        assert!(sentence.starts_with(&facts.format), "{sentence}");
        for field in facts.fields(&guard) {
            assert!(!field.label.is_empty() && !field.line().contains('\n'));
        }
    }
    // The buffered reader may read one buffer ahead of the limit.
    assert!(
        source.read <= limits.max_bytes + 64 * 1024,
        "{}",
        source.read
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_target_takes_any_bytes() {
        image_header(&[]);
        image_header(b"\x89PNG\r\n\x1a\n");
        image_header(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 2]);
        image_header(b"GIF89a\xff\xff\xff\xff\xf7\x00\x00");
        image_header(b"RIFF\x00\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00");
        image_header(&[0, 0, 1, 0, 0xFF, 0xFF]);
        image_header(b"II*\x00\xff\xff\xff\xff");
    }
}
