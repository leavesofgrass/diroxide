//! The cross-check of the zip listing against `7z l -slt` (research
//! archives.md section 11, direction one). It runs only when
//! `DIRO_CROSS_CHECK=1` is set and `7z` is on PATH (the container has
//! 7-Zip 25.01); otherwise it says why it did not run and passes.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;

use diroxide_archive::{ArchiveProvider, Limits};
use diroxide_core::{EntryKind, Place};
use diroxide_vfs::{OpContext, Provider, Registry};

/// Whether the cross-check may run, or why not.
pub fn oracle(tool: &str) -> Result<(), String> {
    if std::env::var_os("DIRO_CROSS_CHECK").is_none_or(|v| v != "1") {
        return Err("set DIRO_CROSS_CHECK=1 to run it".to_owned());
    }
    match Command::new(tool).arg("--help").output() {
        Ok(_) => Ok(()),
        Err(_) => Err(format!("{tool} is not on PATH")),
    }
}

/// `7z l -slt`'s listing: path, size and CRC of each file.
fn seven_z_listing(archive: &std::path::Path) -> BTreeMap<String, (u64, Option<u32>)> {
    let output = Command::new("7z")
        .args(["l", "-slt", "-ba"])
        .arg(archive)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut listing = BTreeMap::new();
    for block in text.split("\n\n").chain(text.split("\r\n\r\n")) {
        let field = |key: &str| {
            block
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key} = ")))
                .map(str::trim)
                .map(str::to_owned)
        };
        let (Some(path), Some(folder)) = (field("Path"), field("Folder")) else {
            continue;
        };
        if folder == "+" {
            continue;
        }
        let size = field("Size").and_then(|s| s.parse().ok()).unwrap_or(0);
        let crc = field("CRC").and_then(|c| u32::from_str_radix(&c, 16).ok());
        listing.insert(path.replace('\\', "/"), (size, crc));
    }
    listing
}

/// The provider's listing of every file, walking its folders.
fn our_listing(
    provider: &ArchiveProvider,
    archive: &Place,
) -> BTreeMap<String, (u64, Option<u32>)> {
    let ctx = OpContext::new();
    let mut listing = BTreeMap::new();
    let mut stack = vec![(archive.clone().into_archive(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for entry in provider.list(&folder, &ctx).unwrap() {
            let path = format!("{prefix}{}", entry.name);
            if entry.kind == EntryKind::Folder {
                stack.push((folder.join(&entry.name).unwrap(), format!("{path}/")));
            } else {
                listing.insert(path, (entry.size.unwrap_or(0), entry.crc32));
            }
        }
    }
    listing
}

#[test]
fn the_zip_listing_matches_seven_z() {
    if let Err(why) = oracle("7z") {
        eprintln!("cross-check against 7z l -slt not run: {why}");
        return;
    }
    let dir = crate::spike_listing::scratch();
    let path = dir.path().join("cross.zip");
    let bytes = crate::build::zip_of(&[
        ("week1/", b""),
        ("week1/notes.txt", b"hello"),
        ("week1/deep/more.txt", &[b'm'; 70_000]),
        ("caf\u{e9}.txt", b"accents"),
        ("empty.txt", b""),
    ]);
    std::fs::write(&path, bytes).unwrap();
    let provider = Arc::new(ArchiveProvider::new(Limits::default()));
    let mut registry = Registry::with_local();
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    let place = Place::parse(&path.to_string_lossy()).unwrap();
    let ours = our_listing(&provider, &place);
    let theirs = seven_z_listing(&path);
    eprintln!("cross-check against 7z l -slt ran: {} files", theirs.len());
    assert_eq!(ours, theirs);
    let _ = provider.capabilities(&place);
}
