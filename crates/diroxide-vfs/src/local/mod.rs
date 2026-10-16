//! The local provider: the disks of this machine through `std::fs`, with
//! the platform's details in the `sys` module.
//!
//! - **Reading.** One stat per entry, the platform's attributes, owners by
//!   name from one cached map, links with their targets, identities for
//!   the plan's re-checks, hidden by the platform's rule (the attribute on
//!   Windows, the leading dot elsewhere), online-only and sparse files
//!   recognized ([`crate::facts`]). Unfinished `.diro-part-` files are left
//!   out of listings; [`LocalProvider::unfinished`] lists them.
//! - **Long paths.** The standard library lists, reads, copies, renames
//!   and removes at any depth; the extended-length prefix is never built
//!   by hand for it, and never shown.
//! - **Per place.** Capabilities, the volume and its kind, and free space
//!   are asked of the volume the place is really on, and the case and
//!   Unicode-form rule is probed in its folder without writing.

pub(crate) mod entry;
pub(crate) mod owners;
pub(crate) mod probe;
pub(crate) mod volumes;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use diroxide_core::{Entry, EntryKind, Place, PlaceError};

use crate::cancel::{OpContext, ProgressUpdate};
use crate::capabilities::Capabilities;
use crate::error::VfsError;
use crate::names;
use crate::provider::{AttributeChange, FileWriter, Provider, ReadSeek, WriteMode};
use crate::sys;
use crate::volume::{LocalPlace, PlaceRole, SpaceInfo, VolumeInfo, VolumeKind};
use entry::{EntryContext, entry_from};

/// The places for the go-to-place list: the drives on Windows (from the
/// drive list and drive types, never touching a drive), the root `/`
/// elsewhere, the home folder, and on Linux and macOS the mounts a person
/// keeps files on. Reading the mounts can block on a dead network server,
/// so this runs off the input thread.
pub fn places() -> Vec<LocalPlace> {
    let mut places = Vec::new();
    if cfg!(windows) {
        for (place, kind) in sys::drives() {
            places.push(LocalPlace::new(place, PlaceRole::Drive, kind));
        }
    } else {
        for place in sys::roots() {
            places.push(LocalPlace::new(place, PlaceRole::Root, VolumeKind::Fixed));
        }
    }
    if let Some(home) = dirs::home_dir()
        .as_deref()
        .and_then(Path::to_str)
        .and_then(|home| Place::parse(home).ok())
    {
        places.push(LocalPlace::new(home, PlaceRole::Home, VolumeKind::Fixed));
    }
    if !cfg!(windows) {
        for (place, kind) in sys::drives() {
            places.push(LocalPlace::new(place, PlaceRole::Mount, kind));
        }
    }
    places
}

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

    /// The path of a local place, refused for any other kind.
    pub(crate) fn path_of(place: &Place) -> Result<PathBuf, VfsError> {
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

    /// The unfinished files in a folder (`.diro-part-` and eight
    /// hexadecimal digits): writes that never committed, left by a crash.
    /// Listings leave them out; this finds them, so a command can say
    /// "Found 2 partial copies in this folder" and offer a listed delete.
    /// Nothing removes them on its own.
    pub fn unfinished(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.read_folder(place, ctx, true)
    }

    fn read_folder(
        &self,
        place: &Place,
        ctx: &OpContext,
        unfinished: bool,
    ) -> Result<Vec<Entry>, VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        let reader = fs::read_dir(&path)?;
        let context = sys::list_context(&path);
        let mut entries = Vec::new();
        let mut seen: u64 = 0;
        for item in reader {
            ctx.check()?;
            let item = item?;
            seen += 1;
            if seen.is_multiple_of(PROGRESS_EVERY) {
                ctx.progress.report(ProgressUpdate {
                    items_done: seen,
                    ..ProgressUpdate::default()
                });
            }
            let (name, lossy) = match item.file_name().into_string() {
                Ok(name) => (name, false),
                Err(raw) => (raw.to_string_lossy().into_owned(), true),
            };
            if names::is_unfinished(&name) != unfinished {
                continue;
            }
            let mut entry = match item.metadata() {
                Ok(metadata) => entry_from(name, &metadata, &item.path(), &context),
                // An entry that cannot be examined is listed with its
                // details unknown, never dropped and never an error.
                Err(_) => {
                    let mut entry = Entry::new(name, EntryKind::Special);
                    entry.needs_care = names::needs_care(&entry.name);
                    entry
                }
            };
            entry.name_not_unicode = lossy;
            entries.push(entry);
        }
        Ok(entries)
    }
}

impl Provider for LocalProvider {
    fn scheme(&self) -> &str {
        "file"
    }

    fn capabilities(&self, place: &Place) -> Capabilities {
        match Self::path_of(place) {
            Ok(path) => volumes::capabilities_at(&path),
            Err(_) => Capabilities::read_only(),
        }
    }

    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        ctx.check()?;
        let facts = volumes::facts_at(&Self::path_of(place)?);
        let root = facts
            .root
            .as_deref()
            .and_then(Path::to_str)
            .and_then(|root| Place::parse(root).ok())
            .unwrap_or_else(|| VolumeInfo::top_of(place));
        let mut info = VolumeInfo::unknown(root).with_kind(facts.kind);
        info.label = facts.label;
        info.file_system = facts.file_system;
        Ok(info)
    }

    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        ctx.check()?;
        let folder = volumes::nearest_existing_folder(&Self::path_of(place)?);
        let (free, total) = sys::space(&folder)?;
        let facts = volumes::facts_at(&folder);
        Ok(SpaceInfo {
            free_to_user: Some(free),
            total: Some(total),
            max_file_size: volumes::max_file_size(facts.file_system.as_deref()),
        })
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.read_folder(place, ctx, false)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        let metadata = fs::symlink_metadata(&path)?;
        let name = place
            .name()
            .map_or_else(|| place.to_string(), str::to_owned);
        let context = stat_context();
        let mut entry = entry_from(name, &metadata, &path, &context);
        sys::fill_from_handle(&mut entry, &path);
        Ok(entry)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        ctx.check()?;
        Ok(Box::new(File::open(Self::path_of(place)?)?))
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        ctx.check()?;
        Ok(Box::new(File::open(Self::path_of(place)?)?))
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        ctx.check()?;
        let target = Self::path_of(place)?;
        if mode == WriteMode::CreateNew && fs::symlink_metadata(&target).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        let folder = target
            .parent()
            .ok_or_else(|| VfsError::unsupported("file", "write at the top of a drive"))?;
        Ok(Box::new(LocalWriter::start(folder, target.clone(), mode)?))
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        Ok(fs::create_dir(Self::path_of(place)?)?)
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let target = Self::path_of(to)?;
        Ok(sys::rename_no_replace(&Self::path_of(from)?, &target)?)
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
    ) -> Result<AttributeChange, VfsError> {
        Err(VfsError::unsupported("file", "change attributes yet"))
    }
}

/// The context of a single stat: the owner names on Unix; nothing more is
/// shared with siblings.
fn stat_context() -> EntryContext {
    #[cfg(unix)]
    {
        EntryContext {
            owners: Some(owners::Owners::shared()),
            ..EntryContext::default()
        }
    }
    #[cfg(not(unix))]
    {
        EntryContext::default()
    }
}

/// A token for an unfinished name: the clock's nanoseconds mixed with a
/// counter, so two writes in the same instant differ.
fn unfinished_token() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    nanos ^ COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed)
}

/// The local committing writer: the bytes go to an unfinished name in the
/// target's folder; the commit renames it over the target; a drop without
/// a commit removes it.
struct LocalWriter {
    file: Option<File>,
    unfinished: PathBuf,
    target: PathBuf,
    mode: WriteMode,
}

impl LocalWriter {
    fn start(folder: &Path, target: PathBuf, mode: WriteMode) -> Result<Self, VfsError> {
        let mut last = VfsError::AlreadyExists;
        for _ in 0..8 {
            let unfinished = folder.join(names::unfinished_name(unfinished_token()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&unfinished)
            {
                Ok(file) => {
                    return Ok(Self {
                        file: Some(file),
                        unfinished,
                        target,
                        mode,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => last = error.into(),
            }
        }
        Err(last)
    }
}

impl Write for LocalWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &mut self.file {
            Some(file) => file.write(bytes),
            None => Err(io::Error::other("the write was already committed")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.file {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

impl FileWriter for LocalWriter {
    fn commit(mut self: Box<Self>, durable: bool) -> Result<(), VfsError> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
            if durable {
                file.sync_all()?;
            }
        }
        if self.mode == WriteMode::CreateNew && fs::symlink_metadata(&self.target).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        fs::rename(&self.unfinished, &self.target)?;
        self.unfinished = PathBuf::new();
        Ok(())
    }
}

impl Drop for LocalWriter {
    fn drop(&mut self) {
        self.file.take();
        if !self.unfinished.as_os_str().is_empty() {
            // Best effort: the unfinished file is ours alone, by its name.
            let _ = fs::remove_file(&self.unfinished);
        }
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
        assert!(manifest.attributes.is_some());
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
        assert!(entry.inode.is_some() && entry.volume.is_some());
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
