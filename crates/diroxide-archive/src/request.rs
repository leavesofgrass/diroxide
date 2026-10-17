//! What to pack and what to unpack: plain data the app builds from its
//! dialogs, for the runners of the write side (W2b).

use diroxide_core::{ConflictPolicy, Place, Secret};
use diroxide_vfs::{OpContext, Registry, VfsError};

use crate::format::ArchiveFormat;
use crate::limits::GIB;

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
