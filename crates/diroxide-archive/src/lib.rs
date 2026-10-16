//! The archive provider: zip, 7z, tar, tar.gz, tar.xz and plain xz, read
//! in place (nothing is extracted to browse) and written as jobs.
//!
//! Places inside an archive are written `course.zip!week1/notes.txt`; a
//! split archive opens as one place (`backup.7z.001!week1`). The provider
//! reaches the archive file itself through the registry, held weakly so
//! the registry can hold the provider. Limits guard against hostile
//! archives: sizes, ratios, dictionaries, entry counts, a nesting depth of
//! four, and member names that would leave the archive.
//!
//! Phase 0 and the Wave 1 contract: the formats, the limits of
//! research/archives.md section 6, the name rules, the pack and unpack
//! requests and the provider's shape; every provider method says "not
//! built yet". Wave 2 (W2a) owns this crate and brings in `zip`,
//! `sevenz-rust2`, `tar`, `flate2` and `lzma-rust2` (pure Rust; `liblzma`
//! is not used).

use std::io::Read;
use std::sync::{Arc, RwLock, Weak};

use diroxide_core::{ConflictPolicy, Entry, Place, Secret};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, VfsError, WriteMode,
};

/// The archive formats. More join as they ship (gzip, bzip2, Zstandard
/// and their tars), so callers match with a fallback arm.
#[non_exhaustive]
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

/// The first bytes of each format the provider knows by its bytes.
const ZIP_MAGIC: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
const ZIP_EMPTY_MAGIC: &[u8] = &[0x50, 0x4B, 0x05, 0x06];
const SEVEN_Z_MAGIC: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
const XZ_MAGIC: &[u8] = &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
const GZIP_MAGIC: &[u8] = &[0x1F, 0x8B];

impl ArchiveFormat {
    /// Every format, in the order the tests and the docs list them.
    pub const ALL: &'static [ArchiveFormat] = &[
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
        ArchiveFormat::Xz,
    ];

    /// The format of a file name, by its extension, ignoring case. A
    /// numbered part of a split archive (`backup.7z.001`) is the format of
    /// the whole set.
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        let lower = match diroxide_core::names::split_part(&lower) {
            Some(part) => part.base.to_owned(),
            None => lower,
        };
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

    /// The format by the first bytes of the file: the outer container. A
    /// compressed tar is known by its name as well; by its bytes alone a
    /// gzip stream reads as tar.gz and an xz stream as xz.
    pub fn sniff(head: &[u8]) -> Option<Self> {
        if head.starts_with(ZIP_MAGIC) || head.starts_with(ZIP_EMPTY_MAGIC) {
            Some(ArchiveFormat::Zip)
        } else if head.starts_with(SEVEN_Z_MAGIC) {
            Some(ArchiveFormat::SevenZ)
        } else if head.starts_with(XZ_MAGIC) {
            Some(ArchiveFormat::Xz)
        } else if head.starts_with(GZIP_MAGIC) {
            Some(ArchiveFormat::TarGz)
        } else if head.get(257..262) == Some(b"ustar".as_slice()) {
            Some(ArchiveFormat::Tar)
        } else {
            None
        }
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

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Limits that keep a hostile archive from filling the disk or memory
/// (research/archives.md section 6). Each becomes a key of the
/// `[archives]` table of `config.toml` (W2a). "Ask" limits are questions
/// ending "y or n", asked once per job; the others refuse with a reason in
/// words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Ask before reading a member larger than this, or one behind this
    /// much compressed data in a sequential archive (soft).
    pub max_member_size: u64,
    /// Ask before an operation decompresses more than this (soft).
    pub ask_total_bytes: u64,
    /// Refuse an operation that would decompress more than this (hard).
    pub max_total_bytes: u64,
    /// Ask when one member's running ratio passes this many to one, once
    /// it has written `ratio_floor_bytes`.
    pub max_ratio: u64,
    /// The output a member writes before its ratio is judged.
    pub ratio_floor_bytes: u64,
    /// Ask before a dictionary or window larger than this (7z, xz,
    /// Zstandard).
    pub ask_dictionary_bytes: u64,
    /// Refuse a dictionary or window larger than this; replaces Phase 0's
    /// 7z-only limit.
    pub max_dictionary: u64,
    /// Refuse a container header or central directory larger than this.
    pub max_header_bytes: u64,
    /// Refuse an index that takes more memory than this.
    pub max_index_bytes: u64,
    /// Refuse an archive with more entries than this.
    pub max_entries: usize,
    /// Skip a member whose name is longer than this, in bytes.
    pub max_name_bytes: usize,
    /// Skip a member with a path component longer than this, in bytes.
    pub max_component_bytes: usize,
    /// Skip a link whose target is longer than this, in bytes.
    pub max_link_target_bytes: usize,
    /// Refuse an operation that needs more memory than this.
    pub max_memory_bytes: u64,
    /// Spool a nested archive in memory up to this size.
    pub max_spool_memory_bytes: u64,
    /// Spool on disk up to this much per job, shared across nesting levels.
    pub max_spool_disk_bytes: u64,
    /// How many archives deep a place may go.
    pub max_nesting: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_member_size: 8 * GIB,
            ask_total_bytes: 32 * GIB,
            max_total_bytes: 1 << 40,
            max_ratio: 1_000,
            ratio_floor_bytes: 64 * MIB,
            ask_dictionary_bytes: 256 * MIB,
            max_dictionary: 1_536 * MIB,
            max_header_bytes: 256 * MIB,
            max_index_bytes: 256 * MIB,
            max_entries: 1_000_000,
            max_name_bytes: 4_096,
            max_component_bytes: 255,
            max_link_target_bytes: 4_096,
            max_memory_bytes: GIB,
            max_spool_memory_bytes: 64 * MIB,
            max_spool_disk_bytes: 4 * GIB,
            max_nesting: 4,
        }
    }
}

impl Limits {
    /// Small limits for the fuzz runs, so a bomb is found in seconds.
    pub fn strict() -> Self {
        Self {
            max_member_size: 16 * MIB,
            ask_total_bytes: 32 * MIB,
            max_total_bytes: 64 * MIB,
            max_ratio: 100,
            ratio_floor_bytes: MIB,
            ask_dictionary_bytes: 16 * MIB,
            max_dictionary: 64 * MIB,
            max_header_bytes: 4 * MIB,
            max_index_bytes: 16 * MIB,
            max_entries: 10_000,
            max_name_bytes: 1_024,
            max_component_bytes: 255,
            max_link_target_bytes: 1_024,
            max_memory_bytes: 128 * MIB,
            max_spool_memory_bytes: 8 * MIB,
            max_spool_disk_bytes: 64 * MIB,
            max_nesting: 2,
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

/// A member name made safe, by the first steps of research/archives.md
/// group D: refused (`None`) when it holds NUL or another control
/// character, climbs out of the archive with `..`, or normalizes to
/// nothing; otherwise with forward slashes, no long-path or device prefix
/// (`\\?\`, `\\.\`), no leading slash, no drive, and no `.` or empty parts.
/// The destination's own name check comes after, at unpack.
pub fn normalize_member_name(name: &str) -> Option<String> {
    if name.chars().any(char::is_control) {
        return None;
    }
    let unified = name.replace('\\', "/");
    let mut rest = unified.as_str();
    for prefix in ["//?/UNC/", "//?/unc/", "//?/", "//./"] {
        if let Some(after) = rest.strip_prefix(prefix) {
            rest = after;
            break;
        }
    }
    let rest = rest.trim_start_matches('/');
    let rest = match rest.as_bytes() {
        [letter, b':', ..] if letter.is_ascii_alphabetic() => &rest[2..],
        _ => rest,
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
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

/// How hard to compress, in words; each format turns the word into its
/// own number (research/archives.md 8.2): deflate 1, 3, 6, 9 and 9; xz
/// and 7z presets 1, 3, 6, 8 and 9.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CompressionLevel {
    /// The fastest.
    Fastest,
    /// Fast.
    Fast,
    /// Normal (the default).
    #[default]
    Normal,
    /// Maximum.
    Maximum,
    /// Ultra: the most the format offers.
    Ultra,
}

impl CompressionLevel {
    /// The level in words, as the pack dialog says it: "normal".
    pub const fn word(self) -> &'static str {
        match self {
            CompressionLevel::Fastest => "fastest",
            CompressionLevel::Fast => "fast",
            CompressionLevel::Normal => "normal",
            CompressionLevel::Maximum => "maximum",
            CompressionLevel::Ultra => "ultra",
        }
    }

    /// The format's own number for this level, out of nine: "normal (6 of
    /// 9)". `None` for a format with no levels (tar).
    pub const fn number_for(self, format: ArchiveFormat) -> Option<u32> {
        let index = match self {
            CompressionLevel::Fastest => 0,
            CompressionLevel::Fast => 1,
            CompressionLevel::Normal => 2,
            CompressionLevel::Maximum => 3,
            CompressionLevel::Ultra => 4,
        };
        match format {
            ArchiveFormat::Tar => None,
            ArchiveFormat::Zip | ArchiveFormat::TarGz => Some([1, 3, 6, 9, 9][index]),
            _ => Some([1, 3, 6, 8, 9][index]),
        }
    }
}

/// Which part of each source's path is stored in a new archive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StorePaths {
    /// Relative to the source folder (the default).
    #[default]
    Relative,
    /// The full path, without its drive or root.
    Full,
    /// Names only, without folders.
    NamesOnly,
}

/// Whether unpacking makes a folder named after the archive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FolderRule {
    /// Make one when the archive has several top-level items (the
    /// default): no scattered files, no `name/name/`.
    #[default]
    WhenSeveral,
    /// Always make one.
    Always,
    /// Never make one.
    Never,
}

/// What to pack and where. Plain data, built by the app from the pack
/// dialog. A password travels here, so the request is never serialized or
/// logged, and its `Debug` hides the secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackRequest {
    /// The entries to pack.
    pub sources: Vec<Place>,
    /// The new archive.
    pub destination: Place,
    /// Its format.
    pub format: ArchiveFormat,
    /// How hard to compress.
    pub level: CompressionLevel,
    /// A solid 7z: one stream for many files.
    pub solid: bool,
    /// The largest solid block, in bytes (1 GiB by default).
    pub solid_block_bytes: Option<u64>,
    /// The password, for zip AES or 7z; `None` for none.
    pub password: Option<Secret>,
    /// Whether a 7z hides its member names behind the password too.
    pub encrypt_names: bool,
    /// Glob patterns of entries to leave out.
    pub exclude: Vec<String>,
    /// Which part of each path is stored.
    pub store_paths: StorePaths,
    /// Whether tar keeps owner names (off: they are personal identifiers).
    pub keep_owner: bool,
    /// How many threads, or `None` for automatic.
    pub threads: Option<usize>,
    /// Whether to read the new archive back and check it (on by default).
    pub test_after: bool,
    /// What to do when the archive's name is taken.
    pub conflicts: ConflictPolicy,
}

impl PackRequest {
    /// A request with the defaults: normal level, solid for 7z with a 1 GiB
    /// block, no password, relative paths, owner names left out, tested
    /// after packing, conflicts asked.
    pub fn new(sources: Vec<Place>, destination: Place, format: ArchiveFormat) -> Self {
        Self {
            sources,
            destination,
            format,
            level: CompressionLevel::Normal,
            solid: format == ArchiveFormat::SevenZ,
            solid_block_bytes: Some(GIB),
            password: None,
            encrypt_names: false,
            exclude: Vec::new(),
            store_paths: StorePaths::Relative,
            keep_owner: false,
            threads: None,
            test_after: true,
            conflicts: ConflictPolicy::Ask,
        }
    }
}

/// What to unpack and where. As with [`PackRequest`], never serialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpackRequest {
    /// The archive.
    pub source: Place,
    /// The members to unpack, as member paths; empty for all.
    pub members: Vec<String>,
    /// Where to unpack: the other panel's folder by default.
    pub destination: Place,
    /// Whether to make a folder named after the archive.
    pub folder_rule: FolderRule,
    /// Names only, without folders (asks before it overwrites).
    pub flatten: bool,
    /// What to do when a name is taken; a replaced file goes to the trash
    /// first.
    pub conflicts: ConflictPolicy,
    /// Whether to make safe relative links (off: links are skipped and
    /// reported).
    pub restore_links: bool,
    /// The password, when one is known; else it is asked when the first
    /// encrypted member is reached.
    pub password: Option<Secret>,
    /// The encoding of names in an older zip without the UTF-8 flag, such
    /// as "Shift-JIS"; `None` for the format's default (CP437).
    pub name_encoding: Option<String>,
    /// Only read and check every member; write nothing ("Test archive").
    pub test_only: bool,
}

impl UnpackRequest {
    /// A request to unpack everything with the defaults.
    pub fn new(source: Place, destination: Place) -> Self {
        Self {
            source,
            members: Vec::new(),
            destination,
            folder_rule: FolderRule::WhenSeveral,
            flatten: false,
            conflicts: ConflictPolicy::Ask,
            restore_links: false,
            password: None,
            name_encoding: None,
            test_only: false,
        }
    }
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
            assert_eq!(ArchiveFormat::from_name(&name), Some(*format));
        }
    }

    #[test]
    fn formats_by_their_bytes_and_their_parts() {
        assert_eq!(
            ArchiveFormat::sniff(&[0x50, 0x4B, 0x03, 0x04, 0x14]),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::sniff(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0]),
            Some(ArchiveFormat::SevenZ)
        );
        assert_eq!(
            ArchiveFormat::sniff(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, 0]),
            Some(ArchiveFormat::Xz)
        );
        let mut tar = vec![0u8; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(ArchiveFormat::sniff(&tar), Some(ArchiveFormat::Tar));
        assert_eq!(ArchiveFormat::sniff(b"%PDF-1.7"), None);
        assert_eq!(
            ArchiveFormat::from_name("backup.7z.001"),
            Some(ArchiveFormat::SevenZ)
        );
        assert_eq!(ArchiveFormat::from_name("photo.jpg.001"), None);
    }

    #[test]
    fn levels_are_words_with_numbers() {
        assert_eq!(
            CompressionLevel::Normal.number_for(ArchiveFormat::Zip),
            Some(6)
        );
        assert_eq!(
            CompressionLevel::Maximum.number_for(ArchiveFormat::SevenZ),
            Some(8)
        );
        assert_eq!(
            CompressionLevel::Normal.number_for(ArchiveFormat::Tar),
            None
        );
        let place = Place::parse_with("/a.7z", PathStyle::Unix).unwrap();
        let request = PackRequest::new(Vec::new(), place, ArchiveFormat::SevenZ);
        assert!(request.solid && request.test_after && !request.keep_owner);
    }

    #[test]
    fn a_password_never_shows_in_a_request() {
        let place = Place::parse_with("/a.7z", PathStyle::Unix).unwrap();
        let mut request = PackRequest::new(Vec::new(), place.clone(), ArchiveFormat::SevenZ);
        request.password = Some(Secret::new("canary-7f3a-not-a-password".to_owned()));
        assert!(!format!("{request:?}").contains("canary"));
        let mut unpack = UnpackRequest::new(place.clone(), place);
        unpack.password = request.password.clone();
        assert!(!format!("{unpack:?}").contains("canary"));
    }

    #[test]
    fn strict_limits_are_smaller() {
        let (strict, normal) = (Limits::strict(), Limits::default());
        assert!(strict.max_total_bytes < normal.max_total_bytes);
        assert!(strict.max_nesting < normal.max_nesting);
        assert_eq!(normal.max_dictionary, 1_536 << 20);
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
        let table: [(&str, Option<&str>); 9] = [
            (r"\\?\C:\x", Some("x")),
            (r"\\.\C:\dev", Some("dev")),
            (r"\\?\UNC\srv\share\x", Some("srv/share/x")),
            (r"\\srv\share\x", Some("srv/share/x")),
            ("a\u{0}b", None),
            ("tab\there", None),
            ("line\nbreak", None),
            ("ok/name!.txt", Some("ok/name!.txt")),
            ("C:", None),
        ];
        for (name, expected) in table {
            assert_eq!(normalize_member_name(name).as_deref(), expected, "{name:?}");
        }
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
