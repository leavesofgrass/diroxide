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
//! The read side (W2a): the name rules ([`names`]), the limits
//! ([`limits`]), the format sniffer, the limiter chain, the index, and a
//! reader per format, all pure Rust (`zip`, `sevenz-rust2`, `tar`,
//! `flate2`, `lzma-rust2`). The pack and unpack runners are the write
//! side's (W2b).

pub mod format;
pub mod fuzzing;
pub mod guard;
pub mod limits;
pub mod names;
pub mod provider;
pub mod request;
pub mod sniff;

pub use format::ArchiveFormat;
pub use guard::{Allowances, Budget, Claims, Guarded, LimitHit, Question, QuestionKind};
pub use limits::{Limits, limit_words};
pub use names::{
    MemberPath, NameFlags, NameRefusal, is_junk, member_path, nesting_depth, normalize_member_name,
};
pub use provider::ArchiveProvider;
pub use request::{CompressionLevel, FolderRule, PackRequest, StorePaths, UnpackRequest, pack};
pub use sniff::{Signature, Sniffed, decide, sniff_signature};

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;
    use diroxide_core::{Place, Secret};
    use diroxide_vfs::{OpContext, Provider};

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
    fn a_numbered_part_is_the_format_of_its_set_and_look_alikes_are_none() {
        let parts: [(&str, Option<ArchiveFormat>); 14] = [
            ("backup.7z.001", Some(ArchiveFormat::SevenZ)),
            ("backup.7z.000", Some(ArchiveFormat::SevenZ)),
            ("BACKUP.ZIP.1000", Some(ArchiveFormat::Zip)),
            ("logs.tar.gz.002", Some(ArchiveFormat::TarGz)),
            ("logs.tgz.0001", Some(ArchiveFormat::TarGz)),
            ("logs.tar.xz.999", Some(ArchiveFormat::TarXz)),
            ("disk.tar.12345", Some(ArchiveFormat::Tar)),
            ("one.xz.003", Some(ArchiveFormat::Xz)),
            ("photo.jpg.001", None),
            ("report.pdf.001", None),
            ("backup.7z.01", None),
            ("backup.7z.001.txt", None),
            ("x.001", None),
            ("backup.zipx.001", None),
        ];
        for (name, format) in parts {
            assert_eq!(ArchiveFormat::from_name(name), format, "{name}");
            assert_eq!(
                diroxide_core::place::opens_as_archive(name),
                format.is_some(),
                "{name}: the place parser agrees"
            );
        }
        for format in ArchiveFormat::ALL {
            let name = format!("set{}.042", format.extension());
            assert_eq!(ArchiveFormat::from_name(&name), Some(*format), "{name}");
        }
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
    fn the_phase_0_normalizer_bug_is_fixed() {
        // A doubled long-path prefix, or one behind a slash, left a `?`
        // and a drive part in the Phase 0 normalizer: `?/C:/x`. A drive in
        // the middle of a path, or a part made only of dots and spaces
        // (which Windows reads as `..`), passed through.
        let table: [(&str, Option<&str>); 9] = [
            (r"\\?\\\?\C:\x", Some("x")),
            (r"/\\?\C:\x", Some("x")),
            (r"\??\C:\x", Some("x")),
            (r"\\.\\\?\UNC\srv\x", Some("srv/x")),
            ("a/C:/x", Some("a/x")),
            ("a/C:x", Some("a/x")),
            ("a/.. /x", None),
            ("a/.../x", None),
            ("C:D:x", Some("x")),
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
