//! The metadata parts of a JPEG, PNG or WebP file, read from the head of
//! the file by walking its segments or chunks and seeking past the
//! picture, never decoding a pixel: the Exif block, whether XMP, IPTC
//! and a color profile are there, comments and text notes, and whether
//! more pictures ride along (a JPEG's multi-picture segment). The image
//! Exif reader and Check before sharing read through it; the strip works
//! on the whole file with `img-parts` ([`crate::strip`]).

use std::io::SeekFrom;

use diroxide_vfs::ReadSeek;

use crate::reader::MetaError;

/// The most bytes of one metadata segment or chunk read: 1 MiB.
pub const PART_CAP: u64 = 1024 * 1024;

/// The most segments or chunks walked.
pub const MOST_PARTS: usize = 10_000;

/// The three containers the strip and Check before sharing know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageKind {
    /// JPEG.
    Jpeg,
    /// PNG.
    Png,
    /// WebP.
    Webp,
}

impl ImageKind {
    /// The kind from a file's first bytes.
    pub fn sniff(head: &[u8]) -> Option<Self> {
        if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageKind::Jpeg)
        } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(ImageKind::Png)
        } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
            Some(ImageKind::Webp)
        } else {
            None
        }
    }

    /// The name as said: "JPEG", "PNG", "WebP".
    pub const fn name(self) -> &'static str {
        match self {
            ImageKind::Jpeg => "JPEG",
            ImageKind::Png => "PNG",
            ImageKind::Webp => "WebP",
        }
    }
}

/// The metadata parts of one picture file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageParts {
    /// Its container.
    pub kind: ImageKind,
    /// The Exif block, without the `Exif\0\0` prefix where it had one.
    pub exif: Option<Vec<u8>>,
    /// Whether XMP is there.
    pub xmp: bool,
    /// Whether IPTC (Photoshop's resources) is there.
    pub iptc: bool,
    /// Whether a color profile is there.
    pub icc: bool,
    /// JPEG comments and PNG text comments.
    pub comments: Vec<String>,
    /// The keywords of PNG text notes ("Author", "Comment", "date:create").
    pub text_keys: Vec<String>,
    /// Whether more pictures ride along (a multi-picture segment).
    pub more_pictures: bool,
}

impl ImageParts {
    fn new(kind: ImageKind) -> Self {
        Self {
            kind,
            exif: None,
            xmp: false,
            iptc: false,
            icc: false,
            comments: Vec::new(),
            text_keys: Vec::new(),
            more_pictures: false,
        }
    }
}

fn io(error: std::io::Error) -> MetaError {
    MetaError::from_io(&error)
}

fn read_exact(source: &mut dyn ReadSeek, len: usize) -> Result<Vec<u8>, MetaError> {
    let mut bytes = vec![0u8; len];
    source.read_exact(&mut bytes).map_err(io)?;
    Ok(bytes)
}

fn skip(source: &mut dyn ReadSeek, len: u64) -> Result<(), MetaError> {
    let len = i64::try_from(len).map_err(|_| MetaError::unreadable("the file is damaged"))?;
    source.seek(SeekFrom::Current(len)).map_err(io)?;
    Ok(())
}

/// Reads the metadata parts of the picture `source` holds, from its
/// start. Not a JPEG, PNG or WebP: "not a JPEG, PNG or WebP picture".
pub fn read_parts(source: &mut dyn ReadSeek) -> Result<ImageParts, MetaError> {
    source.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut head = [0u8; 12];
    let mut got = 0;
    while got < head.len() {
        match source.read(&mut head[got..]).map_err(io)? {
            0 => break,
            n => got += n,
        }
    }
    let kind = ImageKind::sniff(&head[..got])
        .ok_or_else(|| MetaError::unreadable("not a JPEG, PNG or WebP picture"))?;
    source.seek(SeekFrom::Start(0)).map_err(io)?;
    match kind {
        ImageKind::Jpeg => jpeg_parts(source),
        ImageKind::Png => png_parts(source),
        ImageKind::Webp => webp_parts(source),
    }
}

fn jpeg_parts(source: &mut dyn ReadSeek) -> Result<ImageParts, MetaError> {
    let mut parts = ImageParts::new(ImageKind::Jpeg);
    skip(source, 2)?;
    for _ in 0..MOST_PARTS {
        let mut byte = [0u8; 1];
        source.read_exact(&mut byte).map_err(io)?;
        if byte[0] != 0xFF {
            continue;
        }
        let mut marker = 0xFF;
        while marker == 0xFF {
            source.read_exact(&mut byte).map_err(io)?;
            marker = byte[0];
        }
        match marker {
            // The scan or the end: the metadata is all before it.
            0xDA | 0xD9 => break,
            0x00 | 0x01 | 0xD0..=0xD8 => continue,
            _ => {}
        }
        let len = read_exact(source, 2)?;
        let len = u64::from(u16::from_be_bytes([len[0], len[1]]));
        if len < 2 {
            return Err(MetaError::unreadable("the picture is damaged"));
        }
        let len = len - 2;
        let wanted = matches!(marker, 0xE1 | 0xE2 | 0xED | 0xFE);
        if !wanted {
            skip(source, len)?;
            continue;
        }
        let body = read_exact(source, usize::try_from(len).unwrap_or(0))?;
        match marker {
            0xE1 if body.starts_with(crate::exif::EXIF_PREFIX) => {
                if parts.exif.is_none() {
                    parts.exif = Some(body[6..].to_vec());
                }
            }
            0xE1 if body.starts_with(b"http://ns.adobe.com/x") => parts.xmp = true,
            0xE2 if body.starts_with(b"ICC_PROFILE\0") => parts.icc = true,
            0xE2 if body.starts_with(b"MPF\0") => parts.more_pictures = true,
            0xED if body.starts_with(b"Photoshop 3.0\0") => {
                parts.iptc |= body.windows(6).any(|w| w == b"8BIM\x04\x04");
            }
            0xFE => {
                let text = String::from_utf8_lossy(&body)
                    .trim_matches('\0')
                    .trim()
                    .to_owned();
                if !text.is_empty() {
                    parts.comments.push(text);
                }
            }
            _ => {}
        }
    }
    Ok(parts)
}

fn png_parts(source: &mut dyn ReadSeek) -> Result<ImageParts, MetaError> {
    let mut parts = ImageParts::new(ImageKind::Png);
    skip(source, 8)?;
    for _ in 0..MOST_PARTS {
        let header = match read_exact(source, 8) {
            Ok(header) => header,
            // A file cut after its last chunk still told what it has.
            Err(_) => break,
        };
        let len = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind = [header[4], header[5], header[6], header[7]];
        if &kind == b"IEND" {
            break;
        }
        let wanted = matches!(&kind, b"tEXt" | b"iTXt" | b"zTXt" | b"eXIf" | b"iCCP");
        if !wanted || len > PART_CAP {
            if &kind == b"iCCP" {
                parts.icc = true;
            }
            skip(source, len + 4)?;
            continue;
        }
        let body = read_exact(source, usize::try_from(len).unwrap_or(0))?;
        skip(source, 4)?;
        match &kind {
            b"eXIf" => {
                let block = body.strip_prefix(crate::exif::EXIF_PREFIX).unwrap_or(&body);
                parts.exif.get_or_insert_with(|| block.to_vec());
            }
            b"iCCP" => parts.icc = true,
            _ => {
                let end = body.iter().position(|b| *b == 0).unwrap_or(body.len());
                let key = String::from_utf8_lossy(&body[..end]).into_owned();
                match key.as_str() {
                    "XML:com.adobe.xmp" => parts.xmp = true,
                    "Raw profile type exif" | "Raw profile type APP1" => {
                        // Exif written as hexadecimal text by some
                        // programs: counted as present.
                        parts.exif.get_or_insert_with(Vec::new);
                    }
                    "Raw profile type iptc" => parts.iptc = true,
                    "Comment" | "Description" if &kind == b"tEXt" => {
                        let text = String::from_utf8_lossy(body.get(end + 1..).unwrap_or_default())
                            .trim()
                            .to_owned();
                        if !text.is_empty() {
                            parts.comments.push(text);
                        }
                    }
                    _ => {}
                }
                if !key.is_empty() && parts.text_keys.len() < 64 {
                    parts.text_keys.push(key);
                }
            }
        }
    }
    Ok(parts)
}

fn webp_parts(source: &mut dyn ReadSeek) -> Result<ImageParts, MetaError> {
    let mut parts = ImageParts::new(ImageKind::Webp);
    skip(source, 12)?;
    for _ in 0..MOST_PARTS {
        let header = match read_exact(source, 8) {
            Ok(header) => header,
            Err(_) => break,
        };
        let id = [header[0], header[1], header[2], header[3]];
        let len = u64::from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        let padded = len + (len & 1);
        match &id {
            b"EXIF" if len <= PART_CAP => {
                let body = read_exact(source, usize::try_from(len).unwrap_or(0))?;
                skip(source, padded - len)?;
                let block = body.strip_prefix(crate::exif::EXIF_PREFIX).unwrap_or(&body);
                parts.exif.get_or_insert_with(|| block.to_vec());
            }
            _ => {
                match &id {
                    b"XMP " => parts.xmp = true,
                    b"ICCP" => parts.icc = true,
                    _ => {}
                }
                skip(source, padded)?;
            }
        }
    }
    Ok(parts)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;

    /// A JPEG's segments around a tiny scan, made in memory: `segments`
    /// as `(marker, body)`, then a start-of-frame, a scan and the end.
    pub(crate) fn jpeg(segments: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        for (marker, body) in segments {
            out.extend_from_slice(&[0xFF, *marker]);
            out.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
            out.extend_from_slice(body);
        }
        // A baseline frame header for a 1 by 1 picture, one component.
        out.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        out.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
        out.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56]);
        out.extend_from_slice(&[0xFF, 0xD9]);
        out
    }

    /// A PNG's chunks made in memory, CRCs right, around a header and an
    /// empty data chunk.
    pub(crate) fn png(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let chunk = |kind: &[u8; 4], body: &[u8]| {
            let mut c = (body.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(body);
            let mut crc = crc32fast::Hasher::new();
            crc.update(kind);
            crc.update(body);
            c.extend_from_slice(&crc.finalize().to_be_bytes());
            c
        };
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
        for (kind, body) in chunks {
            out.extend(chunk(kind, body));
        }
        out.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 4, 0, 1]));
        out.extend(chunk(b"IEND", &[]));
        out
    }

    #[test]
    fn a_jpeg_head_tells_its_parts() {
        let mut exif = crate::exif::EXIF_PREFIX.to_vec();
        exif.extend_from_slice(&crate::exif::tests::private_block(true));
        let bytes = jpeg(&[
            (0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0".to_vec()),
            (0xE1, exif),
            (0xE1, b"http://ns.adobe.com/xap/1.0/\0<x/>".to_vec()),
            (0xE2, b"ICC_PROFILE\0\x01\x01profile".to_vec()),
            (0xE2, b"MPF\0MM\0*".to_vec()),
            (0xED, b"Photoshop 3.0\08BIM\x04\x04\0\0\0\0\0\0".to_vec()),
            (0xFE, b"a JPEG comment".to_vec()),
        ]);
        let parts = read_parts(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(parts.kind, ImageKind::Jpeg);
        assert!(parts.exif.is_some());
        assert!(parts.xmp && parts.icc && parts.iptc && parts.more_pictures);
        assert_eq!(parts.comments, ["a JPEG comment"]);
    }

    #[test]
    fn a_png_tells_its_notes() {
        let bytes = png(&[
            (b"tEXt", b"Author\0Ada Example".to_vec()),
            (b"tEXt", b"Comment\0a PNG comment".to_vec()),
            (b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x/>".to_vec()),
            (b"eXIf", crate::exif::orientation_only(6)),
            (b"iCCP", b"icm\0\0zz".to_vec()),
        ]);
        let parts = read_parts(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(parts.kind, ImageKind::Png);
        assert_eq!(parts.text_keys, ["Author", "Comment", "XML:com.adobe.xmp"]);
        assert_eq!(parts.comments, ["a PNG comment"]);
        assert!(parts.xmp && parts.icc);
        assert_eq!(
            crate::exif::parse(parts.exif.as_deref().unwrap())
                .unwrap()
                .orientation,
            Some(6)
        );
    }

    #[test]
    fn not_a_picture_and_cut_pictures_are_words_not_panics() {
        assert_eq!(
            read_parts(&mut Cursor::new(b"hello".to_vec())),
            Err(MetaError::unreadable("not a JPEG, PNG or WebP picture"))
        );
        let whole = jpeg(&[(0xFE, b"note".to_vec())]);
        for end in 0..whole.len() {
            let _ = read_parts(&mut Cursor::new(whole[..end].to_vec()));
        }
        let whole = png(&[(b"tEXt", b"Comment\0x".to_vec())]);
        for end in 0..whole.len() {
            let _ = read_parts(&mut Cursor::new(whole[..end].to_vec()));
        }
        // A length of zero is damaged, not a loop.
        let mut bad = vec![0xFF, 0xD8, 0xFF, 0xE1, 0, 0];
        bad.extend_from_slice(&[0; 10]);
        assert!(read_parts(&mut Cursor::new(bad)).is_err());
    }
}
