//! Windows: alternate data streams, the extended items of NTFS and ReFS
//! (alpha.7, R12's contract request 2; R30 commit 48).
//!
//! Streams are listed with `FindFirstStreamW` and `FindNextStreamW` and
//! read, written and removed through the standard library on the path
//! `FILE:STREAM`. That path, the one place a colon joins a name, is built
//! only by [`stream_path`], which refuses a stream name that is empty or
//! holds a colon, a slash, a backslash or a NUL, so no name can reach the
//! file's own unnamed stream. Spike M2 (on a file of the lane's own,
//! marked by itself): `std::fs::remove_file` on `FILE:Zone.Identifier`
//! removes only the stream, the file's bytes and size untouched, also on
//! a read-only file; writing a stream changes the file's modified time
//! and removing one does not; a folder with no streams answers
//! `ERROR_HANDLE_EOF`.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard, WIN32_FIND_STREAM_DATA,
};

use super::windows::wide;
use crate::error::VfsError;
use crate::extended::{ExtendedItem, ExtendedKind};

/// `ERROR_HANDLE_EOF`: no more streams, or none at all.
const ERROR_HANDLE_EOF: i32 = 38;

/// `ERROR_INVALID_PARAMETER`: the file system has no streams.
const ERROR_INVALID_PARAMETER: i32 = 87;

/// The path of a named stream of `file`: `FILE:STREAM`. The one function
/// that joins a path and a stream name with a colon; a stream name that
/// is empty, or holds a colon, a slash, a backslash or a NUL, is refused
/// with [`VfsError::NameInvalid`].
pub(crate) fn stream_path(file: &Path, stream: &str) -> Result<PathBuf, VfsError> {
    if stream.is_empty()
        || stream.len() > 255
        || stream.contains([':', '/', '\\', '\0'])
        || stream.trim().is_empty()
    {
        return Err(VfsError::NameInvalid);
    }
    let mut text = OsString::from(file.as_os_str());
    text.push(":");
    text.push(stream);
    Ok(PathBuf::from(text))
}

/// Whether a file system holds streams: NTFS and ReFS, by the name the
/// volume gives.
pub(crate) fn extended_on(file_system: Option<&str>) -> bool {
    file_system.is_some_and(|fs| fs.eq_ignore_ascii_case("ntfs") || fs.eq_ignore_ascii_case("refs"))
}

/// The stream's own name from what `FindFirstStreamW` gives
/// (`:Zone.Identifier:$DATA`), or `None` for the unnamed main stream.
fn stream_name(raw: &str) -> Option<&str> {
    let named = raw.strip_prefix(':')?.strip_suffix(":$DATA")?;
    (!named.is_empty()).then_some(named)
}

/// The named streams of the entry at `path`, in the system's order, with
/// their sizes. None is managed by the system: a stream the system
/// writes (a name with a control character, such as the old summary
/// information) is marked so.
pub(crate) fn extended_list(path: &Path) -> Result<Vec<ExtendedItem>, VfsError> {
    let path_wide = wide(path);
    let mut data = WIN32_FIND_STREAM_DATA::default();
    // SAFETY: `path_wide` is NUL-terminated and lives across the call;
    // `data` is a WIN32_FIND_STREAM_DATA, what FindStreamInfoStandard
    // writes; the flags must be zero.
    let handle = unsafe {
        FindFirstStreamW(
            path_wide.as_ptr(),
            FindStreamInfoStandard,
            (&raw mut data).cast(),
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(ERROR_HANDLE_EOF) => Ok(Vec::new()),
            Some(ERROR_INVALID_PARAMETER) => Err(not_here()),
            _ => Err(error.into()),
        };
    }
    let mut items = Vec::new();
    let mut failed = None;
    loop {
        let end = data
            .cStreamName
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(data.cStreamName.len());
        let raw = String::from_utf16_lossy(&data.cStreamName[..end]);
        if let Some(name) = stream_name(&raw) {
            let size = u64::try_from(data.StreamSize).unwrap_or(0);
            let item = ExtendedItem::new(name, size, ExtendedKind::Stream);
            items.push(if name.chars().any(char::is_control) {
                item.managed()
            } else {
                item
            });
        }
        // SAFETY: `handle` came from FindFirstStreamW and is not closed
        // yet; `data` is the same structure the first call wrote.
        let more = unsafe { FindNextStreamW(handle, (&raw mut data).cast()) };
        if more == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_HANDLE_EOF) {
                failed = Some(error);
            }
            break;
        }
    }
    // SAFETY: `handle` came from FindFirstStreamW and is closed once.
    unsafe { FindClose(handle) };
    match failed {
        Some(error) => Err(error.into()),
        None => Ok(items),
    }
}

/// The size of one stream, refused as missing when it is not there.
#[cfg(test)]
pub(crate) fn extended_size(path: &Path, name: &str) -> Result<u64, VfsError> {
    Ok(fs::metadata(stream_path(path, name)?)?.len())
}

/// One stream's bytes, refused with [`VfsError::FileTooLarge`] when it
/// holds more than `max_bytes`; never cut short.
pub(crate) fn extended_read(
    path: &Path,
    name: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, VfsError> {
    let file = File::open(stream_path(path, name)?)?;
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(VfsError::FileTooLarge);
    }
    Ok(bytes)
}

/// Sets one stream to these bytes, making it when it is missing.
pub(crate) fn extended_write(path: &Path, name: &str, value: &[u8]) -> Result<(), VfsError> {
    if !path.exists() {
        return Err(VfsError::NotFound);
    }
    fs::write(stream_path(path, name)?, value)?;
    Ok(())
}

/// Removes one stream; the file and its other streams stay.
pub(crate) fn extended_remove(path: &Path, name: &str) -> Result<(), VfsError> {
    fs::remove_file(stream_path(path, name)?)?;
    Ok(())
}

fn not_here() -> VfsError {
    VfsError::unsupported("file", "keep streams on this drive")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::scratch;

    #[test]
    fn the_colon_path_is_built_in_one_place_and_refuses_odd_names() {
        let file = Path::new(r"D:\work\setup.exe");
        assert_eq!(
            stream_path(file, "Zone.Identifier").unwrap(),
            PathBuf::from(r"D:\work\setup.exe:Zone.Identifier")
        );
        for bad in [
            "",
            " ",
            ":",
            "a:b",
            "a/b",
            "a\\b",
            "a\0b",
            "Zone.Identifier:$DATA",
        ] {
            assert_eq!(
                stream_path(file, bad),
                Err(VfsError::NameInvalid),
                "{bad:?}"
            );
        }
        assert_eq!(
            stream_name(":Zone.Identifier:$DATA"),
            Some("Zone.Identifier")
        );
        assert_eq!(stream_name("::$DATA"), None);
        assert_eq!(stream_name("garbage"), None);
        assert!(extended_on(Some("NTFS")));
        assert!(extended_on(Some("ReFS")));
        assert!(!extended_on(Some("exFAT")));
        assert!(!extended_on(None));
    }

    /// Spike M2 as a test, on a file of the test's own under the build's
    /// target folder: mark, list, read, remove, and the file stays whole.
    #[test]
    fn a_stream_is_listed_read_and_removed_and_the_file_stays() {
        let dir = scratch();
        let file = dir.path().join("marked.txt");
        fs::write(&file, b"our own file\n").unwrap();
        if !extended_on(
            crate::local::volumes::facts_at(dir.path())
                .file_system
                .as_deref(),
        ) {
            return;
        }
        assert_eq!(extended_list(&file).unwrap(), Vec::new());
        let mark = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.org/a\r\n";
        extended_write(&file, "Zone.Identifier", mark).unwrap();
        let listed = extended_list(&file).unwrap();
        assert_eq!(
            listed,
            [ExtendedItem::new(
                "Zone.Identifier",
                mark.len() as u64,
                ExtendedKind::Stream
            )]
        );
        assert_eq!(
            extended_size(&file, "Zone.Identifier").unwrap(),
            mark.len() as u64
        );
        assert_eq!(extended_read(&file, "Zone.Identifier", 1024).unwrap(), mark);
        assert_eq!(
            extended_read(&file, "Zone.Identifier", 4),
            Err(VfsError::FileTooLarge)
        );
        extended_remove(&file, "Zone.Identifier").unwrap();
        assert_eq!(extended_list(&file).unwrap(), Vec::new());
        assert_eq!(fs::read(&file).unwrap(), b"our own file\n");
        assert_eq!(
            extended_remove(&file, "Zone.Identifier"),
            Err(VfsError::NotFound)
        );
        // A folder has no streams here.
        assert_eq!(extended_list(dir.path()).unwrap(), Vec::new());
        assert_eq!(
            extended_list(&dir.path().join("gone.txt")),
            Err(VfsError::NotFound)
        );
    }
}
