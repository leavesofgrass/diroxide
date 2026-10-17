//! Provider registration, capabilities per kind, titles and status words,
//! and the read-only refusals in words (commit 15).

use std::io::Write;

use diroxide_archive::{
    ArchiveFormat, ArchiveProvider, Limits, READ_ONLY, capabilities_for, read_only_refusal,
};
use diroxide_vfs::{
    AttributeChange, LinkKind, LinkRequest, OpContext, Provider, Registry, WriteMode,
};

use crate::build::zip_of;
use crate::world::World;

#[test]
fn the_app_can_ask_the_registry_for_a_zip_place() {
    let (registry, provider) = ArchiveProvider::install(Registry::with_local(), Limits::default());
    let zip = diroxide_core::Place::parse(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .join("course.zip")
        .unwrap();
    let inside = zip.into_archive().join("week1").unwrap();
    assert_eq!(registry.resolve(&inside).unwrap().scheme(), "archive");
    assert_eq!(provider.limits(), Limits::default());
}

#[test]
fn capabilities_by_kind() {
    let table = [
        (ArchiveFormat::Zip, false, true),
        (ArchiveFormat::SevenZ, false, true),
        (ArchiveFormat::SevenZ, true, false),
        (ArchiveFormat::Tar, false, true),
        (ArchiveFormat::TarGz, false, false),
        (ArchiveFormat::TarXz, false, false),
        (ArchiveFormat::Xz, false, false),
    ];
    for (format, solid, random) in table {
        let capabilities = capabilities_for(format, solid);
        assert_eq!(
            capabilities.random_read, random,
            "{format:?}, solid {solid}"
        );
        assert!(!capabilities.writable && !capabilities.trash && !capabilities.links);
        assert!(!capabilities.attributes && !capabilities.watch);
        assert!(capabilities.case_sensitive);
        assert!(!capabilities.can_link());
    }
}

#[test]
fn every_change_is_refused_in_words() {
    let world = World::plain();
    let zip = world.put("course.zip", &zip_of(&[("notes.txt", b"hello")]));
    let inside = world.inside(&zip, "notes.txt");
    let ctx = OpContext::new();
    let provider = &world.provider;
    let refusals = [
        provider
            .write(&inside, WriteMode::CreateNew, &ctx)
            .map(|mut w| {
                let _ = w.write_all(b"x");
            }),
        provider.create_dir(&world.inside(&zip, "new"), &ctx),
        provider.rename(&inside, &world.inside(&zip, "renamed.txt"), &ctx),
        provider.remove(&inside, &ctx),
        provider
            .set_attributes(&inside, &AttributeChange::default(), &ctx)
            .map(|_| ()),
        provider.trash(&inside, &ctx).map(|_| ()),
        provider.create_link(
            &world.inside(&zip, "link"),
            &LinkRequest {
                kind: LinkKind::Symbolic,
                target: "notes.txt".to_owned(),
                target_is_folder: false,
            },
            &ctx,
        ),
    ];
    for refusal in refusals {
        let error = refusal.unwrap_err();
        assert_eq!(
            read_only_refusal(&error).unwrap().text,
            READ_ONLY,
            "{error}"
        );
    }
    // The capabilities agree, read from the cache once the archive is open.
    let _ = world.list(&zip.clone().into_archive());
    let capabilities = provider.capabilities(&inside);
    assert!(!capabilities.writable && capabilities.random_read);
}

#[test]
fn titles_and_the_opened_line() {
    let world = World::plain();
    let zip = world.put(
        "course.zip",
        &zip_of(&[("week1/", b""), ("week1/a.txt", b"a")]),
    );
    let root = zip.clone().into_archive();
    let folder = world.root.to_string();
    assert_eq!(
        world.provider.title(&root),
        format!("course.zip, archive, in {folder}")
    );
    let week1 = root.join("week1").unwrap();
    assert_eq!(world.provider.title(&week1), "week1, in course.zip");
    let opened = world
        .provider
        .open_archive(
            &zip,
            &OpContext::new(),
            diroxide_archive::Allowances::default(),
        )
        .unwrap();
    assert_eq!(opened.opened_line(), "Opened course.zip: 2 items, 1 byte.");
    assert!(diroxide_core::format::within_cells(
        &opened.opened_line(),
        "Opened course.zip"
    ));
}
