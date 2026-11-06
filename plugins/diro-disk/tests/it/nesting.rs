//! Nesting, loops and the record-claim guard through the real host (R30
//! commit 95): an image inside an image is read through the outer one's
//! reader; an image inside a zip through the real archive provider; a
//! folder that leads back is cut; a folder record that claims gigabytes is
//! refused before it is read.

use std::io::Write;
use std::sync::Arc;

use diroxide_archive::ArchiveProvider;
use diroxide_config::PluginSettings;
use diroxide_plugins::host::QUIT_BUDGET;
use diroxide_plugins::provider::PluginProvider;
use diroxide_plugins::{Host, Manifest, PluginInfo};
use diroxide_vfs::{OpContext, Registry};
use hadris_iso::write::InputEntry;

use crate::fixture::{Fixture, at, root_of, words};
use crate::make::{IsoNames, fat_volume, fedora_iso, gpt_with, guid, iso_with};
use crate::{plugin_exe, plugin_folder, scratch};

#[test]
fn an_iso_inside_a_fat_partition_opens_through_the_outer_image() {
    let f = Fixture::new();
    let iso = fedora_iso(IsoNames::Joliet);
    let fat = fat_volume(
        fatfs::FatType::Fat16,
        16_384,
        b"OUTER      ",
        &[],
        &[("boot.iso", &iso)],
    );
    let disk = gpt_with(12 * 1024 * 1024, &[(guid::BASIC_DATA, "", &fat)]);
    let root = f.image("disk.img", &disk);
    let inner = at(&root, &["partition-1", "boot.iso"]).into_archive();
    let mut names = f.names(&inner);
    names.sort();
    assert!(names.contains(&"EFI".to_owned()), "{names:?}");
    assert_eq!(
        f.read(&at(&inner, &["EFI", "BOOT", "BOOTX64.EFI"]))
            .unwrap(),
        b"boot loader"
    );
}

#[test]
fn an_iso_inside_a_zip_reads_through_the_zip() {
    let root = scratch();
    let folder = plugin_folder(root.path(), &plugin_exe());
    let manifest =
        Manifest::from_toml(&std::fs::read_to_string(folder.join("plugin.toml")).unwrap()).unwrap();
    // The host reads files through the local provider and the real
    // archive provider, as the app's does.
    let (files, _archives) = ArchiveProvider::install(Registry::with_local(), Default::default());
    let mut host = Host::new(&PluginSettings::default())
        .with_state_dir(root.path().join("state"))
        .with_registry(files);
    host.add(PluginInfo {
        manifest,
        folder,
        enabled: true,
    });
    let host = Arc::new(host);
    let mut base = Registry::with_local();
    for (scheme, provider) in PluginProvider::all(&host) {
        base.register(&scheme, provider);
    }
    let (registry, _) = ArchiveProvider::install(base, Default::default());
    let bundle = root.path().join("bundle.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&bundle).unwrap());
    zip.start_file("disk.iso", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&fedora_iso(IsoNames::Both)).unwrap();
    zip.finish().unwrap();
    let image = root_of(&bundle).join("disk.iso").unwrap().into_archive();
    let efi = image.join("EFI").unwrap();
    let provider = registry.resolve(&efi).unwrap();
    assert_eq!(provider.scheme(), "disk");
    let names: Vec<String> = provider
        .list(&efi, &OpContext::new())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["BOOT"]);
    host.shutdown_all(QUIT_BUDGET);
}

/// The byte offset of a plain ISO folder record named `name`, searched
/// in the image: a record whose length fits its name, whose flags say
/// folder, and whose both-endian extent agrees with itself (so a path
/// table entry with the same name is never taken).
fn record_of(iso: &[u8], name: &[u8]) -> usize {
    (33..iso.len().saturating_sub(name.len()))
        .find(|&at| {
            let start = at - 33;
            let len = usize::from(iso[start]);
            iso[at - 1] == name.len() as u8
                && &iso[at..at + name.len()] == name
                && (33 + name.len()..=34 + name.len()).contains(&len)
                && iso[start + 25] & 0x02 != 0
                && iso[start + 2..start + 6]
                    .iter()
                    .rev()
                    .eq(iso[start + 6..start + 10].iter())
        })
        .map(|at| at - 33)
        .expect("a folder record")
}

#[test]
fn a_folder_that_leads_back_is_cut_and_a_huge_record_refused() {
    let f = Fixture::new();
    let mut iso = iso_with(
        vec![InputEntry::directory(
            "A",
            vec![InputEntry::directory(
                "B",
                vec![InputEntry::file("X.TXT", b"x".to_vec())],
            )],
        )],
        IsoNames::Plain,
    );
    // B's record (inside A) is made to point at A's own extent: a loop.
    let a = record_of(&iso, b"A");
    let b = record_of(&iso, b"B");
    let a_extent: [u8; 8] = iso[a + 2..a + 10].try_into().unwrap();
    iso[b + 2..b + 10].copy_from_slice(&a_extent);
    let root = f.image("loop.iso", &iso);
    let names = f.names(&at(&root, &["A"]));
    assert!(names.is_empty(), "{names:?}");

    // A's record claims 4 GB: refused before a byte of it is read.
    let mut huge = iso_with(vec![InputEntry::directory("A", vec![])], IsoNames::Plain);
    let a = record_of(&huge, b"A");
    huge[a + 10..a + 14].copy_from_slice(&0xffff_f000u32.to_le_bytes());
    huge[a + 14..a + 18].copy_from_slice(&0xffff_f000u32.to_be_bytes());
    let root = f.image("huge.iso", &huge);
    let line = words(&f.list(&at(&root, &["A"])).unwrap_err());
    assert!(line.contains("folder record claims"), "{line}");
}
