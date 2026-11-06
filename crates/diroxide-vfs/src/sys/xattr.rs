//! Unix: extended attributes (alpha.7, R12's contract request 2; R30
//! commit 48), through `rustix`'s safe wrappers of `listxattr`,
//! `getxattr`, `setxattr` and `removexattr` on Linux, Android and macOS.
//! Other Unix systems (the BSDs) say "not available here" until the
//! `xattr` crate's `extattr` side is wanted. Links are followed, as a
//! read of the contents follows them.
//!
//! Only `user.*` on Linux is the owner's; `security.*`, `trusted.*` and
//! `system.*` are listed and marked as managed by the system. On macOS
//! the `com.apple.` names are the system's, except the quarantine mark,
//! Finder tags and comments and where a file came from.
//!
//! Spike M1 (the development container, ext4): `std::fs::copy` keeps
//! none of a file's `user.*` attributes, and neither does a byte copy or
//! a plain `cp`; `cp --preserve=xattr` keeps both.

use std::path::Path;

use crate::error::VfsError;
use crate::extended::{ExtendedItem, ExtendedKind};

/// The file systems that hold no extended attributes, by the name the
/// mount table gives.
const WITHOUT: [&str; 9] = [
    "vfat", "msdos", "fat", "fat16", "fat32", "exfat", "iso9660", "udf", "msdosfs",
];

/// Whether this system and a file system hold extended attributes.
pub(crate) fn extended_on(file_system: Option<&str>) -> bool {
    imp::SUPPORTED
        && !file_system.is_some_and(|fs| WITHOUT.iter().any(|w| fs.eq_ignore_ascii_case(w)))
}

/// Whether a name is the system's rather than the owner's.
pub(crate) fn managed(name: &str) -> bool {
    if cfg!(target_vendor = "apple") {
        name.starts_with("com.apple.")
            && !matches!(
                name,
                "com.apple.quarantine"
                    | "com.apple.metadata:_kMDItemUserTags"
                    | "com.apple.metadata:kMDItemFinderComment"
                    | "com.apple.metadata:kMDItemWhereFroms"
            )
    } else {
        !name.starts_with("user.")
    }
}

/// The attributes of the entry at `path`, in the system's order, with
/// their sizes.
pub(crate) fn extended_list(path: &Path) -> Result<Vec<ExtendedItem>, VfsError> {
    let names = imp::names(path)?;
    Ok(names
        .into_iter()
        .map(|name| {
            let size = imp::size(path, &name).unwrap_or(0);
            let item = ExtendedItem::new(name.clone(), size, ExtendedKind::Attribute);
            if managed(&name) { item.managed() } else { item }
        })
        .collect())
}

/// The size of one attribute.
#[cfg(test)]
pub(crate) fn extended_size(path: &Path, name: &str) -> Result<u64, VfsError> {
    check_name(name)?;
    imp::size(path, name)
}

/// One attribute's bytes, refused with [`VfsError::FileTooLarge`] when
/// it holds more than `max_bytes`.
pub(crate) fn extended_read(
    path: &Path,
    name: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, VfsError> {
    check_name(name)?;
    imp::read(path, name, max_bytes)
}

/// Sets one attribute, making it when it is missing.
pub(crate) fn extended_write(path: &Path, name: &str, value: &[u8]) -> Result<(), VfsError> {
    check_name(name)?;
    imp::write(path, name, value)
}

/// Removes one attribute.
pub(crate) fn extended_remove(path: &Path, name: &str) -> Result<(), VfsError> {
    check_name(name)?;
    imp::remove(path, name)
}

fn check_name(name: &str) -> Result<(), VfsError> {
    if name.is_empty() || name.len() > 255 || name.contains('\0') {
        Err(VfsError::NameInvalid)
    } else {
        Ok(())
    }
}

fn not_here() -> VfsError {
    VfsError::unsupported("file", "keep extended attributes on this drive")
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
mod imp {
    use std::path::Path;

    use rustix::fs::{
        XattrFlags, getxattr, getxattr_query, listxattr, listxattr_query, removexattr, setxattr,
    };
    use rustix::io::Errno;

    use super::not_here;
    use crate::error::VfsError;

    pub(super) const SUPPORTED: bool = true;

    /// The errno of a missing attribute: `ENODATA` on Linux, `ENOATTR`
    /// on macOS.
    #[cfg(not(target_vendor = "apple"))]
    const MISSING: Errno = Errno::NODATA;
    #[cfg(target_vendor = "apple")]
    const MISSING: Errno = Errno::NOATTR;

    fn error(errno: Errno) -> VfsError {
        if errno == MISSING {
            VfsError::NotFound
        } else if errno == Errno::NOTSUP || errno == Errno::OPNOTSUPP {
            not_here()
        } else {
            std::io::Error::from(errno).into()
        }
    }

    /// The names, read again when the list grew between the size query
    /// and the read.
    pub(super) fn names(path: &Path) -> Result<Vec<String>, VfsError> {
        for _ in 0..4 {
            let size = listxattr_query(path).map_err(error)?;
            if size == 0 {
                return Ok(Vec::new());
            }
            let mut buffer = vec![0u8; size];
            match listxattr(path, &mut buffer[..]) {
                Ok(n) => {
                    return Ok(buffer[..n]
                        .split(|&b| b == 0)
                        .filter(|name| !name.is_empty())
                        .map(|name| String::from_utf8_lossy(name).into_owned())
                        .collect());
                }
                Err(Errno::RANGE) => continue,
                Err(errno) => return Err(error(errno)),
            }
        }
        Err(VfsError::Changed)
    }

    pub(super) fn size(path: &Path, name: &str) -> Result<u64, VfsError> {
        getxattr_query(path, name).map(|n| n as u64).map_err(error)
    }

    pub(super) fn read(path: &Path, name: &str, max_bytes: usize) -> Result<Vec<u8>, VfsError> {
        for _ in 0..4 {
            let size = getxattr_query(path, name).map_err(error)?;
            if size > max_bytes {
                return Err(VfsError::FileTooLarge);
            }
            let mut buffer = vec![0u8; size];
            match getxattr(path, name, &mut buffer[..]) {
                Ok(n) => {
                    buffer.truncate(n);
                    return Ok(buffer);
                }
                Err(Errno::RANGE) => continue,
                Err(errno) => return Err(error(errno)),
            }
        }
        Err(VfsError::Changed)
    }

    pub(super) fn write(path: &Path, name: &str, value: &[u8]) -> Result<(), VfsError> {
        setxattr(path, name, value, XattrFlags::empty()).map_err(error)
    }

    pub(super) fn remove(path: &Path, name: &str) -> Result<(), VfsError> {
        removexattr(path, name).map_err(error)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
mod imp {
    use std::path::Path;

    use super::not_here;
    use crate::error::VfsError;

    pub(super) const SUPPORTED: bool = false;

    pub(super) fn names(_path: &Path) -> Result<Vec<String>, VfsError> {
        Err(not_here())
    }

    pub(super) fn size(_path: &Path, _name: &str) -> Result<u64, VfsError> {
        Err(not_here())
    }

    pub(super) fn read(_path: &Path, _name: &str, _max_bytes: usize) -> Result<Vec<u8>, VfsError> {
        Err(not_here())
    }

    pub(super) fn write(_path: &Path, _name: &str, _value: &[u8]) -> Result<(), VfsError> {
        Err(not_here())
    }

    pub(super) fn remove(_path: &Path, _name: &str) -> Result<(), VfsError> {
        Err(not_here())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::scratch;

    #[test]
    fn the_owners_names_and_the_systems() {
        if cfg!(target_vendor = "apple") {
            assert!(managed("com.apple.provenance"));
            assert!(!managed("com.apple.quarantine"));
            assert!(!managed("user.note"));
        } else {
            assert!(!managed("user.xdg.origin.url"));
            assert!(managed("security.selinux"));
            assert!(managed("trusted.overlay.opaque"));
        }
        assert!(!extended_on(Some("vfat")));
        assert!(!extended_on(Some("exFAT")));
        assert_eq!(extended_on(Some("ext4")), imp::SUPPORTED);
        assert_eq!(
            extended_write(Path::new("/x"), "", b""),
            Err(VfsError::NameInvalid)
        );
    }

    /// Spike M1's attribute half as a test, on a file of the test's own:
    /// set, list, read, remove, and the limit.
    #[test]
    fn an_attribute_is_set_listed_read_and_removed() {
        let dir = scratch();
        let file = dir.path().join("marked.txt");
        std::fs::write(&file, b"our own file\n").unwrap();
        let url = b"https://example.org/file.zip";
        match extended_write(&file, "user.xdg.origin.url", url) {
            Ok(()) => {}
            // A file system without user attributes (some container
            // volumes) has nothing more to test.
            Err(_) if !imp::SUPPORTED => return,
            Err(VfsError::Unsupported { .. }) => return,
            Err(other) => panic!("{other:?}"),
        }
        let listed = extended_list(&file).unwrap();
        assert!(
            listed.contains(&ExtendedItem::new(
                "user.xdg.origin.url",
                url.len() as u64,
                ExtendedKind::Attribute
            )),
            "{listed:?}"
        );
        assert_eq!(
            extended_size(&file, "user.xdg.origin.url").unwrap(),
            url.len() as u64
        );
        assert_eq!(
            extended_read(&file, "user.xdg.origin.url", 1024).unwrap(),
            url
        );
        assert_eq!(
            extended_read(&file, "user.xdg.origin.url", 4),
            Err(VfsError::FileTooLarge)
        );
        assert_eq!(
            extended_read(&file, "user.nothing", 4),
            Err(VfsError::NotFound)
        );
        extended_remove(&file, "user.xdg.origin.url").unwrap();
        assert!(
            !extended_list(&file)
                .unwrap()
                .iter()
                .any(|item| item.name == "user.xdg.origin.url")
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"our own file\n");
    }
}
