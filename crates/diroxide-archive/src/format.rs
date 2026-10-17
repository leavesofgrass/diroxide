//! The archive formats, by name and by their first bytes.

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
    /// gzip stream reads as tar.gz and an xz stream as xz. See
    /// [`crate::sniff`] for every signature and the decision with the name.
    pub fn sniff(head: &[u8]) -> Option<Self> {
        crate::sniff::sniff_signature(head).and_then(crate::sniff::Signature::format)
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
