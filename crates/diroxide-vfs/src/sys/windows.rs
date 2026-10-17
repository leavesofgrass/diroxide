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
use std::fs::{File, FileTimes, Metadata, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use diroxide_core::{Attributes, Entry, Place, Timestamp};
use windows_sys::Win32::Foundation::{
    ERROR_PRIVILEGE_NOT_HELD, ERROR_REQUEST_ABORTED, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, COPY_FILE_COPY_SYMLINK, COPY_FILE_FAIL_IF_EXISTS, CopyFileExW,
    ExtendedFileIdType, FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_128, FILE_ID_DESCRIPTOR, FILE_ID_INFO,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_WRITE_ATTRIBUTES, FileBasicInfo, FileIdInfo, FileIdType, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, GetLogicalDrives, GetVolumeInformationW, GetVolumePathNameW,
    MOVEFILE_REPLACE_EXISTING, MoveFileExW, OpenFileById, PROGRESS_CANCEL, PROGRESS_CONTINUE,
    SetFileAttributesW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;

use super::Identity;
use crate::cancel::{OpContext, ProgressUpdate};
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

/// Renames without ever replacing: `MoveFileExW` with no flags, which
/// fails when the target is taken (`AlreadyExists`) and across volumes
/// (`CrossesDevices`, since copying is not allowed). Atomic on one volume.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (wide(from), wide(to));
    // SAFETY: both strings are NUL-terminated and live across the call,
    // which only reads them.
    let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Renames over a file or file link the target holds: `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING` alone, so it never copies across volumes
/// (`CrossesDevices`) and the target name is never empty, one step on one
/// volume. The system refuses to replace a folder, a junction or a folder
/// link (access is denied), and then both names are as they were.
pub(crate) fn rename_replacing(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (wide(from), wide(to));
    // SAFETY: both strings are NUL-terminated and live across the call,
    // which only reads them.
    let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Whether metadata read without following describes something removed
/// as a folder: a folder, or a link that is a folder (a junction or a
/// folder symbolic link), which removing deletes as a link only.
pub(crate) fn is_folder_like(metadata: &Metadata) -> bool {
    use std::os::windows::fs::FileTypeExt;
    metadata.is_dir() || metadata.file_type().is_symlink_dir()
}

/// The attribute bits `SetFileAttributesW` can set: read-only, hidden,
/// system, archive, temporary, offline and not content indexed. The
/// others (compressed, encrypted, sparse, reparse point, the cloud bits)
/// are kept as they are.
pub(crate) const SETTABLE_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x100 | 0x1000 | 0x2000;

/// Sets an entry's Windows attributes, keeping the bits that cannot be
/// set. The link itself, never what it points to.
pub(crate) fn set_os_attributes(path: &Path, attributes: u32) -> io::Result<()> {
    let mut value = attributes & SETTABLE_ATTRIBUTES;
    if value == 0 {
        value = FILE_ATTRIBUTE_NORMAL;
    }
    let path = wide(path);
    // SAFETY: `path` is NUL-terminated and lives across the call.
    let ok = unsafe { SetFileAttributesW(path.as_ptr(), value) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Sets an entry's times through a handle opened for its attributes only
/// (a folder too, and a link itself rather than its target). `None`
/// leaves a time as it is.
pub(crate) fn set_times(
    path: &Path,
    created: Option<SystemTime>,
    modified: Option<SystemTime>,
    accessed: Option<SystemTime>,
) -> io::Result<()> {
    use std::os::windows::fs::FileTimesExt;
    let file = OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut times = FileTimes::new();
    if let Some(time) = modified {
        times = times.set_modified(time);
    }
    if let Some(time) = accessed {
        times = times.set_accessed(time);
    }
    if let Some(time) = created {
        times = times.set_created(time);
    }
    file.set_times(times)
}

/// Makes a directory junction at `link` pointing to the full folder path
/// `target`: the folder is made, then its reparse data written; if that
/// fails, the empty folder just made is removed. A junction needs no
/// privilege and cannot point to a network place.
pub(crate) fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    let target = target.to_string_lossy();
    let target = diroxide_core::place::strip_extended_prefix(&target);
    if target.starts_with(r"\\") || target.as_bytes().get(1) != Some(&b':') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a junction needs a full path on a local drive",
        ));
    }
    let buffer = junction_data(&target)?;
    std::fs::create_dir(link)?;
    let written = (|| {
        let folder = OpenOptions::new()
            .access_mode(GENERIC_WRITE)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(link)?;
        let mut returned = 0u32;
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: the handle is valid while `folder` lives; the input
        // buffer is valid for `length` bytes; there is no output buffer and
        // no overlapped structure; `returned` is a valid u32.
        let ok = unsafe {
            DeviceIoControl(
                folder.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast::<c_void>(),
                length,
                std::ptr::null_mut(),
                0,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })();
    if written.is_err() {
        // The folder is the one just made, and empty.
        let _ = std::fs::remove_dir(link);
    }
    written
}

/// The reparse data of a mount point (a junction) to `target`: the tag,
/// the lengths, then the NT name (`\??\` and the path) and the name to
/// print, each followed by a NUL.
fn junction_data(target: &str) -> io::Result<Vec<u8>> {
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const MAXIMUM_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;
    let substitute: Vec<u16> = r"\??\"
        .encode_utf16()
        .chain(target.encode_utf16())
        .collect();
    let print: Vec<u16> = target.encode_utf16().collect();
    let too_long = || io::Error::new(io::ErrorKind::InvalidFilename, "the target is too long");
    let substitute_bytes = u16::try_from(substitute.len() * 2).map_err(|_| too_long())?;
    let print_bytes = u16::try_from(print.len() * 2).map_err(|_| too_long())?;
    let data_length = u16::try_from(8 + substitute.len() * 2 + 2 + print.len() * 2 + 2)
        .map_err(|_| too_long())?;
    let mut buffer = Vec::with_capacity(8 + usize::from(data_length));
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend(data_length.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(substitute_bytes.to_le_bytes());
    buffer.extend((substitute_bytes + 2).to_le_bytes());
    buffer.extend(print_bytes.to_le_bytes());
    for unit in substitute
        .iter()
        .chain([&0u16])
        .chain(print.iter())
        .chain([&0u16])
    {
        buffer.extend(unit.to_le_bytes());
    }
    if buffer.len() > MAXIMUM_REPARSE_DATA_BUFFER_SIZE {
        return Err(too_long());
    }
    Ok(buffer)
}

/// What the copy call's progress routine reads: the context to check and
/// report to, and the name being copied.
struct CopyState<'a> {
    ctx: &'a OpContext,
    name: Option<String>,
}

/// The copy call's progress routine: cancels when the token says so,
/// reports the bytes copied otherwise. A panic in a progress sink must
/// not cross into Windows, so it is caught and cancels the copy.
unsafe extern "system" fn copy_progress(
    total: i64,
    done: i64,
    _stream_size: i64,
    _stream_done: i64,
    _stream: u32,
    _reason: u32,
    _source: HANDLE,
    _destination: HANDLE,
    data: *const c_void,
) -> u32 {
    // SAFETY: `data` is the `CopyState` that `copy_file` passed to
    // CopyFileExW; it outlives the call, and is only read here.
    let state = unsafe { &*data.cast::<CopyState<'_>>() };
    let keep_going = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.ctx.cancel.is_cancelled() {
            return false;
        }
        state.ctx.progress.report(ProgressUpdate {
            bytes_done: u64::try_from(done).unwrap_or(0),
            bytes_total: u64::try_from(total).ok(),
            current: state.name.clone(),
            ..ProgressUpdate::default()
        });
        true
    }))
    .unwrap_or(false);
    if keep_going {
        PROGRESS_CONTINUE
    } else {
        PROGRESS_CANCEL
    }
}

/// Copies a file with `CopyFileExW`, which keeps the modified time, the
/// attributes and the alternate data streams, and lets a share's server
/// copy it without the bytes passing through this computer. Never
/// replaces (`COPY_FILE_FAIL_IF_EXISTS`); a link is copied as a link. The
/// token cancels between chunks, and Windows removes the partial copy.
pub(crate) fn copy_file(from: &Path, to: &Path, ctx: &OpContext) -> io::Result<()> {
    let state = CopyState {
        ctx,
        name: from
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
    };
    let (from, to) = (wide(from), wide(to));
    // SAFETY: both strings are NUL-terminated and live across the call;
    // `copy_progress` has the signature the call expects; `state` lives
    // until the call returns and the routine only reads it; no cancel flag
    // pointer is passed.
    let ok = unsafe {
        CopyFileExW(
            from.as_ptr(),
            to.as_ptr(),
            Some(copy_progress),
            (&raw const state).cast::<c_void>(),
            std::ptr::null_mut(),
            COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_REQUEST_ABORTED as i32) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "the copy was cancelled",
        ))
    } else {
        Err(error)
    }
}

/// Where an entry is now, found by its identity on the volume of `folder`
/// (`OpenFileById`, then `GetFinalPathNameByHandleW`). `Ok(None)` when no
/// entry has that id any more; an error when the volume cannot be asked.
fn path_of_identity(folder: &Path, identity: Identity) -> io::Result<Option<PathBuf>> {
    let hint = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(folder)?;
    let mut descriptor = FILE_ID_DESCRIPTOR {
        dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
        ..FILE_ID_DESCRIPTOR::default()
    };
    if identity.id >> 64 == 0 {
        descriptor.Type = FileIdType;
        descriptor.Anonymous.FileId = i64::from_le_bytes(identity.inode().to_le_bytes());
    } else {
        descriptor.Type = ExtendedFileIdType;
        descriptor.Anonymous.ExtendedFileId = FILE_ID_128 {
            Identifier: identity.id.to_le_bytes(),
        };
    }
    // SAFETY: the hint handle is valid while `hint` lives; `descriptor` is
    // a filled FILE_ID_DESCRIPTOR whose size is in `dwSize`; no security
    // attributes are passed.
    let handle = unsafe {
        OpenFileById(
            hint.as_raw_handle(),
            &raw const descriptor,
            FILE_READ_ATTRIBUTES,
            SHARE_ALL,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        // The id names no entry: removed for good, or never there.
        return Ok(None);
    }
    // SAFETY: `handle` was just returned open by OpenFileById and nothing
    // else owns it; the OwnedHandle closes it exactly once.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut buffer = vec![0u16; 1024];
    loop {
        let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: the handle is valid while `owned` lives; `buffer` holds
        // `capacity` units, the length the call is told.
        let written = unsafe {
            GetFinalPathNameByHandleW(owned.as_raw_handle(), buffer.as_mut_ptr(), capacity, 0)
        };
        let written = usize::try_from(written).unwrap_or(0);
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        if written < buffer.len() {
            return Ok(Some(path_from_wide(&buffer)));
        }
        buffer.resize(written + 1, 0);
    }
}

/// The trash's post-check on Windows: whether the entry with `identity`,
/// which was in `folder`, now lies inside its volume's `$Recycle.Bin`.
/// `Some(false)` when it lies elsewhere or nowhere (removed for good);
/// `None` when the volume cannot be asked.
pub(crate) fn in_recycle_bin(folder: &Path, identity: Identity) -> Option<bool> {
    let root = volume_root(folder).ok()?;
    let Some(now) = path_of_identity(folder, identity).ok()? else {
        return Some(false);
    };
    let first = now
        .strip_prefix(&root)
        .ok()
        .and_then(|rest| rest.components().next())
        .map(|first| first.as_os_str().to_string_lossy().to_lowercase());
    Some(first.as_deref() == Some("$recycle.bin"))
}

/// Whether an error is Windows's "a required privilege is not held":
/// making a symbolic link without Developer Mode.
pub(crate) fn needs_privilege(error: &io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD as i32)
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
    fn the_copy_call_copies_reports_and_cancels() {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Counting(AtomicU64, Mutex<Option<String>>);
        impl crate::cancel::ProgressSink for Counting {
            fn report(&self, update: ProgressUpdate) {
                self.0.store(update.bytes_done, Ordering::Relaxed);
                if let Ok(mut name) = self.1.lock() {
                    *name = update.current;
                }
            }
        }

        let folder = crate::test_support::scratch();
        let source = folder.path().join("big.bin");
        let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &bytes).unwrap();
        let sink = std::sync::Arc::new(Counting(AtomicU64::new(0), Mutex::new(None)));
        let ctx = OpContext::new().with_progress(sink.clone());
        let copy = folder.path().join("copy.bin");
        copy_file(&source, &copy, &ctx).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), bytes);
        assert_eq!(sink.0.load(Ordering::Relaxed), 3_000_000);
        assert_eq!(sink.1.lock().unwrap().as_deref(), Some("big.bin"));
        // Never replaces.
        assert_eq!(
            copy_file(&source, &copy, &OpContext::new())
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        // A cancelled copy leaves nothing.
        let cancelled = OpContext::new();
        cancelled.cancel.cancel();
        let other = folder.path().join("never.bin");
        assert!(copy_file(&source, &other, &cancelled).is_err());
        assert!(!other.exists());
    }

    #[test]
    fn junction_data_has_both_names() {
        let data = junction_data(r"D:\target").unwrap();
        assert_eq!(&data[..4], &0xA000_0003u32.to_le_bytes());
        let names: Vec<u16> = data[16..]
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&names);
        assert_eq!(text, "\\??\\D:\\target\0D:\\target\0");
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
