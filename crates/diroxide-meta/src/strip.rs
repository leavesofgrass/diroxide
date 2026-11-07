//! Strip into a copy (R12 "Image metadata"; R28 3.2; R30 commit 50):
//! JPEG, PNG and WebP through `img-parts`, never decoding a pixel and
//! never changing the original. The copy keeps what the picture needs
//! to look right and nothing that says who, where or with what:
//!
//! - **Removed:** Exif (with its place, camera, serials, author,
//!   comments, maker's note and stored thumbnail), XMP, IPTC, JPEG
//!   comments and the other application segments, PNG text notes and
//!   their time, and in a JPEG the multi-picture segment and anything
//!   after the end of the picture (a phone's second picture rides there
//!   with its own details).
//! - **Kept:** the color profile; JFIF and Adobe's segment (which says
//!   how CMYK pictures decode); the coded picture, byte for byte; and the
//!   orientation, as an Exif block holding it alone, so a portrait photo
//!   does not turn sideways.
//!
//! The copy is then read again and what is left is said: "Left in the
//! copy: orientation, color profile, image size".
//!
//! Spike M3 (pictures made by ImageMagick and ExifTool in the tools
//! container, nothing downloaded): on a portrait JPEG with a stored
//! thumbnail, GPS, serials, XMP, IPTC and a comment, a progressive JPEG,
//! a CMYK JPEG, a JPEG with a second picture after its end, a PNG with
//! text notes and `eXIf`, and lossy and lossless WebP, ExifTool found
//! only the orientation and the profile left, every orientation kept,
//! the coded data the same byte for byte and ImageMagick's pixel
//! comparison 0. `img-parts` keeps WebP's XMP flag set when the XMP
//! chunk goes, so the strip clears it itself. 3,200 damaged samples were
//! refused or written without a panic, the slowest in 80 microseconds.

use std::io::{Read, Write};

use diroxide_core::Place;
use diroxide_core::names::{display_name, split_extension};
use diroxide_vfs::{OpContext, Provider, WriteMode};
use img_parts::jpeg::{Jpeg, JpegSegment, markers};
use img_parts::png::Png;
use img_parts::riff::RiffContent;
use img_parts::webp::WebP;
use img_parts::{Bytes, ImageEXIF};

use crate::exif;
use crate::guard::Guard;
use crate::image::{ImageKind, read_parts};
use crate::reader::MetaError;

/// The largest picture stripped: 256 MiB, the preview's pixel cap.
pub const MAX_STRIP_BYTES: u64 = 256 * 1024 * 1024;

/// What a strip took out, in words, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Removed {
    /// The Exif block (place, camera, author, thumbnail among it).
    Exif,
    /// XMP.
    Xmp,
    /// IPTC.
    Iptc,
    /// JPEG comments.
    Comments,
    /// PNG text notes.
    TextNotes,
    /// A PNG's time of last change.
    Time,
    /// Other application data.
    OtherData,
    /// Pictures riding along: a multi-picture segment, or bytes after
    /// the end of a JPEG.
    MorePictures,
}

impl Removed {
    /// In words: "Exif data", "XMP data", "comments".
    pub const fn words(self) -> &'static str {
        match self {
            Removed::Exif => "Exif data",
            Removed::Xmp => "XMP data",
            Removed::Iptc => "IPTC data",
            Removed::Comments => "comments",
            Removed::TextNotes => "text notes",
            Removed::Time => "time of change",
            Removed::OtherData => "other program data",
            Removed::MorePictures => "pictures riding along",
        }
    }
}

/// A stripped picture: its bytes and what was taken out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stripped {
    /// Its container.
    pub kind: ImageKind,
    /// The copy's bytes.
    pub bytes: Vec<u8>,
    /// What was taken out, each once, in order.
    pub removed: Vec<Removed>,
    /// The orientation kept, when it was not the normal one.
    pub orientation: Option<u16>,
}

/// Why a picture was not stripped.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StripError {
    /// Not one of the three containers.
    #[error("only JPEG, PNG and WebP pictures are stripped here")]
    NotSupported,
    /// The container is damaged.
    #[error("the picture is damaged")]
    Damaged,
}

fn note(removed: &mut Vec<Removed>, what: Removed) {
    if !removed.contains(&what) {
        removed.push(what);
    }
}

/// Strips a picture's bytes. The coded picture is never touched.
pub fn strip_bytes(bytes: Vec<u8>) -> Result<Stripped, StripError> {
    let kind = ImageKind::sniff(&bytes).ok_or(StripError::NotSupported)?;
    let mut stripped = match kind {
        ImageKind::Jpeg => strip_jpeg(bytes)?,
        ImageKind::Png => strip_png(bytes)?,
        ImageKind::Webp => strip_webp(bytes)?,
    };
    stripped.removed.sort();
    Ok(stripped)
}

fn strip_jpeg(bytes: Vec<u8>) -> Result<Stripped, StripError> {
    let mut jpeg = Jpeg::from_bytes(Bytes::from(bytes)).map_err(|_| StripError::Damaged)?;
    let orientation = jpeg
        .exif()
        .and_then(|block| exif::parse(&block))
        .and_then(|facts| facts.orientation);
    let mut removed = Vec::new();
    jpeg.segments_mut().retain(|segment| {
        let body = segment.contents();
        let keep = match segment.marker() {
            markers::APP0 => body.starts_with(b"JFIF\0") || body.starts_with(b"JFXX\0"),
            markers::APP2 => body.starts_with(b"ICC_PROFILE\0"),
            markers::APP14 => body.starts_with(b"Adobe"),
            markers::APP1 => {
                if body.starts_with(exif::EXIF_PREFIX) {
                    // A block of the orientation alone is written again
                    // below: nothing is taken out.
                    if !exif::only_orientation(&body[exif::EXIF_PREFIX.len()..]) {
                        note(&mut removed, Removed::Exif);
                    }
                } else if body.starts_with(b"http://ns.adobe.com/x") {
                    note(&mut removed, Removed::Xmp);
                } else {
                    note(&mut removed, Removed::OtherData);
                }
                false
            }
            0xE3..=0xED | markers::APP15 => {
                if segment.marker() == 0xED && body.starts_with(b"Photoshop 3.0\0") {
                    note(&mut removed, Removed::Iptc);
                } else {
                    note(&mut removed, Removed::OtherData);
                }
                false
            }
            markers::COM => {
                note(&mut removed, Removed::Comments);
                false
            }
            _ => true,
        };
        if !keep && segment.marker() == markers::APP2 && body.starts_with(b"MPF\0") {
            note(&mut removed, Removed::MorePictures);
        } else if !keep
            && matches!(
                segment.marker(),
                markers::APP0 | markers::APP2 | markers::APP14
            )
        {
            note(&mut removed, Removed::OtherData);
        }
        keep
    });
    // The first scan's data holds the rest of the file: cut it after the
    // end of the picture, and drop metadata between a progressive
    // picture's scans.
    if let Some(at) = jpeg.segments().iter().position(JpegSegment::has_entropy) {
        let segment = jpeg.segments()[at].clone();
        let header_len = 4 + segment.contents().len();
        let mut encoded = Vec::with_capacity(segment.len_with_entropy());
        segment
            .clone()
            .encoder()
            .write_to(&mut encoded)
            .map_err(|_| StripError::Damaged)?;
        let data = encoded.get(header_len..).ok_or(StripError::Damaged)?;
        let (kept, dropped_meta, trailing) = walk_scans(data);
        if dropped_meta {
            note(&mut removed, Removed::OtherData);
        }
        if trailing > 0 {
            note(&mut removed, Removed::MorePictures);
        }
        jpeg.segments_mut()[at] = JpegSegment::new_with_entropy(
            segment.marker(),
            segment.contents().clone(),
            Bytes::from(kept),
        );
    }
    let orientation = orientation.filter(|o| *o != 1);
    if let Some(o) = orientation {
        jpeg.set_exif(Some(Bytes::from(exif::orientation_only(o))));
    }
    let mut out = Vec::new();
    jpeg.encoder()
        .write_to(&mut out)
        .map_err(|_| StripError::Damaged)?;
    Ok(Stripped {
        kind: ImageKind::Jpeg,
        bytes: out,
        removed,
        orientation,
    })
}

/// Walks the data after the first scan's header to the end of the
/// picture: the coded data and, in a progressive picture, the tables and
/// scans between. Gives the bytes kept (ending with the end marker),
/// whether metadata between scans was dropped, and how many bytes after
/// the end were dropped.
fn walk_scans(data: &[u8]) -> (Vec<u8>, bool, usize) {
    let mut out = Vec::with_capacity(data.len());
    let mut dropped = false;
    let mut i = 0;
    loop {
        let start = i;
        while i + 1 < data.len() {
            if data[i] == 0xFF {
                let marker = data[i + 1];
                if marker == 0x00 || (0xD0..=0xD7).contains(&marker) {
                    i += 2;
                    continue;
                }
                if marker == 0xFF {
                    i += 1;
                    continue;
                }
                break;
            }
            i += 1;
        }
        out.extend_from_slice(&data[start..i]);
        if i + 1 >= data.len() {
            out.extend_from_slice(&data[i..]);
            return (out, dropped, 0);
        }
        let marker = data[i + 1];
        if marker == 0xD9 {
            out.extend_from_slice(&[0xFF, 0xD9]);
            return (out, dropped, data.len() - (i + 2));
        }
        if marker == 0xD8 || marker == 0x01 {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        if i + 3 >= data.len() {
            out.extend_from_slice(&data[i..]);
            return (out, dropped, 0);
        }
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        let end = (i + 2 + len.max(2)).min(data.len());
        if (0xE0..=0xEF).contains(&marker) || marker == 0xFE {
            dropped = true;
        } else {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
}

/// The PNG chunks a picture needs to look right; everything else goes.
const PNG_KEEP: [&[u8; 4]; 18] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT",
    b"bKGD", b"pHYs", b"cICP", b"mDCV", b"cLLI", b"acTL", b"fcTL", b"fdAT",
];

fn strip_png(bytes: Vec<u8>) -> Result<Stripped, StripError> {
    let mut png = Png::from_bytes(Bytes::from(bytes)).map_err(|_| StripError::Damaged)?;
    let orientation = png
        .exif()
        .and_then(|block| exif::parse(&block))
        .and_then(|facts| facts.orientation);
    let mut removed = Vec::new();
    png.chunks_mut().retain(|chunk| {
        let kind = chunk.kind();
        if PNG_KEEP.iter().any(|keep| **keep == kind) {
            return true;
        }
        let what = match &kind {
            // A block of the orientation alone is written again below.
            b"eXIf" if exif::only_orientation(chunk.contents()) => return false,
            b"eXIf" => Removed::Exif,
            b"tEXt" | b"zTXt" | b"iTXt" => {
                if chunk.contents().starts_with(b"XML:com.adobe.xmp\0") {
                    Removed::Xmp
                } else {
                    Removed::TextNotes
                }
            }
            b"tIME" => Removed::Time,
            _ => Removed::OtherData,
        };
        note(&mut removed, what);
        false
    });
    let orientation = orientation.filter(|o| *o != 1);
    if let Some(o) = orientation {
        png.set_exif(Some(Bytes::from(exif::orientation_only(o))));
    }
    let mut out = Vec::new();
    png.encoder()
        .write_to(&mut out)
        .map_err(|_| StripError::Damaged)?;
    Ok(Stripped {
        kind: ImageKind::Png,
        bytes: out,
        removed,
        orientation,
    })
}

/// The VP8X flag of XMP.
const WEBP_XMP_FLAG: u8 = 0x04;

fn strip_webp(bytes: Vec<u8>) -> Result<Stripped, StripError> {
    let mut webp = WebP::from_bytes(Bytes::from(bytes)).map_err(|_| StripError::Damaged)?;
    let orientation = webp
        .exif()
        .and_then(|block| exif::parse(&block))
        .and_then(|facts| facts.orientation)
        .filter(|o| *o != 1);
    let mut removed = Vec::new();
    if webp
        .exif()
        .is_some_and(|block| !exif::only_orientation(&block))
    {
        note(&mut removed, Removed::Exif);
    }
    if webp.has_chunk(*b"XMP ") {
        note(&mut removed, Removed::Xmp);
    }
    webp.remove_chunks_by_id(*b"XMP ");
    webp.set_exif(orientation.map(|o| Bytes::from(exif::orientation_only(o))));
    // img-parts keeps the ICC and Exif flags in step, not the XMP flag.
    if let Some(chunk) = webp.chunks_mut().iter_mut().find(|c| c.id() == *b"VP8X")
        && let RiffContent::Data(data) = chunk.content_mut()
        && let Some(&first) = data.first()
    {
        let mut fixed = data.to_vec();
        fixed[0] = first & !WEBP_XMP_FLAG;
        *data = Bytes::from(fixed);
    }
    let mut out = Vec::new();
    webp.encoder()
        .write_to(&mut out)
        .map_err(|_| StripError::Damaged)?;
    Ok(Stripped {
        kind: ImageKind::Webp,
        bytes: out,
        removed,
        orientation,
    })
}

/// What a copy holds after the strip, read again from its bytes: the
/// words of "Left in the copy: ..." and anything that should not be
/// there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftInCopy {
    /// What is left that should be: "orientation", "color profile",
    /// "image size".
    pub left: Vec<&'static str>,
    /// What is left that should not be: "Exif data", "XMP data".
    pub still: Vec<&'static str>,
}

impl LeftInCopy {
    /// "Left in the copy: orientation, color profile, image size".
    pub fn line(&self) -> String {
        format!("Left in the copy: {}", self.left.join(", "))
    }
}

/// Reads a stripped copy again.
pub fn left_in_copy(bytes: &[u8]) -> Result<LeftInCopy, MetaError> {
    let parts = read_parts(&mut std::io::Cursor::new(bytes))?;
    let mut left = Vec::new();
    let mut still = Vec::new();
    match parts.exif.as_deref() {
        Some(block) if exif::only_orientation(block) => left.push("orientation"),
        Some(_) => still.push("Exif data"),
        None => {}
    }
    if parts.icc {
        left.push("color profile");
    }
    left.push("image size");
    if parts.xmp {
        still.push("XMP data");
    }
    if parts.iptc {
        still.push("IPTC data");
    }
    if !parts.comments.is_empty() {
        still.push("comments");
    }
    if parts.more_pictures {
        still.push("pictures riding along");
    }
    Ok(LeftInCopy { left, still })
}

/// The copy's name beside the original: `photo.jpg` gives
/// `photo-clean.jpg`. A taken name is numbered by the caller with
/// `diroxide_vfs::names::unused_name`, as every new name is.
pub fn clean_copy_name(name: &str) -> String {
    let (stem, extension) = split_extension(name);
    format!("{stem}-clean{extension}")
}

/// The question before a strip, its answer starting on No: "Strip 3
/// photos into copies?", or "Strip photo.jpg into a copy?".
pub fn strip_question(names: &[&str]) -> String {
    match names {
        [one] => format!("Strip {} into a copy?", display_name(one)),
        many => format!("Strip {} photos into copies?", many.len()),
    }
}

/// What a strip into a copy did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripReport {
    /// What was taken out.
    pub removed: Vec<Removed>,
    /// What the copy holds, read again.
    pub left: LeftInCopy,
    /// The copy's size in bytes.
    pub bytes: u64,
}

impl StripReport {
    /// "Removed: Exif data, XMP data, comments", or "Removed: nothing,
    /// the picture held no details".
    pub fn removed_line(&self) -> String {
        if self.removed.is_empty() {
            "Removed: nothing, the picture held no details".to_owned()
        } else {
            format!(
                "Removed: {}",
                self.removed
                    .iter()
                    .map(|r| r.words())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    /// The lines after a strip: the copy's name first, what was
    /// removed, what is left.
    pub fn lines(&self, copy_name: &str) -> Vec<String> {
        let mut lines = vec![
            format!("Stripped into {}", display_name(copy_name)),
            self.removed_line(),
            self.left.line(),
        ];
        if !self.left.still.is_empty() {
            lines.push(format!("Still in the copy: {}", self.left.still.join(", ")));
        }
        lines
    }
}

/// Strips the picture at `source` into a new file at `target`, which must
/// not exist: read through its provider (at most 256 MiB), stripped under
/// the guard (a thread, a catch, thirty seconds), written through the
/// committing writer, and read again for what is left. The original is
/// never changed; the job's undo is "trash the new file", which the
/// job's undo record names.
pub fn strip_into_copy(
    provider: &dyn Provider,
    source: &Place,
    target: &Place,
    ctx: &OpContext,
) -> Result<StripReport, MetaError> {
    ctx.check()?;
    let mut bytes = Vec::new();
    provider
        .read(source, ctx)?
        .take(MAX_STRIP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| MetaError::Place(e.into()))?;
    if bytes.len() as u64 > MAX_STRIP_BYTES {
        return Err(MetaError::TooLarge);
    }
    let stripped = Guard::new(1).run(std::time::Duration::from_secs(30), move || {
        strip_bytes(bytes).map_err(|e| MetaError::unreadable(e.to_string()))
    })?;
    ctx.check()?;
    let left = left_in_copy(&stripped.bytes)?;
    let mut writer = provider.write(target, WriteMode::CreateNew, ctx)?;
    writer
        .write_all(&stripped.bytes)
        .map_err(|e| MetaError::Place(e.into()))?;
    writer.commit(true)?;
    // Read the copy back from the disk, as R28 asks, not from memory.
    let mut back = Vec::new();
    provider
        .read(target, ctx)?
        .take(MAX_STRIP_BYTES + 1)
        .read_to_end(&mut back)
        .map_err(|e| MetaError::Place(e.into()))?;
    let left_on_disk = if back == stripped.bytes {
        left
    } else {
        left_in_copy(&back)?
    };
    Ok(StripReport {
        removed: stripped.removed,
        left: left_on_disk,
        bytes: back.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use diroxide_vfs::LocalProvider;

    use super::*;
    use crate::image::tests::{jpeg, png};

    fn private_jpeg() -> Vec<u8> {
        let mut exif = exif::EXIF_PREFIX.to_vec();
        exif.extend_from_slice(&exif::tests::private_block(true));
        let mut bytes = jpeg(&[
            (0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0".to_vec()),
            (0xE1, exif),
            (0xE1, b"http://ns.adobe.com/xap/1.0/\0<x/>".to_vec()),
            (0xE2, b"ICC_PROFILE\0\x01\x01profile".to_vec()),
            (0xE2, b"MPF\0MM\0*".to_vec()),
            (0xED, b"Photoshop 3.0\08BIM\x04\x04\0\0\0\0\0\0".to_vec()),
            (0xEE, b"Adobe\0\x64\0\0\0\0\x01".to_vec()),
            (0xFE, b"a JPEG comment".to_vec()),
        ]);
        // A second picture after the end, as phones add one.
        bytes.extend_from_slice(&jpeg(&[(0xFE, b"second".to_vec())]));
        bytes
    }

    /// The coded data of a JPEG: from the scan header to the end marker.
    fn scan(bytes: &[u8]) -> &[u8] {
        let mut i = 2;
        while i + 4 <= bytes.len() {
            if bytes[i] == 0xFF && bytes[i + 1] == 0xDA {
                let end = bytes[i..]
                    .windows(2)
                    .position(|w| w == [0xFF, 0xD9])
                    .unwrap();
                return &bytes[i..i + end + 2];
            }
            i += 2 + usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        }
        &[]
    }

    #[test]
    fn a_jpeg_keeps_its_picture_profile_and_orientation_and_nothing_else() {
        let original = private_jpeg();
        let stripped = strip_bytes(original.clone()).unwrap();
        assert_eq!(stripped.orientation, Some(6));
        assert_eq!(
            stripped.removed,
            [
                Removed::Exif,
                Removed::Xmp,
                Removed::Iptc,
                Removed::Comments,
                Removed::MorePictures
            ]
        );
        assert_eq!(scan(&stripped.bytes), scan(&original));
        assert!(stripped.bytes.ends_with(&[0xFF, 0xD9]));
        let left = left_in_copy(&stripped.bytes).unwrap();
        assert_eq!(
            left.line(),
            "Left in the copy: orientation, color profile, image size"
        );
        assert!(left.still.is_empty());
        let parts = read_parts(&mut Cursor::new(stripped.bytes.clone())).unwrap();
        let facts = exif::parse(parts.exif.as_deref().unwrap()).unwrap();
        assert_eq!(facts.orientation, Some(6));
        assert_eq!(facts.gps, None);
        assert!(!facts.thumbnail);
        // Adobe's segment stays: it says how a CMYK picture decodes.
        assert!(stripped.bytes.windows(5).any(|w| w == b"Adobe"));
        // Stripping again takes nothing more.
        let again = strip_bytes(stripped.bytes.clone()).unwrap();
        assert_eq!(again.bytes, stripped.bytes);
        assert!(again.removed.is_empty());
    }

    #[test]
    fn a_png_keeps_its_profile_and_loses_its_notes() {
        let original = png(&[
            (b"tEXt", b"Author\0Ada Example".to_vec()),
            (b"tIME", vec![7, 209, 1, 1, 0, 0, 0]),
            (b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x/>".to_vec()),
            (b"eXIf", exif::tests::private_block(false)),
            (b"iCCP", b"icm\0\0zz".to_vec()),
        ]);
        let stripped = strip_bytes(original).unwrap();
        assert_eq!(
            stripped.removed,
            [
                Removed::Exif,
                Removed::Xmp,
                Removed::TextNotes,
                Removed::Time
            ]
        );
        let left = left_in_copy(&stripped.bytes).unwrap();
        assert_eq!(
            left.line(),
            "Left in the copy: orientation, color profile, image size"
        );
        assert!(left.still.is_empty());
    }

    #[test]
    fn the_words_of_the_strip() {
        assert_eq!(clean_copy_name("photo.jpg"), "photo-clean.jpg");
        assert_eq!(clean_copy_name("IMG_2041.JPEG"), "IMG_2041-clean.JPEG");
        assert_eq!(clean_copy_name("noext"), "noext-clean");
        assert_eq!(
            strip_question(&["a.jpg", "b.jpg", "c.png"]),
            "Strip 3 photos into copies?"
        );
        assert_eq!(
            strip_question(&["photo.jpg"]),
            "Strip photo.jpg into a copy?"
        );
        let report = StripReport {
            removed: vec![Removed::Exif, Removed::Comments],
            left: LeftInCopy {
                left: vec!["image size"],
                still: vec![],
            },
            bytes: 10,
        };
        assert_eq!(
            report.lines("photo-clean.jpg"),
            [
                "Stripped into photo-clean.jpg",
                "Removed: Exif data, comments",
                "Left in the copy: image size",
            ]
        );
        for line in report.lines("photo-clean.jpg") {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert!("Strip 3 photos into copies?".chars().count() <= 40);
        assert_eq!(
            strip_bytes(b"GIF89a".to_vec()),
            Err(StripError::NotSupported)
        );
        assert_eq!(
            StripError::NotSupported.to_string(),
            "only JPEG, PNG and WebP pictures are stripped here"
        );
    }

    #[test]
    fn hostile_pictures_are_refused_or_written_never_a_panic() {
        let originals = [private_jpeg(), png(&[(b"tEXt", b"Author\0x".to_vec())])];
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        for original in &originals {
            for round in 0..3000 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let mut bytes = original.clone();
                match round % 3 {
                    0 => bytes.truncate((seed as usize) % bytes.len()),
                    1 => {
                        let at = (seed as usize) % bytes.len();
                        bytes[at] = (seed >> 8) as u8;
                    }
                    _ => {
                        let at = (seed as usize) % bytes.len();
                        bytes.splice(at..at, std::iter::repeat_n(0xFF, 40));
                    }
                }
                if let Ok(stripped) = strip_bytes(bytes)
                    && let Ok(left) = left_in_copy(&stripped.bytes)
                {
                    assert!(left.still.is_empty(), "{:?}", left.still);
                }
            }
        }
    }

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("meta-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("meta-")
            .tempdir_in(&base)
            .unwrap()
    }

    #[test]
    fn a_copy_is_written_beside_and_the_original_never_changes() {
        let dir = scratch();
        let original = private_jpeg();
        let source = dir.path().join("photo.jpg");
        std::fs::write(&source, &original).unwrap();
        let target = dir.path().join(clean_copy_name("photo.jpg"));
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let place = |p: &std::path::Path| Place::parse(&p.display().to_string()).unwrap();
        let report = strip_into_copy(&provider, &place(&source), &place(&target), &ctx).unwrap();
        assert_eq!(
            report.left.line(),
            "Left in the copy: orientation, color profile, image size"
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::metadata(&target).unwrap().len(), report.bytes);
        // The name is taken now: never overwritten.
        assert!(strip_into_copy(&provider, &place(&source), &place(&target), &ctx).is_err());
    }
}
