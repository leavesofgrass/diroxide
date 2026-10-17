//! Integration tests of the archive provider: its contract, the archives
//! built in memory, and the spikes of research/alpha2-plan.md part four.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod build;
mod hostile;
mod memory;
mod spike_hostile;
mod spike_listing;

use std::sync::Arc;

use diroxide_archive::{ArchiveFormat, ArchiveProvider, Limits};
use diroxide_core::Place;
use diroxide_vfs::{OpContext, Registry, VfsError};

#[test]
fn the_provider_registers_and_says_it_is_not_built() {
    let provider = Arc::new(ArchiveProvider::new(Limits::default()));
    let mut registry = Registry::with_local();
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    let place = Place::parse(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .join("course.zip")
        .unwrap()
        .into_archive();
    let found = registry.resolve(&place).unwrap();
    assert_eq!(found.scheme(), "archive");
    assert!(matches!(
        found.list(&place, &OpContext::new()),
        Err(VfsError::Unsupported { .. })
    ));
    assert_eq!(
        ArchiveFormat::from_name("course.zip"),
        Some(ArchiveFormat::Zip)
    );
}
