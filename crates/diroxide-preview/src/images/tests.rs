// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;

use image::ImageEncoder;

use super::*;

/// A little-endian TIFF block for Exif: a description, an orientation,
/// and a position when asked.
pub(crate) fn exif_block(description: Option<&str>, orientation: u16, gps: bool) -> Vec<u8> {
    let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();
    if let Some(d) = description {
        let mut v = d.as_bytes().to_vec();
        v.push(0);
        entries.push((0x010E, 2, v.len() as u32, v));
    }
    entries.push((0x0112, 3, 1, orientation.to_le_bytes().to_vec()));
    if gps {
        entries.push((0x8825, 4, 1, vec![0; 4]));
    }
    let mut data_at = 8 + 2 + entries.len() * 12 + 4;
    let mut out = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    let mut data = Vec::new();
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut gps_slot = None;
    for (tag, kind, count, value) in &entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if *tag == 0x8825 {
            gps_slot = Some(out.len());
            out.extend_from_slice(&[0; 4]);
        } else if value.len() <= 4 {
            let mut v = value.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            out.extend_from_slice(&(data_at as u32).to_le_bytes());
            data.extend_from_slice(value);
            data_at += value.len();
            if data_at % 2 == 1 {
                data.push(0);
                data_at += 1;
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);
    if let Some(slot) = gps_slot {
        let gps_at = out.len() as u32;
        out[slot..slot + 4].copy_from_slice(&gps_at.to_le_bytes());
        let rationals_at = gps_at + 2 + 4 * 12 + 4;
        let mut g = 4u16.to_le_bytes().to_vec();
        for (tag, kind, count, inline) in [
            (1u16, 2u16, 2u32, [b'N', 0, 0, 0]),
            (2, 5, 3, rationals_at.to_le_bytes()),
            (3, 2, 2, [b'W', 0, 0, 0]),
            (4, 5, 3, (rationals_at + 24).to_le_bytes()),
        ] {
            g.extend_from_slice(&tag.to_le_bytes());
            g.extend_from_slice(&kind.to_le_bytes());
            g.extend_from_slice(&count.to_le_bytes());
            g.extend_from_slice(&inline);
        }
        g.extend_from_slice(&0u32.to_le_bytes());
        for (n, d) in [(45u32, 1u32), (30, 1), (0, 1), (122, 1), (40, 1), (48, 1)] {
            g.extend_from_slice(&n.to_le_bytes());
            g.extend_from_slice(&d.to_le_bytes());
        }
        out.extend_from_slice(&g);
    }
    out
}

/// A JPEG's header alone: Exif when given, a baseline frame of this size,
/// and the scan's marker, where the walk stops.
fn jpeg_header(width: u16, height: u16, exif: Option<&[u8]>) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    if let Some(exif) = exif {
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((2 + 6 + exif.len()) as u16).to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(exif);
    }
    out.extend_from_slice(&[0xFF, 0xC0, 0, 17, 8]);
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    out.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
    out
}

fn png_bytes(width: u32, height: u32, color: image::ExtendedColorType) -> Vec<u8> {
    let channels = match color {
        image::ExtendedColorType::L8 => 1,
        image::ExtendedColorType::Rgb8 => 3,
        _ => 4,
    };
    let pixels = vec![0u8; (width * height * channels) as usize];
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&pixels, width, height, color)
        .unwrap();
    out
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc(&[kind.as_slice(), data].concat());
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

/// The PNG chunk checksum, by the table-free rule.
fn crc(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A PNG with chunks put in after its header chunk.
fn png_with(base: &[u8], chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut out = base[..33].to_vec();
    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&base[33..]);
    out
}

fn facts(bytes: Vec<u8>) -> Result<ImageFacts, ImageError> {
    read_header(
        &mut Cursor::new(bytes),
        TypeHint::default(),
        HeaderLimits::default(),
    )
}

fn sentence(bytes: Vec<u8>, size: u64) -> String {
    facts(bytes)
        .unwrap()
        .sentence(Some(size), &PixelGuard::DEFAULT)
}

fn rows(facts: &ImageFacts) -> Vec<String> {
    facts
        .fields(&PixelGuard::DEFAULT)
        .iter()
        .map(Field::line)
        .collect()
}

fn fits(line: &str) {
    assert!(line.chars().count() <= 40, "{line:?} is over 40 cells");
}

#[test]
fn a_png_says_its_size_color_and_bytes() {
    let line = sentence(
        png_bytes(1920, 1080, image::ExtendedColorType::Rgba8),
        2_300_000,
    );
    assert_eq!(line, "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB");
    assert_eq!(line.chars().count(), 37);
    let f = facts(png_bytes(1920, 1080, image::ExtendedColorType::Rgba8)).unwrap();
    assert_eq!(
        rows(&f),
        [
            "Dimensions: 1920 by 1080",
            "Shape: landscape, about 16 to 9",
            "Color: full color, 8 bits, transparency"
        ]
    );
    let gray = facts(png_bytes(64, 64, image::ExtendedColorType::L8)).unwrap();
    assert_eq!(
        gray.sentence(None, &PixelGuard::DEFAULT),
        "PNG, 64 by 64, 8-bit gray"
    );
    assert_eq!(rows(&gray)[1], "Shape: square");
}

#[test]
fn a_huge_jpeg_is_too_large_to_open_from_its_header_alone() {
    let line = sentence(jpeg_header(20_000, 20_000, None), 9_000_000);
    assert_eq!(line, "JPEG, 20000 by 20000, too large to open");
    assert_eq!(line.chars().count(), 39);
    let f = facts(jpeg_header(20_000, 20_000, None)).unwrap();
    assert!(rows(&f).contains(&"Picture: too large to open".to_owned()));
    let guard = PixelGuard::DEFAULT;
    assert!(guard.allows(10_000, 10_000, 1));
    assert!(!guard.allows(32_769, 10, 1), "a side past 32,768");
    assert!(!guard.allows(12_000, 9_000, 1), "past 100 megapixels");
    assert!(!guard.allows(9_000, 9_000, 4), "past 256 MiB of buffer");
    let limits = guard.limits();
    assert_eq!(limits.max_image_width, Some(32_768));
    assert_eq!(limits.max_alloc, Some(256 * 1024 * 1024));
}

#[test]
fn a_turned_photo_says_its_shown_size_and_its_orientation() {
    let exif = exif_block(None, 6, false);
    let f = facts(jpeg_header(4032, 3024, Some(&exif))).unwrap();
    assert_eq!(f.shown_size(), (3024, 4032));
    let line = f.sentence(Some(3_100_000), &PixelGuard::DEFAULT);
    assert_eq!(line, "JPEG, 3024 by 4032, portrait, 3.1 MB");
    let said = rows(&f);
    assert!(said.contains(&"Dimensions: 3024 by 4032, as shown".to_owned()));
    assert!(said.contains(&"Stored size: 4032 by 3024".to_owned()));
    assert!(said.contains(&"Orientation: turned right".to_owned()));
    assert!(said.contains(&"Shape: portrait, about 3 to 4".to_owned()));
}

#[test]
fn a_location_and_a_description_are_read_and_the_place_is_left_to_the_photo_reader() {
    let exif = exif_block(Some("Ada Example\n at the\u{202E} harbor"), 1, true);
    let f = facts(jpeg_header(4032, 3024, Some(&exif))).unwrap();
    assert_eq!(f.description.as_deref(), Some("Ada Example at the? harbor"));
    let line = f.sentence(Some(3_100_000), &PixelGuard::DEFAULT);
    assert_eq!(line, "JPEG, 4032 by 3024, has location, 3.1 MB");
    assert_eq!(line.chars().count(), 40);
    assert_eq!(f.location.as_deref(), Some("45.50 north, 122.68 west"));
    let fields = f.fields(&PixelGuard::DEFAULT);
    assert_eq!(fields[0].line(), "Description: Ada Example at the? harbor");
    assert!(
        fields
            .iter()
            .all(|f| !f.key.contains("gps") && !f.key.contains("location")),
        "the place is the photo reader's field"
    );
}

#[test]
fn the_eight_orientations_say_what_the_image_crate_does() {
    // Spike P2, kept: a 3 by 2 picture whose stored top-left pixel is red
    // and top-right is green, turned by the image crate for each value.
    let mut stored = image::RgbImage::new(3, 2);
    stored.put_pixel(0, 0, image::Rgb([255, 0, 0]));
    stored.put_pixel(2, 0, image::Rgb([0, 255, 0]));
    // Where red and green end up, for each value, said by the words.
    let expected = [
        (1, "Upright, as stored", (0, 0), (2, 0)),
        (2, "Mirrored left to right", (2, 0), (0, 0)),
        (3, "Rotated a half turn", (2, 1), (0, 1)),
        (4, "Mirrored top to bottom", (0, 1), (2, 1)),
        (5, "Quarter turn clockwise, then mirrored", (0, 0), (0, 2)),
        (6, "Rotated a quarter turn clockwise", (1, 0), (1, 2)),
        (7, "Quarter turn counterclockwise, mirrored", (1, 2), (1, 0)),
        (8, "Rotated a quarter turn counterclockwise", (0, 2), (0, 0)),
    ];
    for (value, words, red, green) in expected {
        assert_eq!(orientation_words(value), words);
        fits(words);
        let mut shown = image::DynamicImage::ImageRgb8(stored.clone());
        shown.apply_orientation(image::metadata::Orientation::from_exif(value).unwrap());
        let shown = shown.to_rgb8();
        let at = |c: [u8; 3]| {
            shown
                .enumerate_pixels()
                .find(|(_, _, p)| p.0 == c)
                .map(|(x, y, _)| (x, y))
                .unwrap()
        };
        assert_eq!((at([255, 0, 0]), at([0, 255, 0])), (red, green), "{value}");
        let mut f = ImageFacts::new("JPEG");
        f.width = 3;
        f.height = 2;
        f.orientation = value;
        assert_eq!(f.shown_size(), shown.dimensions(), "{value}");
        fits(&format!("Orientation: {}", orientation_row(value)));
    }
}

#[test]
fn png_text_chunks_give_the_description_and_exif_the_orientation() {
    let base = png_bytes(40, 30, image::ExtendedColorType::Rgb8);
    let bytes = png_with(
        &base,
        &[
            png_chunk(b"tEXt", b"Comment\0a comment"),
            png_chunk(b"tEXt", b"Title\0Sales by quarter"),
            png_chunk(b"eXIf", &exif_block(None, 8, false)),
        ],
    );
    let f = facts(bytes).unwrap();
    assert_eq!(f.description.as_deref(), Some("Sales by quarter"));
    assert_eq!(f.orientation, 8);
    assert_eq!(f.shown_size(), (30, 40));
}

#[test]
fn an_animated_gif_counts_its_frames_without_decoding_them() {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for n in 0..3u8 {
            let frame = image::RgbaImage::from_pixel(48, 27, image::Rgba([n * 40, 0, 0, 255]));
            encoder.encode_frame(image::Frame::new(frame)).unwrap();
        }
    }
    let size = bytes.len() as u64;
    let line = sentence(bytes, size);
    assert!(line.starts_with("GIF, 3 frames, 48 by 27, "), "{line}");
    fits(&line);
}

#[test]
fn webp_bmp_tiff_and_icons_say_their_sizes() {
    let rgb = vec![0u8; 8 * 6 * 3];
    let mut webp = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
        .encode(&rgb, 8, 6, image::ExtendedColorType::Rgb8)
        .unwrap();
    assert_eq!(
        facts(webp).unwrap().sentence(None, &PixelGuard::DEFAULT),
        "WebP, 8 by 6, 8-bit RGB"
    );
    let mut bmp = Vec::new();
    image::codecs::bmp::BmpEncoder::new(&mut bmp)
        .encode(&rgb, 8, 6, image::ExtendedColorType::Rgb8)
        .unwrap();
    assert_eq!(
        facts(bmp).unwrap().sentence(None, &PixelGuard::DEFAULT),
        "BMP, 8 by 6, 8-bit RGB"
    );
    let mut tiff = Cursor::new(Vec::new());
    image::codecs::tiff::TiffEncoder::new(&mut tiff)
        .write_image(&rgb, 8, 6, image::ExtendedColorType::Rgb8)
        .unwrap();
    assert_eq!(
        facts(tiff.into_inner())
            .unwrap()
            .sentence(None, &PixelGuard::DEFAULT),
        "TIFF, 8 by 6, 8-bit RGB"
    );
    let small = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 255]));
    let large = image::RgbaImage::from_pixel(32, 32, image::Rgba([0, 0, 0, 255]));
    let frames = [
        image::codecs::ico::IcoFrame::as_png(
            small.as_raw(),
            16,
            16,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap(),
        image::codecs::ico::IcoFrame::as_png(
            large.as_raw(),
            32,
            32,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap(),
    ];
    let mut ico = Vec::new();
    image::codecs::ico::IcoEncoder::new(&mut ico)
        .encode_images(&frames)
        .unwrap();
    let f = facts(ico).unwrap();
    assert_eq!(
        f.sentence(Some(4_000), &PixelGuard::DEFAULT),
        "ICO, 2 sizes, largest 32 by 32"
    );
    assert!(rows(&f).contains(&"Sizes: 16, 32".to_owned()));
}

#[test]
fn a_heif_file_is_sized_by_its_extents_and_needs_a_component() {
    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }
    let mut ispe = vec![0u8; 4];
    ispe.extend_from_slice(&4032u32.to_be_bytes());
    ispe.extend_from_slice(&3024u32.to_be_bytes());
    let mut thumb = vec![0u8; 4];
    thumb.extend_from_slice(&320u32.to_be_bytes());
    thumb.extend_from_slice(&240u32.to_be_bytes());
    let ipco = boxed(
        b"ipco",
        &[boxed(b"ispe", &thumb), boxed(b"ispe", &ispe)].concat(),
    );
    let iprp = boxed(b"iprp", &ipco);
    let meta = boxed(
        b"meta",
        &[vec![0u8; 4], boxed(b"hdlr", &[0u8; 24]), iprp].concat(),
    );
    let bytes = [boxed(b"ftyp", b"heic\0\0\0\0mif1heic"), meta].concat();
    let f = read_header(
        &mut Cursor::new(bytes),
        TypeHint {
            id: Some("heic"),
            short: Some("HEIC"),
        },
        HeaderLimits::default(),
    )
    .unwrap();
    let line = f.sentence(Some(3_000_000), &PixelGuard::DEFAULT);
    assert_eq!(line, "HEIC, 4032 by 3024, needs a component");
    assert_eq!(line.chars().count(), 37);
    assert!(rows(&f).contains(&"Picture: not shown, needs ImageMagick".to_owned()));
}

#[test]
fn damaged_and_endless_headers_fail_in_words_and_stop() {
    let base = png_bytes(4, 4, image::ExtendedColorType::Rgb8);
    let cut = facts(base[..20].to_vec()).unwrap_err();
    assert_eq!(cut, ImageError::CutShort);
    let line = unreadable_sentence("PNG", &cut);
    assert_eq!(line, "PNG, unreadable: file is cut short");
    fits(&line);
    assert_eq!(facts(b"hello, world".to_vec()), Err(ImageError::NotAnImage));
    // A JPEG of nothing but comment segments never reaches its frame.
    let mut endless = vec![0xFF, 0xD8];
    for _ in 0..200_000 {
        endless.extend_from_slice(&[0xFF, 0xFE, 0, 2]);
    }
    assert_eq!(facts(endless), Err(ImageError::HeaderTooLong));
    // A GIF whose blocks go past the limits is still read, its frames
    // then said as animated.
    let mut gif = b"GIF89a\x10\x00\x10\x00\x00\x00\x00".to_vec();
    for _ in 0..3 {
        gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 16, 0, 16, 0, 0, 2, 0]);
    }
    let limits = HeaderLimits {
        max_bytes: 64 * 1024,
        max_parts: 4,
    };
    let f = read_header(&mut Cursor::new(gif), TypeHint::default(), limits).unwrap();
    assert_eq!(f.frames, Frames::Animated);
}

#[test]
fn the_guard_refuses_a_large_picture_before_decoding_and_decodes_a_small_one() {
    let guard = PixelGuard::DEFAULT;
    let doc_guard = Guard::default();
    let small = png_bytes(4, 3, image::ExtendedColorType::Rgb8);
    let picture = guard.decode(small, &doc_guard).unwrap();
    assert_eq!((picture.width(), picture.height()), (4, 3));
    assert_eq!(
        guard
            .decode(jpeg_header(20_000, 20_000, None), &doc_guard)
            .unwrap_err(),
        ImageError::TooLarge
    );
    let tight = PixelGuard {
        max_side: 2,
        ..PixelGuard::DEFAULT
    };
    assert_eq!(
        tight
            .decode(png_bytes(4, 3, image::ExtendedColorType::Rgb8), &doc_guard)
            .unwrap_err(),
        ImageError::TooLarge
    );
}

#[test]
fn shapes_and_colors_in_words() {
    assert_eq!(shape_words((1920, 1080)), "landscape, about 16 to 9");
    assert_eq!(shape_words((3024, 4032)), "portrait, about 3 to 4");
    assert_eq!(shape_words((1000, 1001)), "square");
    assert_eq!(shape_words((1000, 777)), "landscape");
    let rgba = Color {
        model: ColorModel::Rgb,
        bits: 8,
        alpha: true,
    };
    assert_eq!(rgba.short(), "8-bit RGBA");
    assert_eq!(rgba.words(), "full color, 8 bits, transparency");
    assert_eq!(rgba.bytes_per_pixel(), 4);
    let gray16 = Color {
        model: ColorModel::Gray,
        bits: 16,
        alpha: false,
    };
    assert_eq!(gray16.words(), "gray, 16 bits");
    assert_eq!(gray16.bytes_per_pixel(), 2);
    for line in [
        rgba.words(),
        "Color: full color, 16 bits, transparency".to_owned(),
        "Color: CMYK, 16 bits, transparency".to_owned(),
        "Color: palette, 8 bits, transparency".to_owned(),
        "Animation: animated, frames not counted".to_owned(),
        "Stored size: 32768 by 32768".to_owned(),
        "Shape: landscape, about 16 to 9".to_owned(),
        "Dimensions: 32768 by 32768, as shown".to_owned(),
        "Privacy: location, camera serial".to_owned(),
        "Picture: not shown, needs ImageMagick".to_owned(),
    ] {
        fits(&line);
    }
}

#[test]
fn the_meta_reader_gives_the_same_rows_as_fields() {
    use diroxide_core::{Entry, Place};
    use diroxide_meta::Reader;
    let reader = ImageFields::default();
    assert_eq!((reader.id(), reader.cost()), ("image", Cost::Background));
    let place = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let entry = Entry::new("photo.jpg", EntryKind::File);
    let request = ReadRequest {
        place: &place,
        entry: &entry,
        type_id: Some("jpeg"),
        group: Some("images"),
        cancel: None,
        tools: None,
    };
    assert!(reader.accepts(&request));
    let svg = ReadRequest {
        type_id: Some("svg"),
        ..request
    };
    assert!(!reader.accepts(&svg), "a drawing is read as a drawing");
    let exif = exif_block(None, 6, false);
    let mut source = Cursor::new(jpeg_header(4032, 3024, Some(&exif)));
    let fields = reader
        .read(&mut source, &request, &ReadLimits::default())
        .unwrap();
    assert_eq!(fields[0].line(), "Dimensions: 3024 by 4032, as shown");
    let mut text = Cursor::new(b"not a picture".to_vec());
    assert_eq!(
        reader.read(&mut text, &request, &ReadLimits::default()),
        Err(MetaError::Unreadable("not a picture".to_owned()))
    );
}

/// A PNG whose first chunk is noise (the demo set's `broken.png`: the
/// signature, then noise) fails in plain words, never in the png crate's
/// debug text with the chunk's raw bytes inside braces.
#[test]
fn a_png_of_noise_after_its_signature_fails_in_plain_words() {
    let mut noisy = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    noisy.extend_from_slice(&[0, 0, 0, 13]);
    noisy.extend_from_slice(&[0xF2, 0xFE, 0x8A, 0x01]);
    noisy.extend_from_slice(&[0; 17]);
    let Err(ImageError::Damaged(words)) = facts(noisy) else {
        panic!("noise after the signature must be damaged");
    };
    assert!(
        words.chars().all(|c| c.is_ascii_graphic() || c == ' ') && !words.contains('{'),
        "{words}"
    );
    let line = unreadable_sentence("PNG", &ImageError::Damaged(words));
    assert_eq!(line, "PNG, unreadable: the header is damaged");
    fits(&line);
    assert_eq!(
        header::plain_or("file is cut short".to_owned(), "x"),
        "file is cut short"
    );
    assert_eq!(
        header::plain_or(
            "ChunkType { type: \u{f2}\u{fe} }".to_owned(),
            "the header is damaged"
        ),
        "the header is damaged"
    );
}

/// A source that counts the bytes it gives.
struct Counting {
    inner: Cursor<Vec<u8>>,
    read: u64,
}

impl std::io::Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl std::io::Seek for Counting {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// The nightly fuzz run's `preview_image_header` crash (first found
/// Tuesday, November 10, 2026; its input was too long for the log): the
/// limit held the position, not the bytes read, so seeking back and
/// forth read the same bytes again, each seek refilling the buffer.
#[test]
fn seeking_back_and_forth_spends_the_read_budget() {
    let limits = HeaderLimits {
        max_bytes: 150_000,
        max_parts: 100,
    };
    let mut source = Counting {
        inner: Cursor::new(vec![0u8; 200_000]),
        read: 0,
    };
    let mut refused = false;
    {
        let mut src = header::Src::new(&mut source, limits);
        for _ in 0..1_000 {
            let back = src.seek_to(0).and_then(|()| src.head(1));
            let forth = src.seek_to(100_000).and_then(|()| src.head(1));
            match (back, forth) {
                (Ok(_), Ok(_)) => {}
                (Err(ImageError::HeaderTooLong), _) | (_, Err(ImageError::HeaderTooLong)) => {
                    refused = true;
                    break;
                }
                other => panic!("only the budget may stop it: {other:?}"),
            }
        }
    }
    assert!(refused, "the budget must run out");
    assert!(
        source.read <= limits.max_bytes + header::READ_AHEAD,
        "{}",
        source.read
    );
}

/// A JPEG 2000 file of many small boxes, none the header: a seek inside
/// the buffer keeps it, so each byte is read once.
#[test]
fn walking_small_boxes_reads_each_byte_once() {
    for (boxes, size) in [(100usize, 8u32), (4_000, 8), (5_000, 8), (60, 20_000)] {
        let mut bytes = vec![0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0d, 0x0a, 0x87, 0x0a];
        for _ in 0..boxes {
            bytes.extend_from_slice(&size.to_be_bytes());
            bytes.extend_from_slice(b"free");
            bytes.resize(bytes.len() + size as usize - 8, 0);
        }
        crate::fuzzing::image_header(&bytes);
        let mut source = Counting {
            inner: Cursor::new(bytes.clone()),
            read: 0,
        };
        let _ = read_header(&mut source, TypeHint::default(), HeaderLimits::default());
        assert!(
            source.read <= bytes.len() as u64 + header::READ_AHEAD,
            "{boxes} boxes of {size}: {} of {}",
            source.read,
            bytes.len()
        );
    }
}

/// A HEIF box inside `meta` whose 64-bit size carries the walk to the
/// end of a u64: the walk ends, the sum never overflows.
#[test]
fn a_box_claiming_the_end_of_a_u64_ends_the_walk() {
    let mut bytes = vec![0, 0, 0, 16];
    bytes.extend_from_slice(b"ftypheic");
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&100u32.to_be_bytes());
    bytes.extend_from_slice(b"meta");
    bytes.extend_from_slice(&[0; 4]);
    for size in [u64::MAX, u64::MAX - 8, u64::MAX - 40] {
        let mut file = bytes.clone();
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"iprp");
        file.extend_from_slice(&size.to_be_bytes());
        file.resize(116, 0);
        crate::fuzzing::image_header(&file);
        let mut file = bytes.clone();
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"junk");
        file.extend_from_slice(&size.to_be_bytes());
        file.resize(116, 0);
        crate::fuzzing::image_header(&file);
    }
}
