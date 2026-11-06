//! Spike DS6 of alpha 4 (the contract step, P4; R29 1.2): a plugin's
//! provider claims `.iso`, and an image opens, reads, copies out and nests
//! through a zip, by the same doors the app uses. Enter on a local image
//! opens `IMAGE!` (the entry is listed as an archive, so the app makes
//! `place.into_archive()`), the panel lists it through the registry, F3
//! reads a member, F5 copies one out with the job engine, and
//! `bundle.zip!disk.iso!EFI/` reaches the plugin while the image itself,
//! `bundle.zip!disk.iso`, is read from the zip by the archive provider.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diroxide_archive::ArchiveProvider;
use diroxide_config::PluginSettings;
use diroxide_core::{EntryKind, JobId, JobKind, JobRequest, Place};
use diroxide_ops::{Engine, JobControl};
use diroxide_plugins::provider::PluginProvider;
use diroxide_plugins::testing::memfs::MemFs;
use diroxide_plugins::testing::{FakeLauncher, Mode, SkippingSink, plugin_info};
use diroxide_plugins::{Host, Launcher};
use diroxide_vfs::{OpContext, Registry};

/// The fake plugin's manifest: a read-only provider that claims `.iso`.
const CLAIMING: &str = "\n[permissions]\nfs = \"read\"\n\n[[provider]]\nscheme = \"disk\"\n\
    name = \"Disk images\"\nmethods = [\"connect\", \"list\", \"stat\", \"read\"]\n\
    extensions = [\".iso\"]\nwants = [\"7z\"]\n";

/// A folder for the test under the build's target folder, never the
/// system's temporary folder or a user's.
fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("plugins-it-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("claims-")
        .tempdir_in(base)
        .unwrap()
}

fn place(path: &Path) -> Place {
    Place::parse(&path.display().to_string()).unwrap()
}

fn read_all(registry: &Registry, at: &Place) -> Vec<u8> {
    let mut bytes = Vec::new();
    registry
        .resolve(at)
        .unwrap()
        .read(at, &OpContext::new())
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

fn names(registry: &Registry, at: &Place) -> Vec<String> {
    let mut names: Vec<String> = registry
        .resolve(at)
        .unwrap()
        .list(at, &OpContext::new())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    names.sort();
    names
}

/// A zip holding one member.
fn zip_with(path: &PathBuf, member: &str, bytes: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(member, zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(bytes).unwrap();
    zip.finish().unwrap();
}

#[test]
fn a_claimed_image_opens_reads_copies_out_and_nests_through_a_zip() {
    // The image's contents, as the plugin serves them.
    let mut image = MemFs::default();
    image.add_folder("/EFI");
    image.add_folder("/EFI/BOOT");
    image.add_file("/EFI/BOOT/BOOTX64.EFI", b"boot loader");
    image.add_file("/README.TXT", b"Read me first.");
    let launcher = FakeLauncher::new(Mode::Echo);
    launcher.with_memfs(Arc::new(Mutex::new(image)));
    let mut host =
        Host::new(&PluginSettings::default()).with_launcher(launcher as Arc<dyn Launcher>);
    host.add(plugin_info(CLAIMING));
    let host = Arc::new(host);

    // The registry as the app builds it: the local provider, the enabled
    // plugins' providers by scheme (their claims come with them), then the
    // archive provider installed over it.
    let mut base = Registry::with_local();
    let providers = PluginProvider::all(&host);
    assert_eq!(providers.len(), 1);
    for (scheme, provider) in providers {
        assert_eq!(provider.claimed_extensions(), [".iso"]);
        base.register(&scheme, provider);
    }
    assert_eq!(base.claims(), [".iso"]);
    let (registry, _archives) = ArchiveProvider::install(base, Default::default());

    let dir = scratch();
    std::fs::write(dir.path().join("fedora.iso"), b"the image's bytes").unwrap();
    zip_with(
        &dir.path().join("bundle.zip"),
        "disk.iso",
        b"ISO inside the zip",
    );

    // Enter: the local image is listed as an archive, so the app opens it
    // as a place, and the place lists through the plugin.
    let folder = place(dir.path());
    let listed = registry
        .resolve(&folder)
        .unwrap()
        .list(&folder, &OpContext::new())
        .unwrap();
    let iso = listed.iter().find(|e| e.name == "fedora.iso").unwrap();
    assert_eq!(iso.kind, EntryKind::Archive);
    let root = folder.join("fedora.iso").unwrap().into_archive();
    assert_eq!(registry.resolve(&root).unwrap().scheme(), "disk");
    assert_eq!(names(&registry, &root), ["EFI", "README.TXT"]);
    let efi = root.join("EFI").unwrap();
    assert_eq!(efi.name(), Some("EFI"));
    assert_eq!(names(&registry, &efi), ["BOOT"]);

    // F3: the viewer reads a member through the plugin.
    let readme = root.join("README.TXT").unwrap();
    assert_eq!(read_all(&registry, &readme), b"Read me first.");

    // F5: the job engine copies a member out to a local folder.
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let engine = Engine::new(Arc::clone(&registry));
    let request = JobRequest::new(JobId::new(1), JobKind::Copy, vec![readme]).to(place(&out));
    let plan = engine.plan(&request).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &SkippingSink);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(
        std::fs::read(out.join("README.TXT")).unwrap(),
        b"Read me first."
    );

    // Nesting: the image inside the zip is the plugin's to list, and its
    // bytes come from the zip, through the archive provider.
    let bundle = folder.join("bundle.zip").unwrap().into_archive();
    let inner = bundle.join("disk.iso").unwrap();
    assert_eq!(registry.resolve(&inner).unwrap().scheme(), "archive");
    assert_eq!(read_all(&registry, &inner), b"ISO inside the zip");
    let nested = inner.clone().into_archive().join("EFI").unwrap();
    assert_eq!(registry.resolve(&nested).unwrap().scheme(), "disk");
    assert_eq!(names(&registry, &nested), ["BOOT"]);
    host.shutdown_all(diroxide_plugins::host::QUIT_BUDGET);
}
