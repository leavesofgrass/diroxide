//! Images in words (alpha 4, R30 commit 33; R15's design, R28 part 2.2):
//! what a picture is, read from its header alone through a seekable
//! source, never a pixel decoded. The sentence says the format, the size
//! as shown, then the one fact that matters most, then the file's size:
//! "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB"; "JPEG, 3024 by 4032, has
//! location, 3.1 MB"; "GIF, 24 frames, 480 by 270, 1.2 MB"; "JPEG, 20000
//! by 20000, too large to open". The rows say the author's description,
//! the shape, the color, the animation and the orientation in words; the
//! camera, the place and the serial number are the metadata crate's
//! photo reader's rows, so the details view says each once.
//!
//! **The pixel guard** ([`PixelGuard`]) stands in front of every path
//! that could decode pixels: 100 megapixels to open, 32,768 on a side,
//! 256 MiB of buffer and ten seconds (R15's starting values, the owner's
//! question 2.5 by default). The `image` crate's limits are set from it
//! anyway, and its allocation limit is advisory; a decode runs on the
//! guard's thread, cut off at the deadline ([`PixelGuard::decode`]).
//!
//! The header readers ([`header`]): PNG through the `png` crate up to its
//! first image data (its text chunks, which `image` does not expose);
//! JPEG by walking its markers up to the scan (spike P1: `image`'s JPEG
//! decoder reads the whole file); GIF and WebP by walking their blocks and
//! chunks for the frame count; BMP, ICO, PSD and JPEG 2000 by their
//! headers; TIFF through `image`'s decoder, which reads the directory and
//! not the strips; HEIF and AVIF by their `ispe` property. Exif through
//! `kamadak-exif`. HEIF, AVIF, JPEG XL, JPEG 2000, Photoshop and camera
//! raw files are named and sized where that is cheap, and say they need a
//! component to be shown.

mod exif;
pub mod header;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::format::human_size;
use diroxide_core::{DetailGroup, EntryKind};
use diroxide_filetype::FileType;
use diroxide_meta::{Cost, Field, MetaError, ReadLimits, ReadRequest};
use diroxide_vfs::{OpContext, ReadSeek, Registry, VfsError};
use thiserror::Error;

pub use header::HeaderLimits;

use crate::doc::Guard;
use crate::words::count_words;
use crate::{Preview, PreviewError, PreviewKind, PreviewRequest, Previewer};

/// The most characters kept of a text a picture carries.
pub const MAX_TEXT: usize = 200;

/// The type ids the image words read: the `images` group's raster types.
/// SVG is read as a drawing ([`crate::doc::svg`]), not here.
pub const IMAGE_TYPES: &[&str] = &[
    "png",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "tiff",
    "ico",
    "heic",
    "avif",
    "cr3",
    "jxl",
    "jp2",
    "psd",
    "camera-raw",
    "orf",
    "rw2",
];

/// The types that are named and sized but need a component to be shown.
const NEEDS_COMPONENT: &[&str] = &[
    "heic",
    "avif",
    "cr3",
    "jxl",
    "jp2",
    "psd",
    "camera-raw",
    "orf",
    "rw2",
];

/// The component a picture needs before it can be shown.
pub const SHOWN_BY: &str = "ImageMagick";

/// Why a header could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ImageError {
    /// The bytes are not a picture this reader knows.
    #[error("not a picture")]
    NotAnImage,
    /// The file ends inside its header.
    #[error("file is cut short")]
    CutShort,
    /// The header breaks its format's rules, with how.
    #[error("{0}")]
    Damaged(String),
    /// The header goes on past the read's limits.
    #[error("its header is too long to read here")]
    HeaderTooLong,
    /// The picture is past the pixel guard.
    #[error("too many pixels")]
    TooLarge,
    /// The decode ran past the guard's deadline.
    #[error("it took too long to open")]
    TookTooLong,
    /// The source could not be read.
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for ImageError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            ImageError::CutShort
        } else {
            ImageError::Io(error.to_string())
        }
    }
}

/// The pixel guard: the limits in front of every path that decodes
/// pixels. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelGuard {
    /// The most pixels a picture may have to be opened.
    pub max_pixels: u64,
    /// The most pixels on either side.
    pub max_side: u32,
    /// The most bytes the decoded picture may take.
    pub max_buffer: u64,
    /// How long a decode may take.
    pub deadline: Duration,
}

impl Default for PixelGuard {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl PixelGuard {
    /// 100 megapixels, 32,768 a side, 256 MiB, ten seconds.
    pub const DEFAULT: PixelGuard = PixelGuard {
        max_pixels: 100_000_000,
        max_side: 32_768,
        max_buffer: 256 * 1024 * 1024,
        deadline: Duration::from_secs(10),
    };

    /// Whether a picture of this size, decoded at `bytes_per_pixel`, may
    /// be opened.
    pub fn allows(&self, width: u32, height: u32, bytes_per_pixel: u64) -> bool {
        let pixels = u64::from(width) * u64::from(height);
        width <= self.max_side
            && height <= self.max_side
            && pixels <= self.max_pixels
            && pixels.saturating_mul(bytes_per_pixel.max(1)) <= self.max_buffer
    }

    /// The `image` crate's limits from the guard: the sides strictly, the
    /// buffer as its advisory allocation limit.
    pub fn limits(&self) -> image::Limits {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(self.max_side);
        limits.max_image_height = Some(self.max_side);
        limits.max_alloc = Some(self.max_buffer);
        limits
    }

    /// Decodes a picture's bytes, for a view that shows it: the header is
    /// read and held to the guard first, then the decode runs on `guard`'s
    /// thread with the `image` crate's limits set, and is cut off at the
    /// pixel guard's deadline.
    pub fn decode(&self, bytes: Vec<u8>, guard: &Guard) -> Result<image::DynamicImage, ImageError> {
        let mut cursor = Cursor::new(bytes);
        let facts = read_header(&mut cursor, TypeHint::default(), HeaderLimits::default())?;
        if facts.too_large(self) {
            return Err(ImageError::TooLarge);
        }
        let limits = self.limits();
        let bytes = cursor.into_inner();
        guard
            .clone()
            .with_deadline(self.deadline)
            .run(move || {
                let mut reader =
                    image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
                reader.limits(limits);
                reader.decode()
            })
            .map_err(|e| match e {
                crate::doc::DocError::TookTooLong => ImageError::TookTooLong,
                other => ImageError::Damaged(header::plain_or(
                    other.to_string(),
                    "the picture data is damaged",
                )),
            })?
            .map_err(|e| match e {
                image::ImageError::Limits(_) => ImageError::TooLarge,
                other => ImageError::Damaged(header::plain_or(
                    other.to_string(),
                    "the picture data is damaged",
                )),
            })
    }
}

/// How a picture's colors are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorModel {
    /// Shades of gray.
    Gray,
    /// Full color.
    Rgb,
    /// A palette of colors.
    Indexed,
    /// Print colors.
    Cmyk,
}

/// A picture's color: its model, the bits a channel, and whether it has
/// transparency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    /// The model.
    pub model: ColorModel,
    /// Bits a channel (an index for a palette).
    pub bits: u8,
    /// Whether some of it can be transparent.
    pub alpha: bool,
}

impl Color {
    /// The short form the sentence uses: "8-bit RGBA", "8-bit gray",
    /// "16-bit RGB", "8-bit indexed", "8-bit CMYK".
    pub fn short(&self) -> String {
        let model = match (self.model, self.alpha) {
            (ColorModel::Rgb, true) => "RGBA",
            (ColorModel::Rgb, false) => "RGB",
            (ColorModel::Gray, true) => "gray with alpha",
            (ColorModel::Gray, false) => "gray",
            (ColorModel::Indexed, _) => "indexed",
            (ColorModel::Cmyk, _) => "CMYK",
        };
        format!("{}-bit {model}", self.bits)
    }

    /// The row's words: "full color, 8 bits, transparency", "gray, 8
    /// bits", "palette, 8 bits", "CMYK, 8 bits".
    pub fn words(&self) -> String {
        let model = match self.model {
            ColorModel::Rgb => "full color",
            ColorModel::Gray => "gray",
            ColorModel::Indexed => "palette",
            ColorModel::Cmyk => "CMYK",
        };
        let bits = count_words(u64::from(self.bits), "bit", "bits");
        if self.alpha {
            format!("{model}, {bits}, transparency")
        } else {
            format!("{model}, {bits}")
        }
    }

    /// Bytes a pixel takes once decoded.
    pub fn bytes_per_pixel(&self) -> u64 {
        let channels = match self.model {
            ColorModel::Gray => 1,
            ColorModel::Rgb | ColorModel::Indexed | ColorModel::Cmyk => 3,
        } + u64::from(self.alpha);
        let depth = match self.bits {
            0..=8 => 1,
            9..=16 => 2,
            _ => 4,
        };
        channels * depth
    }
}

/// How many pictures a file holds over time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Frames {
    /// A still picture.
    One,
    /// An animation of this many frames.
    Many(u32),
    /// An animation whose frames were not counted.
    Animated,
}

/// What a picture's header says. Built by [`read_header`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageFacts {
    /// The format as said: "PNG", "JPEG", the type's short name when the
    /// type is known.
    pub format: String,
    /// The width as stored; 0 when not known.
    pub width: u32,
    /// The height as stored; 0 when not known.
    pub height: u32,
    /// The Exif orientation, 1 to 8; 1 when the file has none.
    pub orientation: u8,
    /// The color, when the header says it.
    pub color: Option<Color>,
    /// Still or animated.
    pub frames: Frames,
    /// The author's own words: Exif's description, else a PNG's
    /// Description, Title or Comment, cleaned.
    pub description: Option<String>,
    /// The camera's make and model, cleaned.
    pub camera: Option<String>,
    /// The camera's serial number, cleaned: private.
    pub camera_serial: Option<String>,
    /// Where it was taken, in degrees and directions: private.
    pub location: Option<String>,
    /// Whether it carries a color profile.
    pub has_profile: bool,
    /// An icon file's sizes, in its order.
    pub icon_sizes: Vec<(u32, u32)>,
    /// The component it needs to be shown, when this program cannot.
    pub needs: Option<String>,
}

impl ImageFacts {
    fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
            width: 0,
            height: 0,
            orientation: 1,
            color: None,
            frames: Frames::One,
            description: None,
            camera: None,
            camera_serial: None,
            location: None,
            has_profile: false,
            icon_sizes: Vec::new(),
            needs: None,
        }
    }

    /// Whether the orientation turns it a quarter, so the shown size is
    /// the stored one turned.
    pub fn turned(&self) -> bool {
        (5..=8).contains(&self.orientation)
    }

    /// The size as shown, after the orientation.
    pub fn shown_size(&self) -> (u32, u32) {
        if self.turned() {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    /// Whether the size is known.
    pub fn has_size(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Whether the pixel guard refuses to open it.
    pub fn too_large(&self, guard: &PixelGuard) -> bool {
        let bpp = self.color.map_or(4, |c| c.bytes_per_pixel());
        self.has_size() && !guard.allows(self.width, self.height, bpp)
    }

    /// The sentence: the format, the size as shown, the fact that matters
    /// most, then the file's size. See the module documentation.
    pub fn sentence(&self, file_size: Option<u64>, guard: &PixelGuard) -> String {
        let mut parts = vec![self.format.clone()];
        if !self.icon_sizes.is_empty() {
            let (w, h) = (self.width, self.height);
            parts.push(count_words(self.icon_sizes.len() as u64, "size", "sizes"));
            parts.push(format!("largest {w} by {h}"));
            return parts.join(", ");
        }
        match self.frames {
            Frames::Many(n) => parts.push(count_words(u64::from(n), "frame", "frames")),
            Frames::Animated => parts.push("animated".to_owned()),
            Frames::One => {}
        }
        if self.has_size() {
            let (w, h) = self.shown_size();
            parts.push(format!("{w} by {h}"));
        }
        if self.too_large(guard) {
            parts.push("too large to open".to_owned());
            return parts.join(", ");
        }
        if self.needs.is_some() {
            parts.push("needs a component".to_owned());
            return parts.join(", ");
        }
        if self.location.is_some() {
            parts.push("has location".to_owned());
        } else if self.turned() && self.has_size() {
            parts.push(shape_word(self.shown_size()).to_owned());
        } else if let (Frames::One, Some(color)) = (self.frames, self.color) {
            parts.push(color.short());
        }
        if let Some(size) = file_size {
            parts.push(human_size(size));
        }
        parts.join(", ")
    }

    /// The picture's own facts as metadata fields, in the order of facts:
    /// the description, then what is in it, then the technical facts. The
    /// camera, the place and the serial number are the metadata crate's
    /// photo reader's fields (`image.camera`, `image.gps`,
    /// `image.serial`), read from the same Exif, so the details view
    /// lists each once; the facts keep them for the sentence's "has
    /// location" and for the formats that reader does not take.
    pub fn fields(&self, guard: &PixelGuard) -> Vec<Field> {
        let mut out = Vec::new();
        if let Some(text) = &self.description {
            out.push(Field::new(
                "image.description",
                "Description",
                text,
                DetailGroup::Identity,
            ));
        }
        if self.has_size() {
            let (w, h) = self.shown_size();
            let value = if self.turned() {
                format!("{w} by {h}, as shown")
            } else {
                format!("{w} by {h}")
            };
            out.push(Field::new(
                "image.dimensions",
                "Dimensions",
                value,
                DetailGroup::Content,
            ));
            out.push(Field::new(
                "image.shape",
                "Shape",
                shape_words((w, h)),
                DetailGroup::Content,
            ));
        }
        if let Some(color) = self.color {
            out.push(Field::new(
                "image.color",
                "Color",
                color.words(),
                DetailGroup::Content,
            ));
        }
        match self.frames {
            Frames::Many(n) => out.push(Field::new(
                "image.animation",
                "Animation",
                count_words(u64::from(n), "frame", "frames"),
                DetailGroup::Content,
            )),
            Frames::Animated => out.push(Field::new(
                "image.animation",
                "Animation",
                "animated, frames not counted",
                DetailGroup::Content,
            )),
            Frames::One => {}
        }
        if !self.icon_sizes.is_empty() {
            let sides: Vec<String> = self
                .icon_sizes
                .iter()
                .map(|(w, h)| {
                    if w == h {
                        w.to_string()
                    } else {
                        format!("{w} by {h}")
                    }
                })
                .collect();
            out.push(Field::new(
                "image.sizes",
                "Sizes",
                crate::words::clean_text(&sides.join(", "), MAX_TEXT),
                DetailGroup::Content,
            ));
        }
        if self.turned() && self.has_size() {
            out.push(Field::new(
                "image.stored_size",
                "Stored size",
                format!("{} by {}", self.width, self.height),
                DetailGroup::Technical,
            ));
        }
        if self.orientation != 1 {
            out.push(Field::new(
                "image.orientation",
                "Orientation",
                orientation_row(self.orientation),
                DetailGroup::Technical,
            ));
        }
        if self.has_profile {
            out.push(Field::new(
                "image.profile",
                "Color profile",
                "embedded",
                DetailGroup::Technical,
            ));
        }
        if self.too_large(guard) {
            out.push(Field::new(
                "image.shown",
                "Picture",
                "too large to open",
                DetailGroup::Technical,
            ));
        } else if let Some(tool) = &self.needs {
            out.push(Field::new(
                "image.shown",
                "Picture",
                format!("not shown, needs {tool}"),
                DetailGroup::Technical,
            ));
        }
        out
    }
}

/// The eight Exif orientations in words, as what is done to the stored
/// picture to show it (spike P2, against pictures whose orientation was
/// written by hand and turned by the `image` crate). Each fits 40 cells.
pub fn orientation_words(orientation: u8) -> &'static str {
    match orientation {
        2 => "Mirrored left to right",
        3 => "Rotated a half turn",
        4 => "Mirrored top to bottom",
        5 => "Quarter turn clockwise, then mirrored",
        6 => "Rotated a quarter turn clockwise",
        7 => "Quarter turn counterclockwise, mirrored",
        8 => "Rotated a quarter turn counterclockwise",
        _ => "Upright, as stored",
    }
}

/// The orientation's row value, short enough that "Orientation:" and
/// it fit 40 cells: "turned right", "turned left, mirrored", "turned
/// upside down", "mirrored left to right". A quarter turn is the only
/// turn Exif writes, so "right" is a quarter turn clockwise.
pub fn orientation_row(orientation: u8) -> &'static str {
    match orientation {
        2 => "mirrored left to right",
        3 => "turned upside down",
        4 => "mirrored top to bottom",
        5 => "turned right, mirrored",
        6 => "turned right",
        7 => "turned left, mirrored",
        8 => "turned left",
        _ => "upright",
    }
}

/// "portrait", "landscape" or "square", for a size as shown.
pub fn shape_word((w, h): (u32, u32)) -> &'static str {
    if w == h {
        "square"
    } else if w > h {
        "landscape"
    } else {
        "portrait"
    }
}

/// The shape row's words: "landscape, about 16 to 9", "portrait, about 3
/// to 4", "square"; the shape alone when no common ratio is near.
pub fn shape_words((w, h): (u32, u32)) -> String {
    const RATIOS: [(u32, u32); 9] = [
        (5, 4),
        (4, 3),
        (3, 2),
        (16, 10),
        (16, 9),
        (2, 1),
        (21, 9),
        (3, 1),
        (1, 1),
    ];
    if w == 0 || h == 0 {
        return String::new();
    }
    let (long, short) = (w.max(h), w.min(h));
    let ratio = f64::from(long) / f64::from(short);
    let near = RATIOS.iter().copied().find(|&(a, b)| {
        let r = f64::from(a) / f64::from(b);
        (ratio - r).abs() / r < 0.02
    });
    match (shape_word((w, h)), near) {
        ("square", _) | (_, Some((1, 1))) => "square".to_owned(),
        (word, Some((a, b))) if w > h => format!("{word}, about {a} to {b}"),
        (word, Some((a, b))) => format!("{word}, about {b} to {a}"),
        (word, None) => word.to_owned(),
    }
}

/// Which type a file was checked to be, for the format's word and for
/// the types that need a component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypeHint<'a> {
    /// The type's id: `heic`.
    pub id: Option<&'a str>,
    /// The type's short name: "HEIC".
    pub short: Option<&'a str>,
}

impl<'a> TypeHint<'a> {
    /// The hint a file type gives.
    pub fn of(file_type: Option<&'a FileType>) -> Self {
        Self {
            id: file_type.map(|t| t.id.as_str()),
            short: file_type.map(|t| t.short.as_str()),
        }
    }
}

/// Reads a picture's header from `source`: what it is, its size, color,
/// frames, orientation and the facts its Exif and text carry. Never reads
/// more than `limits` allow and never decodes a pixel.
pub fn read_header(
    source: &mut dyn ReadSeek,
    hint: TypeHint<'_>,
    limits: HeaderLimits,
) -> Result<ImageFacts, ImageError> {
    let mut src = header::Src::new(source, limits);
    src.seek_to(0)?;
    let head = src.head(32)?;
    let sniffed = header::sniff(&head).ok_or(ImageError::NotAnImage)?;
    let raw = hint
        .id
        .is_some_and(|id| matches!(id, "camera-raw" | "orf" | "rw2" | "cr3"));
    let word = match (hint.short, hint.id) {
        (_, Some("camera-raw")) => "Camera raw".to_owned(),
        (Some(short), Some(id)) if IMAGE_TYPES.contains(&id) => short.to_owned(),
        _ => sniffed.word().to_owned(),
    };
    let mut facts = ImageFacts::new(word);
    use header::Sniffed as S;
    match sniffed {
        S::Png => header::png(&mut src, &mut facts)?,
        S::Jpeg => header::jpeg(&mut src, &mut facts)?,
        S::Gif => header::gif(&mut src, &mut facts)?,
        S::WebP => header::webp(&mut src, &mut facts)?,
        S::Bmp => header::bmp(&mut src, &mut facts)?,
        S::Ico => header::ico(&mut src, &mut facts)?,
        S::Tiff if raw => raw_exif(&mut src, &mut facts)?,
        S::Tiff => header::tiff(&mut src, &mut facts)?,
        S::Heif => header::heif(&mut src, &mut facts)?,
        S::Jp2 => header::jp2(&mut src, &mut facts)?,
        S::Psd => header::psd(&mut src, &mut facts)?,
        S::Jxl => {}
    }
    let needs = raw
        || matches!(sniffed, S::Heif | S::Jp2 | S::Jxl | S::Psd)
        || hint.id.is_some_and(|id| NEEDS_COMPONENT.contains(&id));
    if needs {
        facts.needs = Some(SHOWN_BY.to_owned());
    }
    Ok(facts)
}

/// A camera raw file: its first directory is often a small preview, so
/// its size comes from Exif, read from the file's first megabyte.
fn raw_exif(src: &mut header::Src<'_>, facts: &mut ImageFacts) -> Result<(), ImageError> {
    src.seek_to(0)?;
    let head = src.head(1 << 20)?;
    let mut reader = ::exif::Reader::new();
    reader.continue_on_error(true);
    let parsed = match reader.read_raw(head) {
        Ok(exif) => Some(exif),
        Err(::exif::Error::PartialResult(partial)) => Some(partial.into_inner().0),
        Err(_) => None,
    };
    if let Some(exif) = parsed {
        exif::apply_parsed(&exif, facts);
    }
    Ok(())
}

/// What the preview says when the header could not be read: "PNG,
/// unreadable: file is cut short".
pub fn unreadable_sentence(format: &str, error: &ImageError) -> String {
    format!("{format}, unreadable: {error}")
}

/// Whether an entry is a picture the image words read, by its type, or
/// by its name's extension when its type is not known.
pub fn takes(file_type: Option<&FileType>, name: &str) -> bool {
    match file_type {
        Some(t) => {
            IMAGE_TYPES.contains(&t.id.as_str())
                || t.parent
                    .as_deref()
                    .is_some_and(|p| IMAGE_TYPES.contains(&p))
                    && t.group.as_deref() == Some("images")
        }
        None => {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            ext.is_some_and(|e| {
                [
                    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "bmp", "dib", "tif",
                    "tiff", "ico", "heic", "heif", "avif", "jxl", "jp2", "psd",
                ]
                .contains(&e.as_str())
            })
        }
    }
}

/// The image words as a previewer: a details preview whose summary is the
/// sentence and whose lines are the rows.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImagePreviewer {
    /// The guard the sentence holds sizes to.
    pub guard: PixelGuard,
    /// What a header read may cost.
    pub limits: HeaderLimits,
}

impl Previewer for ImagePreviewer {
    fn name(&self) -> &str {
        "image"
    }

    fn version(&self) -> u32 {
        1
    }

    fn accepts(&self, request: &PreviewRequest, file_type: Option<&FileType>) -> bool {
        request.entry.kind == EntryKind::File && takes(file_type, &request.entry.name)
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let mut source = crate::open_source(request, registry, ctx, self.limits.max_bytes)?;
        ctx.check()?;
        let hint = TypeHint::of(file_type);
        let base = crate::detail_rows(&request.entry, file_type);
        match read_header(source.as_mut(), hint, self.limits) {
            Ok(facts) => {
                let fields = facts.fields(&self.guard);
                Ok(crate::details_preview(
                    PreviewKind::Image,
                    facts.sentence(request.entry.size, &self.guard),
                    base,
                    fields.iter().map(Field::row),
                ))
            }
            Err(ImageError::Io(message)) => Err(PreviewError::Vfs(VfsError::Io {
                kind: std::io::ErrorKind::Other,
                message,
            })),
            Err(error) => {
                let word = hint.short.unwrap_or("Picture");
                Ok(crate::details_preview(
                    PreviewKind::Image,
                    unreadable_sentence(word, &error),
                    base,
                    std::iter::empty(),
                ))
            }
        }
    }
}

/// The keys of the fields the image words give, for the column chooser
/// and a failed read's cells (VL's request 2, built in lane VA).
pub const FIELD_KEYS: [&str; 10] = [
    "image.description",
    "image.dimensions",
    "image.shape",
    "image.color",
    "image.animation",
    "image.sizes",
    "image.stored_size",
    "image.orientation",
    "image.profile",
    "image.shown",
];

/// The image words as a metadata reader (R30 commit 12's joint): the
/// same rows as fields, in the background class, since each file costs
/// one small read.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImageFields {
    /// The guard the rows hold sizes to.
    pub guard: PixelGuard,
}

impl ImageFields {
    /// The reader, shared, for a pipeline.
    pub fn shared() -> Arc<dyn diroxide_meta::Reader> {
        Arc::new(Self::default())
    }
}

impl diroxide_meta::Reader for ImageFields {
    fn id(&self) -> &'static str {
        "image"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &FIELD_KEYS
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        match request.type_id {
            Some(id) => IMAGE_TYPES.contains(&id),
            None => takes(None, &request.entry.name),
        }
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        request: &ReadRequest<'_>,
        limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let hint = TypeHint {
            id: request.type_id,
            short: None,
        };
        let header_limits = HeaderLimits {
            max_bytes: limits.max_bytes,
            ..HeaderLimits::default()
        };
        match read_header(source, hint, header_limits) {
            Ok(facts) => Ok(facts.fields(&self.guard)),
            Err(ImageError::HeaderTooLong) => Err(MetaError::TooLarge),
            Err(error) => Err(MetaError::Unreadable(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests;
