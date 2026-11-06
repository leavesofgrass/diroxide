//! The registry: which provider serves a place.
//!
//! **The claim route** (alpha.7, R29 1.2). A disk image is a place inside
//! an archive, `fedora.iso!EFI/BOOT`, so by its kind it would go to the
//! archive provider. A provider may claim container extensions instead
//! (the disk-image plugin claims `.iso`, `.vhd` and the rest, by its
//! manifest's `[[provider]] extensions`): a place inside an archive goes
//! to the provider that claims the extension of the container's name, the
//! name before the last `!`, else to the archive provider. A nested
//! place resolves by its innermost container, so `bundle.zip!disk.iso!EFI/`
//! goes to the claimant, which reads `bundle.zip!disk.iso` back through the
//! registry, from the zip. Only [`claimable_extension`]s are taken: never
//! one the archive provider reads in process. Nesting is counted here,
//! before any claimant is asked, with the archive limits' cap.

use std::collections::BTreeMap;
use std::sync::Arc;

use diroxide_core::place::claimable_extension;
use diroxide_core::{Place, PlaceKind};

use crate::archive::limits::Limits;
use crate::archive::names::nesting_depth;
use crate::error::VfsError;
use crate::local::LocalProvider;
use crate::provider::Provider;

/// Resolves a place to the provider that serves it: local places to the
/// local provider, places inside archives to the provider that claims the
/// container's extension or else the archive provider, remote places by
/// their scheme.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    local: Option<Arc<dyn Provider>>,
    archive: Option<Arc<dyn Provider>>,
    remote: BTreeMap<String, Arc<dyn Provider>>,
    /// Claimed container extensions, lowercase with the dot.
    claims: BTreeMap<String, Arc<dyn Provider>>,
    /// How many containers deep a claimed place may be; the archive
    /// limits' default when unset.
    max_nesting: Option<usize>,
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
    /// replacing any earlier one, and takes the container extensions it
    /// claims ([`Provider::claimed_extensions`]): an enabled plugin's
    /// claim is registered with its provider.
    pub fn register(&mut self, scheme: &str, provider: Arc<dyn Provider>) {
        for extension in provider.claimed_extensions() {
            self.claim(extension, Arc::clone(&provider));
        }
        self.remote.insert(scheme.to_owned(), provider);
    }

    /// Sends places inside containers with this extension (`.iso`) to
    /// `provider`, replacing an earlier claim. Ignored, and false, for an
    /// extension that is not [`claimable_extension`].
    pub fn claim(&mut self, extension: &str, provider: Arc<dyn Provider>) -> bool {
        let extension = normal_extension(extension);
        if !claimable_extension(&extension) {
            return false;
        }
        self.claims.insert(extension, provider);
        true
    }

    /// Sets how many containers deep a claimed place may be, the
    /// `archives.max_nesting` setting.
    pub fn set_max_nesting(&mut self, most: usize) {
        self.max_nesting = Some(most);
    }

    /// The remote schemes registered, in order.
    pub fn schemes(&self) -> Vec<&str> {
        self.remote.keys().map(String::as_str).collect()
    }

    /// The claimed container extensions, in order.
    pub fn claims(&self) -> Vec<&str> {
        self.claims.keys().map(String::as_str).collect()
    }

    /// The provider that claims a place inside a container, by the
    /// extension of the container's name, when any does.
    fn claimant(&self, place: &Place) -> Option<&Arc<dyn Provider>> {
        let (container, _) = place.archive()?;
        let name = container.name()?.to_lowercase();
        self.claims
            .iter()
            .filter(|(extension, _)| {
                name.strip_suffix(extension.as_str())
                    .is_some_and(|stem| !stem.is_empty())
            })
            .max_by_key(|(extension, _)| extension.len())
            .map(|(_, provider)| provider)
    }

    /// The provider that serves a place.
    pub fn resolve(&self, place: &Place) -> Result<Arc<dyn Provider>, VfsError> {
        let found = match place.kind() {
            PlaceKind::Local => self.local.as_ref(),
            PlaceKind::Archive => match self.claimant(place) {
                Some(claimant) => {
                    // The archive provider counts its own places with its
                    // configured cap; a claimant's are counted here, every
                    // container on the way included.
                    let most = self.max_nesting.unwrap_or(Limits::default().max_nesting);
                    if nesting_depth(place) > most {
                        return Err(VfsError::LimitReached {
                            what: "nesting".to_owned(),
                            limit: format!("{most} archives deep"),
                        });
                    }
                    Some(claimant)
                }
                None => self.archive.as_ref(),
            },
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

/// An extension as the registry keeps it: lowercase, with its dot.
fn normal_extension(extension: &str) -> String {
    let lower = extension.trim().to_lowercase();
    if lower.starts_with('.') {
        lower
    } else {
        format!(".{lower}")
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

    /// A provider that claims disk images, as the disk-image plugin's
    /// manifest does, known by its scheme.
    #[derive(Debug)]
    struct Claimant {
        scheme: &'static str,
        claims: Vec<String>,
    }

    impl Provider for Claimant {
        fn scheme(&self) -> &str {
            self.scheme
        }
        fn capabilities(&self, _: &Place) -> crate::Capabilities {
            crate::Capabilities::read_only()
        }
        fn list(
            &self,
            _: &Place,
            _: &crate::OpContext,
        ) -> Result<Vec<diroxide_core::Entry>, VfsError> {
            Ok(Vec::new())
        }
        fn stat(&self, _: &Place, _: &crate::OpContext) -> Result<diroxide_core::Entry, VfsError> {
            Err(VfsError::NotFound)
        }
        fn read(
            &self,
            _: &Place,
            _: &crate::OpContext,
        ) -> Result<Box<dyn std::io::Read + Send>, VfsError> {
            Err(VfsError::NotFound)
        }
        fn write(
            &self,
            _: &Place,
            _: crate::WriteMode,
            _: &crate::OpContext,
        ) -> Result<Box<dyn crate::FileWriter>, VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn create_dir(&self, _: &Place, _: &crate::OpContext) -> Result<(), VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn rename(&self, _: &Place, _: &Place, _: &crate::OpContext) -> Result<(), VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn remove(&self, _: &Place, _: &crate::OpContext) -> Result<(), VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn set_attributes(
            &self,
            _: &Place,
            _: &crate::AttributeChange,
            _: &crate::OpContext,
        ) -> Result<crate::AttributeChange, VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn claimed_extensions(&self) -> &[String] {
            &self.claims
        }
    }

    fn claimant(scheme: &'static str, claims: &[&str]) -> Arc<dyn Provider> {
        Arc::new(Claimant {
            scheme,
            claims: claims.iter().map(|c| (*c).to_owned()).collect(),
        })
    }

    /// The claim route (R29 1.2, spike DS6's registry half): a disk image
    /// goes to its claimant, a zip to the archive provider, a nested
    /// image by its innermost container; the archive provider's own
    /// formats cannot be claimed; a claimed place is counted for nesting.
    #[test]
    fn claimed_extensions_route_places_inside_containers() {
        let mut registry = Registry::with_local();
        registry.set_archive(claimant("archive", &[]));
        registry.register("disk", claimant("disk", &[".iso", ".VHD", "img", ".zip"]));
        assert_eq!(registry.claims(), [".img", ".iso", ".vhd"]);
        let root = Place::parse(if cfg!(windows) { "D:\\dl" } else { "/dl" }).unwrap();
        let scheme_in = |registry: &Registry, text: &str| {
            let place = Place::resolve(text, &root).unwrap();
            registry.resolve(&place).map(|p| p.scheme().to_owned())
        };
        let scheme = |text: &str| scheme_in(&registry, text);
        assert_eq!(scheme("fedora.iso!EFI/BOOT").unwrap(), "disk");
        assert_eq!(scheme("OLD.VHD!").unwrap(), "disk");
        assert_eq!(scheme("card.img!").unwrap(), "disk");
        assert_eq!(scheme("bundle.zip!readme.txt").unwrap(), "archive");
        // Through the zip first: the image inside it is the claimant's,
        // and the image itself, as a member, is the zip's.
        assert_eq!(scheme("bundle.zip!disk.iso!EFI/").unwrap(), "disk");
        assert_eq!(scheme("bundle.zip!disk.iso").unwrap(), "archive");
        // A name that only ends in the letters is not the extension.
        assert_eq!(scheme("bundle.zip!.iso").unwrap(), "archive");
        // Five containers deep is past the cap of four.
        let deep = scheme("a.zip!b.zip!c.zip!d.zip!e.iso!x").unwrap_err();
        assert_eq!(
            deep.to_string(),
            "it passes the limit for nesting: 4 archives deep"
        );
        assert_eq!(scheme("a.zip!b.zip!c.zip!d.iso!x").unwrap(), "disk");
        assert!(!registry.claim(".7z", claimant("other", &[])));
        registry.set_max_nesting(8);
        assert_eq!(
            scheme_in(&registry, "a.zip!b.zip!c.zip!d.zip!e.iso!x").unwrap(),
            "disk"
        );
    }
}
