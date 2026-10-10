//! The registry: which provider serves a place.

use std::collections::BTreeMap;
use std::sync::Arc;

use diroxide_core::{Place, PlaceKind};

use crate::error::VfsError;
use crate::local::LocalProvider;
use crate::provider::Provider;

/// Resolves a place to the provider that serves it: local places to the
/// local provider, places inside archives to the archive provider, remote
/// places by their scheme.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    local: Option<Arc<dyn Provider>>,
    archive: Option<Arc<dyn Provider>>,
    remote: BTreeMap<String, Arc<dyn Provider>>,
}

impl Registry {
    /// An empty registry, which serves nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the local provider and nothing else.
    pub fn with_local() -> Self {
        let mut registry = Self::new();
        registry.set_local(Arc::new(LocalProvider::new()));
        registry
    }

    /// Sets the provider for local places.
    pub fn set_local(&mut self, provider: Arc<dyn Provider>) {
        self.local = Some(provider);
    }

    /// Sets the provider for places inside archives.
    pub fn set_archive(&mut self, provider: Arc<dyn Provider>) {
        self.archive = Some(provider);
    }

    /// Registers the provider for a remote scheme, such as `sftp`,
    /// replacing any earlier one.
    pub fn register(&mut self, scheme: &str, provider: Arc<dyn Provider>) {
        self.remote.insert(scheme.to_owned(), provider);
    }

    /// The remote schemes registered, in order.
    pub fn schemes(&self) -> Vec<&str> {
        self.remote.keys().map(String::as_str).collect()
    }

    /// The provider that serves a place.
    pub fn resolve(&self, place: &Place) -> Result<Arc<dyn Provider>, VfsError> {
        let found = match place.kind() {
            PlaceKind::Local => self.local.as_ref(),
            PlaceKind::Archive => self.archive.as_ref(),
            PlaceKind::Remote => place
                .remote()
                .and_then(|remote| self.remote.get(remote.scheme())),
        };
        found.cloned().ok_or_else(|| {
            let what = match place.remote() {
                Some(remote) => remote.scheme().to_owned(),
                None => place.kind().word().to_owned(),
            };
            VfsError::NoProvider(what)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_resolve_by_kind_and_scheme() {
        let registry = Registry::with_local();
        let local = Place::parse(if cfg!(windows) { "C:\\" } else { "/" }).unwrap();
        assert_eq!(registry.resolve(&local).unwrap().scheme(), "file");
        let remote = Place::parse("sftp://example.org/x").unwrap();
        let err = registry.resolve(&remote).unwrap_err();
        assert_eq!(err.to_string(), "nothing is set up to open sftp places");
        let inside = local.join("a.zip").unwrap().into_archive();
        assert!(matches!(
            registry.resolve(&inside),
            Err(VfsError::NoProvider(_))
        ));
    }
}
