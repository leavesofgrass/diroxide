//! The picture view (alpha 4, R30 commit 112): Enter on a picture in the
//! viewer or the quick view's preview opens it as one graphic named by
//! its file and described by the preview crate's words; Plus, Minus, 0
//! and 1 zoom and fit, said in words; an animation is shown still and
//! says so; a picture past the pixel guard is said in words and never
//! decoded; Escape goes back. Every picture is made here, none fetched.

#![allow(clippy::unwrap_used)]

use diroxide_core::Side;
use diroxide_core::format::BRAILLE_CELLS;
use diroxide_gui::picture::{ENTER_HINT, HELP_LINE, MORE_HELP_LINE};
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, fixture, name_of};

/// CRC-32 as PNG's chunks carry it.
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

/// Adler-32, the zlib stream's check.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend(u32::try_from(data.len()).unwrap().to_be_bytes());
    let mut body = kind.to_vec();
    body.extend(data);
    out.extend(&body);
    out.extend(crc32(&body).to_be_bytes());
}

/// A PNG whose header says `width` by `height` (8-bit RGBA) and whose
/// pixels are `rows` rows of `cols` pixels of `rgba`, stored without
/// compression: the header's size and the pixels' agree for a real
/// picture, and only the header is big for the guard's test.
fn png(width: u32, height: u32, cols: u32, rows: u32, rgba: [u8; 4]) -> Vec<u8> {
    // Each row: the filter byte (none), then its pixels.
    let row: Vec<u8> = std::iter::once(0)
        .chain((0..cols).flat_map(|_| rgba))
        .collect();
    let raw: Vec<u8> = (0..rows).flat_map(|_| row.iter().copied()).collect();
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = u16::try_from(block.len()).unwrap();
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend(*block);
    }
    zlib.extend(adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A picture of `w` by `h` in one color.
fn plain_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    png(w, h, w, h, rgba)
}

/// A GIF of two frames of one pixel each: an animation.
fn two_frame_gif() -> Vec<u8> {
    let mut gif = b"GIF89a".to_vec();
    // One pixel, a global table of two colors: red and blue.
    gif.extend([1, 0, 1, 0, 0x80, 0, 0, 0xff, 0, 0, 0, 0, 0xff]);
    for _ in 0..2 {
        // A tenth of a second a frame.
        gif.extend([0x21, 0xf9, 0x04, 0x00, 0x0a, 0x00, 0x00, 0x00]);
        gif.extend([0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
        // LZW, the smallest code size: clear, color 0, end.
        gif.extend([0x02, 0x02, 0x44, 0x01, 0x00]);
    }
    gif.push(0x3b);
    gif
}

/// The image node: its name and description, if one is in the tree.
fn graphic(rig: &Rig) -> Option<(String, String)> {
    let state = rig.h.access_tree().state();
    let mut stack = vec![state.root()];
    while let Some(node) = stack.pop() {
        if node.is_hidden() {
            continue;
        }
        if node.role() == Role::Image {
            let description = node.data().description().unwrap_or_default().to_owned();
            return Some((name_of(&node), description));
        }
        stack.extend(node.children());
    }
    None
}

/// The lines under the picture, as text nodes in the tree.
fn picture_lines(rig: &Rig) -> Vec<String> {
    rig.f
        .picture_view()
        .map(|v| v.lines.clone())
        .unwrap_or_default()
}

/// A window on a folder holding `photo.png` (`bytes`) and a text file.
fn window_with(name: &str, bytes: &[u8]) -> (Rig, tempfile::TempDir) {
    let dir = fixture(&["notes.txt"], &[]);
    std::fs::write(dir.path().join(name), bytes).unwrap();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, name));
    rig.refresh();
    (rig, dir)
}

/// Waits for the picture's read to come in.
fn opened(rig: &mut Rig) {
    assert!(
        rig.settle_until(|rig| !rig.f.picture_reading()),
        "the picture was never read"
    );
}

#[test]
fn enter_in_the_viewer_opens_a_made_png_as_one_named_graphic_with_its_words() {
    let (mut rig, _dir) = window_with("photo.png", &plain_png(64, 48, [0, 128, 255, 255]));
    let from = rig.spoken().len();
    rig.press("f3");
    assert!(rig.f.app().viewer().is_some());
    // The window says once that Enter shows the picture.
    let spoken = rig.spoken()[from..].join(" ");
    assert_eq!(spoken.matches(ENTER_HINT).count(), 1, "{spoken}");
    rig.press("enter");
    opened(&mut rig);
    // One graphic, named by the file, described by the preview crate's
    // sentence, with the keyboard focus.
    let (name, description) = graphic(&rig).unwrap_or_else(|| panic!("{:?}", rig.walk()));
    assert_eq!(name, "photo.png");
    assert!(
        description.starts_with("PNG, 64 by 48, 8-bit RGBA"),
        "{description}"
    );
    assert!(
        description.chars().count() <= BRAILLE_CELLS,
        "{description}"
    );
    assert_eq!(rig.focus_node(), (Role::Image, "photo.png".to_owned()));
    // The lines under it: the zoom in words, then the keys; each fits a
    // Braille line, and the keys are said once as it opens.
    let lines = picture_lines(&rig);
    assert!(lines[0].starts_with("Fit to the window, "), "{lines:?}");
    assert_eq!(&lines[1..], [HELP_LINE, MORE_HELP_LINE]);
    for line in &lines {
        assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
    }
    assert!(
        rig.walk()
            .iter()
            .any(|(role, text)| *role == Role::Label && text == HELP_LINE),
        "{:?}",
        rig.walk()
    );
    assert_eq!(rig.spoken().last().map(String::as_str), Some(HELP_LINE));
    // Escape closes the picture, and the viewer has the focus again.
    rig.press("escape");
    assert!(rig.f.picture_view().is_none());
    assert!(graphic(&rig).is_none());
    assert!(rig.f.app().viewer().is_some());
    let (role, _) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    let (list, _) = rig.focus_list().unwrap();
    assert!(list.starts_with("Viewer, photo.png"), "{list}");
}

#[test]
fn the_picture_is_drawn_in_the_window() {
    let (mut rig, _dir) = window_with("photo.png", &plain_png(64, 48, [0, 128, 255, 255]));
    rig.press("f3");
    rig.press("enter");
    opened(&mut rig);
    rig.redraw();
    // The middle of the window is the picture's blue, at the fit.
    let image = rig.h.render();
    let (w, h) = (image.width(), image.height());
    let px = image.get_pixel(w / 2, h / 2).0;
    assert_eq!((px[0], px[1], px[2]), (0, 128, 255), "{px:?}");
}

#[test]
fn plus_minus_zero_and_one_zoom_and_fit_and_say_it_in_words() {
    let (mut rig, _dir) = window_with("photo.png", &plain_png(64, 48, [10, 20, 30, 255]));
    rig.press("f3");
    rig.press("enter");
    opened(&mut rig);
    rig.press("1");
    assert_eq!(
        rig.spoken().last().map(String::as_str),
        Some("Actual size, 100 percent")
    );
    assert_eq!(picture_lines(&rig)[0], "Actual size, 100 percent");
    rig.press("+");
    assert_eq!(
        rig.spoken().last().map(String::as_str),
        Some("Zoom 125 percent")
    );
    rig.press("-");
    rig.press("-");
    assert_eq!(
        rig.spoken().last().map(String::as_str),
        Some("Zoom 80 percent")
    );
    rig.press("0");
    let fit = rig.spoken().last().cloned().unwrap();
    assert!(fit.starts_with("Fit to the window, "), "{fit}");
    assert!(fit.ends_with(" percent"), "{fit}");
    // The arrows move a zoomed picture and say nothing; the graphic keeps
    // the focus all along.
    rig.press("1");
    let said = rig.spoken().len();
    rig.press("right");
    assert_eq!(rig.spoken().len(), said);
    assert_eq!(rig.focus_node().0, Role::Image);
}

#[test]
fn a_picture_past_the_pixel_guard_is_said_in_words_and_never_opened() {
    // The header says 20,000 by 20,000; the bytes hold one pixel.
    let (mut rig, _dir) = window_with("huge.png", &png(20_000, 20_000, 1, 1, [0, 0, 0, 255]));
    rig.press("f3");
    rig.press("enter");
    opened(&mut rig);
    assert!(rig.f.picture_view().is_none());
    assert!(graphic(&rig).is_none());
    let said = rig.spoken().last().cloned().unwrap();
    assert_eq!(said, "PNG, 20000 by 20000, too large to open");
    assert!(said.chars().count() <= BRAILLE_CELLS);
    // The viewer keeps the focus.
    assert_eq!(rig.focus_node().0, Role::ListBoxOption);
}

#[test]
fn an_animation_is_shown_still_and_says_it_is_not_playing() {
    let (mut rig, _dir) = window_with("crow.gif", &two_frame_gif());
    rig.press("f3");
    rig.press("enter");
    opened(&mut rig);
    let (name, description) = graphic(&rig).unwrap_or_else(|| panic!("{:?}", rig.spoken()));
    assert_eq!(name, "crow.gif");
    assert!(
        description.starts_with("GIF, 2 frames, 1 by 1"),
        "{description}"
    );
    let lines = picture_lines(&rig);
    assert_eq!(lines[1], "Animation: 2 frames, not playing");
    assert!(lines[1].chars().count() <= BRAILLE_CELLS);
}

#[test]
fn enter_in_the_quick_views_preview_opens_the_picture_and_escape_goes_back() {
    let (mut rig, _dir) = window_with("photo.png", &plain_png(32, 32, [200, 100, 0, 255]));
    rig.press("shift+f3");
    assert!(rig.f.app().quick_view_on());
    assert!(
        rig.settle_until(|rig| rig
            .f
            .app()
            .quick_view()
            .and_then(|q| q.shown())
            .is_some_and(|s| s.name == "photo.png")),
        "{:?}",
        rig.spoken()
    );
    rig.press("tab");
    assert!(rig.f.app().in_preview());
    rig.press("enter");
    opened(&mut rig);
    let (name, description) = graphic(&rig).unwrap_or_else(|| panic!("{:?}", rig.spoken()));
    assert_eq!(name, "photo.png");
    assert!(description.starts_with("PNG, 32 by 32"), "{description}");
    assert_eq!(rig.focus_node().0, Role::Image);
    rig.press("escape");
    assert!(rig.f.picture_view().is_none());
    assert!(rig.f.app().in_preview(), "Escape closes only the picture");
    let (list, _) = rig.focus_list().unwrap();
    assert!(list.starts_with("Right panel, Preview"), "{list}");
}

#[test]
fn the_hint_is_said_once_even_after_a_prompt_over_the_viewer() {
    let (mut rig, _dir) = window_with("photo.png", &plain_png(8, 8, [0, 0, 0, 255]));
    let from = rig.spoken().len();
    rig.press("f3");
    rig.run("viewer.jump");
    assert!(rig.dialog_name().is_some());
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
    let spoken = rig.spoken()[from..].join(" ");
    assert_eq!(spoken.matches(ENTER_HINT).count(), 1, "{spoken}");
}

#[test]
fn enter_on_a_text_file_in_the_viewer_is_the_apps() {
    let (mut rig, _dir) = window_with("photo.png", &plain_png(4, 4, [0, 0, 0, 255]));
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    let from = rig.spoken().len();
    rig.press("f3");
    rig.press("enter");
    assert!(!rig.f.picture_reading());
    assert!(rig.f.picture_view().is_none());
    assert!(!rig.spoken()[from..].iter().any(|l| l == ENTER_HINT));
}
