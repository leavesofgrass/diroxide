//! The header readers: each format's few bytes before its pixels, read
//! through a seekable source and never past [`HeaderLimits::max_bytes`].
//! No pixel is decoded and no pixel buffer is made (spike P1: a JPEG
//! through the `image` crate's decoder reads the whole file, so JPEG's
//! markers are walked here up to the scan instead).

use std::io::{self, BufReader, Read, Seek, SeekFrom};

use image::ImageDecoder;

use super::{Color, ColorModel, Frames, ImageError, ImageFacts};
use crate::words::clean_text;

/// What a header read may cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeaderLimits {
    /// The most bytes read or skipped in one file, frames walked included.
    pub max_bytes: u64,
    /// The most segments, chunks, blocks or boxes looked at.
    pub max_parts: u32,
}

impl Default for HeaderLimits {
    /// 64 MiB and 100,000 parts.
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_parts: 100_000,
        }
    }
}

/// The formats told apart by their first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sniffed {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
    Tiff,
    Ico,
    Heif,
    Jp2,
    Jxl,
    Psd,
}

impl Sniffed {
    pub(super) fn word(self) -> &'static str {
        match self {
            Sniffed::Png => "PNG",
            Sniffed::Jpeg => "JPEG",
            Sniffed::Gif => "GIF",
            Sniffed::WebP => "WebP",
            Sniffed::Bmp => "BMP",
            Sniffed::Tiff => "TIFF",
            Sniffed::Ico => "ICO",
            Sniffed::Heif => "HEIF",
            Sniffed::Jp2 => "JP2",
            Sniffed::Jxl => "JXL",
            Sniffed::Psd => "PSD",
        }
    }
}

/// The format the first bytes say, if they say an image.
pub(super) fn sniff(head: &[u8]) -> Option<Sniffed> {
    let starts = |sig: &[u8]| head.starts_with(sig);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some(Sniffed::Png)
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some(Sniffed::Jpeg)
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some(Sniffed::Gif)
    } else if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        Some(Sniffed::WebP)
    } else if starts(b"BM") && head.len() >= 18 {
        Some(Sniffed::Bmp)
    } else if starts(b"II*\0") || starts(b"MM\0*") {
        Some(Sniffed::Tiff)
    } else if starts(&[0, 0, 1, 0]) || starts(&[0, 0, 2, 0]) {
        Some(Sniffed::Ico)
    } else if head.get(4..8) == Some(b"ftyp")
        && head.get(8..12).is_some_and(|brand| {
            [
                b"heic", b"heix", b"hevc", b"heim", b"heis", b"mif1", b"msf1", b"avif", b"avis",
                b"crx ",
            ]
            .iter()
            .any(|b| *b == brand)
        })
    {
        Some(Sniffed::Heif)
    } else if starts(&[0, 0, 0, 0x0C, b'j', b'P', b' ', b' ']) {
        Some(Sniffed::Jp2)
    } else if starts(&[0xFF, 0x0A]) || starts(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ']) {
        Some(Sniffed::Jxl)
    } else if starts(b"8BPS") {
        Some(Sniffed::Psd)
    } else {
        None
    }
}

/// The bytes a header read may take from its source past
/// [`HeaderLimits::max_bytes`]: the buffered reader's refills after a
/// seek, four buffers of 16 KiB.
pub(super) const READ_AHEAD: u64 = 64 * 1024;

/// The error a [`Metered`] source gives when its budget is spent; it
/// becomes [`ImageError::HeaderTooLong`].
#[derive(Debug)]
pub(super) struct OverBudget;

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("its header is too long to read here")
    }
}

impl std::error::Error for OverBudget {}

/// The source under the buffer, counting every byte it gives. The limit
/// on the position alone let a reader that seeks back and forth read the
/// same bytes again without end, each seek refilling the buffer (the
/// nightly fuzz run read past 1 MiB from a file under its limit of
/// 1 MiB, Tuesday, November 10, 2026), so the bytes taken are bounded
/// too, by the limit and [`READ_AHEAD`].
pub(super) struct Metered<'a> {
    inner: &'a mut dyn diroxide_vfs::ReadSeek,
    left: u64,
}

impl Read for Metered<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            return Err(io::Error::other(OverBudget));
        }
        let room = usize::try_from(self.left).unwrap_or(usize::MAX);
        let n = buf.len().min(room);
        let got = self.inner.read(&mut buf[..n])?;
        self.left = self.left.saturating_sub(got as u64);
        Ok(got)
    }
}

impl Seek for Metered<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// A reader over the source that counts what it may still read.
pub(super) struct Src<'a> {
    inner: BufReader<Metered<'a>>,
    limits: HeaderLimits,
    parts: u32,
}

impl<'a> Src<'a> {
    pub(super) fn new(source: &'a mut dyn diroxide_vfs::ReadSeek, limits: HeaderLimits) -> Self {
        let metered = Metered {
            inner: source,
            left: limits.max_bytes.saturating_add(READ_AHEAD),
        };
        Self {
            inner: BufReader::with_capacity(16 * 1024, metered),
            limits,
            parts: 0,
        }
    }

    fn pos(&mut self) -> Result<u64, ImageError> {
        Ok(self.inner.stream_position()?)
    }

    /// Counts one part, and refuses past the limits.
    fn part(&mut self) -> Result<(), ImageError> {
        self.parts += 1;
        if self.parts > self.limits.max_parts || self.pos()? > self.limits.max_bytes {
            return Err(ImageError::HeaderTooLong);
        }
        Ok(())
    }

    pub(super) fn seek_to(&mut self, at: u64) -> Result<(), ImageError> {
        if at > self.limits.max_bytes {
            return Err(ImageError::HeaderTooLong);
        }
        // A seek inside what is buffered keeps the buffer, so walking a
        // file's small boxes or chunks reads each byte once.
        let here = self.pos()?;
        match i64::try_from(at)
            .ok()
            .zip(i64::try_from(here).ok())
            .and_then(|(at, here)| at.checked_sub(here))
        {
            Some(by) => self.inner.seek_relative(by)?,
            None => {
                self.inner.seek(SeekFrom::Start(at))?;
            }
        }
        Ok(())
    }

    fn skip(&mut self, n: u64) -> Result<(), ImageError> {
        let at = self.pos()?.saturating_add(n);
        self.seek_to(at)
    }

    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, ImageError> {
        if self.pos()?.saturating_add(n as u64) > self.limits.max_bytes {
            return Err(ImageError::HeaderTooLong);
        }
        let mut out = vec![0; n];
        self.inner.read_exact(&mut out)?;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ImageError> {
        let mut out = [0; N];
        self.inner.read_exact(&mut out)?;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, ImageError> {
        Ok(self.array::<1>()?[0])
    }

    /// Up to `n` bytes, fewer at the end of the file.
    pub(super) fn head(&mut self, n: usize) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::with_capacity(n);
        (&mut self.inner).take(n as u64).read_to_end(&mut out)?;
        Ok(out)
    }

    /// The whole file, when it is no longer than `cap` and what is left
    /// of the read's budget holds it; else nothing, so a picture whose
    /// header has read is not refused for its Exif.
    fn whole(&mut self, cap: u64) -> Result<Option<Vec<u8>>, ImageError> {
        self.seek_to(0)?;
        let want = usize::try_from(cap.saturating_add(1)).unwrap_or(usize::MAX);
        match self.head(want) {
            Ok(bytes) if bytes.len() as u64 <= cap => Ok(Some(bytes)),
            Ok(_) | Err(ImageError::HeaderTooLong) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The source as a buffered reader for a decoder that reads its own
    /// header, from the start.
    fn decoder_input(&mut self) -> Result<&mut BufReader<Metered<'a>>, ImageError> {
        self.seek_to(0)?;
        Ok(&mut self.inner)
    }
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Reads a PNG's facts through the `png` crate up to its first image
/// data: size, color, the text chunks and the Exif chunk, and an APNG's
/// frame count.
pub(super) fn png(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    let input = src.decoder_input()?;
    let mut decoder = png::Decoder::new_with_limits(input, png::Limits { bytes: 8 << 20 });
    decoder.set_ignore_text_chunk(false);
    let reader = decoder.read_info().map_err(png_error)?;
    let info = reader.info();
    facts.width = info.width;
    facts.height = info.height;
    let (model, alpha) = match info.color_type {
        png::ColorType::Grayscale => (ColorModel::Gray, false),
        png::ColorType::GrayscaleAlpha => (ColorModel::Gray, true),
        png::ColorType::Rgb => (ColorModel::Rgb, false),
        png::ColorType::Rgba => (ColorModel::Rgb, true),
        png::ColorType::Indexed => (ColorModel::Indexed, false),
    };
    facts.color = Some(Color {
        model,
        bits: info.bit_depth as u8,
        alpha: alpha || info.trns.is_some(),
    });
    facts.has_profile = info.icc_profile.is_some();
    if let Some(control) = &info.animation_control
        && control.num_frames > 1
    {
        facts.frames = Frames::Many(control.num_frames);
    }
    // The author's words: Description, then Title, then Comment.
    let mut texts: Vec<(String, String)> = info
        .uncompressed_latin1_text
        .iter()
        .map(|t| (t.keyword.clone(), t.text.clone()))
        .collect();
    for chunk in &info.compressed_latin1_text {
        let mut chunk = chunk.clone();
        if chunk.decompress_text_with_limit(64 * 1024).is_ok()
            && let Ok(text) = chunk.get_text()
        {
            texts.push((chunk.keyword.clone(), text));
        }
    }
    for chunk in &info.utf8_text {
        let mut chunk = chunk.clone();
        if chunk.decompress_text_with_limit(64 * 1024).is_ok()
            && let Ok(text) = chunk.get_text()
        {
            texts.push((chunk.keyword.clone(), text));
        }
    }
    for keyword in ["Description", "Title", "Comment"] {
        if facts.description.is_none() {
            facts.description = texts
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(keyword))
                .map(|(_, t)| clean_text(t, super::MAX_TEXT))
                .find(|t| !t.is_empty());
        }
    }
    let exif = info.exif_metadata.as_ref().map(|e| e.to_vec());
    drop(reader);
    if let Some(exif) = exif {
        super::exif::apply(exif, facts);
    }
    Ok(())
}

fn png_error(error: png::DecodingError) -> ImageError {
    match error {
        png::DecodingError::IoError(e) => ImageError::from(e),
        png::DecodingError::LimitsExceeded => ImageError::HeaderTooLong,
        other => ImageError::Damaged(plain_or(other.to_string(), "the header is damaged")),
    }
}

/// Walks a JPEG's markers up to its scan: the frame's size, precision
/// and components, the Exif segment, and whether a color profile is
/// there.
pub(super) fn jpeg(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(2)?;
    let mut frame = None;
    let mut exif = None;
    loop {
        src.part()?;
        // A marker: one or more 0xFF, then its code.
        let mut code = src.u8()?;
        if code != 0xFF {
            return Err(ImageError::Damaged("a JPEG marker is missing".to_owned()));
        }
        while code == 0xFF {
            code = src.u8()?;
        }
        match code {
            0x01 | 0xD0..=0xD8 => continue,
            0xD9 | 0xDA => break,
            _ => {}
        }
        let len = usize::from(be16(&src.array::<2>()?));
        if len < 2 {
            return Err(ImageError::Damaged(
                "a JPEG segment is too short".to_owned(),
            ));
        }
        let body = len - 2;
        match code {
            // Every start of frame but the Huffman, extension and
            // arithmetic-conditioning tables that share the range.
            0xC0..=0xCF if !matches!(code, 0xC4 | 0xC8 | 0xCC) => {
                let b = src.bytes(body)?;
                if b.len() < 6 {
                    return Err(ImageError::Damaged(
                        "its frame header is cut short".to_owned(),
                    ));
                }
                frame = Some((b[0], be16(&b[1..3]), be16(&b[3..5]), b[5]));
            }
            0xE1 if exif.is_none() => {
                let b = src.bytes(body)?;
                if let Some(rest) = b.strip_prefix(b"Exif\0\0") {
                    exif = Some(rest.to_vec());
                }
            }
            0xE2 => {
                let b = src.bytes(body)?;
                if b.starts_with(b"ICC_PROFILE\0") {
                    facts.has_profile = true;
                }
            }
            _ => src.skip(body as u64)?,
        }
    }
    let Some((precision, height, width, components)) = frame else {
        return Err(ImageError::Damaged("it has no frame header".to_owned()));
    };
    facts.width = u32::from(width);
    facts.height = u32::from(height);
    facts.color = Some(Color {
        model: match components {
            1 => ColorModel::Gray,
            4 => ColorModel::Cmyk,
            _ => ColorModel::Rgb,
        },
        bits: precision,
        alpha: false,
    });
    if let Some(exif) = exif {
        super::exif::apply(exif, facts);
    }
    Ok(())
}

/// A GIF's screen size and color table, then a walk over its blocks that
/// counts the frames and looks for transparency, never reading a frame's
/// data.
pub(super) fn gif(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(0)?;
    let head = src.bytes(13)?;
    facts.width = u32::from(le16(&head[6..8]));
    facts.height = u32::from(le16(&head[8..10]));
    let packed = head[10];
    let mut alpha = false;
    if packed & 0x80 != 0 {
        src.skip(3 << ((packed & 7) + 1))?;
    }
    let mut frames = 0u32;
    // The walk stops quietly at its limits: the count is then unknown.
    let walked = (|| -> Result<(), ImageError> {
        loop {
            src.part()?;
            match src.u8()? {
                0x21 => {
                    let label = src.u8()?;
                    if label == 0xF9 {
                        let size = src.u8()?;
                        let b = src.bytes(usize::from(size))?;
                        if b.first().is_some_and(|flags| flags & 1 != 0) {
                            alpha = true;
                        }
                    }
                    skip_sub_blocks(src)?;
                }
                0x2C => {
                    frames += 1;
                    let d = src.bytes(9)?;
                    if d[8] & 0x80 != 0 {
                        src.skip(3 << ((d[8] & 7) + 1))?;
                    }
                    let _min_code = src.u8()?;
                    skip_sub_blocks(src)?;
                }
                _ => return Ok(()),
            }
        }
    })();
    facts.color = Some(Color {
        model: ColorModel::Indexed,
        bits: 8,
        alpha,
    });
    facts.frames = match (walked, frames) {
        (Ok(()), 0 | 1) => Frames::One,
        (Ok(()), n) => Frames::Many(n),
        (Err(_), 0 | 1) => Frames::One,
        (Err(_), _) => Frames::Animated,
    };
    Ok(())
}

fn skip_sub_blocks(src: &mut Src<'_>) -> Result<(), ImageError> {
    loop {
        src.part()?;
        let size = src.u8()?;
        if size == 0 {
            return Ok(());
        }
        src.skip(u64::from(size))?;
    }
}

/// A WebP's chunks: the size from its first chunk, the extended header's
/// flags, the frames and the Exif chunk.
pub(super) fn webp(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(12)?;
    let mut frames = 0u32;
    let mut animated = false;
    let mut alpha = false;
    let mut first = true;
    let mut exif = None;
    loop {
        if src.part().is_err() {
            break;
        }
        let Ok(head) = src.array::<8>() else { break };
        let kind = [head[0], head[1], head[2], head[3]];
        let size = u64::from(le32(&head[4..8]));
        let padded = size + (size & 1);
        let start = src.pos()?;
        match &kind {
            b"VP8X" => {
                let b = src.bytes(10)?;
                facts.has_profile = b[0] & 0x20 != 0;
                alpha = b[0] & 0x10 != 0;
                animated = b[0] & 0x02 != 0;
                facts.width = 1 + (u32::from(b[4]) | u32::from(b[5]) << 8 | u32::from(b[6]) << 16);
                facts.height = 1 + (u32::from(b[7]) | u32::from(b[8]) << 8 | u32::from(b[9]) << 16);
            }
            b"VP8 " if first => {
                let b = src.bytes(10)?;
                if b[3..6] != [0x9D, 0x01, 0x2A] {
                    return Err(ImageError::Damaged(
                        "its frame start code is wrong".to_owned(),
                    ));
                }
                facts.width = u32::from(le16(&b[6..8]) & 0x3FFF);
                facts.height = u32::from(le16(&b[8..10]) & 0x3FFF);
            }
            b"VP8L" if first => {
                let b = src.bytes(5)?;
                if b[0] != 0x2F {
                    return Err(ImageError::Damaged(
                        "its lossless signature is wrong".to_owned(),
                    ));
                }
                let bits = le32(&b[1..5]);
                facts.width = (bits & 0x3FFF) + 1;
                facts.height = ((bits >> 14) & 0x3FFF) + 1;
                alpha = (bits >> 28) & 1 != 0;
            }
            b"ANMF" => frames += 1,
            b"EXIF" if exif.is_none() && size <= 1 << 20 => {
                let b = src.bytes(size as usize)?;
                // Some writers keep the JPEG segment's prefix.
                exif = Some(
                    b.strip_prefix(b"Exif\0\0")
                        .map_or(b.clone(), <[u8]>::to_vec),
                );
            }
            _ => {}
        }
        first = false;
        if src.seek_to(start + padded).is_err() {
            break;
        }
    }
    if facts.width == 0 {
        return Err(ImageError::Damaged("it has no image chunk".to_owned()));
    }
    facts.color = Some(Color {
        model: ColorModel::Rgb,
        bits: 8,
        alpha,
    });
    facts.frames = match (animated, frames) {
        (true, 0) => Frames::Animated,
        (_, 0 | 1) => Frames::One,
        (_, n) => Frames::Many(n),
    };
    if let Some(exif) = exif {
        super::exif::apply(exif, facts);
    }
    Ok(())
}

/// A bitmap's file and information headers.
pub(super) fn bmp(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(14)?;
    let size = le32(&src.array::<4>()?);
    let (width, height, bits) = if size == 12 {
        let b = src.bytes(8)?;
        (
            u32::from(le16(&b[0..2])),
            u32::from(le16(&b[2..4])),
            le16(&b[6..8]),
        )
    } else if (40..=1024).contains(&size) {
        let b = src.bytes(12)?;
        let w = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let h = i32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        (w.unsigned_abs(), h.unsigned_abs(), le16(&b[10..12]))
    } else {
        return Err(ImageError::Damaged("its header size is unknown".to_owned()));
    };
    facts.width = width;
    facts.height = height;
    facts.color = Some(match bits {
        1 | 2 | 4 | 8 => Color {
            model: ColorModel::Indexed,
            bits: bits as u8,
            alpha: false,
        },
        32 => Color {
            model: ColorModel::Rgb,
            bits: 8,
            alpha: true,
        },
        16 => Color {
            model: ColorModel::Rgb,
            bits: 5,
            alpha: false,
        },
        _ => Color {
            model: ColorModel::Rgb,
            bits: 8,
            alpha: false,
        },
    });
    Ok(())
}

/// An icon file's directory: every size it holds.
pub(super) fn ico(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(4)?;
    let count = le16(&src.array::<2>()?);
    if count == 0 {
        return Err(ImageError::Damaged("it holds no icons".to_owned()));
    }
    let mut largest = (0u32, 0u32, 0u16);
    for _ in 0..count.min(512) {
        src.part()?;
        let e = src.bytes(16)?;
        let side = |b: u8| if b == 0 { 256 } else { u32::from(b) };
        let (w, h) = (side(e[0]), side(e[1]));
        let bits = le16(&e[6..8]);
        if !facts.icon_sizes.contains(&(w, h)) {
            facts.icon_sizes.push((w, h));
        }
        if u64::from(w) * u64::from(h) > u64::from(largest.0) * u64::from(largest.1) {
            largest = (w, h, bits);
        }
    }
    facts.width = largest.0;
    facts.height = largest.1;
    facts.color = Some(Color {
        model: if largest.2 != 0 && largest.2 <= 8 {
            ColorModel::Indexed
        } else {
            ColorModel::Rgb
        },
        bits: if largest.2 == 0 || largest.2 >= 24 {
            8
        } else {
            largest.2 as u8
        },
        alpha: largest.2 == 32 || largest.2 == 0,
    });
    Ok(())
}

/// A TIFF's first directory, through the `image` crate's decoder, which
/// reads the directory and not the strips; then its Exif, from the file
/// itself, when the file is small enough to read whole.
pub(super) fn tiff(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    let limits = src.limits;
    let decoded = {
        let input = src.decoder_input()?;
        let mut reader = image::ImageReader::with_format(input, image::ImageFormat::Tiff);
        reader.limits(super::PixelGuard::DEFAULT.limits());
        reader.into_decoder().map(|decoder| {
            let (w, h) = decoder.dimensions();
            (w, h, color_of(decoder.original_color_type()))
        })
    };
    // A size past the guard is refused by the decoder before it reads
    // further; the size is then read from the directory by hand.
    match decoded {
        Ok((w, h, color)) => {
            facts.width = w;
            facts.height = h;
            facts.color = Some(color);
        }
        Err(image::ImageError::Limits(_)) => {
            let (w, h) = tiff_size(src)?;
            facts.width = w;
            facts.height = h;
        }
        Err(e) => return Err(ImageError::Damaged(e.to_string())),
    }
    if let Some(whole) = src.whole(limits.max_bytes.min(16 << 20))? {
        super::exif::apply(whole, facts);
    }
    Ok(())
}

/// The width and height from a TIFF's first directory, read by hand.
fn tiff_size(src: &mut Src<'_>) -> Result<(u32, u32), ImageError> {
    src.seek_to(0)?;
    let head = src.bytes(8)?;
    let little = head[0] == b'I';
    let u16_of = |b: &[u8]| if little { le16(b) } else { be16(b) };
    let u32_of = |b: &[u8]| if little { le32(b) } else { be32(b) };
    src.seek_to(u64::from(u32_of(&head[4..8])))?;
    let count = u16_of(&src.array::<2>()?);
    let (mut w, mut h) = (0, 0);
    for _ in 0..count.min(4096) {
        let e = src.bytes(12)?;
        let value = match u16_of(&e[2..4]) {
            3 => u32::from(u16_of(&e[8..10])),
            _ => u32_of(&e[8..12]),
        };
        match u16_of(&e[0..2]) {
            256 => w = value,
            257 => h = value,
            _ => {}
        }
    }
    Ok((w, h))
}

/// A color in words from the `image` crate's extended type.
fn color_of(kind: image::ExtendedColorType) -> Color {
    use image::ExtendedColorType as E;
    let (model, bits, alpha) = match kind {
        E::A8 => (ColorModel::Gray, 8, true),
        E::L1 => (ColorModel::Gray, 1, false),
        E::La1 => (ColorModel::Gray, 1, true),
        E::L2 => (ColorModel::Gray, 2, false),
        E::La2 => (ColorModel::Gray, 2, true),
        E::L4 => (ColorModel::Gray, 4, false),
        E::La4 => (ColorModel::Gray, 4, true),
        E::L8 => (ColorModel::Gray, 8, false),
        E::La8 => (ColorModel::Gray, 8, true),
        E::L16 => (ColorModel::Gray, 16, false),
        E::La16 => (ColorModel::Gray, 16, true),
        E::Rgb1 => (ColorModel::Rgb, 1, false),
        E::Rgba1 => (ColorModel::Rgb, 1, true),
        E::Rgb2 => (ColorModel::Rgb, 2, false),
        E::Rgba2 => (ColorModel::Rgb, 2, true),
        E::Rgb4 => (ColorModel::Rgb, 4, false),
        E::Rgba4 => (ColorModel::Rgb, 4, true),
        E::Rgb16 => (ColorModel::Rgb, 16, false),
        E::Rgba16 => (ColorModel::Rgb, 16, true),
        E::Rgb32F => (ColorModel::Rgb, 32, false),
        E::Rgba32F => (ColorModel::Rgb, 32, true),
        E::Rgba8 | E::Bgra8 => (ColorModel::Rgb, 8, true),
        E::Cmyk8 => (ColorModel::Cmyk, 8, false),
        E::Cmyk16 => (ColorModel::Cmyk, 16, false),
        _ => (ColorModel::Rgb, 8, false),
    };
    Color { model, bits, alpha }
}

/// A Photoshop file's header: its size and depth.
pub(super) fn psd(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(0)?;
    let b = src.bytes(26)?;
    facts.height = be32(&b[14..18]);
    facts.width = be32(&b[18..22]);
    let bits = be16(&b[22..24]);
    let model = match be16(&b[24..26]) {
        1 | 8 => ColorModel::Gray,
        2 => ColorModel::Indexed,
        4 => ColorModel::Cmyk,
        _ => ColorModel::Rgb,
    };
    facts.color = Some(Color {
        model,
        bits: u8::try_from(bits).unwrap_or(8),
        alpha: false,
    });
    Ok(())
}

/// A JPEG 2000 file's image header box, inside its header box.
pub(super) fn jp2(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    if let Some((at, _)) = find_box(src, 0, None, b"jp2h")?
        && let Some((ihdr, _)) = find_box(src, at, None, b"ihdr")?
    {
        src.seek_to(ihdr)?;
        let b = src.bytes(8)?;
        facts.height = be32(&b[0..4]);
        facts.width = be32(&b[4..8]);
    }
    Ok(())
}

/// A HEIF or AVIF file's size, from the image spatial extents property
/// (`ispe`) in its `meta` box, and its Exif through `kamadak-exif`.
pub(super) fn heif(src: &mut Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    if let Some((meta, meta_end)) = find_box(src, 0, None, b"meta")? {
        // `meta` is a full box: a version and flags come first.
        let inner = meta + 4;
        if let Some((iprp, iprp_end)) = find_box(src, inner, Some(meta_end), b"iprp")?
            && let Some((ipco, ipco_end)) = find_box(src, iprp, Some(iprp_end), b"ipco")?
        {
            let _ = iprp_end;
            // The largest extent is the primary picture's, the others
            // being tiles and thumbnails.
            let mut at = ipco;
            while let Some((ispe, end)) = find_box(src, at, Some(ipco_end), b"ispe")? {
                src.seek_to(ispe + 4)?;
                let b = src.bytes(8)?;
                let (w, h) = (be32(&b[0..4]), be32(&b[4..8]));
                if u64::from(w) * u64::from(h) > u64::from(facts.width) * u64::from(facts.height) {
                    facts.width = w;
                    facts.height = h;
                }
                at = end;
            }
        }
    }
    // The Exif item can sit anywhere in the file, so a small file is read
    // whole for it and a larger one goes without.
    let cap = src.limits.max_bytes.min(16 << 20);
    if let Some(whole) = src.whole(cap)?
        && let Ok(exif) = ::exif::Reader::new().read_from_container(&mut io::Cursor::new(whole))
    {
        super::exif::apply_parsed(&exif, facts);
    }
    Ok(())
}

/// The first box of a type from `start`, before `end`: where its body
/// starts and where the box ends. Boxes are walked, never read.
fn find_box(
    src: &mut Src<'_>,
    start: u64,
    end: Option<u64>,
    kind: &[u8; 4],
) -> Result<Option<(u64, u64)>, ImageError> {
    let mut at = start;
    loop {
        // A box's claimed size may carry `at` to the end of a u64.
        if end.is_some_and(|e| at.saturating_add(8) > e) {
            return Ok(None);
        }
        src.part()?;
        if src.seek_to(at).is_err() {
            return Ok(None);
        }
        let Ok(head) = src.array::<8>() else {
            return Ok(None);
        };
        let mut size = u64::from(be32(&head[0..4]));
        let mut body = at + 8;
        if size == 1 {
            let Ok(large) = src.array::<8>() else {
                return Ok(None);
            };
            size = u64::from_be_bytes(large);
            body += 8;
        } else if size == 0 {
            size = end.unwrap_or(u64::MAX).saturating_sub(at);
        }
        if size < body - at {
            return Ok(None);
        }
        let box_end = at.saturating_add(size);
        if head[4..8] == kind[..] {
            return Ok(Some((body, box_end)));
        }
        at = box_end;
    }
}

/// A decoder's words, kept when they are plain (printable ASCII and no
/// debug braces), else the fallback: the png crate prints a chunk's raw
/// type bytes inside braces ("ChunkType { type: ..., critical: false }
/// chunk appeared before IHDR chunk"), which a screen reader cannot say.
pub(crate) fn plain_or(words: String, fallback: &str) -> String {
    let plain = !words.is_empty()
        && words.chars().all(|c| c.is_ascii_graphic() || c == ' ')
        && !words.contains(['{', '}']);
    if plain { words } else { fallback.to_owned() }
}
