//! The archive provider: zip, 7z, tar, tar.gz, tar.xz and plain xz, read
//! in place (nothing is extracted to browse) and written as jobs.
//!
//! Places inside an archive are written `course.zip!week1/notes.txt`. The
//! provider reaches the archive file itself through the registry, held
//! weakly so the registry can hold the provider. Limits guard against
//! hostile archives: member size, bytes decompressed, the 7z dictionary,
//! the entry count, a nesting depth of four, and member names that would
//! leave the archive.
//!
//! Phase 0: the formats, limits, name rules and the provider's shape; every
//! provider method says "not built yet". Wave 2 (W2a) owns this crate and
//! brings in `zip`, `sevenz-rust2`, `tar`, `flate2` and `liblzma`.

use std::io::Read;
use std::sync::{Arc, RwLock, Weak};

use diroxide_core::{Entry, Place};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, VfsError, WriteMode,
};

/// The archive formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    /// zip.
    Zip,
    /// 7z.
    SevenZ,
    /// An uncompressed tar.
    Tar,
    /// tar compressed with gzip.
    TarGz,
    /// tar compressed with xz.
    TarXz,
    /// One file compressed with xz, shown as an archive of one member.
    Xz,
}

impl ArchiveFormat {
    /// Every format, in the order the tests and the docs list them.
    pub const ALL: [ArchiveFormat; 6] = [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
        ArchiveFormat::Xz,
    ];

    /// The format of a file name, by its extension, ignoring case.
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        let table = [
            (".tar.gz", ArchiveFormat::TarGz),
            (".tgz", ArchiveFormat::TarGz),
            (".tar.xz", ArchiveFormat::TarXz),
            (".txz", ArchiveFormat::TarXz),
            (".zip", ArchiveFormat::Zip),
            (".7z", ArchiveFormat::SevenZ),
            (".tar", ArchiveFormat::Tar),
            (".xz", ArchiveFormat::Xz),
        ];
        table
            .iter()
            .find(|(ext, _)| lower.len() > ext.len() && lower.ends_with(ext))
            .map(|(_, format)| *format)
    }

    /// The usual extension, with the dot.
    pub const fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => ".zip",
            ArchiveFormat::SevenZ => ".7z",
            ArchiveFormat::Tar => ".tar",
            ArchiveFormat::TarGz => ".tar.gz",
            ArchiveFormat::TarXz => ".tar.xz",
            ArchiveFormat::Xz => ".xz",
        }
    }

    /// The format's name as said: "zip archive", "tar archive compressed
    /// with gzip".
    pub const fn spoken_name(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip archive",
            ArchiveFormat::SevenZ => "7z archive",
            ArchiveFormat::Tar => "tar archive",
            ArchiveFormat::TarGz => "tar archive compressed with gzip",
            ArchiveFormat::TarXz => "tar archive compressed with xz",
            ArchiveFormat::Xz => "file compressed with xz",
        }
    }
}

/// Limits that keep a hostile archive from filling the disk or memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest member, in bytes.
    pub max_member_size: u64,
    /// The most bytes decompressed in one operation.
    pub max_total_bytes: u64,
    /// The largest 7z dictionary, in bytes.
    pub max_7z_dictionary: u64,
    /// The most entries in one archive.
    pub max_entries: usize,
    /// How many archives deep a place may go.
    pub max_nesting: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_member_size: 8 << 30,
            max_total_bytes: 64 << 30,
            max_7z_dictionary: 1 << 30,
            max_entries: 1_000_000,
            max_nesting: 4,
        }
    }
}

/// How many archives deep a place is: 0 for a plain place, 1 inside
/// `a.zip!`, 2 inside `a.zip!b.zip!`.
pub fn nesting_depth(place: &Place) -> usize {
    let mut depth = 0;
    let mut current = place;
    while let Some((container, _)) = current.archive() {
        depth += 1;
        current = container;
    }
    depth
}

/// A member name made safe: forward slashes, no leading slash or drive, no
/// `.`; `None` when `..` would climb out of the archive or nothing is left.
pub fn normalize_member_name(name: &str) -> Option<String> {
    let unified = name.replace('\\', "/");
    let without_drive = match unified.as_bytes() {
        [letter, b':', ..] if letter.is_ascii_alphabetic() => &unified[2..],
        _ => unified.as_str(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in without_drive.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Whether a member is junk that other systems leave in archives and that
/// is hidden from listings: `__MACOSX/`, `.DS_Store`, `Thumbs.db`,
/// `desktop.ini`.
pub fn is_junk(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    let last = normalized.rsplit('/').find(|p| !p.is_empty()).unwrap_or("");
    normalized.starts_with("__MACOSX/")
        || normalized.contains("/__MACOSX/")
        || matches!(
            last.to_lowercase().as_str(),
            ".ds_store" | "thumbs.db" | "desktop.ini"
        )
}

/// What to pack and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackRequest {
    /// The entries to pack.
    pub sources: Vec<Place>,
    /// The new archive.
    pub destination: Place,
    /// Its format.
    pub format: ArchiveFormat,
    /// The compression level, when the format has one.
    pub level: Option<u32>,
}

/// Packs entries into a new archive. Phase 0: not built.
pub fn pack(request: &PackRequest, registry: &Registry, ctx: &OpContext) -> Result<(), VfsError> {
    let _ = (request, registry, ctx);
    Err(VfsError::unsupported("archive", "pack yet"))
}

/// The provider for places inside archives.
#[derive(Debug, Default)]
pub struct ArchiveProvider {
    limits: Limits,
    registry: RwLock<Weak<Registry>>,
}

impl ArchiveProvider {
    /// A provider with these limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            registry: RwLock::new(Weak::new()),
        }
    }

    /// The limits.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Connects the provider to the registry it reaches archive files
    /// through, after the registry is built.
    pub fn connect(&self, registry: &Arc<Registry>) {
        if let Ok(mut held) = self.registry.write() {
            *held = Arc::downgrade(registry);
        }
    }

    fn not_built(&self, operation: &str) -> VfsError {
        VfsError::unsupported(self.scheme(), &format!("{operation} yet"))
    }
}

impl Provider for ArchiveProvider {
    fn scheme(&self) -> &str {
        "archive"
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        Capabilities::read_only()
    }

    fn list(&self, place: &Place, _ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        if nesting_depth(place) > self.limits.max_nesting {
            return Err(self.not_built("open archives nested this deep"));
        }
        Err(self.not_built("list archives"))
    }

    fn stat(&self, _place: &Place, _ctx: &OpContext) -> Result<Entry, VfsError> {
        Err(self.not_built("read archive members"))
    }

    fn read(&self, _place: &Place, _ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Err(self.not_built("read archive members"))
    }

    fn write(
        &self,
        _place: &Place,
        _mode: WriteMode,
        _ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(self.not_built("write into archives"))
    }

    fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("make folders in archives"))
    }

    fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("rename in archives"))
    }

    fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("delete from archives"))
    }

    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(self.not_built("change attributes in archives"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;

    #[test]
    fn formats_from_names() {
        assert_eq!(
            ArchiveFormat::from_name("a.TAR.GZ"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_name("a.txz"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(ArchiveFormat::from_name("a.xz"), Some(ArchiveFormat::Xz));
        assert_eq!(ArchiveFormat::from_name(".zip"), None);
        assert_eq!(ArchiveFormat::from_name("notes.txt"), None);
        for format in ArchiveFormat::ALL {
            let name = format!("x{}", format.extension());
            assert_eq!(ArchiveFormat::from_name(&name), Some(format));
        }
    }

    #[test]
    fn hostile_member_names_are_refused() {
        assert_eq!(normalize_member_name("../../x"), None);
        assert_eq!(normalize_member_name("a/../../x"), None);
        assert_eq!(
            normalize_member_name("/etc/passwd").as_deref(),
            Some("etc/passwd")
        );
        assert_eq!(
            normalize_member_name("C:\\win\\x.dll").as_deref(),
            Some("win/x.dll")
        );
        assert_eq!(normalize_member_name("a/./b/../c").as_deref(), Some("a/c"));
        assert_eq!(normalize_member_name("./"), None);
    }

    #[test]
    fn junk_is_recognized() {
        assert!(is_junk("__MACOSX/._a.txt"));
        assert!(is_junk("photos/.DS_Store"));
        assert!(is_junk("photos\\Thumbs.db"));
        assert!(!is_junk("notes/thumbs.txt"));
    }

    #[test]
    fn nesting_is_counted() {
        let place = Place::parse_with("/a.zip!b.7z!c", PathStyle::Unix).unwrap();
        assert_eq!(nesting_depth(&place), 2);
        let provider = ArchiveProvider::new(Limits::default());
        let err = provider.list(&place, &OpContext::new()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the archive provider cannot list archives yet"
        );
    }
}
