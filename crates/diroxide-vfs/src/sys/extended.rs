//! The local provider's extended items (alpha.7, R30 commit 48): the
//! `ExtendedAttributes` trait over each platform's calls, streams on
//! Windows (`streams.rs`), extended attributes on Unix (`xattr.rs`).
//! The provider finds it through `Provider::as_extended`, and a place
//! says it can through `Capabilities::extended_attributes`, set from the
//! volume's file system (`extended_on`).

use diroxide_core::Place;
use diroxide_core::format::human_size;

use crate::cancel::OpContext;
use crate::error::VfsError;
use crate::extended::{ExtendedAttributes, ExtendedItem};
use crate::local::LocalProvider;

impl ExtendedAttributes for LocalProvider {
    fn list_extended(&self, place: &Place, ctx: &OpContext) -> Result<Vec<ExtendedItem>, VfsError> {
        ctx.check()?;
        super::extended_list(&LocalProvider::path_of(place)?)
    }

    fn read_extended(
        &self,
        place: &Place,
        name: &str,
        max_bytes: usize,
        ctx: &OpContext,
    ) -> Result<Vec<u8>, VfsError> {
        ctx.check()?;
        let path = LocalProvider::path_of(place)?;
        super::extended_read(&path, name, max_bytes).map_err(|error| match error {
            VfsError::FileTooLarge => VfsError::LimitReached {
                what: "an extended item".to_owned(),
                limit: human_size(max_bytes as u64),
            },
            other => other,
        })
    }

    fn write_extended(
        &self,
        place: &Place,
        name: &str,
        value: &[u8],
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        ctx.check()?;
        super::extended_write(&LocalProvider::path_of(place)?, name, value)
    }

    fn remove_extended(&self, place: &Place, name: &str, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        super::extended_remove(&LocalProvider::path_of(place)?, name)
    }
}

#[cfg(test)]
mod tests {
    use crate::Provider;
    use crate::test_support::scratch;

    use super::*;

    /// The provider's door, on a file of the test's own: whatever the
    /// platform, an item written is listed, read back, refused past its
    /// limit in words, and removed.
    #[test]
    fn the_local_provider_reads_and_changes_extended_items() {
        let dir = scratch();
        let path = dir.path().join("own.txt");
        std::fs::write(&path, b"contents").unwrap();
        let place = Place::parse(&path.display().to_string()).unwrap();
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let extended = provider
            .as_extended()
            .expect("the local provider reads them");
        if !provider.capabilities(&place).extended_attributes {
            // A volume without them (FAT, a container's overlay without
            // user attributes) has nothing more to test.
            return;
        }
        let name = if cfg!(windows) {
            "diroxide.test"
        } else {
            "user.diroxide.test"
        };
        extended
            .write_extended(&place, name, b"twelve bytes", &ctx)
            .unwrap();
        let items = extended.list_extended(&place, &ctx).unwrap();
        let item = items.iter().find(|i| i.name == name).expect("listed");
        assert_eq!(item.size, 12);
        assert!(!item.system_managed);
        assert_eq!(
            extended.read_extended(&place, name, 64, &ctx).unwrap(),
            b"twelve bytes"
        );
        let refused = extended.read_extended(&place, name, 4, &ctx).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "it passes the limit for an extended item: 4 bytes"
        );
        extended.remove_extended(&place, name, &ctx).unwrap();
        assert!(
            !extended
                .list_extended(&place, &ctx)
                .unwrap()
                .iter()
                .any(|i| i.name == name)
        );
        let cancelled = OpContext::new();
        cancelled.cancel.cancel();
        assert_eq!(
            extended.list_extended(&place, &cancelled),
            Err(VfsError::Cancelled)
        );
    }
}
