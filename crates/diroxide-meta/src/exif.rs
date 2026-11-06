//! A small, careful Exif reader: the TIFF structure inside a photo's
//! Exif block, read for the facts Check before sharing names and the
//! strip keeps (R12, "Image metadata"), and the one block the strip
//! writes, holding the orientation alone.
//!
//! It reads IFD0, the Exif and GPS folders it points to, and IFD1 (the
//! stored thumbnail); every offset is checked against the block, every
//! folder is read once (a loop is cut), a folder holds at most 512
//! entries, and a value at most 64 KiB. It never panics on any input
//! (the `meta_file_bytes` fuzz target runs it).

use std::collections::HashSet;

/// The prefix of an Exif block inside a JPEG segment.
pub const EXIF_PREFIX: &[u8] = b"Exif\0\0";

/// The most entries read from one folder.
const MOST_ENTRIES: usize = 512;

/// The longest value read.
const MOST_VALUE: usize = 64 * 1024;

/// What an Exif block says that matters for privacy and the strip.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ExifFacts {
    /// The orientation, 1 to 8.
    pub orientation: Option<u16>,
    /// The camera's maker.
    pub make: Option<String>,
    /// The camera's model.
    pub model: Option<String>,
    /// The program that wrote it.
    pub software: Option<String>,
    /// The author (Artist, or Windows' XPAuthor).
    pub artist: Option<String>,
    /// The copyright line.
    pub copyright: Option<String>,
    /// The description (ImageDescription, or Windows' XPTitle).
    pub description: Option<String>,
    /// A comment (UserComment, or Windows' XPComment).
    pub comment: Option<String>,
    /// When it was taken (DateTimeOriginal, else DateTime), as written.
    pub taken: Option<String>,
    /// The camera owner's name.
    pub owner: Option<String>,
    /// The camera's serial number.
    pub body_serial: Option<String>,
    /// The lens.
    pub lens_model: Option<String>,
    /// The lens's serial number.
    pub lens_serial: Option<String>,
    /// The computer that wrote it.
    pub host_computer: Option<String>,
    /// Latitude and longitude, degrees, south and west negative.
    pub gps: Option<(f64, f64)>,
    /// Whether a GPS folder is there at all (it can hold a time or a
    /// height without a place).
    pub gps_folder: bool,
    /// Whether a thumbnail is stored (IFD1).
    pub thumbnail: bool,
    /// Whether a maker's note is stored (opaque, often with ids).
    pub maker_note: bool,
}

/// A TIFF block's bytes and byte order.
struct Tiff<'a> {
    bytes: &'a [u8],
    little: bool,
}

/// One folder entry.
#[derive(Clone, Copy)]
struct EntryRef {
    tag: u16,
    kind: u16,
    count: u32,
    /// Where the value or its offset sits.
    at: usize,
}

impl<'a> Tiff<'a> {
    fn new(bytes: &'a [u8]) -> Option<Self> {
        let bytes = bytes.strip_prefix(EXIF_PREFIX).unwrap_or(bytes);
        let little = match bytes.get(..4)? {
            [b'I', b'I', 42, 0] => true,
            [b'M', b'M', 0, 42] => false,
            _ => return None,
        };
        Some(Self { bytes, little })
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        let b = self.bytes.get(at..at.checked_add(2)?)?;
        Some(if self.little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        let b = self.bytes.get(at..at.checked_add(4)?)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if self.little {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    }

    /// A folder's entries and the offset of the next folder.
    fn folder(&self, offset: usize) -> Option<(Vec<EntryRef>, usize)> {
        let count = usize::from(self.u16_at(offset)?);
        if count > MOST_ENTRIES {
            return None;
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let at = offset + 2 + i * 12;
            entries.push(EntryRef {
                tag: self.u16_at(at)?,
                kind: self.u16_at(at + 2)?,
                count: self.u32_at(at + 4)?,
                at: at + 8,
            });
        }
        let next = self.u32_at(offset + 2 + count * 12).unwrap_or(0) as usize;
        Some((entries, next))
    }

    /// An entry's value bytes, when they lie inside the block.
    fn value(&self, entry: &EntryRef) -> Option<&'a [u8]> {
        let unit: usize = match entry.kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => return None,
        };
        let len = unit.checked_mul(entry.count as usize)?;
        if len > MOST_VALUE {
            return None;
        }
        let start = if len <= 4 {
            entry.at
        } else {
            self.u32_at(entry.at)? as usize
        };
        self.bytes.get(start..start.checked_add(len)?)
    }

    fn short(&self, entry: &EntryRef) -> Option<u16> {
        match entry.kind {
            3 => self.u16_at(entry.at),
            4 => self.u32_at(entry.at).and_then(|v| u16::try_from(v).ok()),
            _ => None,
        }
    }

    fn long(&self, entry: &EntryRef) -> Option<u32> {
        match entry.kind {
            3 => self.u16_at(entry.at).map(u32::from),
            4 => self.u32_at(entry.at),
            _ => None,
        }
    }

    fn text(&self, entry: &EntryRef) -> Option<String> {
        let bytes = self.value(entry)?;
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// Windows' XP tags: UCS-2, little-endian, NUL-ended.
    fn ucs2(&self, entry: &EntryRef) -> Option<String> {
        let bytes = self.value(entry)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .take_while(|u| *u != 0)
            .collect();
        let text = String::from_utf16_lossy(&units).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    /// UserComment: an 8-byte character code, then the text.
    fn user_comment(&self, entry: &EntryRef) -> Option<String> {
        let bytes = self.value(entry)?;
        let (code, rest) = (bytes.get(..8)?, bytes.get(8..)?);
        let text = if code.starts_with(b"UNICODE") {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|p| {
                    if self.little {
                        u16::from_le_bytes([p[0], p[1]])
                    } else {
                        u16::from_be_bytes([p[0], p[1]])
                    }
                })
                .take_while(|u| *u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(rest).into_owned()
        };
        let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// Three rationals as degrees.
    fn degrees(&self, entry: &EntryRef) -> Option<f64> {
        if entry.kind != 5 || entry.count != 3 {
            return None;
        }
        let bytes = self.value(entry)?;
        let mut parts = [0f64; 3];
        for (i, part) in parts.iter_mut().enumerate() {
            let at = i * 8;
            let get = |o: usize| -> Option<u32> {
                let b = bytes.get(at + o..at + o + 4)?;
                let a = [b[0], b[1], b[2], b[3]];
                Some(if self.little {
                    u32::from_le_bytes(a)
                } else {
                    u32::from_be_bytes(a)
                })
            };
            let (num, den) = (get(0)?, get(4)?);
            if den == 0 {
                return None;
            }
            *part = f64::from(num) / f64::from(den);
        }
        Some(parts[0] + parts[1] / 60.0 + parts[2] / 3600.0)
    }
}

/// Reads an Exif block (with or without the `Exif\0\0` prefix).
/// `None` when it is not a TIFF structure.
pub fn parse(block: &[u8]) -> Option<ExifFacts> {
    let tiff = Tiff::new(block)?;
    let mut facts = ExifFacts::default();
    let mut seen = HashSet::new();
    let ifd0 = tiff.u32_at(4)? as usize;
    let mut exif_ifd = None;
    let mut gps_ifd = None;
    let mut ifd1 = 0;
    if seen.insert(ifd0)
        && let Some((entries, next)) = tiff.folder(ifd0)
    {
        ifd1 = next;
        for e in &entries {
            match e.tag {
                0x0112 => facts.orientation = tiff.short(e).filter(|o| (1..=8).contains(o)),
                0x010F => facts.make = tiff.text(e),
                0x0110 => facts.model = tiff.text(e),
                0x0131 => facts.software = tiff.text(e),
                0x013B => facts.artist = facts.artist.take().or(tiff.text(e)),
                0x8298 => facts.copyright = tiff.text(e),
                0x010E => facts.description = facts.description.take().or(tiff.text(e)),
                0x0132 => facts.taken = facts.taken.take().or(tiff.text(e)),
                0x013C => facts.host_computer = tiff.text(e),
                0x9C9B => facts.description = facts.description.take().or(tiff.ucs2(e)),
                0x9C9C => facts.comment = facts.comment.take().or(tiff.ucs2(e)),
                0x9C9D => facts.artist = facts.artist.take().or(tiff.ucs2(e)),
                0x8769 => exif_ifd = tiff.long(e).map(|o| o as usize),
                0x8825 => gps_ifd = tiff.long(e).map(|o| o as usize),
                _ => {}
            }
        }
    }
    if let Some(offset) = exif_ifd
        && seen.insert(offset)
        && let Some((entries, _)) = tiff.folder(offset)
    {
        for e in &entries {
            match e.tag {
                0x9003 => {
                    if let Some(t) = tiff.text(e) {
                        facts.taken = Some(t);
                    }
                }
                0x9286 => facts.comment = tiff.user_comment(e).or(facts.comment.take()),
                0x927C => facts.maker_note = true,
                0xA430 => facts.owner = tiff.text(e),
                0xA431 => facts.body_serial = tiff.text(e),
                0xA434 => facts.lens_model = tiff.text(e),
                0xA435 => facts.lens_serial = tiff.text(e),
                _ => {}
            }
        }
    }
    if let Some(offset) = gps_ifd
        && seen.insert(offset)
        && let Some((entries, _)) = tiff.folder(offset)
    {
        facts.gps_folder = !entries.is_empty();
        let find = |tag: u16| entries.iter().find(|e| e.tag == tag);
        let reference = |tag: u16| find(tag).and_then(|e| tiff.text(e));
        let lat = find(2).and_then(|e| tiff.degrees(e));
        let lon = find(4).and_then(|e| tiff.degrees(e));
        if let (Some(lat), Some(lon)) = (lat, lon) {
            let lat = if reference(1).is_some_and(|r| r.starts_with(['S', 's'])) {
                -lat
            } else {
                lat
            };
            let lon = if reference(3).is_some_and(|r| r.starts_with(['W', 'w'])) {
                -lon
            } else {
                lon
            };
            if lat.abs() <= 90.0 && lon.abs() <= 180.0 {
                facts.gps = Some((lat, lon));
            }
        }
    }
    if ifd1 != 0
        && seen.insert(ifd1)
        && let Some((entries, _)) = tiff.folder(ifd1)
    {
        facts.thumbnail = entries
            .iter()
            .any(|e| matches!(e.tag, 0x0201 | 0x0111 | 0x0202));
    }
    Some(facts)
}

/// The Exif block the strip writes: a little-endian TIFF structure whose
/// one folder holds the orientation alone (26 bytes, no prefix; the
/// container's writer adds what its format wants).
pub fn orientation_only(orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::with_capacity(26);
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff
}

/// Whether an Exif block holds nothing but the orientation.
pub fn only_orientation(block: &[u8]) -> bool {
    let Some(tiff) = Tiff::new(block) else {
        return false;
    };
    let Some(ifd0) = tiff.u32_at(4) else {
        return false;
    };
    match tiff.folder(ifd0 as usize) {
        Some((entries, next)) => next == 0 && entries.iter().all(|e| e.tag == 0x0112),
        None => false,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a TIFF block from folders of `(tag, kind, count, value)`;
    /// IFD0 is first, then the Exif, GPS and thumbnail folders when given.
    pub(crate) struct Builder {
        little: bool,
    }

    /// One entry: tag, kind, count and its value's bytes.
    pub(crate) type Raw = (u16, u16, u32, Vec<u8>);

    impl Builder {
        pub(crate) fn new(little: bool) -> Self {
            Self { little }
        }

        fn u16(&self, v: u16) -> [u8; 2] {
            if self.little {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            }
        }

        fn u32(&self, v: u32) -> [u8; 4] {
            if self.little {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            }
        }

        pub(crate) fn ascii(text: &str) -> (u16, u32, Vec<u8>) {
            let mut v = text.as_bytes().to_vec();
            v.push(0);
            (2, v.len() as u32, v)
        }

        pub(crate) fn short(&self, v: u16) -> (u16, u32, Vec<u8>) {
            (3, 1, self.u16(v).to_vec())
        }

        pub(crate) fn rationals(&self, parts: &[(u32, u32)]) -> (u16, u32, Vec<u8>) {
            let mut v = Vec::new();
            for (n, d) in parts {
                v.extend_from_slice(&self.u32(*n));
                v.extend_from_slice(&self.u32(*d));
            }
            (5, parts.len() as u32, v)
        }

        /// The block: IFD0's entries, then optional Exif, GPS and IFD1
        /// folders, pointers filled in.
        pub(crate) fn build(
            &self,
            ifd0: Vec<Raw>,
            exif: Option<Vec<Raw>>,
            gps: Option<Vec<Raw>>,
            ifd1: Option<Vec<Raw>>,
        ) -> Vec<u8> {
            // Lay the folders out one after another, values after each.
            let mut folders: Vec<Vec<Raw>> = vec![ifd0];
            let exif_index = exif.map(|f| {
                folders.push(f);
                folders.len() - 1
            });
            let gps_index = gps.map(|f| {
                folders.push(f);
                folders.len() - 1
            });
            let ifd1_index = ifd1.map(|f| {
                folders.push(f);
                folders.len() - 1
            });
            // Pointers in IFD0 (placeholders, fixed below).
            if exif_index.is_some() {
                folders[0].push((0x8769, 4, 1, vec![0; 4]));
            }
            if gps_index.is_some() {
                folders[0].push((0x8825, 4, 1, vec![0; 4]));
            }
            let size = |f: &Vec<Raw>| {
                2 + f.len() * 12
                    + 4
                    + f.iter()
                        .map(|(_, _, _, v)| if v.len() > 4 { v.len() } else { 0 })
                        .sum::<usize>()
            };
            let mut offsets = Vec::new();
            let mut at = 8;
            for f in &folders {
                offsets.push(at);
                at += size(f);
            }
            if let Some(i) = exif_index {
                let n = folders[0].len() - 1 - usize::from(gps_index.is_some());
                folders[0][n].3 = self.u32(offsets[i] as u32).to_vec();
            }
            if let Some(i) = gps_index {
                let n = folders[0].len() - 1;
                folders[0][n].3 = self.u32(offsets[i] as u32).to_vec();
            }
            let mut out = Vec::new();
            out.extend_from_slice(if self.little { b"II" } else { b"MM" });
            out.extend_from_slice(&self.u16(42));
            out.extend_from_slice(&self.u32(8));
            for (index, folder) in folders.iter().enumerate() {
                let start = offsets[index];
                let mut values_at = start + 2 + folder.len() * 12 + 4;
                let mut values = Vec::new();
                out.extend_from_slice(&self.u16(folder.len() as u16));
                for (tag, kind, count, value) in folder {
                    out.extend_from_slice(&self.u16(*tag));
                    out.extend_from_slice(&self.u16(*kind));
                    out.extend_from_slice(&self.u32(*count));
                    if value.len() > 4 {
                        out.extend_from_slice(&self.u32(values_at as u32));
                        values.extend_from_slice(value);
                        values_at += value.len();
                    } else {
                        let mut v = value.clone();
                        v.resize(4, 0);
                        out.extend_from_slice(&v);
                    }
                }
                let next = if index == 0 {
                    ifd1_index.map_or(0, |i| offsets[i])
                } else {
                    0
                };
                out.extend_from_slice(&self.u32(next as u32));
                out.extend_from_slice(&values);
            }
            out
        }
    }

    /// The private sample of the tests: a portrait photo's block with a
    /// place, a camera serial, an author, a comment and a thumbnail.
    pub(crate) fn private_block(little: bool) -> Vec<u8> {
        let b = Builder::new(little);
        let raw = |tag: u16, (kind, count, value): (u16, u32, Vec<u8>)| (tag, kind, count, value);
        let mut comment = b"ASCII\0\0\0".to_vec();
        comment.extend_from_slice(b"a private note");
        b.build(
            vec![
                raw(0x010F, Builder::ascii("SpikeCam")),
                raw(0x0110, Builder::ascii("Spike 1")),
                raw(0x0112, b.short(6)),
                raw(0x013B, Builder::ascii("Ada Example")),
            ],
            Some(vec![
                raw(0x9003, Builder::ascii("2001:01:01 08:30:00")),
                (0x9286, 7, comment.len() as u32, comment),
                raw(0xA431, Builder::ascii("SN12345")),
            ]),
            Some(vec![
                raw(1, Builder::ascii("N")),
                raw(2, b.rationals(&[(45, 1), (29, 1), (5496, 100)])),
                raw(3, Builder::ascii("W")),
                raw(4, b.rationals(&[(122, 1), (41, 1), (720, 100)])),
            ]),
            Some(vec![
                (0x0201, 4, 1, vec![0, 0, 0, 0]),
                (0x0202, 4, 1, vec![0, 0, 0, 0]),
            ]),
        )
    }

    #[test]
    fn the_private_facts_are_read_in_either_byte_order() {
        for little in [true, false] {
            let facts = parse(&private_block(little)).unwrap();
            assert_eq!(facts.orientation, Some(6));
            assert_eq!(facts.make.as_deref(), Some("SpikeCam"));
            assert_eq!(facts.model.as_deref(), Some("Spike 1"));
            assert_eq!(facts.artist.as_deref(), Some("Ada Example"));
            assert_eq!(facts.taken.as_deref(), Some("2001:01:01 08:30:00"));
            assert_eq!(facts.comment.as_deref(), Some("a private note"));
            assert_eq!(facts.body_serial.as_deref(), Some("SN12345"));
            let (lat, lon) = facts.gps.unwrap();
            assert!((lat - 45.4986).abs() < 1e-3, "{lat}");
            assert!((lon + 122.6853).abs() < 1e-3, "{lon}");
            assert!(facts.thumbnail);
        }
        let mut prefixed = EXIF_PREFIX.to_vec();
        prefixed.extend_from_slice(&private_block(true));
        assert_eq!(parse(&prefixed).unwrap().orientation, Some(6));
    }

    #[test]
    fn the_orientation_block_holds_the_orientation_alone() {
        let block = orientation_only(8);
        assert_eq!(block.len(), 26);
        let facts = parse(&block).unwrap();
        assert_eq!(facts.orientation, Some(8));
        assert_eq!(
            facts,
            ExifFacts {
                orientation: Some(8),
                ..ExifFacts::default()
            }
        );
        assert!(only_orientation(&block));
        assert!(!only_orientation(&private_block(true)));
    }

    #[test]
    fn hostile_blocks_are_refused_or_read_without_a_panic() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"II*\0"), None);
        assert_eq!(parse(b"XX*\0\x08\0\0\0"), None);
        // A folder that points at itself as the next folder and the Exif
        // folder: each is read once.
        let mut looped = b"II*\0\x08\0\0\0".to_vec();
        looped.extend_from_slice(&1u16.to_le_bytes());
        looped.extend_from_slice(&0x8769u16.to_le_bytes());
        looped.extend_from_slice(&4u16.to_le_bytes());
        looped.extend_from_slice(&1u32.to_le_bytes());
        looped.extend_from_slice(&8u32.to_le_bytes());
        looped.extend_from_slice(&8u32.to_le_bytes());
        assert!(parse(&looped).is_some());
        // Every cut of a real block.
        let block = private_block(false);
        for end in 0..block.len() {
            let _ = parse(&block[..end]);
        }
        // Every single byte changed.
        for at in 0..block.len() {
            let mut bent = block.clone();
            bent[at] ^= 0xFF;
            let _ = parse(&bent);
        }
    }
}
