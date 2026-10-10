//! The local provider: the disks of this machine through `std::fs`.
//!
//! Phase 0 bodies are deliberately naive: no Windows attributes, no long
//! paths, no owners, no trash, no watching. Wave 1 (W1a) replaces them,
//! with platform calls in the `sys` module.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

use diroxide_core::place::ARCHIVE_EXTENSIONS;
use diroxide_core::{Entry, EntryKind, Place, PlaceError, Timestamp};

use crate::cancel::{OpContext, ProgressUpdate};
use crate::capabilities::Capabilities;
use crate::error::VfsError;
use crate::names;
use crate::provider::{AttributeChange, Provider, WriteMode};

/// How often a listing reports progress, in entries.
const PROGRESS_EVERY: u64 = 1000;

/// The provider for local places.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalProvider {
    _private: (),
}

impl LocalProvider {
    /// The local provider.
    pub fn new() -> Self {
        Self::default()
    }

    fn path_of(place: &Place) -> Result<PathBuf, VfsError> {
        let path = place
            .local_path()
            .ok_or_else(|| VfsError::unsupported("file", "open places that are not local"))?;
        if cfg!(windows) && path.starts_with('/') {
            return Err(PlaceError::NoDrive(path.to_owned()).into());
        }
        if cfg!(not(windows)) && !path.starts_with('/') {
            return Err(PlaceError::NotAbsolute(path.to_owned()).into());
        }
        Ok(PathBuf::from(path))
    }

    fn entry_from(name: String, metadata: &Metadata) -> Entry {
        let file_type = metadata.file_type();
        let is_archive = {
            let lower = name.to_lowercase();
            ARCHIVE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        };
        let kind = if file_type.is_symlink() {
            EntryKind::Link
        } else if file_type.is_dir() {
            EntryKind::Folder
        } else if file_type.is_file() && is_archive {
            EntryKind::Archive
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Special
        };
        let mut entry = Entry::new(name, kind);
        if file_type.is_file() {
            entry.size = Some(metadata.len());
        }
        entry.modified = metadata.modified().ok().map(Timestamp::from_system_time);
        entry.accessed = metadata.accessed().ok().map(Timestamp::from_system_time);
        entry.hidden = entry.name.starts_with('.');
        entry.needs_care = names::needs_care(&entry.name);
        entry
    }
}

impl Provider for LocalProvider {
    fn scheme(&self) -> &str {
        "file"
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        Capabilities {
            writable: true,
            trash: false,
            links: cfg!(unix),
            attributes: false,
            case_sensitive: cfg!(target_os = "linux"),
            max_name_length: Some(255),
            reserved_names: cfg!(windows),
            watch: false,
        }
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        let path = Self::path_of(place)?;
        let mut entries = Vec::new();
        for item in fs::read_dir(&path)? {
            ctx.check()?;
            let item = item?;
            let (name, lossy) = match item.file_name().into_string() {
                Ok(name) => (name, false),
                Err(raw) => (raw.to_string_lossy().into_owned(), true),
            };
            let mut entry = match item.metadata() {
                Ok(metadata) => Self::entry_from(name, &metadata),
                Err(_) => Entry::new(name, EntryKind::Special),
            };
            entry.name_not_unicode = lossy;
            entries.push(entry);
            let done = entries.len() as u64;
            if done.is_multiple_of(PROGRESS_EVERY) {
                ctx.progress.report(ProgressUpdate {
                    items_done: done,
                    ..ProgressUpdate::default()
                });
            }
        }
        Ok(entries)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        let metadata = fs::symlink_metadata(&path)?;
        let name = place
            .name()
            .map_or_else(|| place.to_string(), str::to_owned);
        Ok(Self::entry_from(name, &metadata))
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        ctx.check()?;
        Ok(Box::new(File::open(Self::path_of(place)?)?))
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn Write + Send>, VfsError> {
        ctx.check()?;
        let mut options = OpenOptions::new();
        options.write(true);
        match mode {
            WriteMode::CreateNew => options.create_new(true),
            WriteMode::Overwrite => options.create(true).truncate(true),
        };
        Ok(Box::new(options.open(Self::path_of(place)?)?))
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        Ok(fs::create_dir(Self::path_of(place)?)?)
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let target = Self::path_of(to)?;
        if fs::symlink_metadata(&target).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        Ok(fs::rename(Self::path_of(from)?, target)?)
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            Ok(fs::remove_dir(&path)?)
        } else {
            Ok(fs::remove_file(&path)?)
        }
    }

    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<(), VfsError> {
        Err(VfsError::unsupported("file", "change attributes yet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> Place {
        Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap()
    }

    #[test]
    fn lists_its_own_crate_folder() {
        let entries = LocalProvider::new()
            .list(&here(), &OpContext::new())
            .unwrap();
        let manifest = entries.iter().find(|e| e.name == "Cargo.toml").unwrap();
        assert_eq!(manifest.kind, EntryKind::File);
        assert!(manifest.size.unwrap_or(0) > 0);
        let src = entries.iter().find(|e| e.name == "src").unwrap();
        assert_eq!(src.kind, EntryKind::Folder);
    }

    #[test]
    fn a_cancelled_listing_stops() {
        let ctx = OpContext::new();
        ctx.cancel.cancel();
        let result = LocalProvider::new().list(&here(), &ctx);
        assert!(matches!(result, Err(VfsError::Cancelled)));
    }

    #[test]
    fn stat_and_read_a_file() {
        let manifest = here().join("Cargo.toml").unwrap();
        let provider = LocalProvider::new();
        let entry = provider.stat(&manifest, &OpContext::new()).unwrap();
        assert_eq!(entry.name, "Cargo.toml");
        let mut text = String::new();
        provider
            .read(&manifest, &OpContext::new())
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("diroxide-vfs"));
        let missing = here().join("no-such-file").unwrap();
        assert_eq!(
            provider.stat(&missing, &OpContext::new()).unwrap_err(),
            VfsError::NotFound
        );
    }
}
