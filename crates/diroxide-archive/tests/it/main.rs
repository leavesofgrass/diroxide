//! Integration tests of the archive provider: its contract, the archives
//! built in memory, and the spikes of research/alpha2-plan.md part four.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod build;
mod cross_check;
mod hostile;
mod jobs;
mod memory;
mod nested;
mod odd_names;
mod pack_tar;
mod pack_zip;
mod prompt;
mod registration;
mod seeds;
mod sevenz_read;
mod spike_hostile;
mod spike_listing;
mod spike_password;
mod spike_sequential;
mod split_sets;
mod tar_read;
mod test_run;
mod unpack_plan;
mod unpack_run;
mod world;
mod xz_read;
mod zip_read;

use std::sync::Arc;

use diroxide_archive::{ArchiveFormat, ArchiveProvider, Limits};
use diroxide_core::Place;
use diroxide_vfs::{OpContext, Registry, VfsError};

#[test]
fn the_provider_registers_and_a_missing_archive_is_not_found() {
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
        Err(VfsError::NotFound)
    ));
    assert_eq!(
        ArchiveFormat::from_name("course.zip"),
        Some(ArchiveFormat::Zip)
    );
}
