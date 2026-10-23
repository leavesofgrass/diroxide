//! The local provider's resumable write (R25 commit 69): a kept write
//! leaves its `.diro-part-` file when it is dropped without its commit, and
//! a later write reopens it at its end. A network share that drops comes
//! back to it, and the engine's resume is tested without a network.
//!
//! Only an unfinished name in the target's own folder is ever reopened,
//! read or removed here; anything else is refused as unsafe.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use diroxide_core::Place;

use super::LocalProvider;
use super::writer::LocalWriter;
use crate::cancel::OpContext;
use crate::error::VfsError;
use crate::names;
use crate::provider::{FileWriter, ResumableWrite, WriteMode};

/// The path of an unfinished file of ours; anything else is refused.
fn unfinished_path(place: &Place) -> Result<PathBuf, VfsError> {
    let name = place.name().unwrap_or_default();
    if !names::is_unfinished(name) {
        return Err(VfsError::Unsafe {
            why: "only an unfinished file is reopened or removed here".to_owned(),
        });
    }
    LocalProvider::path_of(place)
}

impl ResumableWrite for LocalProvider {
    fn write_kept(
        &self,
        target: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        ctx.check()?;
        let path = Self::path_of(target)?;
        Self::refuse_name(target, &path)?;
        if mode == WriteMode::CreateNew && fs::symlink_metadata(&path).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        Ok(Box::new(LocalWriter::start_kept(path, mode)?))
    }

    fn reopen(
        &self,
        unfinished: &Place,
        target: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<(Box<dyn FileWriter>, u64), VfsError> {
        ctx.check()?;
        if unfinished.parent() != target.parent() {
            return Err(VfsError::Unsafe {
                why: "the unfinished file is not beside its target".to_owned(),
            });
        }
        let part = unfinished_path(unfinished)?;
        let (writer, length) = LocalWriter::reopen(part, Self::path_of(target)?, mode)?;
        Ok((Box::new(writer), length))
    }

    fn unfinished_len(&self, unfinished: &Place, ctx: &OpContext) -> Result<u64, VfsError> {
        ctx.check()?;
        let metadata = fs::symlink_metadata(unfinished_path(unfinished)?)?;
        if !metadata.is_file() {
            return Err(VfsError::NotFound);
        }
        Ok(metadata.len())
    }

    fn read_unfinished(
        &self,
        unfinished: &Place,
        offset: u64,
        length: usize,
        ctx: &OpContext,
    ) -> Result<Vec<u8>, VfsError> {
        ctx.check()?;
        let mut file = File::open(unfinished_path(unfinished)?)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::with_capacity(length.min(1 << 20));
        file.take(length as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn discard(&self, unfinished: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        Ok(fs::remove_file(unfinished_path(unfinished)?)?)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::provider::Provider;
    use crate::test_support::scratch;

    fn place(path: &std::path::Path) -> Place {
        Place::parse(path.to_str().unwrap()).unwrap()
    }

    #[test]
    fn a_kept_write_stays_reopens_at_its_end_and_commits() {
        let dir = scratch();
        let provider = LocalProvider::new();
        let target = place(&dir.path().join("big.bin"));
        let ctx = OpContext::new();
        assert!(provider.capabilities(&target).resume);
        let resumable = provider.as_resumable().unwrap();
        let mut writer = resumable
            .write_kept(&target, WriteMode::CreateNew, &ctx)
            .unwrap();
        let part = writer.unfinished_place().unwrap();
        writer.write_all(b"first half, ").unwrap();
        drop(writer);
        assert!(
            !dir.path().join("big.bin").exists(),
            "nothing at the target"
        );
        assert_eq!(resumable.unfinished_len(&part, &ctx).unwrap(), 12);
        assert_eq!(
            resumable.read_unfinished(&part, 6, 4, &ctx).unwrap(),
            b"half"
        );
        let (mut again, at) = resumable
            .reopen(&part, &target, WriteMode::CreateNew, &ctx)
            .unwrap();
        assert_eq!(at, 12);
        again.write_all(b"second half").unwrap();
        again.commit(true).unwrap();
        assert_eq!(
            fs::read(dir.path().join("big.bin")).unwrap(),
            b"first half, second half"
        );
        assert!(resumable.unfinished_len(&part, &ctx).is_err(), "renamed");
    }

    #[test]
    fn only_an_unfinished_file_beside_its_target_is_touched() {
        let dir = scratch();
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let keep = dir.path().join("keep.txt");
        fs::write(&keep, b"mine").unwrap();
        let resumable = provider.as_resumable().unwrap();
        assert!(matches!(
            resumable.discard(&place(&keep), &ctx),
            Err(VfsError::Unsafe { .. })
        ));
        assert!(keep.exists());
        let target = place(&dir.path().join("t.bin"));
        let writer = resumable
            .write_kept(&target, WriteMode::CreateNew, &ctx)
            .unwrap();
        let part = writer.unfinished_place().unwrap();
        drop(writer);
        fs::create_dir(dir.path().join("other")).unwrap();
        let elsewhere = place(&dir.path().join("other").join("t.bin"));
        assert!(matches!(
            resumable.reopen(&part, &elsewhere, WriteMode::CreateNew, &ctx),
            Err(VfsError::Unsafe { .. })
        ));
        resumable.discard(&part, &ctx).unwrap();
        assert!(resumable.unfinished_len(&part, &ctx).is_err());
        assert!(
            provider
                .unfinished(&place(dir.path()), &ctx)
                .unwrap()
                .is_empty()
        );
    }
}
