//! Unix (Linux and macOS): the twelve mode bits, owners and groups by
//! name, identities, the no-replace rename, times, free space and the
//! mounts. Everything here goes through the standard library or
//! `rustix`'s safe wrappers; there is no `unsafe` on this side.

use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

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

/// Renames without ever replacing: `renameat2` with `RENAME_NOREPLACE`
/// on Linux, `renameatx_np` with `RENAME_EXCL` on macOS, both atomic.
/// Where the file system lacks the flag, a file is linked to its new name
/// and then unlinked (the link fails when the name is taken), and a
/// folder falls back to a check and a rename, which leaves a short race
/// the documentation of `Provider::rename` admits.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;
    match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(errno)
            if errno == Errno::INVAL
                || errno == Errno::NOSYS
                || errno == Errno::NOTSUP
                || errno == Errno::OPNOTSUPP =>
        {
            rename_without_the_flag(from, to, errno)
        }
        Err(errno) => Err(errno.into()),
    }
}

fn rename_without_the_flag(from: &Path, to: &Path, errno: rustix::io::Errno) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(from)?;
    if metadata.is_dir() && to.starts_with(from) {
        // Into itself: the original refusal stands.
        return Err(errno.into());
    }
    if !metadata.is_dir() {
        match std::fs::hard_link(from, to) {
            Ok(()) => {
                if let Err(error) = std::fs::remove_file(from) {
                    // Two names now; take the new one back.
                    let _ = std::fs::remove_file(to);
                    return Err(error);
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
            Err(_) => {}
        }
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// Whether metadata read without following describes a folder (a link to
/// a folder is a link, removed as a file).
pub(crate) fn is_folder_like(metadata: &Metadata) -> bool {
    metadata.is_dir()
}

/// Sets all twelve mode bits: the nine permission bits and setuid, setgid
/// and sticky. Never `set_readonly`, which would touch only the write
/// bits. The caller refuses links first, since `chmod` follows them.
pub(crate) fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
}

/// Sets the owner and the group by number, on the entry itself, never
/// through a link.
pub(crate) fn set_owner(path: &Path, owner: Option<u32>, group: Option<u32>) -> io::Result<()> {
    use rustix::fs::{AtFlags, CWD, Gid, Uid, chownat};
    chownat(
        CWD,
        path,
        owner.map(Uid::from_raw),
        group.map(Gid::from_raw),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(Into::into)
}

fn timespec(time: Option<SystemTime>) -> rustix::fs::Timespec {
    let omit = rustix::fs::Timespec {
        tv_sec: 0,
        tv_nsec: rustix::fs::UTIME_OMIT,
    };
    let Some(time) = time else {
        return omit;
    };
    let stamp = Timestamp::from_system_time(time);
    rustix::fs::Timespec {
        tv_sec: stamp.unix_seconds(),
        tv_nsec: rustix::fs::Nsecs::from(stamp.subsec_nanos()),
    }
}

/// Sets an entry's times without following a final link (`utimensat`);
/// the created time through the file on macOS, the one Unix that can set
/// it. `None` leaves a time as it is.
pub(crate) fn set_times(
    path: &Path,
    created: Option<SystemTime>,
    modified: Option<SystemTime>,
    accessed: Option<SystemTime>,
) -> io::Result<()> {
    use rustix::fs::{AtFlags, CWD, Timestamps, utimensat};
    if modified.is_some() || accessed.is_some() {
        let times = Timestamps {
            last_access: timespec(accessed),
            last_modification: timespec(modified),
        };
        utimensat(CWD, path, &times, AtFlags::SYMLINK_NOFOLLOW)?;
    }
    if let Some(created) = created {
        set_created(path, created)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_created(path: &Path, created: SystemTime) -> io::Result<()> {
    use std::os::macos::fs::FileTimesExt;
    let file = std::fs::File::open(path)?;
    file.set_times(std::fs::FileTimes::new().set_created(created))
}

#[cfg(not(target_os = "macos"))]
fn set_created(_path: &Path, _created: SystemTime) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this system cannot set a created time",
    ))
}

/// The server-side copy call is Windows's; nothing on Unix asks for it.
pub(crate) fn copy_file(
    _from: &Path,
    _to: &Path,
    _ctx: &crate::cancel::OpContext,
) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Making a symbolic link needs no privilege on Unix.
pub(crate) fn needs_privilege(_error: &io::Error) -> bool {
    false
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
