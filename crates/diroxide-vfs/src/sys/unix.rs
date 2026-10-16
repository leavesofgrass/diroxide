//! Unix (Linux and macOS): the twelve mode bits, owners and groups by
//! name, identities, the no-replace rename, times, free space and the
//! mounts. Everything here goes through the standard library or
//! `rustix`'s safe wrappers; there is no `unsafe` on this side.

use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use diroxide_core::{Attributes, Entry, Place, Timestamp};

use super::Identity;
use crate::local::entry::EntryContext;
use crate::local::owners::Owners;
use crate::local::volumes::{self, Mount, VolumeFacts};
use crate::volume::VolumeKind;

/// How long a read of the mount table is trusted.
const MOUNTS_KEEP_FOR: Duration = Duration::from_secs(10);

/// The macOS flag that hides a file from the Finder (`chflags hidden`).
#[cfg(target_os = "macos")]
const UF_HIDDEN: u32 = 0x8000;

/// The macOS flag of a dataless file: its contents are in the cloud and
/// reading it downloads them.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

/// The identity of the entry at `path`, without following a final link.
pub(crate) fn identity(path: &Path) -> io::Result<Identity> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(Identity {
        volume: metadata.dev(),
        id: u128::from(metadata.ino()),
    })
}

/// What a listing knows once per folder: the owner and group names, read
/// once (never a per-entry lookup, and no libc call that is not safe
/// across threads).
pub(crate) fn list_context(_folder: &Path) -> EntryContext {
    EntryContext {
        owners: Some(Owners::shared()),
        ..EntryContext::default()
    }
}

/// Fills an entry with what `lstat` holds: the twelve mode bits, the
/// owner and group by name (the number when no name is known), the
/// identity, the link and block counts, the change time, and hidden by
/// the leading dot (and the Finder's flag on macOS).
pub(crate) fn fill_platform(entry: &mut Entry, metadata: &Metadata, context: &EntryContext) {
    entry.attributes = Some(Attributes::Unix(metadata.mode() & 0o7777));
    entry.volume = Some(metadata.dev());
    entry.inode = Some(metadata.ino());
    entry.hard_links = Some(metadata.nlink());
    entry.blocks = Some(metadata.blocks());
    entry.changed = timestamp(metadata.ctime(), metadata.ctime_nsec());
    let (owner, group) = match &context.owners {
        Some(owners) => (owners.user(metadata.uid()), owners.group(metadata.gid())),
        None => (metadata.uid().to_string(), metadata.gid().to_string()),
    };
    entry.owner = Some(owner);
    entry.group = Some(group);
    entry.hidden = entry.name.starts_with('.');
    #[cfg(target_os = "macos")]
    {
        let flags = std::os::macos::fs::MetadataExt::st_flags(metadata);
        if flags & UF_HIDDEN != 0 {
            entry.hidden = true;
        }
        if flags & SF_DATALESS != 0 {
            entry.extra.insert(
                crate::facts::ONLINE_ONLY_KEY.to_owned(),
                crate::facts::ONLINE_ONLY_WORDS.to_owned(),
            );
        }
    }
}

/// Nothing more to read: `lstat` gave every fact a stat needs.
pub(crate) fn fill_from_handle(_entry: &mut Entry, _path: &Path) {}

/// Renames without ever replacing. Phase 0's body, replaced with the
/// platform's no-replace rename below.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// The top of the local file system: `/`.
pub(crate) fn roots() -> Vec<Place> {
    Place::parse("/").into_iter().collect()
}

/// The mounted file systems, read at most every ten seconds: on Linux the
/// kernel's own table (`/proc/self/mounts`, which also lists network
/// mounts), with the removable flag from `sysinfo`; elsewhere `sysinfo`'s
/// list. Reading the kernel's table never touches a mount; `sysinfo`
/// skips network mounts, which could hang on a dead server.
pub(crate) fn mounts() -> Vec<Mount> {
    /// The last read and when it was made.
    type Cached = Option<(Instant, Vec<Mount>)>;
    static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut slot = match cache.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((read_at, mounts)) = slot.as_ref()
        && read_at.elapsed() < MOUNTS_KEEP_FOR
    {
        return mounts.clone();
    }
    let mounts = read_mounts();
    *slot = Some((Instant::now(), mounts.clone()));
    mounts
}

fn read_mounts() -> Vec<Mount> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let listed: Vec<Mount> = disks
        .list()
        .iter()
        .map(|disk| Mount {
            point: disk.mount_point().to_path_buf(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
            removable: disk.is_removable(),
        })
        .collect();
    if cfg!(target_os = "linux") {
        let table = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let mut all = volumes::parse_mounts(&table);
        if !all.is_empty() {
            for mount in &mut all {
                mount.removable = listed
                    .iter()
                    .any(|disk| disk.removable && disk.point == mount.point);
            }
            return all;
        }
    }
    listed
}

/// The mounts a person keeps files on, with their kinds, for the places
/// list.
pub(crate) fn drives() -> Vec<(Place, VolumeKind)> {
    mounts()
        .iter()
        .filter(|mount| volumes::is_user_mount(mount))
        .filter_map(|mount| {
            let place = Place::parse(mount.point.to_str()?).ok()?;
            Some((place, volumes::mount_kind(mount)))
        })
        .collect()
}

/// The facts of the volume a path is on: the mount that holds the
/// path's real location (links resolved), its kind and file system, and
/// what `statvfs` says (read only, the longest name).
pub(crate) fn volume_facts(path: &Path) -> VolumeFacts {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mounts = mounts();
    let mut facts = VolumeFacts::default();
    match volumes::best_mount(&mounts, &real) {
        Some(mount) => {
            facts.root = Some(mount.point.clone());
            facts.kind = volumes::mount_kind(mount);
            facts.file_system = Some(mount.file_system.clone());
        }
        None => facts.root = Some(PathBuf::from("/")),
    }
    if let Ok(stat) = rustix::fs::statvfs(&real) {
        facts.read_only = stat.f_flag.contains(rustix::fs::StatVfsMountFlags::RDONLY);
        facts.max_name = usize::try_from(stat.f_namemax).ok().filter(|&n| n > 0);
    }
    facts
}

/// The free space the caller may use and the volume's size, in bytes.
pub(crate) fn space(path: &Path) -> io::Result<(u64, u64)> {
    let stat = rustix::fs::statvfs(path)?;
    Ok((
        stat.f_bavail.saturating_mul(stat.f_frsize),
        stat.f_blocks.saturating_mul(stat.f_frsize),
    ))
}

fn timestamp(secs: i64, nanos: i64) -> Option<Timestamp> {
    u32::try_from(nanos)
        .ok()
        .map(|nanos| Timestamp::from_unix(secs, nanos))
}
