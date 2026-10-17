//! A small world for the provider's tests: the fake provider as the local
//! disk (so archives and their hostile names live in memory), a registry,
//! and the archive provider connected to it.

#![allow(dead_code)]

use std::io::Read;
use std::sync::Arc;

use diroxide_archive::{Allowances, ArchiveError, ArchiveProvider, Limits, member_row};
use diroxide_core::{Entry, Place};
use diroxide_ops::FakeProvider;
use diroxide_vfs::{OpContext, Provider, Registry};

/// The fake disk, the registry and the provider.
pub struct World {
    /// The fake disk.
    pub fake: FakeProvider,
    /// The provider under test.
    pub provider: Arc<ArchiveProvider>,
    /// The registry it reaches the fake through.
    pub registry: Arc<Registry>,
    /// The folder the archives are put in.
    pub root: Place,
}

/// The fake's folder for archives: a place in the platform's style that
/// no disk is ever asked about.
pub fn fake_folder() -> Place {
    Place::parse(if cfg!(windows) {
        r"D:\fake\archives"
    } else {
        "/fake/archives"
    })
    .unwrap()
}

impl World {
    /// A world with these limits.
    pub fn new(limits: Limits) -> Self {
        let fake = FakeProvider::new();
        let root = fake_folder();
        fake.add_folder(&root);
        let provider = Arc::new(ArchiveProvider::new(limits));
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        registry.set_archive(provider.clone());
        let registry = Arc::new(registry);
        provider.connect(&registry);
        Self {
            fake,
            provider,
            registry,
            root,
        }
    }

    /// A world with the default limits.
    pub fn plain() -> Self {
        Self::new(Limits::default())
    }

    /// Puts an archive on the fake disk and gives its place.
    pub fn put(&self, name: &str, bytes: &[u8]) -> Place {
        let place = self.root.join(name).unwrap();
        self.fake.add_file(&place, bytes);
        place
    }

    /// A place inside an archive: `member` may hold slashes, and `!` after
    /// a nested archive's name.
    pub fn inside(&self, archive: &Place, member: &str) -> Place {
        Place::parse(&format!("{archive}!{member}")).unwrap()
    }

    /// The root listing of an archive, as rows.
    pub fn rows(&self, place: &Place) -> Result<Vec<String>, String> {
        let entries = self
            .provider
            .list(place, &OpContext::new())
            .map_err(|e| e.to_string())?;
        Ok(entries.iter().map(member_row).collect())
    }

    /// The entries of a folder inside an archive.
    pub fn list(&self, place: &Place) -> Vec<Entry> {
        self.provider.list(place, &OpContext::new()).unwrap()
    }

    /// Reads a member whole through the generic `Provider::read`.
    pub fn read(&self, place: &Place) -> Result<Vec<u8>, String> {
        let mut reader = self
            .provider
            .read(place, &OpContext::new())
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        reader
            .read_to_end(&mut out)
            .map_err(|e| diroxide_archive::guard::from_io(e).to_string())?;
        Ok(out)
    }

    /// Reads a member through the provider's own call, with answers.
    pub fn read_with(&self, place: &Place, allow: Allowances) -> Result<Vec<u8>, ArchiveError> {
        let mut reader = self
            .provider
            .read_member(place, &OpContext::new(), allow, None)?;
        let mut out = Vec::new();
        match reader.read_to_end(&mut out) {
            Ok(_) => Ok(out),
            Err(e) => match diroxide_archive::guard::limit_of(&e) {
                Some(hit) => Err(hit.clone().into()),
                None => Err(diroxide_archive::guard::from_io(e).into()),
            },
        }
    }
}
