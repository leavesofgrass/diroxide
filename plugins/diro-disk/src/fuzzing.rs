//! The bodies of the fuzz targets (R29 1.10, R30 commit 96), here so the
//! plugin's own tests replay them on every fixture and on cut and bent
//! copies, and the fuzz crate under `plugins/diro-disk/fuzz` stays a thin
//! wrapper (`docs/dev/fuzzing.md`'s shape).
//!
//! Each takes any bytes and must not panic, loop or allocate by a claimed
//! size: they run under the archive limits' strict set, which a bomb
//! passes in seconds, and each lists at most two levels and reads at most
//! a few kilobytes.

use std::sync::Arc;

use diroxide_vfs::archive::limits::Limits;

use crate::host::Stamp;
use crate::image::fs::NodeKind;
use crate::image::sniff::{self, Content};
use crate::image::source::{Backing, Memory, read_vec};
use crate::image::tree::Tree;
use crate::image::{Image, Volume, container, table};
use crate::tool::sevenz;

fn backing(data: &[u8]) -> Arc<dyn Backing> {
    Arc::new(Memory::new(data.to_vec()))
}

/// Lists the root and the first folder in it, and reads the start of the
/// first file, under the strict limits.
fn walk(tree: &mut Tree) {
    let Ok(root) = tree.list(&[]) else {
        return;
    };
    for row in root.rows.iter().take(8) {
        let name = row.shown.as_str();
        match row.node.kind {
            NodeKind::Folder => {
                let _ = tree.list(&[name]);
            }
            NodeKind::File | NodeKind::Link => {
                let _ = tree.read(&[name], 0, 4096);
            }
            NodeKind::Special => {}
        }
    }
}

/// A file system of a known kind over any bytes, opened as a volume opens
/// it (a panic inside a reader crate is caught there and becomes words).
fn file_system(data: &[u8], content: Content) {
    let volume = Volume::with_content(backing(data), content, Limits::strict());
    let _ = volume.with_tree(|tree| {
        walk(tree);
        Ok(())
    });
}

/// The MBR and GPT readers over any bytes.
pub fn partition_table(data: &[u8]) {
    let disk = backing(data);
    let _ = table::mbr::read(disk.as_ref());
    let _ = table::gpt::read(disk.as_ref());
}

/// The ISO 9660 reader over any bytes (its directory records and their
/// claims).
pub fn iso(data: &[u8]) {
    file_system(data, Content::Iso);
}

/// The FAT reader over any bytes (its boot record and folders).
pub fn fat(data: &[u8]) {
    file_system(data, Content::Fat);
}

/// The ext reader over any bytes (its superblock, inodes and extents).
pub fn ext(data: &[u8]) {
    file_system(data, Content::Ext);
}

/// The NTFS reader over any bytes.
pub fn ntfs(data: &[u8]) {
    file_system(data, Content::Ntfs);
}

/// The squashfs reader over any bytes.
pub fn squashfs(data: &[u8]) {
    file_system(data, Content::Squashfs);
}

/// The five containers' headers over any bytes: the container the bytes
/// say, decoded, and the start of its disk read.
pub fn container_header(data: &[u8]) {
    let file = backing(data);
    let head = read_vec(file.as_ref(), 0, 4096).unwrap_or_default();
    let tail = read_vec(file.as_ref(), file.len().saturating_sub(512), 512).unwrap_or_default();
    let kind = sniff::container(&head, &tail);
    let siblings = |_: &str| Err(crate::error::DiskError::NotFound);
    if let Ok(disk) = container::disk_of(kind, file, &Limits::strict(), &siblings) {
        let _ = read_vec(disk.as_ref(), 0, 65536);
        let _ = read_vec(disk.as_ref(), disk.len().saturating_sub(4096), 4096);
    }
}

/// A `7z l -slt` listing over any text: parsed and made a tree.
pub fn seven_listing(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let items = sevenz::parse_slt(&text);
    let _ = sevenz::Tree::from_items(&items);
    let _ = sevenz::Version::from_banner(&text);
}

/// The whole open: container, table or file system, the root listed and
/// one level below it.
pub fn image(data: &[u8]) {
    let file = backing(data);
    let stamp = Stamp {
        size: file.len(),
        modified: None,
    };
    let siblings = |_: &str| Err(crate::error::DiskError::NotFound);
    let Ok(image) = Image::open(file, "fuzz.img", stamp, &Limits::strict(), &siblings) else {
        return;
    };
    let _ = image.details_line();
    let volumes: Vec<Option<u32>> = match &image.layout {
        crate::image::Layout::Single(_) => vec![None],
        crate::image::Layout::Table { partitions, .. } => {
            partitions.iter().take(4).map(|p| Some(p.number)).collect()
        }
    };
    for partition in volumes {
        if let Ok(volume) = image.volume(partition) {
            let _ = volume.with_tree(|tree| {
                walk(tree);
                Ok(())
            });
        }
    }
}

/// Every target, for a seed that is not known to be one format.
pub fn all(data: &[u8]) {
    partition_table(data);
    iso(data);
    fat(data);
    ext(data);
    ntfs(data);
    squashfs(data);
    container_header(data);
    seven_listing(data);
    image(data);
}
