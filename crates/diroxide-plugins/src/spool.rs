//! The spool: the folder where file contents travel between the host and
//! a plugin (`spool_path` in `provider/read`, `provider/write_chunk`,
//! `fs/read` and `fs/write`), confined so the host is never a confused
//! deputy (R22, section 7.2; spike S8).
//!
//! - The host makes the folder, readable by the owner alone on Unix
//!   (Windows' own state folder is already the owner's).
//! - Every `spool_path` must name a plain file **directly inside** the
//!   folder: a bare name, or a full path whose folder is the spool's,
//!   written the same way (an 8.3 short form or another spelling is
//!   refused, never resolved). Separators in the name, `.` and `..`,
//!   a colon (an alternate data stream), a device name, a trailing dot or
//!   space, hidden characters, a verbatim or device prefix and
//!   `GLOBALROOT` are refused before anything is opened.
//! - **Reading from the spool** opens the name without following a link:
//!   on Windows with the reparse-point flag, then the opened handle must be
//!   a plain file with no reparse point; on Unix the opened handle must be
//!   the very entry the name had before the open (device and inode), and
//!   that entry a plain file, so a link, or a swap between the check and
//!   the use, is refused.
//! - **Writing into the spool** makes a new file and never opens one that
//!   exists, so a planted link or hard link is never written through.
//! - The folder has a size cap; the host removes only the files it made.
//!
//! Refusals are -32030 and logged by the caller. A hard link the plugin
//! made inside the folder to a file elsewhere reads as that file: the
//! plugin could read the file itself, so the host lends it no new reach.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use diroxide_vfs::{NameIssue, NameRules, check_name_with};

use crate::protocol::{ErrorKind, RpcError};

/// The most the spool folder may hold: 4 GiB.
pub const DEFAULT_SPOOL_CAP: u64 = 4 * 1024 * 1024 * 1024;

/// Windows' flag that opens a reparse point itself instead of what it
/// points to.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// Windows' attribute of a reparse point (a symbolic link, a junction).
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Why a spool path was refused, meaning first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpoolRefusal {
    /// The path leaves the spool folder, or names a folder in it.
    Outside(String),
    /// The name breaks a rule.
    Name(String),
    /// The entry is a link, a folder or a device, not a plain file.
    NotAFile,
    /// Nothing has that name.
    Missing,
    /// A file of that name exists, and the host writes only new files.
    Exists,
    /// The spool would pass its cap.
    Full,
    /// The system refused, in its words.
    Io(String),
}

impl SpoolRefusal {
    /// The refusal as the plugin's answer: -32030 for confinement, -32010
    /// for a missing file, -32021 for the cap.
    pub fn to_rpc(&self) -> RpcError {
        match self {
            SpoolRefusal::Outside(why) => RpcError::new(
                ErrorKind::DeniedByHost,
                format!("the spool path is refused: {why}"),
            ),
            SpoolRefusal::Name(why) => RpcError::new(
                ErrorKind::DeniedByHost,
                format!("the spool name is refused: {why}"),
            ),
            SpoolRefusal::NotAFile => RpcError::new(
                ErrorKind::DeniedByHost,
                "the spool path is not a plain file",
            ),
            SpoolRefusal::Missing => {
                RpcError::new(ErrorKind::NotFound, "the spool file does not exist")
            }
            SpoolRefusal::Exists => RpcError::new(
                ErrorKind::DeniedByHost,
                "the spool file exists; name a new one",
            ),
            SpoolRefusal::Full => RpcError::new(ErrorKind::TooLarge, "the spool is full"),
            SpoolRefusal::Io(words) => RpcError::new(ErrorKind::Failed, words.clone()),
        }
    }
}

/// One plugin's spool folder.
#[derive(Debug)]
pub struct Spool {
    folder: PathBuf,
    cap: u64,
    made: Mutex<Vec<PathBuf>>,
    next: AtomicU64,
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let text = |p: &Path| {
        let t = p.to_string_lossy().replace('\\', "/");
        let t = t.trim_end_matches('/').to_owned();
        if cfg!(windows) { t.to_lowercase() } else { t }
    };
    text(a) == text(b)
}

/// The windows rules apply to spool names on every system: what a plugin
/// may name must be portable.
const PORTABLE: NameRules = NameRules {
    windows: true,
    max_length: Some(200),
    length_in_utf16: true,
};

impl Spool {
    /// Makes the folder if needed, the owner's alone on Unix.
    pub fn create(folder: impl Into<PathBuf>) -> io::Result<Self> {
        let folder = folder.into();
        fs::create_dir_all(&folder)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            folder,
            cap: DEFAULT_SPOOL_CAP,
            made: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        })
    }

    /// The same spool with another cap, for tests.
    pub fn with_cap(mut self, cap: u64) -> Self {
        self.cap = cap;
        self
    }

    /// The folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Where a `spool_path` points, by its text alone: a plain name
    /// directly inside the folder, or the refusal.
    pub fn confine(&self, spool_path: &str) -> Result<PathBuf, SpoolRefusal> {
        let unified = spool_path.replace('\\', "/");
        if spool_path.is_empty() {
            return Err(SpoolRefusal::Name("it is empty".to_owned()));
        }
        if spool_path.contains('\0') {
            return Err(SpoolRefusal::Name("it holds a NUL".to_owned()));
        }
        if unified.starts_with("//?/")
            || unified.starts_with("//./")
            || unified.starts_with("/??/")
            || unified.to_ascii_uppercase().contains("GLOBALROOT")
        {
            return Err(SpoolRefusal::Outside(
                "a verbatim or device path".to_owned(),
            ));
        }
        let name = match unified.rsplit_once('/') {
            None => spool_path,
            Some((parent, name)) => {
                let parent = if parent.is_empty() { "/" } else { parent };
                let parent = Path::new(parent);
                if !parent.is_absolute() || !same_folder(parent, &self.folder) {
                    return Err(SpoolRefusal::Outside(
                        "it is not directly in the spool folder".to_owned(),
                    ));
                }
                name
            }
        };
        if name.contains(':') {
            return Err(SpoolRefusal::Name(
                "a colon names an alternate stream".to_owned(),
            ));
        }
        if let Some(issue) = check_name_with(name, &[], &PORTABLE)
            .into_iter()
            .find(|issue| issue.refused(&PORTABLE) || *issue == NameIssue::HiddenCharacters)
        {
            return Err(SpoolRefusal::Name(issue.describe()));
        }
        Ok(self.folder.join(name))
    }

    /// Opens a spool file the plugin wrote, for the host to read, without
    /// following a link.
    pub fn open_to_read(&self, spool_path: &str) -> Result<File, SpoolRefusal> {
        let path = self.confine(spool_path)?;
        open_plain_file(&path)
    }

    /// Makes a new spool file the plugin named, for the host to write
    /// into; an existing name, link or not, is refused.
    pub fn create_to_write(&self, spool_path: &str, size: u64) -> Result<File, SpoolRefusal> {
        let path = self.confine(spool_path)?;
        self.reserve(size)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                io::ErrorKind::AlreadyExists => SpoolRefusal::Exists,
                _ => SpoolRefusal::Io(e.to_string()),
            })?;
        self.made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path);
        Ok(file)
    }

    /// Makes a new spool file under a name the host chooses (for
    /// `provider/read` and `provider/write_chunk`), and returns its full
    /// path for the plugin.
    pub fn new_file(&self, size: u64) -> Result<(PathBuf, File), SpoolRefusal> {
        loop {
            let n = self.next.fetch_add(1, Ordering::SeqCst);
            let name = format!("diro-{}-{n}.part", std::process::id());
            match self.create_to_write(&name, size) {
                Ok(file) => return Ok((self.folder.join(name), file)),
                Err(SpoolRefusal::Exists) => continue,
                Err(other) => return Err(other),
            }
        }
    }

    /// What the spool holds now, in bytes: the plain files directly in it.
    pub fn used(&self) -> u64 {
        fs::read_dir(&self.folder)
            .map(|items| {
                items
                    .flatten()
                    .filter_map(|item| item.metadata().ok())
                    .filter(fs::Metadata::is_file)
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    fn reserve(&self, size: u64) -> Result<(), SpoolRefusal> {
        if self.used().saturating_add(size) > self.cap {
            Err(SpoolRefusal::Full)
        } else {
            Ok(())
        }
    }

    /// Removes the files the host made that are still directly in the
    /// folder, by the exact paths it made; nothing else.
    pub fn remove_made(&self) {
        let made = std::mem::take(&mut *self.made.lock().unwrap_or_else(PoisonError::into_inner));
        for path in made {
            let inside = path.parent().is_some_and(|p| same_folder(p, &self.folder));
            let plain = fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
            if inside && plain {
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// Forgets a file the host made, once the plugin owns it again.
    pub fn forget(&self, path: &Path) {
        self.made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|made| made != path);
    }
}

/// Opens a plain file without following a link at its name.
#[cfg(windows)]
fn open_plain_file(path: &Path) -> Result<File, SpoolRefusal> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => SpoolRefusal::Missing,
            _ if path.is_dir() => SpoolRefusal::NotAFile,
            _ => SpoolRefusal::Io(e.to_string()),
        })?;
    let metadata = file
        .metadata()
        .map_err(|e| SpoolRefusal::Io(e.to_string()))?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(SpoolRefusal::NotAFile);
    }
    Ok(file)
}

/// Opens a plain file and checks the handle is the entry the name held a
/// moment before, so a link or a swap is refused.
#[cfg(unix)]
fn open_plain_file(path: &Path) -> Result<File, SpoolRefusal> {
    let before = fs::symlink_metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => SpoolRefusal::Missing,
        _ => SpoolRefusal::Io(e.to_string()),
    })?;
    if !before.file_type().is_file() {
        return Err(SpoolRefusal::NotAFile);
    }
    let file = File::open(path).map_err(|e| SpoolRefusal::Io(e.to_string()))?;
    let after = file
        .metadata()
        .map_err(|e| SpoolRefusal::Io(e.to_string()))?;
    if !same_entry(&before, &after) {
        return Err(SpoolRefusal::NotAFile);
    }
    Ok(file)
}

/// Whether two looks are at one entry: the same device and inode, and a
/// plain file.
#[cfg(unix)]
fn same_entry(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    after.is_file() && before.dev() == after.dev() && before.ino() == after.ino()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn spool() -> (tempfile::TempDir, Spool) {
        let state = tempfile::tempdir().unwrap();
        let spool = Spool::create(state.path().join("spool")).unwrap();
        (state, spool)
    }

    /// Spike S8, the table: every hostile spool path is refused by its
    /// text before anything is opened.
    #[test]
    fn hostile_spool_paths_are_refused() {
        let (state, spool) = spool();
        let folder = spool.folder().display().to_string();
        let outside = state.path().join("secret.txt").display().to_string();
        let mut hostile = vec![
            String::new(),
            "..".to_owned(),
            ".".to_owned(),
            "../secret.txt".to_owned(),
            "a/b".to_owned(),
            "a\\b".to_owned(),
            outside,
            format!("{folder}/../secret.txt"),
            format!("{folder}/sub/x.part"),
            "x.part:hidden".to_owned(),
            "x.part::$DATA".to_owned(),
            "CON".to_owned(),
            "nul.txt".to_owned(),
            "COM1".to_owned(),
            "trailing.".to_owned(),
            "trailing ".to_owned(),
            "bidi\u{202E}txt.exe".to_owned(),
            "nul\0x".to_owned(),
            "\\\\?\\C:\\Windows\\win.ini".to_owned(),
            "\\\\.\\PhysicalDrive0".to_owned(),
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1\\x".to_owned(),
            "\\\\server\\share\\x.part".to_owned(),
            "/etc/passwd".to_owned(),
            "relative/spool/x.part".to_owned(),
        ];
        // An 8.3 short form of the folder is another spelling: refused.
        hostile.push(format!("{}~1/x.part", folder.trim_end_matches("spool")));
        for text in &hostile {
            let refused = spool.confine(text);
            assert!(refused.is_err(), "{text:?} was accepted: {refused:?}");
            assert_eq!(
                refused.unwrap_err().to_rpc().code,
                -32030,
                "{text:?} is not -32030"
            );
        }
    }

    #[test]
    fn a_plain_name_or_a_full_path_inside_is_accepted() {
        let (_state, spool) = spool();
        let inside = spool.confine("x.part").unwrap();
        assert_eq!(inside, spool.folder().join("x.part"));
        let full = spool.folder().join("y.part").display().to_string();
        assert_eq!(spool.confine(&full).unwrap(), spool.folder().join("y.part"));
        if cfg!(windows) {
            // A change of case is the same folder on Windows.
            let upper = spool
                .folder()
                .join("z.part")
                .display()
                .to_string()
                .to_uppercase();
            assert!(spool.confine(&upper).is_ok(), "{upper}");
        }
    }

    #[test]
    fn writing_makes_new_files_only_and_reading_needs_a_plain_file() {
        let (_state, spool) = spool();
        let mut file = spool.create_to_write("a.part", 5).unwrap();
        file.write_all(b"hello").unwrap();
        drop(file);
        assert_eq!(
            spool.create_to_write("a.part", 5).unwrap_err(),
            SpoolRefusal::Exists
        );
        let mut text = String::new();
        spool
            .open_to_read("a.part")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "hello");
        assert_eq!(
            spool.open_to_read("missing.part").unwrap_err(),
            SpoolRefusal::Missing
        );
        fs::create_dir(spool.folder().join("folder.part")).unwrap();
        assert_eq!(
            spool.open_to_read("folder.part").unwrap_err(),
            SpoolRefusal::NotAFile
        );
        let (path, _file) = spool.new_file(0).unwrap();
        assert!(path.starts_with(spool.folder()));
        spool.remove_made();
        assert!(!spool.folder().join("a.part").exists());
        assert!(!path.exists());
        assert!(spool.folder().join("folder.part").is_dir());
    }

    #[test]
    fn the_cap_is_kept() {
        let state = tempfile::tempdir().unwrap();
        let spool = Spool::create(state.path().join("spool"))
            .unwrap()
            .with_cap(10);
        let mut file = spool.create_to_write("a.part", 8).unwrap();
        file.write_all(b"12345678").unwrap();
        drop(file);
        assert_eq!(
            spool.create_to_write("b.part", 4).unwrap_err(),
            SpoolRefusal::Full
        );
        assert_eq!(SpoolRefusal::Full.to_rpc().code, -32021);
    }

    /// Spike S8, the links: a hard link planted at a name the host would
    /// write is never written through; a symbolic link inside the spool
    /// that points out is never read through (where the system lets this
    /// account make one).
    #[test]
    fn links_inside_the_spool_are_never_followed() {
        let (state, spool) = spool();
        let secret = state.path().join("secret.txt");
        fs::write(&secret, b"owner's secret").unwrap();
        let planted = spool.folder().join("planted.part");
        fs::hard_link(&secret, &planted).unwrap();
        assert_eq!(
            spool.create_to_write("planted.part", 1).unwrap_err(),
            SpoolRefusal::Exists
        );
        assert_eq!(fs::read(&secret).unwrap(), b"owner's secret");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&secret, spool.folder().join("link.part"));
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&secret, spool.folder().join("link.part"));
        match made {
            Ok(()) => {
                assert_eq!(
                    spool.open_to_read("link.part").unwrap_err(),
                    SpoolRefusal::NotAFile
                );
                assert_eq!(
                    spool.create_to_write("link.part", 1).unwrap_err(),
                    SpoolRefusal::Exists
                );
            }
            Err(e) => println!("S8: no symbolic link for this account: {e}"),
        }
    }

    /// Spike S8, the swap: the check is made on the opened handle, so an
    /// entry swapped after the name was looked at is refused.
    #[cfg(unix)]
    #[test]
    fn a_swap_between_check_and_use_is_refused() {
        let (_state, spool) = spool();
        let a = spool.folder().join("a.part");
        let b = spool.folder().join("b.part");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let before = fs::symlink_metadata(&a).unwrap();
        let other = File::open(&b).unwrap().metadata().unwrap();
        assert!(!same_entry(&before, &other));
        let same = File::open(&a).unwrap().metadata().unwrap();
        assert!(same_entry(&before, &same));
    }
}
