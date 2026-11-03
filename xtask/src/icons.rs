//! `cargo xtask icons [--check]`: the application icon, every file a
//! platform asks for, drawn from the approved mark
//! (`assets/logo/diroxide-mark.svg`, concept B's icon) with resvg, pure
//! Rust, the same bytes on every run:
//!
//! - `assets/logo/png/diroxide-mark-N.png`, the mark at 16 to 512 pixels:
//!   the images of the Windows and macOS icons below and the Linux
//!   desktop's hicolor sizes ([`PNG_SIZES`], [`HICOLOR_SIZES`]);
//! - `assets/logo/icons/diroxide.ico`, the Windows icon file: the eight
//!   sizes Windows asks for at 100 to 200 percent in the title bar, the
//!   taskbar, Explorer and Start ([`ICO_SIZES`]);
//! - `assets/logo/icons/diroxide.icns`, the macOS icon file;
//! - `assets/logo/icons/diroxide.res`, the compiled Windows resource that
//!   `crates/diro/build.rs` links into `diro.exe` on windows-msvc, written
//!   here by hand with no resource compiler and no C, so Explorer,
//!   shortcuts and the taskbar show the mark; its icon group is
//!   [`RESOURCE_ICON`], which the window loads by id;
//! - `assets/logo/icons/diroxide-64.rgba`, the 64-pixel mark as straight
//!   RGBA, the window's icon where the resource is not linked (Linux on
//!   X11, a GNU build of Windows).
//!
//! The Linux desktop's scalable icon is the mark itself, installed as
//! `diroxide.svg` by `packaging/appimage/build.sh`.
//!
//! `--check` draws every file in memory and compares it with the one on
//! disk, byte for byte, as `cargo xtask regen --check` does too. The
//! drawing is the same on every x86_64 computer (the check runs there in
//! CI); another processor may round a few pixels differently. The ICO and
//! resource writers are adapted from textweaver's
//! `crates/textweaver-xilem/src/icon.rs` (the owner's own project).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use resvg::{tiny_skia, usvg};

/// The approved mark, concept B's icon (32 by 32 units).
pub const MARK: &str = "assets/logo/diroxide-mark.svg";
/// Where the PNG renders go.
pub const PNG_DIR: &str = "assets/logo/png";
/// Where the icon files go.
pub const ICON_DIR: &str = "assets/logo/icons";

/// Every size of the mark as a PNG, in pixels: the union of the sizes
/// below, and 128 and 512 for the web and macOS.
pub const PNG_SIZES: [u32; 11] = [16, 20, 22, 24, 32, 40, 48, 64, 128, 256, 512];
/// The Windows icon's sizes, smallest first.
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// The sizes a Linux desktop's hicolor theme installs
/// (`share/icons/hicolor/NxN/apps/diroxide.png`).
pub const HICOLOR_SIZES: [u32; 9] = [16, 22, 24, 32, 48, 64, 128, 256, 512];
/// The macOS icon's elements: the type and the image's side in pixels.
/// The `@2x` types reuse the next size up.
pub const ICNS_TYPES: [(&str, u32); 9] = [
    ("icp4", 16),
    ("icp5", 32),
    ("ic07", 128),
    ("ic08", 256),
    ("ic09", 512),
    ("ic11", 32),
    ("ic12", 64),
    ("ic13", 256),
    ("ic14", 512),
];
/// The side of the window's own icon, in pixels.
pub const WINDOW_ICON_SIZE: u32 = 64;

/// The id of the icon group in `diroxide.res`, the first, so Explorer and
/// shortcuts use it; the window loads it by this id (`ICON_RESOURCE` in
/// `crates/diroxide-gui/src/window.rs`).
pub const RESOURCE_ICON: u16 = 1;

/// The PNG file of one size, relative to the root.
pub fn png_path(size: u32) -> String {
    format!("{PNG_DIR}/diroxide-mark-{size}.png")
}

/// The ICO file, relative to the root.
pub fn ico_path() -> String {
    format!("{ICON_DIR}/diroxide.ico")
}

/// The ICNS file, relative to the root.
pub fn icns_path() -> String {
    format!("{ICON_DIR}/diroxide.icns")
}

/// The compiled resource, relative to the root.
pub fn res_path() -> String {
    format!("{ICON_DIR}/diroxide.res")
}

/// The window's straight RGBA icon, relative to the root.
pub fn rgba_path() -> String {
    format!("{ICON_DIR}/diroxide-{WINDOW_ICON_SIZE}.rgba")
}

/// Every icon file: its path from the root, and its bytes.
pub fn files(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    if let Some(size) = ICO_SIZES
        .iter()
        .chain(HICOLOR_SIZES.iter())
        .find(|size| !PNG_SIZES.contains(size))
    {
        bail!("the {size}-pixel image is asked for but never drawn; add it to PNG_SIZES");
    }
    let svg = fs::read_to_string(root.join(MARK)).with_context(|| format!("reading {MARK}"))?;
    // The mark has no text, so no font is loaded: the drawing cannot
    // depend on the fonts a computer has.
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).context("parsing the mark")?;
    let mut pngs = Vec::new();
    let mut out = Vec::new();
    for size in PNG_SIZES {
        let pixmap = draw(&tree, size)?;
        let png = pixmap
            .encode_png()
            .with_context(|| format!("encoding the {size}-pixel PNG"))?;
        out.push((png_path(size), png.clone()));
        pngs.push((size, png));
        if size == WINDOW_ICON_SIZE {
            out.push((rgba_path(), straight_rgba(&pixmap)));
        }
    }
    let image = |size: u32| -> Result<(u32, Vec<u8>)> {
        pngs.iter()
            .find(|(side, _)| *side == size)
            .cloned()
            .with_context(|| format!("no {size}-pixel image was drawn"))
    };
    let ico_images = ICO_SIZES
        .iter()
        .map(|size| image(*size))
        .collect::<Result<Vec<_>>>()?;
    out.push((ico_path(), ico(&ico_images)));
    let icns_images = ICNS_TYPES
        .iter()
        .map(|(kind, size)| image(*size).map(|(_, png)| (*kind, png)))
        .collect::<Result<Vec<_>>>()?;
    out.push((icns_path(), icns(&icns_images)));
    out.push((res_path(), res(&[(RESOURCE_ICON, ico_images.as_slice())])));
    Ok(out)
}

/// The mark drawn in a `size`-pixel square over a transparent background.
fn draw(tree: &usvg::Tree, size: u32) -> Result<tiny_skia::Pixmap> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).context("allocating the pixmap")?;
    let scale_x = size as f32 / tree.size().width();
    let scale_y = size as f32 / tree.size().height();
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale_x, scale_y),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// A pixmap's pixels as straight (not premultiplied) RGBA, as winit takes
/// a window's icon.
fn straight_rgba(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// The paths whose bytes on disk differ from the drawn ones.
pub fn stale(root: &Path) -> Result<Vec<String>> {
    Ok(files(root)?
        .into_iter()
        .filter(|(path, bytes)| fs::read(root.join(path)).ok().as_deref() != Some(bytes))
        .map(|(path, _)| path)
        .collect())
}

/// Writes every icon file whose bytes changed; says which.
pub fn write(root: &Path) -> Result<bool> {
    fs::create_dir_all(root.join(PNG_DIR)).with_context(|| format!("creating {PNG_DIR}"))?;
    fs::create_dir_all(root.join(ICON_DIR)).with_context(|| format!("creating {ICON_DIR}"))?;
    for (path, bytes) in files(root)? {
        if fs::read(root.join(&path)).is_ok_and(|on_disk| on_disk == bytes) {
            println!("icons: {path} is current.");
            continue;
        }
        fs::write(root.join(&path), &bytes).with_context(|| format!("writing {path}"))?;
        println!("icons: wrote {path}, {} bytes.", bytes.len());
    }
    Ok(true)
}

/// Checks that every icon file is the drawn one.
pub fn check(root: &Path) -> Result<bool> {
    let stale = stale(root)?;
    if stale.is_empty() {
        println!("icons: Pass. Every icon file is current.");
        Ok(true)
    } else {
        println!(
            "icons: Fail. Out of date: {}. Run cargo xtask icons and commit what it writes.",
            stale.join(", ")
        );
        Ok(false)
    }
}

/// Runs `cargo xtask icons` with its arguments.
pub fn run(root: &Path, args: &[&str]) -> Result<bool> {
    match args {
        [] => write(root),
        ["--check"] => check(root),
        _ => bail!("Usage: cargo xtask icons [--check]"),
    }
}

fn len32(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len()).unwrap_or(u32::MAX)
}

/// An icon directory entry's first 8 bytes: width and height (0 means
/// 256), no palette, one plane, 32 bits per pixel. The image's size and
/// its offset (in a file) or id (in a resource) follow.
fn dir_entry_head(side: u32) -> [u8; 8] {
    let s = u8::try_from(side).unwrap_or(0);
    let mut e = [0u8; 8];
    e[0] = s;
    e[1] = s;
    e[4..6].copy_from_slice(&1u16.to_le_bytes());
    e[6..8].copy_from_slice(&32u16.to_le_bytes());
    e
}

/// A multi-size `.ico` file holding `images`: (side in pixels, PNG
/// bytes), smallest first. Windows Vista and later read PNG images in
/// icons.
pub fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = u16::try_from(images.len()).unwrap_or(u16::MAX);
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (side, png) in images {
        out.extend_from_slice(&dir_entry_head(*side));
        out.extend_from_slice(&len32(png).to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).unwrap_or(u32::MAX).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

/// A macOS `.icns` file holding `images`: (element type, PNG bytes). Each
/// element is its type, its length with this 8-byte head (big-endian),
/// then the PNG; the file starts with `icns` and its whole length.
pub fn icns(images: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, png) in images {
        body.extend_from_slice(kind.as_bytes());
        body.extend_from_slice(&(len32(png).saturating_add(8)).to_be_bytes());
        body.extend_from_slice(png);
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(len32(&body).saturating_add(8)).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// The resource type of one icon image.
pub const RT_ICON: u16 = 3;
/// The resource type of an icon group.
pub const RT_GROUP_ICON: u16 = 14;
/// The language of the resources: English, United States.
const LANG_EN_US: u16 = 0x0409;

/// A compiled Windows resource file (`.res`) holding icon groups: each
/// group is (group id, images), and its images get ids from 1 up. The
/// Microsoft linker takes a `.res` file as an input, so a program links
/// it with no resource compiler.
pub fn res(groups: &[(u16, &[(u32, Vec<u8>)])]) -> Vec<u8> {
    fn entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, lang: u16, data: &[u8]) {
        // The head: the data's size, the head's own size (32 bytes, with
        // a numbered type and name), the type and name as 0xFFFF and a
        // number, the data version, the memory flags, the language, the
        // version and the characteristics.
        out.extend_from_slice(&len32(data).to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&lang.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    let mut out = Vec::new();
    // The empty first entry every 32-bit resource file starts with.
    entry(&mut out, 0, 0, 0, 0, &[]);
    let mut next_image: u16 = 1;
    for (group, images) in groups {
        let mut dir = Vec::new();
        dir.extend_from_slice(&0u16.to_le_bytes());
        dir.extend_from_slice(&1u16.to_le_bytes());
        dir.extend_from_slice(
            &u16::try_from(images.len())
                .unwrap_or(u16::MAX)
                .to_le_bytes(),
        );
        for (side, png) in images.iter() {
            entry(&mut out, RT_ICON, next_image, 0x1010, LANG_EN_US, png);
            dir.extend_from_slice(&dir_entry_head(*side));
            dir.extend_from_slice(&len32(png).to_le_bytes());
            dir.extend_from_slice(&next_image.to_le_bytes());
            next_image += 1;
        }
        entry(&mut out, RT_GROUP_ICON, *group, 0x1030, LANG_EN_US, &dir);
    }
    out
}

/// One entry of a compiled resource file, read back: its type, its id
/// and its data. Only numbered types and names are read, the only kind
/// [`res`] writes.
#[cfg(test)]
fn res_entries(bytes: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
    let u32_at =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let mut at = 0;
    let mut entries = Vec::new();
    while at + 32 <= bytes.len() {
        let size = u32_at(at) as usize;
        let head = u32_at(at + 4) as usize;
        assert_eq!(head, 32, "a numbered head");
        assert_eq!(u16_at(at + 8), 0xFFFF);
        assert_eq!(u16_at(at + 12), 0xFFFF);
        let (kind, id) = (u16_at(at + 10), u16_at(at + 14));
        entries.push((kind, id, bytes[at + head..at + head + size].to_vec()));
        at += head + size;
        at = at.div_ceil(4) * 4;
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn an_ico_lists_its_images() {
        let images = vec![(16, vec![1, 2, 3]), (256, vec![4, 5])];
        let f = ico(&images);
        assert_eq!(&f[..6], &[0, 0, 1, 0, 2, 0]);
        // The first entry: 16 by 16, 3 bytes at offset 6 + 32.
        assert_eq!(f[6], 16);
        assert_eq!(&f[6 + 8..6 + 12], &3u32.to_le_bytes());
        assert_eq!(&f[6 + 12..6 + 16], &38u32.to_le_bytes());
        // The second: 256 is written as 0.
        assert_eq!(f[22], 0);
        assert_eq!(&f[38..], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn an_icns_names_each_element_and_its_length() {
        let f = icns(&[("icp4", vec![9, 9]), ("ic08", vec![7])]);
        assert_eq!(&f[..4], b"icns");
        assert_eq!(u32::from_be_bytes([f[4], f[5], f[6], f[7]]), 27);
        assert_eq!(f.len(), 27);
        assert_eq!(&f[8..12], b"icp4");
        assert_eq!(u32::from_be_bytes([f[12], f[13], f[14], f[15]]), 10);
        assert_eq!(&f[16..18], &[9, 9]);
        assert_eq!(&f[18..22], b"ic08");
        assert_eq!(&f[26..], &[7]);
    }

    #[test]
    fn a_res_holds_each_image_and_its_group() {
        let images = vec![(16, vec![1, 2, 3]), (256, vec![4, 5, 6, 7, 8])];
        let r = res(&[(RESOURCE_ICON, images.as_slice())]);
        assert_eq!(r.len() % 4, 0, "every entry ends on a 4-byte boundary");
        let entries = res_entries(&r);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0], (0, 0, Vec::new()));
        assert_eq!(entries[1], (RT_ICON, 1, vec![1, 2, 3]));
        assert_eq!(entries[2], (RT_ICON, 2, vec![4, 5, 6, 7, 8]));
        let (kind, id, group) = &entries[3];
        assert_eq!((*kind, *id), (RT_GROUP_ICON, RESOURCE_ICON));
        // The group: reserved, type 1, two entries of 14 bytes, each
        // naming its image by id.
        assert_eq!(&group[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(group.len(), 6 + 2 * 14);
        assert_eq!(group[6], 16);
        assert_eq!(&group[6 + 12..6 + 14], &1u16.to_le_bytes());
        assert_eq!(group[6 + 14], 0, "256 is written as 0");
        assert_eq!(&group[6 + 14 + 12..6 + 14 + 14], &2u16.to_le_bytes());
    }

    #[test]
    fn the_sizes_cover_every_platform() {
        for size in ICO_SIZES.iter().chain(HICOLOR_SIZES.iter()) {
            assert!(PNG_SIZES.contains(size), "{size} is drawn");
        }
        for (_, size) in ICNS_TYPES {
            assert!(PNG_SIZES.contains(&size), "{size} is drawn");
        }
        assert!(PNG_SIZES.contains(&WINDOW_ICON_SIZE));
        assert!(PNG_SIZES.windows(2).all(|w| w[0] < w[1]));
    }

    /// The committed files hold what each platform reads. The bytes are
    /// compared by `cargo xtask regen --check` on x86_64; here only their
    /// shape, so the check holds on every processor.
    #[test]
    fn the_committed_files_hold_every_size() {
        let root = root();
        for size in PNG_SIZES {
            let png = fs::read(root.join(png_path(size))).unwrap();
            assert_eq!(&png[1..4], b"PNG", "{size}");
            // The width and height in the image header.
            assert_eq!(
                u32::from_be_bytes([png[16], png[17], png[18], png[19]]),
                size
            );
            assert_eq!(
                u32::from_be_bytes([png[20], png[21], png[22], png[23]]),
                size
            );
        }
        let f = fs::read(root.join(ico_path())).unwrap();
        assert_eq!(&f[..4], &[0, 0, 1, 0]);
        assert_eq!(
            usize::from(u16::from_le_bytes([f[4], f[5]])),
            ICO_SIZES.len()
        );
        for (i, side) in ICO_SIZES.iter().enumerate() {
            assert_eq!(u32::from(f[6 + 16 * i]), *side % 256);
        }
        let f = fs::read(root.join(icns_path())).unwrap();
        assert_eq!(&f[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes([f[4], f[5], f[6], f[7]]) as usize,
            f.len()
        );
        let entries = res_entries(&fs::read(root.join(res_path())).unwrap());
        assert_eq!(entries.len(), 1 + ICO_SIZES.len() + 1);
        let (kind, id, group) = entries.last().unwrap();
        assert_eq!((*kind, *id), (RT_GROUP_ICON, RESOURCE_ICON));
        assert_eq!(
            usize::from(u16::from_le_bytes([group[4], group[5]])),
            ICO_SIZES.len()
        );
        let rgba = fs::read(root.join(rgba_path())).unwrap();
        let side = WINDOW_ICON_SIZE as usize;
        assert_eq!(rgba.len(), side * side * 4);
        // A corner is transparent; the cabinet's middle is solid.
        assert_eq!(rgba[3], 0);
        let middle = ((side * 3 / 4) * side + side / 2) * 4;
        assert_eq!(rgba[middle + 3], 255);
    }

    /// The AppImage's scripts install and check the hicolor sizes this
    /// task draws, and the scalable mark.
    #[test]
    fn the_appimage_installs_every_hicolor_size() {
        let want: Vec<String> = HICOLOR_SIZES.iter().map(u32::to_string).collect();
        let want = format!("for size in {}; do", want.join(" "));
        for script in ["packaging/appimage/build.sh", "packaging/appimage/check.sh"] {
            let text = fs::read_to_string(root().join(script)).unwrap();
            assert!(
                text.lines().any(|l| l.trim() == want),
                "{script} lists the sizes as {want:?}"
            );
            assert!(
                text.contains("hicolor/scalable/apps/diroxide.svg"),
                "{script}"
            );
        }
    }

    #[test]
    fn the_mark_draws_the_same_twice() {
        let svg = fs::read_to_string(root().join(MARK)).unwrap();
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        let a = draw(&tree, 32).unwrap().encode_png().unwrap();
        let b = draw(&tree, 32).unwrap().encode_png().unwrap();
        assert_eq!(a, b);
    }
}
