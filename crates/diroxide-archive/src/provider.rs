//! The provider for places inside archives.

use std::io::Read;
use std::sync::{Arc, RwLock, Weak};

use diroxide_core::{Entry, Place};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, VfsError, WriteMode,
};

use crate::limits::Limits;
use crate::names::nesting_depth;

/// The provider for places inside archives.
#[derive(Debug, Default)]
pub struct ArchiveProvider {
    limits: Limits,
    registry: RwLock<Weak<Registry>>,
}

impl ArchiveProvider {
    /// A provider with these limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            registry: RwLock::new(Weak::new()),
        }
    }

    /// The limits.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Connects the provider to the registry it reaches archive files
    /// through, after the registry is built.
    pub fn connect(&self, registry: &Arc<Registry>) {
        if let Ok(mut held) = self.registry.write() {
            *held = Arc::downgrade(registry);
        }
    }

    fn not_built(&self, operation: &str) -> VfsError {
        VfsError::unsupported(self.scheme(), &format!("{operation} yet"))
    }
}

impl Provider for ArchiveProvider {
    fn scheme(&self) -> &str {
        "archive"
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        Capabilities::read_only()
    }

    fn list(&self, place: &Place, _ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        if nesting_depth(place) > self.limits.max_nesting {
            return Err(self.not_built("open archives nested this deep"));
        }
        Err(self.not_built("list archives"))
    }

    fn stat(&self, _place: &Place, _ctx: &OpContext) -> Result<Entry, VfsError> {
        Err(self.not_built("read archive members"))
    }

    fn read(&self, _place: &Place, _ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Err(self.not_built("read archive members"))
    }

    fn write(
        &self,
        _place: &Place,
        _mode: WriteMode,
        _ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(self.not_built("write into archives"))
    }

    fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("make folders in archives"))
    }

    fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("rename in archives"))
    }

    fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("delete from archives"))
    }

    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(self.not_built("change attributes in archives"))
    }
}
