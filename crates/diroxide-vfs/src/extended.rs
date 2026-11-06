//! Extended attributes and alternate data streams (alpha.7, R12's
//! contract request 2): the names a file carries beside its contents.
//! The internet mark (Windows' `Zone.Identifier` stream, macOS's
//! `com.apple.quarantine`, Linux's `user.xdg.origin.url`), Finder tags
//! and SELinux labels are read through it; the editor comes in a later
//! alpha (R28, disagreement 22).
//!
//! An extension beside [`Provider`](crate::Provider), found through
//! [`Provider::as_extended`](crate::Provider::as_extended), the pattern of
//! [`StreamingList`](crate::StreamingList); a place says it can with
//! [`Capabilities::extended_attributes`](crate::Capabilities::extended_attributes).
//! A remote place has none, and the details say "not available here".
//!
//! A stream's name never becomes part of a path typed by hand: the local
//! provider builds the one path with a colon in its platform module.

use diroxide_core::Place;

use crate::cancel::OpContext;
use crate::error::VfsError;

/// What carries an extended item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtendedKind {
    /// An extended attribute (Linux, macOS, the BSDs).
    Attribute,
    /// An alternate data stream (Windows, NTFS).
    Stream,
}

/// One extended attribute or stream.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExtendedItem {
    /// Its name as the system gives it: `user.xdg.origin.url`,
    /// `Zone.Identifier`.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// What carries it.
    pub kind: ExtendedKind,
    /// Whether the system manages it (a `security.*` label, a stream the
    /// system writes): listed, and marked read only in words.
    pub system_managed: bool,
}

impl ExtendedItem {
    /// An item of a kind, its name and size, not managed by the system.
    pub fn new(name: impl Into<String>, size: u64, kind: ExtendedKind) -> Self {
        Self {
            name: name.into(),
            size,
            kind,
            system_managed: false,
        }
    }

    /// The same item, managed by the system.
    pub fn managed(mut self) -> Self {
        self.system_managed = true;
        self
    }
}

/// Reads and changes a file's extended attributes or streams.
pub trait ExtendedAttributes: Send + Sync {
    /// The items the entry at `place` carries, in the system's order.
    fn list_extended(&self, place: &Place, ctx: &OpContext) -> Result<Vec<ExtendedItem>, VfsError>;

    /// The bytes of one item, at most `max_bytes` of them; a longer item
    /// is refused with [`VfsError::LimitReached`], never cut short.
    fn read_extended(
        &self,
        place: &Place,
        name: &str,
        max_bytes: usize,
        ctx: &OpContext,
    ) -> Result<Vec<u8>, VfsError>;

    /// Sets one item to these bytes, making it when it is missing.
    fn write_extended(
        &self,
        place: &Place,
        name: &str,
        value: &[u8],
        ctx: &OpContext,
    ) -> Result<(), VfsError>;

    /// Removes one item: the internet mark's removal is this call.
    fn remove_extended(&self, place: &Place, name: &str, ctx: &OpContext) -> Result<(), VfsError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_the_users_unless_marked() {
        let item = ExtendedItem::new("Zone.Identifier", 26, ExtendedKind::Stream);
        assert!(!item.system_managed);
        assert!(
            ExtendedItem::new("security.selinux", 20, ExtendedKind::Attribute)
                .managed()
                .system_managed
        );
    }
}
