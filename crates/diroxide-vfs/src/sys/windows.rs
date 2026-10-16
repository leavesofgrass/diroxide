//! Windows: attributes, file ids, the change time and the link count read
//! through a handle, the drive list, volumes, the no-replace rename,
//! junctions, the copy call and the trash's post-check.
//!
//! Paths given to the calls made here directly (not through the standard
//! library, which converts long paths itself) go through [`wide`], which
//! adds the extended-length prefix only to a long, full, already checked
//! path. The prefix never comes back out: every path read from Windows is
//! passed through `strip_extended_prefix` before it becomes a place.

use std::ffi::{OsString, c_void};
use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use diroxide_core::{Attributes, Entry, Place, Timestamp};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileBasicInfo, FileIdInfo, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetFileInformationByHandle, GetFileInformationByHandleEx, GetLogicalDrives,
    GetVolumeInformationW, GetVolumePathNameW,
};

use super::Identity;
use crate::local::entry::EntryContext;
use crate::local::volumes::VolumeFacts;
use crate::volume::VolumeKind;

/// Every share mode, so opening an entry to read its facts never blocks
/// another program from renaming or removing it.
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

/// `GetDriveTypeW`'s answers (WinBase.h).
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

/// The volume flag of a read-only volume (`FILE_READ_ONLY_VOLUME`).
const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;

/// The hidden attribute.
const ATTRIBUTE_HIDDEN: u32 = 0x2;

/// The paths at or past this length get the extended-length prefix in
/// direct calls: `MAX_PATH` less the 12 units a folder must leave for an
/// 8.3 name.
const LONG_PATH_UNITS: usize = 248;

/// 100-nanosecond intervals between 1601 and 1970.
const FILETIME_UNIX_OFFSET_SECONDS: i64 = 11_644_473_600;

/// A path as a NUL-terminated wide string for a direct Windows call, with
/// the extended-length prefix added when the path is long. The path must
/// be full and normalized (a place's path always is) and its names
/// already checked, since the prefix turns off Windows's own checks.
pub(crate) fn wide(path: &Path) -> Vec<u16> {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut out = Vec::with_capacity(units.len() + 9);
    let already = units.starts_with(&[92, 92, 63, 92]);
    if units.len() >= LONG_PATH_UNITS && !already {
        if units.starts_with(&[92, 92]) {
            out.extend(r"\\?\UNC\".encode_utf16());
            out.extend_from_slice(&units[2..]);
        } else {
            out.extend(r"\\?\".encode_utf16());
            out.extend_from_slice(&units);
        }
    } else {
        out.extend_from_slice(&units);
    }
    out.push(0);
    out
}

/// A wide buffer read back from Windows, up to its first NUL, as a path
/// without the extended-length prefix.
pub(crate) fn path_from_wide(buffer: &[u16]) -> PathBuf {
    let end = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
    let text = OsString::from_wide(&buffer[..end]);
    match text.to_str() {
        Some(text) => PathBuf::from(diroxide_core::place::strip_extended_prefix(text).as_ref()),
        None => PathBuf::from(text),
    }
}

/// A FILETIME value (100-nanosecond intervals since 1601) as a timestamp.
pub(crate) fn timestamp_from_filetime(value: i64) -> Option<Timestamp> {
    if value <= 0 {
        return None;
    }
    let secs = value.div_euclid(10_000_000) - FILETIME_UNIX_OFFSET_SECONDS;
    let nanos = u32::try_from(value.rem_euclid(10_000_000) * 100).ok()?;
    Some(Timestamp::from_unix(secs, nanos))
}

/// Opens an entry, file or folder, without following a final link, only
/// to read its attributes.
fn open_for_facts(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// The identity of the open file: the 128-bit id where the file system
/// has one (`FileIdInfo`, NTFS and ReFS), else the 64-bit index.
fn identity_of(file: &File) -> io::Result<Identity> {
    let handle = file.as_raw_handle();
    let mut info = FILE_ID_INFO::default();
    // SAFETY: `handle` is valid while `file` lives; `info` is a
    // FILE_ID_INFO and the size passed is its size, so the call writes
    // inside it.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&raw mut info).cast::<c_void>(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok != 0 {
        return Ok(Identity {
            volume: info.VolumeSerialNumber,
            id: u128::from_le_bytes(info.FileId.Identifier),
        });
    }
    let by_handle = by_handle(file)?;
    Ok(Identity {
        volume: u64::from(by_handle.dwVolumeSerialNumber),
        id: (u128::from(by_handle.nFileIndexHigh) << 32) | u128::from(by_handle.nFileIndexLow),
    })
}

fn by_handle(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is valid while `file` lives, and `info` is the
    // structure the call fills.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(info)
    }
}

fn basic_info(file: &File) -> io::Result<FILE_BASIC_INFO> {
    let mut info = FILE_BASIC_INFO::default();
    // SAFETY: the handle is valid while `file` lives; `info` is a
    // FILE_BASIC_INFO and the size passed is its size.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&raw mut info).cast::<c_void>(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(info)
    }
}

/// The identity of the entry at `path`, without following a final link.
pub(crate) fn identity(path: &Path) -> io::Result<Identity> {
    identity_of(&open_for_facts(path)?)
}

/// What a listing knows once per folder: the folder's volume, which every
/// entry read from it shares (a folder mounted from another volume is a
/// reparse point, listed as a link and never entered).
pub(crate) fn list_context(folder: &Path) -> EntryContext {
    EntryContext {
        volume: identity(folder).ok().map(|id| id.volume),
        ..EntryContext::default()
    }
}

/// Fills an entry with what this platform's metadata holds: the
/// attributes, hidden by the attribute (a leading dot hides nothing on
/// Windows), and the volume from the listing.
pub(crate) fn fill_platform(entry: &mut Entry, metadata: &Metadata, context: &EntryContext) {
    let attributes = metadata.file_attributes();
    entry.attributes = Some(Attributes::Windows(attributes));
    entry.hidden = attributes & ATTRIBUTE_HIDDEN != 0;
    entry.volume = context.volume;
}

/// The facts a stat reads through a handle: the identity, the link count
/// and the change time. Missing facts stay unknown; a stat never fails
/// for them.
pub(crate) fn fill_from_handle(entry: &mut Entry, path: &Path) {
    let Ok(file) = open_for_facts(path) else {
        return;
    };
    if let Ok(identity) = identity_of(&file) {
        entry.volume = Some(identity.volume);
        entry.inode = Some(identity.inode());
    }
    if let Ok(info) = by_handle(&file) {
        entry.hard_links = Some(u64::from(info.nNumberOfLinks));
    }
    if let Ok(info) = basic_info(&file) {
        entry.changed = timestamp_from_filetime(info.ChangeTime);
    }
}

/// Renames without ever replacing. Phase 0's body, replaced with
/// `MoveFileExW` below.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// What `GetDriveTypeW` answers, in diroxide's words: a fixed disk, a
/// removable drive (a stick, a card, an optical drive), a network drive,
/// or not known (a RAM disk among them, which has no trash).
pub(crate) fn drive_kind(code: u32) -> VolumeKind {
    match code {
        DRIVE_FIXED => VolumeKind::Fixed,
        DRIVE_REMOVABLE | DRIVE_CDROM => VolumeKind::Removable,
        DRIVE_REMOTE => VolumeKind::Network,
        _ => VolumeKind::Unknown,
    }
}

/// The kind of the drive whose root is `root` (`D:\`, `\\server\share\`,
/// or a mounted folder's root). Asks the volume manager; the drive's
/// media is not read.
fn drive_type(root: &Path) -> VolumeKind {
    let root = wide(root);
    // SAFETY: `root` is a NUL-terminated wide string that lives across the
    // call, which only reads it.
    drive_kind(unsafe { GetDriveTypeW(root.as_ptr()) })
}

/// The drive letters in use, from `GetLogicalDrives`'s bitmask: nothing
/// is asked of any drive.
fn drive_letters() -> Vec<char> {
    // SAFETY: GetLogicalDrives takes no arguments and returns a bitmask.
    let mask = unsafe { GetLogicalDrives() };
    (b'A'..=b'Z')
        .enumerate()
        .filter(|(bit, _)| mask & (1 << bit) != 0)
        .map(|(_, letter)| char::from(letter))
        .collect()
}

/// The tops of the local file system: the drives, from the bitmask alone.
pub(crate) fn roots() -> Vec<Place> {
    drive_letters()
        .into_iter()
        .filter_map(|letter| Place::parse(&format!("{letter}:\\")).ok())
        .collect()
}

/// The drives with their kinds, for the places list: the bitmask, then
/// the drive type of each root, which does not read the drive's media.
pub(crate) fn drives() -> Vec<(Place, VolumeKind)> {
    drive_letters()
        .into_iter()
        .filter_map(|letter| {
            let root = format!("{letter}:\\");
            let kind = drive_type(Path::new(&root));
            Place::parse(&root).ok().map(|place| (place, kind))
        })
        .collect()
}

/// The root of the volume a path is on (`GetVolumePathNameW`): a drive,
/// a share, or the folder a volume is mounted on; with a final backslash.
fn volume_root(path: &Path) -> io::Result<PathBuf> {
    let path = wide(path);
    let mut buffer = vec![0u16; path.len() + 64];
    let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: `path` is NUL-terminated; `buffer` holds `capacity` units,
    // the length the call is told it may write.
    let ok = unsafe { GetVolumePathNameW(path.as_ptr(), buffer.as_mut_ptr(), capacity) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(path_from_wide(&buffer))
}

/// The drive root a path starts with, when the volume cannot be asked.
fn written_root(path: &Path) -> PathBuf {
    path.ancestors()
        .last()
        .map_or_else(|| path.to_path_buf(), Path::to_path_buf)
}

/// What `GetVolumeInformationW` says of a volume root: its label, the
/// longest name, the file system's flags and name.
fn volume_information(root: &Path) -> Option<(String, u32, u32, String)> {
    let root = wide(root);
    let mut label = [0u16; 261];
    let mut file_system = [0u16; 261];
    let (mut serial, mut max_component, mut flags) = (0u32, 0u32, 0u32);
    // SAFETY: `root` is NUL-terminated; each buffer is passed with its own
    // length; the three numbers are written through valid pointers.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &raw mut serial,
            &raw mut max_component,
            &raw mut flags,
            file_system.as_mut_ptr(),
            file_system.len() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let text = |buffer: &[u16]| path_from_wide(buffer).to_string_lossy().into_owned();
    Some((text(&label), max_component, flags, text(&file_system)))
}

/// The facts of the volume a path is on: the volume root asked of the
/// path itself (so a folder mounted from another drive is judged by that
/// drive), its drive type, and what the volume says of itself.
pub(crate) fn volume_facts(path: &Path) -> VolumeFacts {
    let root = volume_root(path).unwrap_or_else(|_| written_root(path));
    let network_path = root.as_os_str().encode_wide().take(2).eq([92, 92]);
    let kind = drive_type(&root);
    let mut facts = VolumeFacts {
        kind,
        network_path,
        ..VolumeFacts::default()
    };
    if let Some((label, max_component, flags, file_system)) = volume_information(&root) {
        facts.label = (!label.is_empty()).then_some(label);
        facts.max_name = usize::try_from(max_component).ok().filter(|&n| n > 0);
        facts.read_only = flags & FILE_READ_ONLY_VOLUME != 0;
        facts.file_system = (!file_system.is_empty()).then_some(file_system);
    }
    facts.root = Some(root);
    facts
}

/// The free space the caller may use and the volume's size, in bytes.
pub(crate) fn space(path: &Path) -> io::Result<(u64, u64)> {
    let root = volume_root(path)?;
    let root = wide(&root);
    let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
    // SAFETY: `root` is NUL-terminated; the three pointers are to u64s
    // that live across the call.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            root.as_ptr(),
            &raw mut free,
            &raw mut total,
            &raw mut total_free,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((free, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(units: &[u16]) -> String {
        String::from_utf16_lossy(&units[..units.len() - 1])
    }

    #[test]
    fn only_long_paths_get_the_prefix() {
        assert_eq!(text(&wide(Path::new(r"D:\a\b"))), r"D:\a\b");
        let long = format!(r"D:\{}", "x".repeat(260));
        assert_eq!(text(&wide(Path::new(&long))), format!(r"\\?\{long}"));
        let share = format!(r"\\srv\sh\{}", "y".repeat(260));
        assert_eq!(
            text(&wide(Path::new(&share))),
            format!(r"\\?\UNC\srv\sh\{}", "y".repeat(260))
        );
        let prefixed = format!(r"\\?\D:\{}", "z".repeat(260));
        assert_eq!(text(&wide(Path::new(&prefixed))), prefixed);
    }

    #[test]
    fn paths_read_back_lose_the_prefix() {
        let buffer: Vec<u16> = r"\\?\D:\x\y".encode_utf16().chain([0, 65]).collect();
        assert_eq!(path_from_wide(&buffer), PathBuf::from(r"D:\x\y"));
    }

    #[test]
    fn drive_types_read_as_kinds() {
        assert_eq!(drive_kind(3), VolumeKind::Fixed);
        assert_eq!(drive_kind(2), VolumeKind::Removable);
        assert_eq!(drive_kind(5), VolumeKind::Removable);
        assert_eq!(drive_kind(4), VolumeKind::Network);
        assert_eq!(drive_kind(6), VolumeKind::Unknown);
        assert_eq!(drive_kind(0), VolumeKind::Unknown);
    }

    #[test]
    fn this_checkout_is_on_a_fixed_drive_with_its_facts() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let facts = volume_facts(here);
        assert!(facts.root.is_some());
        assert!(!facts.network_path);
        assert!(facts.file_system.is_some());
        let (free, total) = space(here).unwrap();
        assert!(free > 0 && total >= free);
        assert!(!roots().is_empty());
    }

    #[test]
    fn filetimes_become_timestamps() {
        // 1970-01-01T00:00:01.5 in FILETIME units.
        let t = (FILETIME_UNIX_OFFSET_SECONDS + 1) * 10_000_000 + 5_000_000;
        assert_eq!(
            timestamp_from_filetime(t),
            Some(Timestamp::from_unix(1, 500_000_000))
        );
        assert_eq!(timestamp_from_filetime(0), None);
    }
}
