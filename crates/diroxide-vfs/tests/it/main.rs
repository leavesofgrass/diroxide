//! Integration tests of the provider contract, through the registry.

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
