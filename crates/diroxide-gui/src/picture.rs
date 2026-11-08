//! The picture view (alpha 4, R30 commit 112; plan 5.15's one
//! named-graphic view): a picture the viewer or the quick view shows,
//! opened in the window as one graphic named by its file and described
//! by the preview crate's words ("PNG, 1920 by 1080, 8-bit RGBA, 2.3
//! MB"), with zoom and fit by key and no autoplay.
//!
//! - **One pixel guard.** The header is read first through the preview
//!   crate's reader ([`images::read_header`]), and a picture past the
//!   guard ([`PixelGuard::DEFAULT`]: 100 megapixels, 32,768 a side, 256
//!   MiB, ten seconds), or one that needs a component, is said in words
//!   and never decoded. The pixels come only through
//!   [`PixelGuard::decode`], on the guard's own thread with its deadline.
//!   No thumbnail grid, no cache and no worker of the window's own beyond
//!   the one read (the owner's answer 112).
//! - **Words first.** The graphic's name is the file's name and its
//!   description is the sentence the quick view and the details say, so a
//!   screen reader says the same words everywhere; the zoom is said in
//!   words ("Zoom 150 percent", "Fit to the window, 42 percent"), and an
//!   animation shows its first frame still, said as "Animation: 12
//!   frames, not playing".
//! - **The keys are the window's own** until the app has a command for
//!   the view (a contract request): Enter on a picture in the viewer or
//!   in the quick view's preview opens it; in it, Plus and Minus zoom, 0
//!   fits, 1 shows the actual size, the arrows move a zoomed picture, and
//!   Escape closes it. Every other key is the app's. The keys are said in
//!   the view's own help lines.
//!
//! The reading half here needs no window, so the tests read it with
//! none; the frontend runs [`read_picture`] on a thread of its own.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use diroxide_core::format::{BRAILLE_CELLS, count};
use diroxide_preview::doc::Guard;
use diroxide_preview::images::{
    self, Frames, HeaderLimits, ImageError, PixelGuard, TypeHint, unreadable_sentence,
};

/// The help line drawn under the picture, and said once as it opens.
pub const HELP_LINE: &str = "Plus, Minus zoom; 0 fits; Escape closes";

/// The second help line, drawn under the first.
pub const MORE_HELP_LINE: &str = "1 actual size; arrows move when zoomed";

/// The help line drawn under a PDF's page (lane VP).
pub const PAGE_HELP_LINE: &str = "Page Down, Page Up: next, previous page";

/// Said when a helper would need a file on a local disk.
pub const NOT_LOCAL: &str = "Picture: not shown, not on a local disk";

/// How much one Plus or Minus zooms.
pub const ZOOM_STEP: f64 = 1.25;

/// The smallest zoom, as a fraction of the actual size.
pub const ZOOM_MIN: f64 = 0.05;

/// The largest zoom.
pub const ZOOM_MAX: f64 = 16.0;

/// How far one arrow moves a zoomed picture: this part of the view.
pub const PAN_PART: f64 = 0.125;

/// Whether a file is a picture the view opens, by its name: the types
/// the preview crate's image words read ([`images::takes`]).
pub fn is_picture(name: &str) -> bool {
    images::takes(None, name)
}

/// A decoded picture, upright, as straight RGBA rows. Two are the same
/// when they share their bytes, never by comparing them.
#[derive(Clone, Debug)]
pub struct Pixels {
    /// The width in pixels, as shown.
    pub width: u32,
    /// The height in pixels, as shown.
    pub height: u32,
    /// Four bytes a pixel, row after row.
    pub rgba: Arc<Vec<u8>>,
}

impl PartialEq for Pixels {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && Arc::ptr_eq(&self.rgba, &other.rgba)
    }
}

impl Eq for Pixels {}

/// A picture ready to show: its pixels and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// The pixels, upright (the Exif orientation applied).
    pub pixels: Pixels,
    /// The preview crate's sentence: "PNG, 1920 by 1080, 8-bit RGBA, 2.3
    /// MB".
    pub sentence: String,
    /// "Animation: 12 frames, not playing" for an animation, whose first
    /// frame is shown still.
    pub animation: Option<String>,
}

/// What reading a picture came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    /// It can be shown.
    Shown(Picture),
    /// It is not shown, said in words: "JPEG, 20000 by 20000, too large to
    /// open"; "HEIC, 4032 by 3024, needs a component"; "PNG, unreadable:
    /// file is cut short".
    Refused(String),
}

/// The format's word for a picture whose header could not be read: its
/// extension in capitals, "Picture" without one.
fn format_word(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpe" | "jfif" => "JPEG".to_owned(),
            "tif" => "TIFF".to_owned(),
            other => other.to_ascii_uppercase(),
        },
        _ => "Picture".to_owned(),
    }
}

/// "Animation: 12 frames, not playing", for a picture of more than one
/// frame; `None` for a still one.
pub fn animation_line(frames: Frames) -> Option<String> {
    match frames {
        Frames::Many(n) => Some(format!(
            "Animation: {}, not playing",
            count(u64::from(n), "frame", "frames")
        )),
        Frames::Animated => Some("Animation, not playing".to_owned()),
        Frames::One => None,
    }
}

/// Reads the picture `name` from `source`: the header first, held to the
/// guard; then, if it may be opened, its bytes (at most the guard's
/// buffer) decoded through [`PixelGuard::decode`] under `decoder`, and
/// turned upright. Blocks for at most the guard's deadline past the
/// read; the frontend runs it on a thread of its own.
pub fn read_picture<S: Read + Seek + Send>(
    source: &mut S,
    name: &str,
    guard: &PixelGuard,
    decoder: &Guard,
) -> Opened {
    read_picture_with(source, name, guard, decoder, None)
}

/// What draws a picture the decoder cannot read as PNG: ImageMagick
/// through the app's tools (lane VP), or the lines that say why not (its
/// refusal, then its install line).
pub type Converter<'a> = &'a dyn Fn() -> Result<Vec<u8>, Vec<String>>;

/// [`read_picture`], and for a picture that needs a component (HEIC,
/// AVIF, PSD and the rest) its PNG from `convert`, decoded under the same
/// guard (lane VP): "HEIC, 4032 by 3024, through ImageMagick". Without
/// the helper the words say so, a line each in [`Opened::Refused`]:
/// "HEIC, 4032 by 3024, needs ImageMagick" and the install line.
pub fn read_picture_with<S: Read + Seek + Send>(
    source: &mut S,
    name: &str,
    guard: &PixelGuard,
    decoder: &Guard,
    convert: Option<Converter<'_>>,
) -> Opened {
    let size = source.seek(SeekFrom::End(0)).ok();
    if source.seek(SeekFrom::Start(0)).is_err() {
        return Opened::Refused(format!("{}, unreadable", format_word(name)));
    }
    let facts = match images::read_header(source, TypeHint::default(), HeaderLimits::default()) {
        Ok(facts) => facts,
        Err(error) => return Opened::Refused(unreadable_sentence(&format_word(name), &error)),
    };
    let sentence = facts.sentence(size, guard);
    if facts.needs.is_some()
        && !facts.too_large(guard)
        && let Some(convert) = convert
    {
        return converted(&sentence, convert(), guard, decoder);
    }
    if facts.too_large(guard) || facts.needs.is_some() {
        // Never decoded: the sentence already says why.
        return Opened::Refused(sentence);
    }
    if size.is_some_and(|s| s > guard.max_buffer) {
        return Opened::Refused(format!("{}, file too large to open", facts.format));
    }
    if source.seek(SeekFrom::Start(0)).is_err() {
        return Opened::Refused(format!("{}, unreadable", facts.format));
    }
    let mut bytes = Vec::new();
    if let Err(error) = source.take(guard.max_buffer).read_to_end(&mut bytes) {
        return Opened::Refused(unreadable_sentence(&facts.format, &ImageError::from(error)));
    }
    let decoded = match guard.decode(bytes, decoder) {
        Ok(decoded) => decoded,
        Err(ImageError::TooLarge) => {
            return Opened::Refused(format!("{}, too large to open", facts.format));
        }
        Err(error) => return Opened::Refused(unreadable_sentence(&facts.format, &error)),
    };
    // Upright, as the sentence's size says it is shown (Exif 2 to 8).
    let upright = match facts.orientation {
        2 => decoded.fliph(),
        3 => decoded.rotate180(),
        4 => decoded.flipv(),
        5 => decoded.rotate90().fliph(),
        6 => decoded.rotate90(),
        7 => decoded.rotate270().fliph(),
        8 => decoded.rotate270(),
        _ => decoded,
    };
    let rgba = upright.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Opened::Shown(Picture {
        pixels: Pixels {
            width,
            height,
            rgba: Arc::new(rgba.into_raw()),
        },
        sentence,
        animation: animation_line(facts.frames),
    })
}

/// A decoded picture as one to show (a macro, since the decoder's
/// picture type belongs to a crate the window does not name).
macro_rules! shown {
    ($decoded:expr, $sentence:expr, $animation:expr $(,)?) => {{
        let rgba = $decoded.to_rgba8();
        let (width, height) = (rgba.width(), rgba.height());
        Opened::Shown(Picture {
            pixels: Pixels {
                width,
                height,
                rgba: Arc::new(rgba.into_raw()),
            },
            sentence: $sentence,
            animation: $animation,
        })
    }};
}

/// The words a header says when the decoder lacks the format.
const NEEDS_A_COMPONENT: &str = "needs a component";

/// A picture a helper drew as PNG, decoded under the guard, upright as
/// the helper turned it; or the helper's words, the sentence first.
fn converted(
    sentence: &str,
    png: Result<Vec<u8>, Vec<String>>,
    guard: &PixelGuard,
    decoder: &Guard,
) -> Opened {
    let png = match png {
        Ok(png) => png,
        Err(lines) => {
            let missing = lines.first().map(String::as_str)
                == Some(diroxide_app::app::picture_view::NEEDS_MAGICK);
            let mut said = Vec::new();
            if missing {
                said.push(sentence.replace(NEEDS_A_COMPONENT, "needs ImageMagick"));
                said.extend(lines.into_iter().skip(1));
            } else {
                said.push(sentence.to_owned());
                said.extend(lines);
            }
            return Opened::Refused(said.join("\n"));
        }
    };
    if png.len() as u64 > guard.max_buffer {
        return Opened::Refused(format!("{sentence}\nPicture: too large to open"));
    }
    match guard.decode(png, decoder) {
        Ok(decoded) => shown!(
            decoded,
            sentence.replace(NEEDS_A_COMPONENT, "through ImageMagick"),
            None,
        ),
        Err(error) => Opened::Refused(unreadable_sentence(
            sentence.split(',').next().unwrap_or("Picture"),
            &error,
        )),
    }
}

/// A PDF page drawn as PNG through pdftoppm (lane VP), decoded under the
/// guard; its description the page in words ("Page 1 of 3"). Without the
/// helper, its words: "Page pictures need pdftoppm" and the install line.
pub fn read_page(
    png: Result<Vec<u8>, Vec<String>>,
    page_line: &str,
    guard: &PixelGuard,
    decoder: &Guard,
) -> Opened {
    let png = match png {
        Ok(png) => png,
        Err(lines) => return Opened::Refused(lines.join("\n")),
    };
    if png.len() as u64 > guard.max_buffer {
        return Opened::Refused(format!("{page_line}, too large to open"));
    }
    match guard.decode(png, decoder) {
        Ok(decoded) => shown!(decoded, page_line.to_owned(), None),
        Err(error) => Opened::Refused(unreadable_sentence(page_line, &error)),
    }
}

/// How the picture is scaled in the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The whole picture fills the view, as large as fits.
    Fit,
    /// A fraction of the actual size: 1.0 is one picture pixel a logical
    /// pixel.
    Scale(f64),
}

impl Zoom {
    /// The scale in a view of `view` logical pixels for a picture of
    /// `picture` pixels.
    pub fn scale(self, view: (f64, f64), picture: (u32, u32)) -> f64 {
        match self {
            Zoom::Scale(s) => s,
            Zoom::Fit => fit_scale(view, picture),
        }
    }
}

/// The scale at which the whole picture fits `view`.
pub fn fit_scale(view: (f64, f64), picture: (u32, u32)) -> f64 {
    let (w, h) = (f64::from(picture.0.max(1)), f64::from(picture.1.max(1)));
    let s = (view.0 / w).min(view.1 / h);
    if s.is_finite() && s > 0.0 { s } else { 1.0 }
}

/// A scale as a whole percent, at least 1.
pub fn percent(scale: f64) -> u64 {
    (scale * 100.0).round().max(1.0) as u64
}

/// The zoom in words: "Fit to the window, 42 percent", "Actual size, 100
/// percent", "Zoom 150 percent".
pub fn zoom_words(zoom: Zoom, view: (f64, f64), picture: (u32, u32)) -> String {
    let p = percent(zoom.scale(view, picture));
    match zoom {
        Zoom::Fit => format!("Fit to the window, {p} percent"),
        Zoom::Scale(_) if p == 100 => "Actual size, 100 percent".to_owned(),
        Zoom::Scale(_) => format!("Zoom {p} percent"),
    }
}

/// One zoom step in (`true`) or out from `zoom`, within the limits.
pub fn zoom_step(zoom: Zoom, view: (f64, f64), picture: (u32, u32), inward: bool) -> Zoom {
    let s = zoom.scale(view, picture);
    let next = if inward { s * ZOOM_STEP } else { s / ZOOM_STEP };
    Zoom::Scale(next.clamp(ZOOM_MIN, ZOOM_MAX))
}

/// What the picture view shows, as plain data: the graphic's name and
/// description, the lines under it, the pixels, and how they are placed.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureView {
    /// The graphic's name: the file's name, hidden characters spelled
    /// out.
    pub name: String,
    /// The graphic's description: the preview crate's sentence.
    pub description: String,
    /// The lines drawn under it: the zoom, the animation if any, and the
    /// two help lines; each fits a 40-cell Braille line.
    pub lines: Vec<String>,
    /// The picture.
    pub pixels: Pixels,
    /// The zoom.
    pub zoom: Zoom,
    /// The point of the picture at the view's middle, in picture pixels.
    pub center: (f64, f64),
    /// Changes for another picture.
    pub generation: u64,
}

/// The lines under the graphic: the zoom in words, the animation line,
/// then the help lines.
pub fn view_lines(zoom_line: String, animation: Option<&str>) -> Vec<String> {
    let mut lines = vec![zoom_line];
    lines.extend(animation.map(str::to_owned));
    lines.push(HELP_LINE.to_owned());
    lines.push(MORE_HELP_LINE.to_owned());
    lines
}

/// The middle of a picture, where the view starts.
pub fn middle(picture: (u32, u32)) -> (f64, f64) {
    (f64::from(picture.0) / 2.0, f64::from(picture.1) / 2.0)
}

/// The view's middle moved by an arrow (`dx`, `dy` of -1, 0 or 1) by
/// [`PAN_PART`] of the view, kept inside the picture; unchanged when the
/// whole picture is in view.
pub fn pan(
    center: (f64, f64),
    (dx, dy): (i8, i8),
    scale: f64,
    view: (f64, f64),
    picture: (u32, u32),
) -> (f64, f64) {
    let (pw, ph) = (f64::from(picture.0), f64::from(picture.1));
    let scale = scale.max(f64::MIN_POSITIVE);
    // Half the view, in picture pixels.
    let (hw, hh) = (view.0 / scale / 2.0, view.1 / scale / 2.0);
    let step_x = view.0 / scale * PAN_PART;
    let step_y = view.1 / scale * PAN_PART;
    let keep = |c: f64, half: f64, len: f64| {
        if half * 2.0 >= len {
            len / 2.0
        } else {
            c.clamp(half, len - half)
        }
    };
    (
        keep(center.0 + f64::from(dx) * step_x, hw, pw),
        keep(center.1 + f64::from(dy) * step_y, hh, ph),
    )
}

/// Whether every line the view says fits a 40-cell Braille line.
pub fn fits_braille(line: &str) -> bool {
    line.chars().count() <= BRAILLE_CELLS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_fit_a_braille_line_with_their_meaning_first() {
        for line in [HELP_LINE, MORE_HELP_LINE] {
            assert!(fits_braille(line), "{line}: {}", line.chars().count());
        }
        assert_eq!(HELP_LINE.chars().count(), 39);
        assert_eq!(MORE_HELP_LINE.chars().count(), 38);
        let fit = zoom_words(Zoom::Fit, (800.0, 600.0), (1920, 1080));
        assert_eq!(fit, "Fit to the window, 42 percent");
        assert!(fits_braille(&fit));
        assert_eq!(
            zoom_words(Zoom::Scale(1.0), (800.0, 600.0), (1920, 1080)),
            "Actual size, 100 percent"
        );
        assert_eq!(
            zoom_words(Zoom::Scale(1.5), (800.0, 600.0), (1920, 1080)),
            "Zoom 150 percent"
        );
        let line = animation_line(Frames::Many(12)).unwrap();
        assert_eq!(line, "Animation: 12 frames, not playing");
        assert_eq!(line.chars().count(), 33);
        assert_eq!(animation_line(Frames::One), None);
        for line in [PAGE_HELP_LINE, NOT_LOCAL] {
            assert!(fits_braille(line), "{line}");
        }
    }

    /// A blue PNG of `w` by `h`, 8-bit RGBA, stored without compression,
    /// as a helper program would answer (made in memory, never fetched).
    fn tests_png(w: u32, h: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFF_u32;
            for &byte in data {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc32(&body).to_be_bytes());
        }
        let row: Vec<u8> = std::iter::once(0)
            .chain((0..w).flat_map(|_| [0, 0, 255, 255]))
            .collect();
        let raw: Vec<u8> = (0..h).flat_map(|_| row.iter().copied()).collect();
        let (mut a, mut b) = (1_u32, 0_u32);
        for &byte in &raw {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        let mut zlib = vec![0x78, 0x01, 1];
        let len = raw.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend(&raw);
        zlib.extend(((b << 16) | a).to_be_bytes());
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn a_converted_picture_shows_and_a_missing_helper_says_its_install_line() {
        let png = tests_png(6, 4);
        let guard = PixelGuard::DEFAULT;
        let decoder = Guard::default();
        let opened = converted("HEIC, 6 by 4, needs a component", Ok(png), &guard, &decoder);
        let Opened::Shown(picture) = opened else {
            panic!("{opened:?}")
        };
        assert_eq!(picture.sentence, "HEIC, 6 by 4, through ImageMagick");
        assert_eq!((picture.pixels.width, picture.pixels.height), (6, 4));
        let missing = converted(
            "HEIC, 6 by 4, needs a component",
            Err(vec![
                diroxide_app::app::picture_view::NEEDS_MAGICK.to_owned(),
                "Install: winget install ImageMagick.ImageMagick".to_owned(),
            ]),
            &guard,
            &decoder,
        );
        assert_eq!(
            missing,
            Opened::Refused(
                "HEIC, 6 by 4, needs ImageMagick\nInstall: winget install ImageMagick.ImageMagick"
                    .to_owned()
            )
        );
        let page = read_page(Ok(tests_png(8, 10)), "Page 1 of 3", &guard, &decoder);
        let Opened::Shown(page) = page else {
            panic!("{page:?}")
        };
        assert_eq!(page.sentence, "Page 1 of 3");
    }

    #[test]
    fn zoom_steps_stay_inside_their_limits() {
        let view = (800.0, 600.0);
        let picture = (1920, 1080);
        let mut zoom = Zoom::Fit;
        for _ in 0..40 {
            zoom = zoom_step(zoom, view, picture, true);
        }
        assert_eq!(zoom, Zoom::Scale(ZOOM_MAX));
        for _ in 0..80 {
            zoom = zoom_step(zoom, view, picture, false);
        }
        assert_eq!(zoom, Zoom::Scale(ZOOM_MIN));
        // One step in from the fit is a quarter more than the fit.
        let fit = fit_scale(view, picture);
        match zoom_step(Zoom::Fit, view, picture, true) {
            Zoom::Scale(s) => assert!((s - fit * ZOOM_STEP).abs() < 1e-9),
            Zoom::Fit => panic!("a step leaves the fit"),
        }
    }

    #[test]
    fn a_zoomed_picture_moves_inside_itself_and_a_fitted_one_stays() {
        let view = (100.0, 100.0);
        let picture = (1000, 1000);
        let start = middle(picture);
        // At the fit the whole picture is in view: it stays in the middle.
        let fit = fit_scale(view, picture);
        assert_eq!(pan(start, (1, 0), fit, view, picture), start);
        // At actual size it moves by an eighth of the view, and stops at
        // the edge.
        let moved = pan(start, (1, 0), 1.0, view, picture);
        assert_eq!(moved, (512.5, 500.0));
        let mut at = start;
        for _ in 0..1000 {
            at = pan(at, (-1, -1), 1.0, view, picture);
        }
        assert_eq!(at, (50.0, 50.0));
    }

    #[test]
    fn the_format_word_comes_from_the_name_when_the_header_says_nothing() {
        assert_eq!(format_word("photo.jpg"), "JPEG");
        assert_eq!(format_word("scan.tif"), "TIFF");
        assert_eq!(format_word("icon.png"), "PNG");
        assert_eq!(format_word("noextension"), "Picture");
        assert!(is_picture("photo.PNG"));
        assert!(is_picture("still.gif"));
        assert!(!is_picture("notes.txt"));
    }
}
