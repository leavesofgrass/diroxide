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
//! - **Safe primitives.** A new name the place refuses never reaches the
//!   disk ([`crate::names::first_refused`]). A rename never replaces: the
//!   platform's own no-replace rename, with a change of case or Unicode
//!   form of the same entry let through, in two steps through an
//!   unfinished name where the file system needs it. A remove takes away a
//!   link, never what it points to, and a read-only file on Windows.
//!   Attributes, times and owners change with the inverse returned.

pub(crate) mod attributes;
pub(crate) mod copy;
pub(crate) mod entry;
pub(crate) mod owners;
pub(crate) mod probe;
pub(crate) mod trashing;
pub(crate) mod volumes;
pub(crate) mod watch;
pub(crate) mod writer;

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use diroxide_core::{Attributes, Entry, EntryKind, Place, PlaceError};

use crate::cancel::{OpContext, ProgressUpdate};
use crate::capabilities::Capabilities;
use crate::error::VfsError;
use crate::names;
use crate::provider::{
    AttributeChange, CopyHint, FileWriter, LinkKind, LinkRequest, Provider, ReadSeek, WatchHandle,
    WatchSink, WriteMode,
};
use crate::sys;
use crate::trash::{TrashCheck, TrashFacts, TrashGate, TrashRules, trash_gate};
use crate::volume::{LocalPlace, PlaceRole, SpaceInfo, VolumeInfo, VolumeKind};
use crate::{limits, walk};
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

    /// Refuses a new name the place would change or cannot hold, before
    /// anything is made: `NameInvalid`.
    fn refuse_name(place: &Place, path: &Path) -> Result<(), VfsError> {
        let Some(name) = place.name() else {
            return Ok(());
        };
        let rules = names::NameRules::from_capabilities(&volumes::capabilities_at(path));
        match names::first_refused(name, &rules) {
            Some(_) => Err(VfsError::NameInvalid),
            None => Ok(()),
        }
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
        Self::refuse_name(place, &target)?;
        if mode == WriteMode::CreateNew && fs::symlink_metadata(&target).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        Ok(Box::new(writer::LocalWriter::start(target, mode)?))
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        Self::refuse_name(place, &path)?;
        Ok(fs::create_dir(path)?)
    }

    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        Self::refuse_name(place, &path)?;
        if request.target.is_empty() {
            return Err(VfsError::NotFound);
        }
        // A relative target is stored as written in a symbolic link; a
        // junction and a hard link need the real path, read from the
        // link's own folder.
        let resolved = || match path.parent() {
            Some(folder) => folder.join(&request.target),
            None => PathBuf::from(&request.target),
        };
        let made = match request.kind {
            LinkKind::Symbolic => {
                make_symbolic_link(&request.target, &path, request.target_is_folder)
            }
            LinkKind::Hard => fs::hard_link(resolved(), &path),
            LinkKind::Junction => make_junction(&path, &resolved()),
        };
        made.map_err(|error| {
            if sys::needs_privilege(&error) {
                VfsError::Io {
                    kind: io::ErrorKind::PermissionDenied,
                    message: "Windows needs Developer Mode to make symbolic links".to_owned(),
                }
            } else {
                error.into()
            }
        })
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let source = Self::path_of(from)?;
        let target = Self::path_of(to)?;
        if source == target {
            return Ok(());
        }
        Self::refuse_name(to, &target)?;
        // When the new name already answers with this same entry, it is
        // either a new spelling (a place that ignores case or form) or a
        // second name of one file (a hard link). The first is renamed,
        // through an unfinished name if the platform will not do it in
        // one step; the second is taken, since one name would vanish.
        if let Ok(there) = sys::identity(&target)
            && sys::identity(&source).is_ok_and(|here| here == there)
        {
            if !spelled_alike(&source, &target) {
                return Err(VfsError::AlreadyExists);
            }
            return match sys::rename_no_replace(&source, &target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    rename_through_unfinished(&source, &target)
                }
                Err(error) => Err(error.into()),
            };
        }
        Ok(sys::rename_no_replace(&source, &target)?)
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        let metadata = fs::symlink_metadata(&path)?;
        let remove = || {
            if sys::is_folder_like(&metadata) {
                fs::remove_dir(&path)
            } else {
                fs::remove_file(&path)
            }
        };
        match remove() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                remove_read_only(&path, &metadata, remove).map_err(|_| error.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        ctx.check()?;
        attributes::set_attributes(&Self::path_of(place)?, change)
    }

    fn trash_gate(&self, place: &Place, ctx: &OpContext) -> Result<TrashGate, VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        let metadata = fs::symlink_metadata(&path)?;
        let shell = shell_path(&path)?;
        let facts = volumes::facts_at(&shell);
        let own_units = limits::utf16_units(&shell.to_string_lossy());
        // A folder is measured by the longest path inside it, as the trash
        // will see it (spike four): the walk's longest, moved by the
        // difference between the place's path and the trash's.
        let longest_path_units = if metadata.is_dir() {
            let inside = walk::longest_path_units(self, place, ctx)?;
            let written = limits::utf16_units(&path.to_string_lossy());
            (inside + own_units).saturating_sub(written).max(own_units)
        } else {
            own_units
        };
        let network_path = facts.network_path || shell.to_string_lossy().starts_with(r"\\");
        let gate = trash_gate(
            &TrashFacts {
                volume: facts.kind,
                longest_path_units,
                network_path,
            },
            TrashRules::native(),
        );
        // A read-only volume, or anything the capabilities say has no
        // trash, is refused even where the platform's rules would allow it.
        if gate.allows() && !volumes::capabilities_at(&shell).trash {
            return Ok(TrashGate::NoTrashHere(facts.kind));
        }
        Ok(gate)
    }

    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        ctx.check()?;
        let path = Self::path_of(place)?;
        fs::symlink_metadata(&path)?;
        let gate = self.trash_gate(place, ctx)?;
        if !gate.allows() {
            return Err(VfsError::TrashRefused { why: gate.why() });
        }
        ctx.check()?;
        trashing::trash_with(
            &shell_path(&path)?,
            &trashing::system_trash,
            &trashing::system_look,
        )
    }

    fn restore(&self, original: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let path = Self::path_of(original)?;
        if !self.capabilities(original).restore {
            return Err(VfsError::unsupported("file", "restore from the trash here"));
        }
        if fs::symlink_metadata(&path).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        let shell = shell_path(&path).unwrap_or(path);
        trashing::restore_newest(&shell)
    }

    fn watch(&self, place: &Place, sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        let folder = Self::path_of(place)?;
        watch::watch(place, &folder, sink)
    }

    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        match (Self::path_of(from), Self::path_of(to)) {
            (Ok(from), Ok(to)) => copy::hint(&from, &to),
            _ => CopyHint::Stream,
        }
    }

    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        ctx.check()?;
        let source = Self::path_of(from)?;
        let target = Self::path_of(to)?;
        Self::refuse_name(to, &target)?;
        copy::copy_within(&source, &target, copy::hint(&source, &target), ctx)
    }
}

/// The path as the trash sees it: the parent folder with its links and
/// junctions resolved (the `trash` crate canonicalizes it), the name kept,
/// and never the extended-length prefix. A folder reached through a
/// junction is judged by the drive it is really on.
fn shell_path(path: &Path) -> Result<PathBuf, VfsError> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(VfsError::Unsafe {
            why: "it is the top of a drive".to_owned(),
        });
    };
    let parent = fs::canonicalize(parent)?;
    let parent = match parent.to_str() {
        Some(text) => PathBuf::from(diroxide_core::place::strip_extended_prefix(text).as_ref()),
        None => parent,
    };
    Ok(parent.join(name))
}

/// Makes a symbolic link, a folder link or a file link on Windows.
fn make_symbolic_link(target: &str, link: &Path, folder: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        if folder {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
    #[cfg(unix)]
    {
        let _ = folder;
        std::os::unix::fs::symlink(target, link)
    }
}

/// Makes a junction on Windows; elsewhere there are none.
fn make_junction(link: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        sys::create_junction(link, target)
    }
    #[cfg(not(windows))]
    {
        let _ = (link, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "junctions are made on Windows only",
        ))
    }
}

/// Whether two paths in one folder differ only by the case or the
/// Unicode form of the name: a new spelling, not a new name.
fn spelled_alike(source: &Path, target: &Path) -> bool {
    if source.parent() != target.parent() {
        return false;
    }
    let (Some(a), Some(b)) = (
        source.file_name().and_then(|n| n.to_str()),
        target.file_name().and_then(|n| n.to_str()),
    ) else {
        return false;
    };
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    a == b
        || probe::other_form(&a).is_some_and(|form| form == b)
        || probe::other_form(&b).is_some_and(|form| form == a)
}

/// A rename in two steps through an unfinished name in the same folder,
/// for a change of case or form the file system will not make in one:
/// each step never replaces, and a failed second step is taken back.
fn rename_through_unfinished(source: &Path, target: &Path) -> Result<(), VfsError> {
    let folder = target.parent().ok_or(VfsError::NotFound)?;
    let middle = folder.join(names::unfinished_name(writer::unfinished_token()));
    sys::rename_no_replace(source, &middle)?;
    if let Err(error) = sys::rename_no_replace(&middle, target) {
        let _ = sys::rename_no_replace(&middle, source);
        return Err(error.into());
    }
    Ok(())
}

/// Removes a read-only file on Windows, where the attribute blocks the
/// delete: the attribute is cleared, the remove tried again, and the
/// attribute put back if it still fails. Elsewhere read-only is not the
/// cause, so the first error stands.
fn remove_read_only(
    path: &Path,
    metadata: &fs::Metadata,
    remove: impl Fn() -> io::Result<()>,
) -> io::Result<()> {
    let Attributes::Windows(flags) = attributes::current_attributes(metadata) else {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    };
    if flags & 0x1 == 0 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    #[cfg(windows)]
    {
        sys::set_os_attributes(path, flags & !0x1)?;
        let removed = remove();
        if removed.is_err() {
            let _ = sys::set_os_attributes(path, flags);
        }
        removed
    }
    #[cfg(not(windows))]
    {
        let _ = (path, remove);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
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
