//! The mark linked into `diro.exe` (`build.rs`): the program's resources
//! read back by id from the file itself, as Explorer and the window find
//! them, with a small reader of the PE format here, so the test needs no
//! Windows call and no `unsafe`.

use std::path::Path;

/// The resource type of one icon image.
const RT_ICON: u32 = 3;
/// The resource type of an icon group.
const RT_GROUP_ICON: u32 = 14;
/// The icon group's id, as `cargo xtask icons` writes it and the window
/// loads it (`ICON_RESOURCE` in `crates/diroxide-gui/src/window.rs`).
const ICON_GROUP: u32 = 1;
/// The sizes in the group, as `cargo xtask icons` writes them.
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// A program's resource section, read from its file.
struct Resources {
    file: Vec<u8>,
    /// Each section: its address in memory, its size there, and where its
    /// bytes are in the file.
    sections: Vec<(u32, u32, u32)>,
    /// Where the resource section starts in the file.
    base: usize,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl Resources {
    /// Reads the PE headers: the optional header's resource directory and
    /// the section table. `None` when the file is not a PE program or has
    /// no resources.
    fn read(path: &Path) -> Option<Self> {
        let file = std::fs::read(path).ok()?;
        let pe = u32_at(&file, 0x3C)? as usize;
        if file.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let sections = usize::from(u16_at(&file, coff + 2)?);
        let optional = coff + 20;
        let optional_size = usize::from(u16_at(&file, coff + 16)?);
        // The data directories: after 112 bytes in PE32+, 96 in PE32; the
        // resource table is the third.
        let directories = match u16_at(&file, optional)? {
            0x20B => optional + 112,
            0x10B => optional + 96,
            _ => return None,
        };
        let resource_rva = u32_at(&file, directories + 2 * 8)?;
        if resource_rva == 0 {
            return None;
        }
        let table = optional + optional_size;
        let sections = (0..sections)
            .map(|i| {
                let s = table + i * 40;
                let virtual_size = u32_at(&file, s + 8)?;
                let address = u32_at(&file, s + 12)?;
                let raw_size = u32_at(&file, s + 16)?;
                let raw = u32_at(&file, s + 20)?;
                Some((address, virtual_size.max(raw_size), raw))
            })
            .collect::<Option<Vec<_>>>()?;
        let mut resources = Self {
            file,
            sections,
            base: 0,
        };
        resources.base = resources.offset(resource_rva)?;
        Some(resources)
    }

    /// The file offset of an address in memory.
    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections
            .iter()
            .find(|(address, size, _)| rva >= *address && rva < address + size)
            .map(|(address, _, raw)| (rva - address + raw) as usize)
    }

    /// The entry with this id in the directory at `at` (an offset in the
    /// resource section): its target, with the high bit still marking a
    /// subdirectory. With no id, the first entry (the first language).
    fn entry(&self, at: usize, id: Option<u32>) -> Option<u32> {
        let dir = self.base + at;
        let named = usize::from(u16_at(&self.file, dir + 12)?);
        let numbered = usize::from(u16_at(&self.file, dir + 14)?);
        (0..named + numbered).find_map(|i| {
            let e = dir + 16 + i * 8;
            let name = u32_at(&self.file, e)?;
            let target = u32_at(&self.file, e + 4)?;
            (id.is_none() || id == Some(name)).then_some(target)
        })
    }

    /// The bytes of the resource of this type and id, in its first
    /// language.
    fn get(&self, kind: u32, id: u32) -> Option<&[u8]> {
        const DIRECTORY: u32 = 0x8000_0000;
        let types = self.entry(0, Some(kind))?;
        let names = self.entry((types & !DIRECTORY) as usize, Some(id))?;
        let leaf = self.entry((names & !DIRECTORY) as usize, None)?;
        if leaf & DIRECTORY != 0 {
            return None;
        }
        let data = self.base + leaf as usize;
        let rva = u32_at(&self.file, data)?;
        let size = u32_at(&self.file, data + 4)? as usize;
        let at = self.offset(rva)?;
        self.file.get(at..at + size)
    }
}

/// On Windows with the Microsoft toolchain, `diro.exe` holds the icon
/// group by its id, with every size, each image a PNG that the same
/// resource file names; the bytes are the committed `.res` file's.
#[test]
fn the_program_holds_the_icon_by_its_id() {
    if !cfg!(all(windows, target_env = "msvc")) {
        return;
    }
    let exe = Path::new(env!("CARGO_BIN_EXE_diro"));
    let resources = Resources::read(exe).expect("diro.exe has a resource section");
    let group = resources
        .get(RT_GROUP_ICON, ICON_GROUP)
        .expect("the icon group, by its id");
    assert_eq!(&group[..4], &[0, 0, 1, 0], "an icon group");
    let count = usize::from(u16_at(group, 4).unwrap());
    assert_eq!(count, SIZES.len());
    let committed = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/logo/icons/diroxide.res"),
    )
    .unwrap();
    for (i, size) in SIZES.iter().enumerate() {
        let e = 6 + i * 14;
        assert_eq!(u32::from(group[e]), size % 256, "the size of image {i}");
        let image_id = u32::from(u16_at(group, e + 12).unwrap());
        let image = resources
            .get(RT_ICON, image_id)
            .unwrap_or_else(|| panic!("icon image {image_id}"));
        assert_eq!(
            &image[..8],
            b"\x89PNG\r\n\x1a\n",
            "image {image_id} is a PNG"
        );
        assert_eq!(
            u32_at(group, e + 8).unwrap() as usize,
            image.len(),
            "image {image_id}'s size"
        );
        assert!(
            committed.windows(image.len()).any(|w| w == image),
            "image {image_id} is the committed resource file's"
        );
    }
}
