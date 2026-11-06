//! The disk-image plugin through the host's own checks: its manifest, the
//! conformance suite against the built program, the claim the host takes
//! from it, and every reader driven through the real host as the app
//! drives it (`PluginProvider` over the plugin's program, the image's
//! bytes through the host's `fs/read`).

// Test helpers may unwrap: a failure is the test's failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod ext;
mod fat;
mod fixture;
mod fixtures;
mod images;
mod iso;
mod make;
mod ntfs;
mod tables;

use std::path::{Path, PathBuf};

use diroxide_plugins::Manifest;
use diroxide_plugins::conformance::{Group, Verdict, run};

/// The built plugin program.
pub fn plugin_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diro-disk"))
}

/// The plugin's own manifest with its program replaced by `exe`, in a
/// folder of its own under `root`.
pub fn plugin_folder(root: &Path, exe: &Path) -> PathBuf {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin.toml")).unwrap();
    let command = format!("['{}']", exe.display());
    let manifest = manifest
        .replace("[\"./diro-disk.exe\"]", &command)
        .replace("[\"./diro-disk\"]", &command);
    let folder = root.join("disk");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("plugin.toml"), manifest).unwrap();
    folder
}

/// A folder for the test under the build's target folder, never the
/// system's temporary folder.
pub fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("disk-it-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("disk-")
        .tempdir_in(base)
        .unwrap()
}

#[test]
fn the_manifest_reads_claims_the_disk_images_and_keeps_the_40_cell_rule() {
    let text = include_str!("../../plugin.toml");
    let report = Manifest::check(text);
    assert_eq!(report.text().trim(), "Pass.", "{}", report.text());
    let manifest = Manifest::from_toml(text).unwrap();
    assert_eq!(manifest.id, "disk");
    assert_eq!(manifest.name, "Disk images");
    let provider = &manifest.providers[0];
    assert_eq!(provider.scheme, "disk");
    for claimed in [
        ".iso",
        ".vhd",
        ".vhdx",
        ".vdi",
        ".vmdk",
        ".qcow2",
        ".squashfs",
        ".img",
    ] {
        assert!(
            provider.extensions.iter().any(|e| e == claimed),
            "{claimed}"
        );
    }
    // The plugin's own list of what it reads inside itself is the
    // manifest's claim, one for one.
    let mut ours: Vec<&str> = diro_disk::provider::CLAIMED.to_vec();
    ours.sort_unstable();
    let mut theirs: Vec<&str> = provider.extensions.iter().map(String::as_str).collect();
    theirs.sort_unstable();
    assert_eq!(ours, theirs);
    assert_eq!(provider.wants, ["7z", "qemu-img"]);
    assert!(!provider.methods.contains("write_begin"), "read only");
}

#[test]
fn the_plugin_passes_the_conformance_suite() {
    let root = scratch();
    let report = run(&plugin_folder(root.path(), &plugin_exe()));
    assert!(report.passed(), "{}", report.text());
    for group in [
        Group::Manifest,
        Group::Accessibility,
        Group::Lifecycle,
        Group::Framing,
        Group::Methods,
        Group::Providers,
    ] {
        assert!(
            matches!(report.verdict(group), Some(Verdict::Pass(_))),
            "{group:?}: {}",
            report.text()
        );
    }
}
