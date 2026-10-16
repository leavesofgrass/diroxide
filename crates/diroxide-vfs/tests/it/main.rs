//! Integration tests of the provider contract, through the registry.

mod names;

use diroxide_core::{EntryKind, Place};
use diroxide_vfs::{OpContext, Registry, VfsError, local_roots};

#[test]
fn the_registry_lists_a_local_folder() {
    let registry = Registry::with_local();
    let place = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let provider = registry.resolve(&place).unwrap();
    let entries = provider.list(&place, &OpContext::new()).unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.name == "tests" && e.kind == EntryKind::Folder)
    );
    assert!(provider.capabilities(&place).writable);
}

#[test]
fn the_local_provider_has_no_trash_yet() {
    let registry = Registry::with_local();
    let place = Place::parse(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .join("no-such-file")
        .unwrap();
    let provider = registry.resolve(&place).unwrap();
    assert!(matches!(
        provider.trash(&place, &OpContext::new()),
        Err(VfsError::Unsupported { .. })
    ));
}

#[test]
fn roots_are_full_places() {
    for root in local_roots() {
        assert!(root.is_root());
    }
}

/// A folder for a test that writes, under the build's own temporary
/// folder, never the user's.
#[allow(clippy::unwrap_used, reason = "a test helper: a failure is the test's")]
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("vfs-it-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

#[test]
fn a_committed_write_appears_and_a_dropped_one_leaves_nothing() {
    use diroxide_vfs::{LocalProvider, Provider, WriteMode};
    use std::io::Write;

    let folder = scratch();
    let place = Place::parse(folder.path().to_str().unwrap()).unwrap();
    let provider = LocalProvider::new();
    let ctx = OpContext::new();
    let target = place.join("report.txt").unwrap();

    let mut writer = provider.write(&target, WriteMode::CreateNew, &ctx).unwrap();
    writer.write_all(b"hello").unwrap();
    drop(writer);
    assert!(provider.list(&place, &ctx).unwrap().is_empty());

    let mut writer = provider.write(&target, WriteMode::CreateNew, &ctx).unwrap();
    writer.write_all(b"hello").unwrap();
    writer.commit(false).unwrap();
    let names: Vec<String> = provider
        .list(&place, &ctx)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["report.txt".to_owned()]);

    assert!(matches!(
        provider.write(&target, WriteMode::CreateNew, &ctx),
        Err(VfsError::AlreadyExists)
    ));
    let mut writer = provider.write(&target, WriteMode::Overwrite, &ctx).unwrap();
    writer.write_all(b"again").unwrap();
    writer.commit(true).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut provider.read(&target, &ctx).unwrap(), &mut text).unwrap();
    assert_eq!(text, "again");
}
