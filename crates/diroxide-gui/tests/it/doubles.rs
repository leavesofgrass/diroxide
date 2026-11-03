//! Test doubles from the file system crate (a dev-dependency of the GUI
//! crate alone, lane A5's `xtask deps` allowance): a provider over the
//! local disk that refuses links for the privilege or one file's reading,
//! as Windows without Developer Mode and a locked file would, on every
//! system; and a trash of one item, so no test lists the owner's trash.
//! Adapted from the app crate's `tests/it/support.rs` (`TestTrash`).

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::ops::ControlFlow;
use std::path::PathBuf;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{
    AttributeChange, Capabilities, CopyHint, FileWriter, LinkRequest, LocalProvider, OpContext,
    Provider, PurgeCheck, ReadSeek, SpaceInfo, TrashBin, TrashCheck, TrashGate, TrashId,
    TrashedItem, VfsError, VolumeInfo, WatchHandle, WatchSink, WriteMode,
};

/// The local disk, with a link refused for the privilege and one file
/// that cannot be read.
#[derive(Debug, Default)]
pub struct Doubles {
    /// The local disk underneath.
    pub local: LocalProvider,
    /// Every new link is refused: "making a symbolic link needs a
    /// privilege this account does not have".
    pub refuse_links: bool,
    /// This file cannot be read, as a file another program holds.
    pub unreadable: Option<PathBuf>,
}

impl Doubles {
    fn refused(&self, place: &Place) -> bool {
        self.unreadable.as_ref().is_some_and(|path| {
            place
                .local_path()
                .is_some_and(|p| std::path::Path::new(p) == path.as_path())
        })
    }
}

impl Provider for Doubles {
    fn scheme(&self) -> &str {
        "file"
    }
    fn capabilities(&self, place: &Place) -> Capabilities {
        self.local.capabilities(place)
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.local.list(place, ctx)
    }
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.local.stat(place, ctx)
    }
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        if self.refused(place) {
            return Err(VfsError::PermissionDenied);
        }
        self.local.read(place, ctx)
    }
    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        if self.refused(place) {
            return Err(VfsError::PermissionDenied);
        }
        self.local.open_seekable(place, ctx)
    }
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.local.write(place, mode, ctx)
    }
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.create_dir(place, ctx)
    }
    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        if self.refuse_links {
            return Err(VfsError::NeedsPrivilege {
                what: "making a symbolic link".to_owned(),
            });
        }
        self.local.create_link(place, request, ctx)
    }
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.rename(from, to, ctx)
    }
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.remove(place, ctx)
    }
    fn trash_gate(&self, _place: &Place, _ctx: &OpContext) -> Result<TrashGate, VfsError> {
        // No test here sends anything to a trash.
        Err(VfsError::PermissionDenied)
    }
    fn trash(&self, _place: &Place, _ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        Err(VfsError::PermissionDenied)
    }
    fn restore(&self, _original: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::NotFound)
    }
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.local.set_attributes(place, change, ctx)
    }
    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        if self.refused(from) {
            // Read through `read`, so the refusal is met.
            return CopyHint::default();
        }
        self.local.copy_hint(from, to)
    }
    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        if self.refused(from) {
            return Err(VfsError::PermissionDenied);
        }
        self.local.copy_within(from, to, ctx)
    }
    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        self.local.volume(place, ctx)
    }
    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        self.local.space(place, ctx)
    }
    fn watch(&self, _place: &Place, _sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        Ok(None)
    }
}

/// A trash holding one item, so the trash browser lists something and
/// never the owner's trash.
#[derive(Debug)]
pub struct OneItemBin {
    item: TrashedItem,
}

impl OneItemBin {
    /// A trash holding `original`, deleted at no known time.
    pub fn holding(original: Place) -> Self {
        Self {
            item: TrashedItem::new(TrashId::new("item-1"), original, None),
        }
    }
}

impl TrashBin for OneItemBin {
    fn list(
        &self,
        _ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<TrashedItem>) -> ControlFlow<()>,
    ) -> Result<(), VfsError> {
        let _ = batch(vec![self.item.clone()]);
        Ok(())
    }
    fn restore(&self, _item: &TrashedItem, _ctx: &OpContext) -> Result<(), VfsError> {
        Ok(())
    }
    fn purge(&self, _item: &TrashedItem, _ctx: &OpContext) -> Result<PurgeCheck, VfsError> {
        Ok(PurgeCheck::Gone)
    }
}
