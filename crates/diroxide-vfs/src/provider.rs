//! The provider trait: everything a kind of place can do.

use std::fmt;
use std::io::{Read, Write};

use diroxide_core::{Attributes, Entry, Place, Timestamp};

use crate::cancel::OpContext;
use crate::capabilities::Capabilities;
use crate::error::VfsError;

/// How a write opens its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WriteMode {
    /// Make a new entry; fail if the name is taken. The safe default.
    #[default]
    CreateNew,
    /// Replace an existing entry's contents, or make it.
    Overwrite,
}

/// A change to an entry's attributes and times. `None` leaves a value as
/// it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttributeChange {
    /// New permissions or attributes.
    pub attributes: Option<Attributes>,
    /// A new modification time.
    pub modified: Option<Timestamp>,
    /// A new access time.
    pub accessed: Option<Timestamp>,
    /// A new owner, by user name.
    pub owner: Option<String>,
    /// A new group, by name.
    pub group: Option<String>,
}

/// How a provider can copy within itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CopyHint {
    /// No fast path: the job engine reads and writes the bytes.
    #[default]
    Stream,
    /// A copy-on-write clone (reflink) is possible.
    Reflink,
    /// The server can copy without sending the bytes through diroxide.
    ServerSide,
}

/// What changed in a watched place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// An entry appeared.
    Created(Place),
    /// An entry changed.
    Changed(Place),
    /// An entry went away.
    Removed(Place),
    /// Too much changed to say; list the place again.
    Rescan,
}

/// Receives watch events, on whatever thread the provider uses.
pub type WatchSink = Box<dyn Fn(WatchEvent) + Send + Sync>;

/// Keeps a watch alive; dropping it stops the watch.
pub struct WatchHandle {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl WatchHandle {
    /// A handle that runs `stop` when dropped.
    pub fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        Self {
            stop: Some(Box::new(stop)),
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

impl fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WatchHandle").finish_non_exhaustive()
    }
}

/// A kind of place: the local disk, archives, SFTP, or a plugin's protocol.
///
/// Every method blocks, so callers run them on worker threads, never on the
/// input thread. Every method takes an [`OpContext`], checks its token
/// between steps, and reports progress where it can. Operations act on one
/// entry; the job engine does the walking.
pub trait Provider: Send + Sync + fmt::Debug {
    /// The scheme this provider serves: `file`, `archive`, `sftp`, or a
    /// plugin's.
    fn scheme(&self) -> &str;

    /// What can be done at a place.
    fn capabilities(&self, place: &Place) -> Capabilities;

    /// The entries in a folder, in no particular order.
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError>;

    /// One entry's details, without following a final link.
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError>;

    /// Opens a file for reading.
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError>;

    /// Opens a file for writing.
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn Write + Send>, VfsError>;

    /// Makes one folder; its parent must exist.
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError>;

    /// Renames or moves one entry within this provider. Fails if the target
    /// name is taken.
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError>;

    /// Removes one file, link or empty folder for good. The job engine
    /// lists everything first and uses the trash by default.
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError>;

    /// Moves one entry to the trash. Unsupported unless the capabilities
    /// say `trash`.
    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let _ = (place, ctx);
        Err(VfsError::unsupported(self.scheme(), "move to the trash"))
    }

    /// Changes attributes and times.
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<(), VfsError>;

    /// Whether a copy from `from` to `to`, both served by this provider,
    /// has a fast path.
    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        let _ = (from, to);
        CopyHint::Stream
    }

    /// Copies one file by the fast path that [`Provider::copy_hint`]
    /// named. Unsupported when the hint is `Stream`.
    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let _ = (from, to, ctx);
        Err(VfsError::unsupported(
            self.scheme(),
            "copy without streaming",
        ))
    }

    /// Starts watching a folder. `Ok(None)` when this provider cannot.
    fn watch(&self, place: &Place, sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        let _ = (place, sink);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn dropping_a_watch_handle_stops_it() {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        let handle = WatchHandle::new(move || flag.store(true, Ordering::Relaxed));
        assert!(!stopped.load(Ordering::Relaxed));
        drop(handle);
        assert!(stopped.load(Ordering::Relaxed));
    }
}
