//! Unix: who may do what with one entry (R29 4.2, R11 6.7), for the
//! focused row only. The owner and the group by name (from
//! [`Owners`]), the twelve mode bits, "you can" for the running account
//! by `accessat` with the effective-ids flag, the flags beside the mode
//! (Linux's inode flags through an open handle, macOS's `st_flags`), the
//! Linux access lists read by hand from their extended attributes, and
//! the SELinux label with any other `security.*` attribute as read-only
//! text.
//!
//! Everything goes through the standard library, `rustix`'s safe
//! wrappers and ML's extended attributes (`xattr.rs`); there is no
//! `unsafe` here. A link has no permissions of its own: its facts are its
//! owner and group.

use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use diroxide_core::EntryKind;

use crate::access::{AccessFacts, AccessSource, FileFlags, Rights};
use crate::error::VfsError;
use crate::local::owners::Owners;

/// The most of a `security.*` attribute said as text.
const SECURITY_TEXT_MAX: usize = 256;

/// The kind of entry metadata read without following describes.
fn kind_of(metadata: &Metadata) -> EntryKind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        EntryKind::Link
    } else if file_type.is_dir() {
        EntryKind::Folder
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Special
    }
}

/// Who may do what with the entry at `path`, never through a final link.
pub(crate) fn access_facts(path: &Path) -> Result<AccessFacts, VfsError> {
    let metadata = std::fs::symlink_metadata(path)?;
    let kind = kind_of(&metadata);
    let owners = Owners::shared();
    let mut facts = AccessFacts {
        source: AccessSource::Local,
        kind: Some(kind),
        owner: Some(owners.user(metadata.uid())),
        owner_id: Some(metadata.uid()),
        group: Some(owners.group(metadata.gid())),
        group_id: Some(metadata.gid()),
        ..AccessFacts::default()
    };
    if kind == EntryKind::Link {
        return Ok(facts);
    }
    facts.mode = Some(metadata.mode() & 0o7777);
    facts.you_can = Some(you_can(path));
    facts.flags = flags(path, &metadata);
    lists::read(path, kind, &owners, &mut facts);
    labels(path, &mut facts);
    Ok(facts)
}

/// What the running account can do, as the kernel answers `access` with
/// the effective ids: advice (another program's lock or a disk turning
/// read only still stops a write). A kernel without the effective-ids
/// flag is asked with the real ids, which are the same unless the
/// program runs set-id.
fn you_can(path: &Path) -> Rights {
    use rustix::fs::{Access, AtFlags, CWD, accessat};
    use rustix::io::Errno;
    let may = |access: Access| match accessat(CWD, path, access, AtFlags::EACCESS) {
        Ok(()) => true,
        Err(errno) if errno == Errno::INVAL || errno == Errno::NOSYS => {
            accessat(CWD, path, access, AtFlags::empty()).is_ok()
        }
        Err(_) => false,
    };
    Rights {
        read: may(Access::READ_OK),
        write: may(Access::WRITE_OK),
        run: may(Access::EXEC_OK),
    }
}

/// Linux's inode flags (`lsattr`): through an open handle with
/// `FS_IOC_GETFLAGS`, for files and folders only (opening a device or a
/// pipe could do something); without read access, from `statx`'s
/// attributes, which need no handle.
#[cfg(target_os = "linux")]
fn flags(path: &Path, metadata: &Metadata) -> Option<FileFlags> {
    use rustix::fs::{CWD, IFlags, Mode, OFlags, ioctl_getflags, openat};
    /// `FS_ENCRYPT_FL`, which rustix's flags do not name.
    const ENCRYPT: u32 = 0x800;
    if !(metadata.is_file() || metadata.is_dir()) {
        return None;
    }
    let open = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match openat(CWD, path, open | OFlags::NOCTTY, Mode::empty()) {
        // The handle closes when it drops, at the end of this arm.
        Ok(handle) => ioctl_getflags(&handle).ok().map(|bits| {
            let locked = bits.contains(IFlags::IMMUTABLE);
            let append_only = bits.contains(IFlags::APPEND);
            FileFlags {
                locked,
                append_only,
                administrator_flag: locked || append_only,
                no_backup: bits.contains(IFlags::NODUMP),
                compressed: bits.contains(IFlags::COMPRESSED),
                encrypted: bits.bits() & ENCRYPT != 0,
                no_copy_on_write: bits.contains(IFlags::NOCOW),
                hidden: false,
            }
        }),
        Err(_) => statx_flags(path),
    }
}

/// The flags `statx` reports, where the file system says it knows them.
#[cfg(target_os = "linux")]
fn statx_flags(path: &Path) -> Option<FileFlags> {
    use rustix::fs::{AtFlags, CWD, StatxAttributes, StatxFlags, statx};
    let found = statx(
        CWD,
        path,
        AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::BASIC_STATS,
    )
    .ok()?;
    let known = found.stx_attributes_mask;
    if !known.contains(StatxAttributes::IMMUTABLE) {
        return None;
    }
    let set = |flag: StatxAttributes| known.contains(flag) && found.stx_attributes.contains(flag);
    let locked = set(StatxAttributes::IMMUTABLE);
    let append_only = set(StatxAttributes::APPEND);
    Some(FileFlags {
        locked,
        append_only,
        administrator_flag: locked || append_only,
        no_backup: set(StatxAttributes::NODUMP),
        compressed: set(StatxAttributes::COMPRESSED),
        encrypted: set(StatxAttributes::ENCRYPTED),
        ..FileFlags::default()
    })
}

/// macOS's file flags (`ls -lO`), from the metadata already read: the
/// Finder's Locked is the owner's immutable flag.
#[cfg(target_os = "macos")]
fn flags(_path: &Path, metadata: &Metadata) -> Option<FileFlags> {
    use std::os::macos::fs::MetadataExt as _;
    Some(mac_flags(metadata.st_flags()))
}

/// The flags of a `st_flags` value (macOS's `sys/stat.h`).
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn mac_flags(bits: u32) -> FileFlags {
    const UF_NODUMP: u32 = 0x1;
    const UF_IMMUTABLE: u32 = 0x2;
    const UF_APPEND: u32 = 0x4;
    const UF_COMPRESSED: u32 = 0x20;
    const UF_HIDDEN: u32 = 0x8000;
    const SF_IMMUTABLE: u32 = 0x2_0000;
    const SF_APPEND: u32 = 0x4_0000;
    let system = bits & (SF_IMMUTABLE | SF_APPEND) != 0;
    FileFlags {
        locked: bits & (UF_IMMUTABLE | SF_IMMUTABLE) != 0,
        append_only: bits & (UF_APPEND | SF_APPEND) != 0,
        administrator_flag: system,
        no_backup: bits & UF_NODUMP != 0,
        hidden: bits & UF_HIDDEN != 0,
        compressed: bits & UF_COMPRESSED != 0,
        ..FileFlags::default()
    }
}

/// Other Unix systems' flags are not read.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn flags(_path: &Path, _metadata: &Metadata) -> Option<FileFlags> {
    None
}

/// The SELinux label, whole, and any other `security.*` attribute by
/// name, as text where it is printable, else by its size. None of them
/// is the owner's to change here.
fn labels(path: &Path, facts: &mut AccessFacts) {
    if cfg!(target_vendor = "apple") {
        return;
    }
    let Ok(items) = super::extended_list(path) else {
        return;
    };
    for item in items {
        if !item.name.starts_with("security.") {
            continue;
        }
        let Ok(bytes) = super::extended_read(path, &item.name, SECURITY_TEXT_MAX) else {
            facts
                .security
                .push((item.name.clone(), bytes_words(item.size)));
            continue;
        };
        let text = printable(&bytes);
        if item.name == crate::access::SELINUX_ATTRIBUTE {
            facts.label = text;
        } else {
            let words = text.unwrap_or_else(|| bytes_words(bytes.len() as u64));
            facts.security.push((item.name.clone(), words));
        }
    }
}

fn bytes_words(n: u64) -> String {
    diroxide_core::format::count(n, "byte", "bytes")
}

/// An attribute's value as text: UTF-8 with its trailing NULs cut, and
/// no control characters; `None` otherwise.
fn printable(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    (!text.is_empty() && !text.chars().any(char::is_control)).then(|| text.to_owned())
}

/// The access lists of Linux; macOS's are not read this alpha (no
/// `exacl`), which the facts say.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod lists {
    use std::path::Path;

    use diroxide_core::EntryKind;

    use crate::access::{
        ACL_ACCESS_ATTRIBUTE, ACL_DEFAULT_ATTRIBUTE, ACL_MAX_BYTES, AccessFacts, AclTag,
        ListUnread, PosixAcl,
    };
    use crate::local::owners::Owners;

    /// Reads both lists: a missing attribute is no list beyond the mode;
    /// one that does not parse is said as damaged.
    pub(super) fn read(path: &Path, kind: EntryKind, owners: &Owners, facts: &mut AccessFacts) {
        facts.access_list = one(path, ACL_ACCESS_ATTRIBUTE, owners, facts);
        if kind == EntryKind::Folder {
            facts.default_list = one(path, ACL_DEFAULT_ATTRIBUTE, owners, facts);
        }
    }

    fn one(path: &Path, name: &str, owners: &Owners, facts: &mut AccessFacts) -> Option<PosixAcl> {
        // A missing attribute (`NotFound`) is a list no larger than the
        // mode; a file system without the attributes has none either.
        let Ok(bytes) = super::super::extended_read(path, name, ACL_MAX_BYTES) else {
            return None;
        };
        match PosixAcl::parse(&bytes) {
            Ok(mut list) => {
                name_entries(&mut list, owners);
                Some(list)
            }
            Err(_) => {
                facts.list_unread = Some(ListUnread::Damaged);
                None
            }
        }
    }

    /// Names each named user and group through [`Owners`]: a number this
    /// computer knows no name for keeps no name.
    pub(super) fn name_entries(list: &mut PosixAcl, owners: &Owners) {
        for entry in &mut list.entries {
            let Some(id) = entry.id else {
                continue;
            };
            let name = match entry.tag {
                AclTag::User => owners.user(id),
                AclTag::Group => owners.group(id),
                _ => continue,
            };
            if name != id.to_string() {
                entry.name = Some(name);
            }
            if entry.tag == AclTag::User {
                entry.full_name = owners.full_name(id).map(str::to_owned);
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod lists {
    use std::path::Path;

    use diroxide_core::EntryKind;

    use crate::access::{AccessFacts, ListUnread};
    use crate::local::owners::Owners;

    /// macOS and the BSDs keep access lists this alpha does not read.
    pub(super) fn read(_path: &Path, _kind: EntryKind, _owners: &Owners, facts: &mut AccessFacts) {
        facts.list_unread = Some(ListUnread::NotOnThisSystem);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_flags_say_the_finders_lock_and_the_systems() {
        let finder = mac_flags(0x2);
        assert!(finder.locked && !finder.administrator_flag);
        let system = mac_flags(0x2_0000 | 0x4_0000);
        assert!(system.locked && system.append_only && system.administrator_flag);
        let hidden = mac_flags(0x8000 | 0x1);
        assert!(hidden.hidden && hidden.no_backup && !hidden.locked);
        assert!(!mac_flags(0).any());
    }

    #[test]
    fn security_text_is_cut_at_its_nul_and_never_control_characters() {
        assert_eq!(
            printable(b"unconfined_u:object_r:user_home_t:s0\0").as_deref(),
            Some("unconfined_u:object_r:user_home_t:s0")
        );
        assert_eq!(printable(b"\x01\x00\x00\x02"), None);
        assert_eq!(printable(b"\0"), None);
    }

    #[test]
    fn the_owners_own_file_says_what_it_can_do() {
        let dir = crate::test_support::scratch();
        let path = dir.path().join("mine.txt");
        std::fs::write(&path, b"x").expect("a test file");
        let facts = access_facts(&path).expect("facts of a file of ours");
        assert_eq!(facts.source, AccessSource::Local);
        assert_eq!(facts.kind, Some(EntryKind::File));
        assert!(facts.owner.is_some() && facts.group.is_some());
        let you = facts.you_can.expect("you can");
        assert!(you.read && you.write, "{you:?}");
        assert!(!you.run || facts.mode.is_some_and(|m| m & 0o111 != 0));
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).expect("a link");
        let facts = access_facts(&link).expect("facts of a link");
        assert_eq!(facts.kind, Some(EntryKind::Link));
        assert_eq!(facts.mode, None, "a link has no permissions of its own");
    }
}
