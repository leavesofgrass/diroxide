//! The spool of a nested archive (research/archives.md 5.4 and group F):
//! an inner archive needs a seekable source, so a compressed one is read
//! out into memory up to [`Limits::max_spool_memory_bytes`], and past that
//! into a file in the spool folder the app names (the state folder's
//! `spool`, never the platform's temporary folder, which is drive C on the
//! owner's machine), under one budget for every nested archive open at
//! once. A stored member of a zip needs no spool at all: it is a window of
//! the outer archive's bytes.
//!
//! A spool file has a plain name (`diroxide-spool-` and a number and
//! `.tmp`), is made new so it never replaces anything, and is removed when
//! the last reader of it is dropped; [`sweep`] removes what an earlier run
//! left behind, by that name only.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use diroxide_vfs::{CancelToken, VfsError};

use crate::error::ArchiveError;
use crate::guard::refuse;
use crate::limits::{Limits, limit_words};
use crate::source::Source;

/// The start of every spool file's name.
pub const SPOOL_PREFIX: &str = "diroxide-spool-";

/// The end of every spool file's name.
pub const SPOOL_SUFFIX: &str = ".tmp";

/// How old a left-over spool file must be before [`sweep`] removes it.
pub const SWEEP_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The spool bytes in use, shared by every nested archive open at once.
#[derive(Clone, Debug, Default)]
pub struct SpoolBudget(Arc<AtomicU64>);

impl SpoolBudget {
    /// The bytes on disk now.
    pub fn in_use(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A spool file: read and seek like the file it is, removed when dropped.
struct SpoolFile {
    file: File,
    path: PathBuf,
    charged: u64,
    budget: SpoolBudget,
}

impl Read for SpoolFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for SpoolFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.file.seek(to)
    }
}

impl Drop for SpoolFile {
    fn drop(&mut self) {
        self.budget.0.fetch_sub(self.charged, Ordering::Relaxed);
        // The file this provider made, by the path it made it at.
        let _ = fs::remove_file(&self.path);
    }
}

/// A fresh, plain spool name in `dir`, made new so nothing is replaced.
fn create(dir: &Path) -> Result<(File, PathBuf), VfsError> {
    let base = std::process::id();
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for attempt in 0..64u32 {
        let name = format!("{SPOOL_PREFIX}{base}-{stamp:08x}-{attempt}{SPOOL_SUFFIX}");
        let path = dir.join(&name);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(VfsError::from(e)),
        }
    }
    Err(VfsError::AlreadyExists)
}

/// Reads a member out into a seekable source: in memory up to the memory
/// limit, then into a spool file in `dir` under the shared disk budget,
/// with `free` the bytes free on the spool folder's drive when known. A
/// member too large for memory with no spool folder set is refused.
pub(crate) fn spool(
    mut reader: Box<dyn Read + Send>,
    limits: &Limits,
    dir: Option<&Path>,
    budget: &SpoolBudget,
    free: Option<u64>,
    cancel: &CancelToken,
) -> Result<Source, ArchiveError> {
    let memory = limits.max_spool_memory_bytes;
    let mut held = Vec::new();
    (&mut reader)
        .take(memory.saturating_add(1))
        .read_to_end(&mut held)
        .map_err(crate::guard::from_io)?;
    if held.len() as u64 <= memory {
        return Ok(Source::from_bytes(held));
    }
    let Some(dir) = dir else {
        return Err(refuse(
            "holding an inner archive in memory",
            format!("{}, and no spool folder is set", limit_words(memory)),
        )
        .into());
    };
    let (file, path) = create(dir)?;
    let mut spooled = SpoolFile {
        file,
        path,
        charged: 0,
        budget: budget.clone(),
    };
    let charge = |bytes: u64, spooled: &mut SpoolFile| -> Result<(), ArchiveError> {
        let before = budget.0.fetch_add(bytes, Ordering::Relaxed);
        spooled.charged += bytes;
        if before.saturating_add(bytes) > limits.max_spool_disk_bytes {
            return Err(refuse("spool space", limit_words(limits.max_spool_disk_bytes)).into());
        }
        if let Some(free) = free
            && spooled.charged > free
        {
            return Err(VfsError::Io {
                kind: io::ErrorKind::StorageFull,
                message: format!("the spool folder's drive has {} free", limit_words(free)),
            }
            .into());
        }
        Ok(())
    };
    charge(held.len() as u64, &mut spooled)?;
    spooled.file.write_all(&held).map_err(VfsError::from)?;
    drop(held);
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        cancel.check()?;
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(crate::guard::from_io(e).into()),
        };
        charge(n as u64, &mut spooled)?;
        spooled
            .file
            .write_all(&chunk[..n])
            .map_err(VfsError::from)?;
    }
    spooled.file.flush().map_err(VfsError::from)?;
    spooled.seek(SeekFrom::Start(0)).map_err(VfsError::from)?;
    Source::new(Box::new(spooled)).map_err(|e| VfsError::from(e).into())
}

/// Whether a name is one this provider gives its spool files.
pub fn is_spool_name(name: &str) -> bool {
    name.strip_prefix(SPOOL_PREFIX)
        .and_then(|rest| rest.strip_suffix(SPOOL_SUFFIX))
        .is_some_and(|middle| {
            !middle.is_empty() && middle.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        })
}

/// Removes the spool files an earlier run left in `dir`: regular files
/// directly in it, named as [`is_spool_name`] says, older than
/// [`SWEEP_AFTER`]. Nothing else is touched. Returns how many it removed.
pub fn sweep(dir: &Path) -> io::Result<usize> {
    let mut removed = 0;
    let now = SystemTime::now();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        if !is_spool_name(&name) {
            continue;
        }
        let kind = item.file_type()?;
        if !kind.is_file() || kind.is_symlink() {
            continue;
        }
        let old = item
            .metadata()?
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= SWEEP_AFTER);
        if old {
            fs::remove_file(item.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spool_names_are_plain_and_known() {
        assert!(is_spool_name("diroxide-spool-1234-0abc12ff-0.tmp"));
        assert!(!is_spool_name("diroxide-spool-.tmp"));
        assert!(!is_spool_name("diroxide-spool-../x.tmp"));
        assert!(!is_spool_name("notes.tmp"));
        assert!(!is_spool_name("diroxide-spool-12.tmp.txt"));
    }
}
