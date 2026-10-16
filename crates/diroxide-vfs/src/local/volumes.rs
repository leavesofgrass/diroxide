//! Volumes and capabilities per place (research/file-operations.md C7 and
//! "Case-only renames"; bulk-rename.md "Names and equivalence"). Every
//! fact is asked of the volume the path is really on, never of the
//! operating system alone: a USB stick has no trash on Windows, a FAT
//! stick stores no permissions on Linux, a Btrfs volume can clone.
//!
//! The pure parts (the file system tables, the mount table parser, the
//! capabilities from the facts, the watch mode) are compiled and tested
//! on every platform; the platform calls are in `sys`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::capabilities::{Can, Capabilities};
use crate::local::probe::{self, NameRule};
use crate::sys;
use crate::volume::VolumeKind;

/// How long the facts of a folder's volume are trusted.
const FACTS_KEEP_FOR: Duration = Duration::from_secs(5);

/// How long a folder's capabilities are trusted (they include the case
/// probe, which reads the folder).
const CAPABILITIES_KEEP_FOR: Duration = Duration::from_secs(30);

/// The most folders either cache remembers before it starts over.
const CACHE_LIMIT: usize = 512;

/// The largest file the FAT family takes: 4 GiB less one byte.
pub(crate) const FAT_MAX_FILE: u64 = 4 * 1024 * 1024 * 1024 - 1;

/// What the platform says about the volume a path is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct VolumeFacts {
    /// The volume's root: `D:\`, a mount point, a share, `/`.
    pub(crate) root: Option<PathBuf>,
    /// The kind of drive.
    pub(crate) kind: VolumeKind,
    /// The path is a network path (a share, or a mapped drive).
    pub(crate) network_path: bool,
    /// The volume's label, when the platform gives one.
    pub(crate) label: Option<String>,
    /// The file system's name, as the platform writes it ("NTFS", "ext4").
    pub(crate) file_system: Option<String>,
    /// The volume is mounted read only.
    pub(crate) read_only: bool,
    /// The longest name the file system takes.
    pub(crate) max_name: Option<usize>,
}

/// One mounted file system on Unix.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
pub(crate) struct Mount {
    /// Where it is mounted.
    pub(crate) point: PathBuf,
    /// The file system's name.
    pub(crate) file_system: String,
    /// Whether the system calls the drive removable.
    pub(crate) removable: bool,
}

/// How a folder's changes reach a panel (spike three).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WatchMode {
    /// The system's change notifications, through `notify`.
    Native,
    /// A look at the folder every two seconds: removable drives, so a
    /// watch never holds the drive and its eject keeps working.
    Poll,
    /// Not watched: network places, where notifications may never come.
    None,
}

/// File systems on a network, by their Unix names (lowercase).
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
const NETWORK_FILE_SYSTEMS: [&str; 22] = [
    "nfs",
    "nfs4",
    "cifs",
    "smb",
    "smb2",
    "smb3",
    "smbfs",
    "ncpfs",
    "afs",
    "coda",
    "9p",
    "sshfs",
    "fuse.sshfs",
    "fuse.rclone",
    "davfs",
    "fuse.davfs2",
    "webdav",
    "afpfs",
    "glusterfs",
    "fuse.glusterfs",
    "ceph",
    "lustre",
];

/// File systems that clone a file in place (reflink, `clonefile`, ReFS
/// block cloning).
const CLONING_FILE_SYSTEMS: [&str; 6] = ["btrfs", "xfs", "bcachefs", "apfs", "refs", "ocfs2"];

/// File systems that follow Windows's name rules and store no Unix
/// permissions, as Linux and macOS mount them.
const WINDOWS_RULES_FILE_SYSTEMS: [&str; 10] = [
    "vfat", "msdos", "fat", "fat16", "fat32", "exfat", "ntfs", "ntfs3", "fuseblk", "msdosfs",
];

/// The FAT family, which holds no file of 4 GiB or more.
const FAT_FILE_SYSTEMS: [&str; 6] = ["vfat", "msdos", "fat", "fat16", "fat32", "msdosfs"];

fn named(file_system: Option<&str>, table: &[&str]) -> bool {
    file_system.is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        table.contains(&name.as_str())
    })
}

/// Whether a file system is on a network.
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
pub(crate) fn is_network_file_system(file_system: &str) -> bool {
    named(Some(file_system), &NETWORK_FILE_SYSTEMS)
}

/// The largest file a file system takes, when it has a limit worth
/// checking before a copy.
pub(crate) fn max_file_size(file_system: Option<&str>) -> Option<u64> {
    named(file_system, &FAT_FILE_SYSTEMS).then_some(FAT_MAX_FILE)
}

/// How a volume's folders are watched.
pub(crate) fn watch_mode(facts: &VolumeFacts) -> WatchMode {
    if facts.network_path || facts.kind == VolumeKind::Network {
        WatchMode::None
    } else if facts.kind == VolumeKind::Removable {
        WatchMode::Poll
    } else {
        WatchMode::Native
    }
}

/// The capabilities of a place from its volume's facts and its folder's
/// name rule. Pure, so every platform's answer is tested with fake facts.
pub(crate) fn capabilities_for(facts: &VolumeFacts, rule: NameRule) -> Capabilities {
    let mut capabilities = Capabilities::read_only();
    let network = facts.network_path || facts.kind == VolumeKind::Network;
    let fs = facts.file_system.as_deref();
    let writable = !facts.read_only;
    capabilities.writable = writable;
    capabilities.random_read = true;
    capabilities.case_sensitive = rule.case_sensitive;
    capabilities.normalization_insensitive = rule.normalization_insensitive;
    capabilities.max_name_length = facts.max_name.or(Some(255));
    capabilities.watch = watch_mode(facts) != WatchMode::None;
    let yes = if writable { Can::Yes } else { Can::No };
    if cfg!(windows) {
        capabilities.trash = writable && facts.kind == VolumeKind::Fixed && !network;
        capabilities.restore = capabilities.trash;
        capabilities.reserved_names = true;
        // Junctions need no privilege; symbolic links may need Developer
        // Mode, which the error says in words.
        capabilities.links = true;
        capabilities.access.os_attributes = yes;
        capabilities.access.times = yes;
        capabilities.access.created_time = yes;
        // A network share copies on the server through the copy call.
        capabilities.clone_copy = writable && (named(fs, &CLONING_FILE_SYSTEMS) || network);
    } else {
        let windows_rules = named(fs, &WINDOWS_RULES_FILE_SYSTEMS);
        capabilities.trash = writable && !network;
        capabilities.restore = capabilities.trash && cfg!(not(target_vendor = "apple"));
        capabilities.reserved_names = windows_rules;
        capabilities.links = !windows_rules;
        capabilities.access.times = yes;
        capabilities.access.created_time = if cfg!(target_vendor = "apple") {
            yes
        } else {
            Can::No
        };
        if !windows_rules && writable {
            capabilities.access.mode = Can::Yes;
            capabilities.access.owner = Can::WithAdministrator;
            capabilities.access.group = Can::Yes;
        }
        capabilities.clone_copy = writable && named(fs, &CLONING_FILE_SYSTEMS);
    }
    capabilities.attributes = capabilities.access.any();
    capabilities
}

/// Reads the Linux mount table (`/proc/self/mounts`): the mount point and
/// the file system of each line, with the table's octal escapes (`\040`
/// for a space) undone. Lines it cannot read are skipped.
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
pub(crate) fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _device = fields.next()?;
            let point = unescape_mount(fields.next()?);
            let file_system = fields.next()?.to_owned();
            point.starts_with('/').then(|| Mount {
                point: PathBuf::from(point),
                file_system,
                removable: false,
            })
        })
        .collect()
}

#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
fn unescape_mount(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).and_then(|digits| {
            if !digits.iter().all(|b| (b'0'..=b'7').contains(b)) {
                return None;
            }
            let value = digits
                .iter()
                .fold(0u32, |all, b| all * 8 + u32::from(b - b'0'));
            u8::try_from(value).ok()
        });
        if let (b'\\', Some(value)) = (bytes[i], octal) {
            out.push(value);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mount a path is on: the longest mount point that holds it, by whole
/// components.
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
pub(crate) fn best_mount<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.components().count())
}

/// The volume's kind from a mount: on a network, removable, or a disk.
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
pub(crate) fn mount_kind(mount: &Mount) -> VolumeKind {
    if is_network_file_system(&mount.file_system) {
        VolumeKind::Network
    } else if mount.removable {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
    }
}

/// Whether a mount belongs in the places list: a real file system a
/// person keeps files on, not the system's own (`/proc`, snaps, the boot
/// partition, macOS's system volumes).
#[cfg_attr(windows, allow(dead_code, reason = "the Unix mount table"))]
pub(crate) fn is_user_mount(mount: &Mount) -> bool {
    const SYSTEM_FILE_SYSTEMS: [&str; 26] = [
        "squashfs",
        "overlay",
        "tmpfs",
        "devtmpfs",
        "proc",
        "sysfs",
        "cgroup",
        "cgroup2",
        "autofs",
        "devpts",
        "mqueue",
        "debugfs",
        "tracefs",
        "securityfs",
        "pstore",
        "bpf",
        "configfs",
        "fusectl",
        "hugetlbfs",
        "efivarfs",
        "binfmt_misc",
        "nsfs",
        "ramfs",
        "rpc_pipefs",
        "devfs",
        "nullfs",
    ];
    const SYSTEM_FOLDERS: [&str; 9] = [
        "/proc",
        "/sys",
        "/dev",
        "/snap",
        "/boot",
        "/var/lib",
        "/System/Volumes",
        "/private/var",
        "/run",
    ];
    let point = &mount.point;
    if point == Path::new("/") || named(Some(&mount.file_system), &SYSTEM_FILE_SYSTEMS) {
        return false;
    }
    if point.starts_with("/run/media") {
        return true;
    }
    !SYSTEM_FOLDERS
        .iter()
        .any(|folder| point.starts_with(folder))
}

/// The nearest folder at or above a path that exists, for asking about a
/// place that is not made yet.
pub(crate) fn nearest_existing_folder(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if std::fs::metadata(&current).is_ok_and(|m| m.is_dir()) {
            return current;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return path.to_path_buf(),
        }
    }
}

/// A cache of answers by folder, trusted for a while.
struct Cache<T> {
    keep_for: Duration,
    entries: Mutex<HashMap<PathBuf, (Instant, T)>>,
}

impl<T: Clone> Cache<T> {
    fn new(keep_for: Duration) -> Self {
        Self {
            keep_for,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, (Instant, T)>> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn get_or(&self, key: &Path, make: impl FnOnce() -> T) -> T {
        if let Some((at, value)) = self.lock().get(key)
            && at.elapsed() < self.keep_for
        {
            return value.clone();
        }
        let value = make();
        let mut entries = self.lock();
        if entries.len() >= CACHE_LIMIT {
            entries.clear();
        }
        entries.insert(key.to_path_buf(), (Instant::now(), value.clone()));
        value
    }
}

/// The facts of the volume a path is on, asked of the nearest existing
/// folder and kept for a few seconds.
pub(crate) fn facts_at(path: &Path) -> VolumeFacts {
    static CACHE: OnceLock<Cache<VolumeFacts>> = OnceLock::new();
    let folder = nearest_existing_folder(path);
    CACHE
        .get_or_init(|| Cache::new(FACTS_KEEP_FOR))
        .get_or(&folder, || sys::volume_facts(&folder))
}

/// The capabilities at a path: its folder's volume facts and name rule.
pub(crate) fn capabilities_at(path: &Path) -> Capabilities {
    static CACHE: OnceLock<Cache<Capabilities>> = OnceLock::new();
    let folder = nearest_existing_folder(path);
    CACHE
        .get_or_init(|| Cache::new(CAPABILITIES_KEEP_FOR))
        .get_or(&folder, || {
            capabilities_for(&facts_at(&folder), probe::name_rule(&folder))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRICT: NameRule = NameRule {
        case_sensitive: false,
        normalization_insensitive: true,
    };

    fn facts(kind: VolumeKind, file_system: &str) -> VolumeFacts {
        VolumeFacts {
            kind,
            file_system: Some(file_system.to_owned()),
            max_name: Some(255),
            ..VolumeFacts::default()
        }
    }

    #[test]
    fn the_trash_follows_the_drive_kind() {
        let fixed = capabilities_for(&facts(VolumeKind::Fixed, "NTFS"), STRICT);
        assert!(fixed.trash && fixed.writable && fixed.watch);
        let network = capabilities_for(&facts(VolumeKind::Network, "NTFS"), STRICT);
        assert!(!network.trash && !network.watch && !network.restore);
        let removable = capabilities_for(&facts(VolumeKind::Removable, "exFAT"), STRICT);
        assert_eq!(removable.trash, !cfg!(windows));
        assert!(removable.watch, "polled, so still watched");
        let share = VolumeFacts {
            network_path: true,
            ..facts(VolumeKind::Fixed, "NTFS")
        };
        assert!(!capabilities_for(&share, STRICT).trash);
        let read_only = VolumeFacts {
            read_only: true,
            ..facts(VolumeKind::Fixed, "NTFS")
        };
        let read_only = capabilities_for(&read_only, STRICT);
        assert!(!read_only.writable && !read_only.trash && !read_only.attributes);
        assert_eq!(read_only.limits_in_words(), "read only");
    }

    #[test]
    fn unix_places_follow_their_file_system() {
        if cfg!(windows) {
            return;
        }
        let stick = capabilities_for(&facts(VolumeKind::Removable, "vfat"), STRICT);
        assert!(stick.reserved_names && !stick.links);
        assert_eq!(stick.access.mode, Can::No);
        let home = capabilities_for(&facts(VolumeKind::Fixed, "ext4"), STRICT);
        assert!(!home.reserved_names && home.links && !home.clone_copy);
        assert_eq!(home.access.mode, Can::Yes);
        assert_eq!(home.access.owner, Can::WithAdministrator);
        assert!(capabilities_for(&facts(VolumeKind::Fixed, "btrfs"), STRICT).clone_copy);
    }

    #[test]
    fn the_name_rule_comes_from_the_probe() {
        let rule = NameRule {
            case_sensitive: true,
            normalization_insensitive: false,
        };
        let caps = capabilities_for(&facts(VolumeKind::Fixed, "ext4"), rule);
        assert!(caps.case_sensitive && !caps.normalization_insensitive);
        assert_eq!(caps.max_name_length, Some(255));
    }

    #[test]
    fn watching_follows_spike_three() {
        assert_eq!(
            watch_mode(&facts(VolumeKind::Fixed, "NTFS")),
            WatchMode::Native
        );
        assert_eq!(
            watch_mode(&facts(VolumeKind::Removable, "exFAT")),
            WatchMode::Poll
        );
        assert_eq!(
            watch_mode(&facts(VolumeKind::Network, "cifs")),
            WatchMode::None
        );
        let mapped = VolumeFacts {
            network_path: true,
            ..facts(VolumeKind::Unknown, "NTFS")
        };
        assert_eq!(watch_mode(&mapped), WatchMode::None);
        assert_eq!(
            watch_mode(&facts(VolumeKind::Unknown, "")),
            WatchMode::Native
        );
    }

    #[test]
    fn the_mount_table_reads_with_its_escapes() {
        let table = "sysfs /sys sysfs rw 0 0\n/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /media/ada/My\\040Stick vfat rw 0 0\nserver:/x /mnt/nfs nfs4 rw 0 0\nbad\n";
        let mounts = parse_mounts(table);
        assert_eq!(mounts.len(), 4);
        assert_eq!(mounts[2].point, PathBuf::from("/media/ada/My Stick"));
        let on = |path: &str| best_mount(&mounts, Path::new(path)).map(|m| m.point.clone());
        assert_eq!(on("/home/ada"), Some(PathBuf::from("/")));
        assert_eq!(
            on("/media/ada/My Stick/photos"),
            Some(PathBuf::from("/media/ada/My Stick"))
        );
        assert_eq!(on("/mnt/nfsx"), Some(PathBuf::from("/")));
        assert_eq!(mount_kind(&mounts[3]), VolumeKind::Network);
        assert_eq!(mount_kind(&mounts[1]), VolumeKind::Fixed);
        assert!(!is_user_mount(&mounts[0]) && !is_user_mount(&mounts[1]));
        assert!(is_user_mount(&mounts[2]) && is_user_mount(&mounts[3]));
    }

    #[test]
    fn fat_has_a_largest_file() {
        assert_eq!(max_file_size(Some("FAT32")), Some(FAT_MAX_FILE));
        assert_eq!(max_file_size(Some("vfat")), Some(4_294_967_295));
        assert_eq!(max_file_size(Some("NTFS")), None);
        assert_eq!(max_file_size(None), None);
    }

    #[test]
    fn a_new_place_is_asked_of_its_nearest_folder() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let new = here.join("no-such-folder").join("no-such-file.txt");
        assert_eq!(nearest_existing_folder(&new), here);
        assert_eq!(nearest_existing_folder(here), here);
    }
}
