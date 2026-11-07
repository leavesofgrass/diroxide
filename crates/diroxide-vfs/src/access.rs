//! Who may do what with one entry: the facts behind the access details
//! view (alpha.7; R29 part 4, R11 sections 6.1 and 6.7). The facts are
//! data; `diroxide-ops`'s `access` module says them in words.
//!
//! [`Provider::access`](crate::Provider::access) reads them for the
//! focused row only, off the input thread: the local provider asks the
//! system (the owner and the mode, "you can" for the running account, the
//! flags, the Linux access lists and security label, the Windows owner and
//! access list with an Authz check), and every other provider gives what
//! its listing holds, read only ([`AccessSource::Listing`]), or nothing
//! ([`AccessSource::None`], "This place has no permissions").
//!
//! The Linux access lists are read by hand from the two extended
//! attributes `system.posix_acl_access` and `system.posix_acl_default`
//! ([`PosixAcl::parse`]): a 4-byte version (2), then 8-byte entries of
//! tag, permission and id, all little-endian. No C library is needed.
//! macOS access lists are not read this alpha (no `exacl`), and the facts
//! say so ([`ListUnread::NotOnThisSystem`]).

use diroxide_core::{Attributes, Entry, EntryKind};
use thiserror::Error;

use crate::provider::Principal;

/// The access list attribute on Linux.
pub const ACL_ACCESS_ATTRIBUTE: &str = "system.posix_acl_access";
/// The default access list attribute of a Linux folder.
pub const ACL_DEFAULT_ATTRIBUTE: &str = "system.posix_acl_default";
/// The SELinux label attribute.
pub const SELINUX_ATTRIBUTE: &str = "security.selinux";

/// The three classic rights. On a folder, read is list, write is change
/// its contents, and run is enter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rights {
    /// Read a file; list a folder.
    pub read: bool,
    /// Write a file; add, remove and rename inside a folder.
    pub write: bool,
    /// Run a file; enter a folder.
    pub run: bool,
}

impl Rights {
    /// No rights at all.
    pub const NONE: Rights = Rights {
        read: false,
        write: false,
        run: false,
    };

    /// The rights of one octal digit of a mode or an access-list entry:
    /// 4 read, 2 write, 1 run.
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            read: bits & 0o4 != 0,
            write: bits & 0o2 != 0,
            run: bits & 0o1 != 0,
        }
    }

    /// The rights as one octal digit.
    pub const fn bits(self) -> u32 {
        (self.read as u32) << 2 | (self.write as u32) << 1 | self.run as u32
    }

    /// The rights both have: an entry's rights under a mask.
    pub const fn and(self, other: Rights) -> Rights {
        Rights::from_bits(self.bits() & other.bits())
    }

    /// Whether there are none.
    pub const fn is_none(self) -> bool {
        self.bits() == 0
    }
}

/// Where the facts came from, which says how far they go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessSource {
    /// The place keeps no permissions (most plugin places): "This place
    /// has no permissions".
    #[default]
    None,
    /// Read from the entry a listing gave (SFTP's numeric ids and mode,
    /// an archive's or a disk image's mode): shown read only, without
    /// "you can".
    Listing,
    /// Read from this computer's own system, for the running account.
    Local,
}

/// Flags beside the mode: Linux's inode flags (`chattr`), macOS's file
/// flags (`chflags`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FileFlags {
    /// Nobody can change, rename or remove it, not even an
    /// administrator, until the flag is cleared: Linux `i`, macOS `uchg`
    /// (the Finder's Locked) or `schg`.
    pub locked: bool,
    /// It can only grow: Linux `a`, macOS `uappnd` or `sappnd`.
    pub append_only: bool,
    /// The lock or the append-only flag is the system's (macOS `schg`,
    /// `sappnd`): only an administrator clears it. On Linux both flags
    /// always need one.
    pub administrator_flag: bool,
    /// Backup programs skip it: `d` and `nodump`.
    pub no_backup: bool,
    /// Hidden from the Finder (macOS `hidden`).
    pub hidden: bool,
    /// The file system compresses it (Linux `c`, macOS `compressed`).
    pub compressed: bool,
    /// The file system encrypts it (Linux `E`).
    pub encrypted: bool,
    /// Copies of its blocks are not shared (Linux `C`).
    pub no_copy_on_write: bool,
}

impl FileFlags {
    /// Whether any flag is set.
    pub fn any(&self) -> bool {
        self.locked
            || self.append_only
            || self.no_backup
            || self.hidden
            || self.compressed
            || self.encrypted
            || self.no_copy_on_write
    }
}

/// Which entry of a POSIX access list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AclTag {
    /// The owner (`user::`), tag 0x01.
    Owner,
    /// A named user (`user:ada:`), tag 0x02.
    User,
    /// The owning group (`group::`), tag 0x04.
    OwningGroup,
    /// A named group (`group:staff:`), tag 0x08.
    Group,
    /// The mask (`mask::`), tag 0x10: the most a named user, the owning
    /// group or a named group may have.
    Mask,
    /// Everyone else (`other::`), tag 0x20.
    Other,
}

impl AclTag {
    /// The tag as the kernel stores it.
    pub const fn code(self) -> u16 {
        match self {
            AclTag::Owner => 0x01,
            AclTag::User => 0x02,
            AclTag::OwningGroup => 0x04,
            AclTag::Group => 0x08,
            AclTag::Mask => 0x10,
            AclTag::Other => 0x20,
        }
    }

    /// The tag of a stored code.
    pub const fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            0x01 => AclTag::Owner,
            0x02 => AclTag::User,
            0x04 => AclTag::OwningGroup,
            0x08 => AclTag::Group,
            0x10 => AclTag::Mask,
            0x20 => AclTag::Other,
            _ => return None,
        })
    }

    /// Whether the entry names someone by number.
    pub const fn is_named(self) -> bool {
        matches!(self, AclTag::User | AclTag::Group)
    }
}

/// One entry of a POSIX access list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AclEntry {
    /// Which entry.
    pub tag: AclTag,
    /// Its rights, as stored (before the mask).
    pub rights: Rights,
    /// The user or group number of a named entry.
    pub id: Option<u32>,
    /// The name of a named entry, where this computer knows it.
    pub name: Option<String>,
    /// The full name of a named user, from the account's description,
    /// where one is given and differs from the name.
    pub full_name: Option<String>,
}

impl AclEntry {
    /// An entry without a name.
    pub fn new(tag: AclTag, rights: Rights, id: Option<u32>) -> Self {
        Self {
            tag,
            rights,
            id,
            name: None,
            full_name: None,
        }
    }
}

/// Why an access list could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AclError {
    /// Shorter than its header, or not a whole number of entries.
    #[error("the list is cut short")]
    Length,
    /// A version this reader does not know.
    #[error("the list is version {0}, not 2")]
    Version(u32),
    /// An entry with a tag this reader does not know.
    #[error("an entry has the unknown tag {0:#x}")]
    Tag(u16),
}

/// A POSIX access list, as Linux stores it in an extended attribute.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PosixAcl {
    /// The entries, in the stored order: the owner, named users, the
    /// owning group, named groups, the mask, everyone else.
    pub entries: Vec<AclEntry>,
}

/// The version of the stored form.
const ACL_VERSION: u32 = 2;
/// The id stored in an entry that names nobody.
const ACL_UNDEFINED_ID: u32 = u32::MAX;
/// The longest list read: 32 KiB, far beyond the kernel's own limit of
/// a page's worth of entries on most file systems.
pub const ACL_MAX_BYTES: usize = 32 * 1024;

impl PosixAcl {
    /// Reads the stored form: a 4-byte version (2), then 8-byte entries of
    /// a 2-byte tag, a 2-byte permission and a 4-byte id, little-endian.
    /// Bits of the permission beyond read, write and run are ignored.
    pub fn parse(bytes: &[u8]) -> Result<Self, AclError> {
        let (head, body) = bytes.split_first_chunk::<4>().ok_or(AclError::Length)?;
        let version = u32::from_le_bytes(*head);
        if version != ACL_VERSION {
            return Err(AclError::Version(version));
        }
        if body.len() % 8 != 0 {
            return Err(AclError::Length);
        }
        let mut entries = Vec::with_capacity(body.len() / 8);
        for chunk in body.chunks_exact(8) {
            let tag = u16::from_le_bytes([chunk[0], chunk[1]]);
            let perm = u16::from_le_bytes([chunk[2], chunk[3]]);
            let id = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            let tag = AclTag::from_code(tag).ok_or(AclError::Tag(tag))?;
            let id = (tag.is_named() && id != ACL_UNDEFINED_ID).then_some(id);
            entries.push(AclEntry::new(tag, Rights::from_bits(u32::from(perm)), id));
        }
        Ok(Self { entries })
    }

    /// The stored form, the inverse of [`PosixAcl::parse`]. The kernel
    /// checks a list it is given (the order, a mask wherever named
    /// entries are), so a list made here is refused there when it is not
    /// one the kernel keeps.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + 8 * self.entries.len());
        bytes.extend_from_slice(&ACL_VERSION.to_le_bytes());
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.tag.code().to_le_bytes());
            let perm = entry.rights.bits() as u16;
            bytes.extend_from_slice(&perm.to_le_bytes());
            let id = entry.id.unwrap_or(ACL_UNDEFINED_ID);
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        bytes
    }

    /// The first entry with a tag.
    pub fn find(&self, tag: AclTag) -> Option<&AclEntry> {
        self.entries.iter().find(|entry| entry.tag == tag)
    }

    /// The mask, when the list has one.
    pub fn mask(&self) -> Option<Rights> {
        self.find(AclTag::Mask).map(|entry| entry.rights)
    }

    /// The named users and named groups: the entries beyond the mode.
    pub fn named(&self) -> impl Iterator<Item = &AclEntry> {
        self.entries.iter().filter(|entry| entry.tag.is_named())
    }

    /// The rights an entry really gives: a named user's, the owning
    /// group's and a named group's under the mask; the owner's and
    /// everyone else's as stored.
    pub fn effective(&self, entry: &AclEntry) -> Rights {
        match (entry.tag, self.mask()) {
            (AclTag::User | AclTag::OwningGroup | AclTag::Group, Some(mask)) => {
                entry.rights.and(mask)
            }
            _ => entry.rights,
        }
    }
}

/// Why an access list is not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ListUnread {
    /// This system's lists are not read this alpha (macOS: no `exacl`).
    NotOnThisSystem,
    /// The list is stored but could not be read: damaged, or in a form
    /// this reader does not know.
    Damaged,
}

/// A Windows account, by its security id, with its name where Windows
/// knows one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Account {
    /// The name as Windows says it: `BUILTIN\Administrators`,
    /// `NT AUTHORITY\SYSTEM`, `Everyone`, `PC\ada`. `None` for an account
    /// that no longer exists, which is then said by its security id.
    pub name: Option<String>,
    /// The security id in text: `S-1-5-32-544`.
    pub sid: String,
    /// Whether it is a group of accounts (a group, an alias, a well-known
    /// group) rather than one account.
    pub group: bool,
}

impl Account {
    /// The name without its domain (`Administrators`, `SYSTEM`), or the
    /// security id when no name is known.
    pub fn short_name(&self) -> &str {
        match &self.name {
            Some(name) => name.rsplit('\\').next().unwrap_or(name),
            None => &self.sid,
        }
    }
}

/// What a Windows access-list entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EntryEffect {
    /// It grants the rights.
    Allow,
    /// It denies the rights.
    Deny,
    /// Another kind of entry (an object entry, a conditional entry), by
    /// its type number; listed, not explained.
    Other(u8),
}

/// The inheritance flags of a Windows access-list entry.
pub mod inherit {
    /// Files inside inherit it (OI).
    pub const FILES: u8 = 0x1;
    /// Folders inside inherit it (CI).
    pub const FOLDERS: u8 = 0x2;
    /// It goes one level down only (NP).
    pub const ONE_LEVEL: u8 = 0x4;
    /// It applies only to what is inside, not here (IO).
    pub const INSIDE_ONLY: u8 = 0x8;
    /// It came from a folder above (I).
    pub const INHERITED: u8 = 0x10;
}

/// One Windows access-list entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccessEntry {
    /// Allow, deny or another kind.
    pub effect: EntryEffect,
    /// Whom it is for.
    pub who: Account,
    /// The rights, as stored (generic rights not mapped).
    pub mask: u32,
    /// The inheritance flags ([`inherit`]).
    pub flags: u8,
    /// The folder it was inherited from, where Windows says.
    pub inherited_from: Option<String>,
}

impl AccessEntry {
    /// Whether it came from a folder above.
    pub const fn inherited(&self) -> bool {
        self.flags & inherit::INHERITED != 0
    }
}

/// A Windows access list, as read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowsList {
    /// The entries, in the stored order, which is the order Windows
    /// reads them in.
    Entries(Vec<AccessEntry>),
    /// No access list at all: everyone has full control.
    Missing,
    /// The security descriptor could not be read (no right to read it).
    Unreadable,
}

/// The Windows owner and access list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct WindowsAccess {
    /// The owner.
    pub owner: Option<Account>,
    /// The access list.
    pub list: WindowsList,
    /// Whether inheritance from the folder above is turned off here.
    pub protected: bool,
}

impl WindowsAccess {
    /// Facts with an owner and a list.
    pub fn new(owner: Option<Account>, list: WindowsList, protected: bool) -> Self {
        Self {
            owner,
            list,
            protected,
        }
    }
}

/// Who may do what with one entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AccessFacts {
    /// Where the facts came from.
    pub source: AccessSource,
    /// What the entry is: the rights read "list, enter and change" on a
    /// folder.
    pub kind: Option<EntryKind>,
    /// The owner's name, or number when no name is known.
    pub owner: Option<String>,
    /// The owner's number (Unix).
    pub owner_id: Option<u32>,
    /// The group's name, or number when no name is known.
    pub group: Option<String>,
    /// The group's number (Unix).
    pub group_id: Option<u32>,
    /// The twelve mode bits (Unix and SFTP, an archive's mode).
    pub mode: Option<u32>,
    /// What the running account can do: advice, not a promise (a file
    /// another program holds, or a disk that turns read only, still
    /// stops a write).
    pub you_can: Option<Rights>,
    /// The flags beside the mode, where read.
    pub flags: Option<FileFlags>,
    /// The Linux access list beyond the mode.
    pub access_list: Option<PosixAcl>,
    /// A Linux folder's default access list, given to new items.
    pub default_list: Option<PosixAcl>,
    /// Why an access list is not shown, when one may exist.
    pub list_unread: Option<ListUnread>,
    /// The SELinux label, whole: `unconfined_u:object_r:user_home_t:s0`.
    pub label: Option<String>,
    /// Other `security.*` attributes, by name, with their text or size.
    pub security: Vec<(String, String)>,
    /// The Windows owner and access list.
    pub windows: Option<WindowsAccess>,
}

impl AccessFacts {
    /// The facts of a place that keeps no permissions.
    pub fn none() -> Self {
        Self::default()
    }

    /// The facts a listing's entry holds: the mode, the owner and the
    /// group, read only, without "you can". A place whose entries hold
    /// none of them keeps no permissions ([`AccessSource::None`]).
    pub fn from_entry(entry: &Entry) -> Self {
        let mode = match entry.attributes {
            Some(Attributes::Unix(mode)) => Some(mode & 0o7777),
            _ => None,
        };
        if mode.is_none() && entry.owner.is_none() && entry.group.is_none() {
            return Self::none();
        }
        Self {
            source: AccessSource::Listing,
            kind: Some(entry.kind),
            owner_id: entry.owner.as_deref().and_then(|o| o.parse().ok()),
            owner: entry.owner.clone(),
            group_id: entry.group.as_deref().and_then(|g| g.parse().ok()),
            group: entry.group.clone(),
            mode,
            ..Self::default()
        }
    }

    /// Whether the entry is a folder.
    pub fn is_folder(&self) -> bool {
        self.kind == Some(EntryKind::Folder)
    }
}

/// A user or group, looked up on this computer by name or number, for a
/// change of owner: its number as the id, and its name where known.
/// `None` when this computer knows no such name, or on Windows, where
/// owners are changed in the system's own Security dialog.
pub fn find_user(text: &str) -> Option<Principal> {
    find(text, false)
}

/// A group, looked up as [`find_user`] looks up a user.
pub fn find_group(text: &str) -> Option<Principal> {
    find(text, true)
}

#[cfg(unix)]
fn find(text: &str, group: bool) -> Option<Principal> {
    let owners = crate::local::owners::Owners::shared();
    owners.principal(text, group)
}

#[cfg(not(unix))]
fn find(_text: &str, _group: bool) -> Option<Principal> {
    None
}

/// Who the running program is, for the owner plan: an administrator may
/// give anything to anyone; anyone else may change only the group, and
/// only to one of their own groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Caller {
    /// The effective user number (Unix).
    pub user: Option<u32>,
    /// The effective group and the supplementary groups (Unix), when
    /// they could be read.
    pub groups: Option<Vec<u32>>,
    /// Whether the program runs as root (Unix). Windows plans no owner
    /// change, so it is never set there.
    pub administrator: bool,
}

impl Caller {
    /// A caller that is root.
    pub fn administrator() -> Self {
        Self {
            user: Some(0),
            groups: None,
            administrator: true,
        }
    }

    /// A caller with a user number and groups.
    pub fn user(user: u32, groups: Vec<u32>) -> Self {
        Self {
            user: Some(user),
            administrator: user == 0,
            groups: Some(groups),
        }
    }

    /// Whether a group is one of the caller's, or unknown (`None`) when
    /// the groups could not be read: the system then decides.
    pub fn in_group(&self, gid: u32) -> Option<bool> {
        self.groups.as_ref().map(|groups| groups.contains(&gid))
    }

    /// The running program: on Linux and Android from
    /// `/proc/self/status` (its `Uid` and `Groups` lines; the `getgroups`
    /// list the kernel keeps), elsewhere unknown.
    pub fn current() -> Self {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
                return Self::from_status(&status);
            }
        }
        Self::default()
    }

    /// Reads the `Uid:`, `Gid:` and `Groups:` lines of a process status
    /// file: the effective ids are the second numbers.
    pub fn from_status(status: &str) -> Self {
        let field = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .map(|rest| {
                    rest.split_whitespace()
                        .filter_map(|n| n.parse::<u32>().ok())
                        .collect::<Vec<u32>>()
                })
        };
        let user = field("Uid:").and_then(|ids| ids.get(1).copied());
        let group = field("Gid:").and_then(|ids| ids.get(1).copied());
        let groups = field("Groups:").map(|mut groups| {
            if let Some(group) = group
                && !groups.contains(&group)
            {
                groups.insert(0, group);
            }
            groups
        });
        Self {
            user,
            administrator: user == Some(0),
            groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acl(entries: &[(AclTag, u32, Option<u32>)]) -> PosixAcl {
        PosixAcl {
            entries: entries
                .iter()
                .map(|&(tag, bits, id)| AclEntry::new(tag, Rights::from_bits(bits), id))
                .collect(),
        }
    }

    #[test]
    fn a_stored_list_reads_back_the_same() {
        let list = acl(&[
            (AclTag::Owner, 6, None),
            (AclTag::User, 6, Some(1001)),
            (AclTag::OwningGroup, 4, None),
            (AclTag::Group, 5, Some(50)),
            (AclTag::Mask, 4, None),
            (AclTag::Other, 4, None),
        ]);
        let bytes = list.encode();
        assert_eq!(bytes.len(), 4 + 6 * 8);
        assert_eq!(&bytes[..4], &[2, 0, 0, 0]);
        // The named user: tag 0x02, read and write, id 1001.
        assert_eq!(&bytes[12..20], &[0x02, 0, 6, 0, 0xe9, 0x03, 0, 0]);
        // The owner names nobody: the undefined id.
        assert_eq!(&bytes[8..12], &[0xff; 4]);
        assert_eq!(PosixAcl::parse(&bytes), Ok(list.clone()));
        assert_eq!(list.mask(), Some(Rights::from_bits(4)));
        assert_eq!(list.named().count(), 2);
        let user = &list.entries[1];
        assert_eq!(list.effective(user), Rights::from_bits(4), "the mask caps");
        assert_eq!(list.effective(&list.entries[0]), Rights::from_bits(6));
        assert_eq!(list.effective(&list.entries[5]), Rights::from_bits(4));
    }

    #[test]
    fn a_damaged_list_is_refused_in_words() {
        assert_eq!(PosixAcl::parse(&[2, 0, 0]), Err(AclError::Length));
        assert_eq!(PosixAcl::parse(&[1, 0, 0, 0]), Err(AclError::Version(1)));
        assert_eq!(
            PosixAcl::parse(&[2, 0, 0, 0, 1, 0, 6, 0, 0xff, 0xff, 0xff]),
            Err(AclError::Length)
        );
        assert_eq!(
            PosixAcl::parse(&[2, 0, 0, 0, 0x40, 0, 6, 0, 0xff, 0xff, 0xff, 0xff]),
            Err(AclError::Tag(0x40))
        );
        assert_eq!(PosixAcl::parse(&[2, 0, 0, 0]), Ok(PosixAcl::default()));
        assert_eq!(
            AclError::Tag(0x40).to_string(),
            "an entry has the unknown tag 0x40"
        );
    }

    #[test]
    fn rights_are_the_octal_digit() {
        for bits in 0..8 {
            assert_eq!(Rights::from_bits(bits).bits(), bits);
        }
        assert!(Rights::NONE.is_none());
        assert_eq!(
            Rights::from_bits(7).and(Rights::from_bits(5)),
            Rights::from_bits(5)
        );
    }

    #[test]
    fn a_listing_gives_its_mode_and_numbers_read_only() {
        let mut entry = Entry::new("a.txt", EntryKind::File);
        assert_eq!(AccessFacts::from_entry(&entry).source, AccessSource::None);
        entry.attributes = Some(Attributes::Unix(0o100_644));
        entry.owner = Some("1000".to_owned());
        entry.group = Some("staff".to_owned());
        let facts = AccessFacts::from_entry(&entry);
        assert_eq!(facts.source, AccessSource::Listing);
        assert_eq!(facts.mode, Some(0o644));
        assert_eq!(facts.owner_id, Some(1000));
        assert_eq!(facts.group_id, None);
        assert_eq!(facts.you_can, None, "no advice from a listing");
        let mut windows = Entry::new("b.txt", EntryKind::File);
        windows.attributes = Some(Attributes::Windows(0x20));
        assert_eq!(AccessFacts::from_entry(&windows).source, AccessSource::None);
    }

    #[test]
    fn the_caller_is_read_from_the_status_lines() {
        let status = "Name:\tcargo\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\nGroups:\t27 100 \n";
        let caller = Caller::from_status(status);
        assert_eq!(caller.user, Some(1000));
        assert!(!caller.administrator);
        assert_eq!(caller.groups, Some(vec![1000, 27, 100]));
        assert_eq!(caller.in_group(27), Some(true));
        assert_eq!(caller.in_group(0), Some(false));
        let root = Caller::from_status("Uid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\nGroups:\t\n");
        assert!(root.administrator);
        assert_eq!(root.groups, Some(vec![0]));
        assert_eq!(Caller::default().in_group(5), None);
    }

    #[test]
    fn a_windows_account_is_said_without_its_domain() {
        let admins = Account {
            name: Some("BUILTIN\\Administrators".to_owned()),
            sid: "S-1-5-32-544".to_owned(),
            group: true,
        };
        assert_eq!(admins.short_name(), "Administrators");
        let gone = Account {
            name: None,
            sid: "S-1-5-21-1-2-3-1001".to_owned(),
            group: false,
        };
        assert_eq!(gone.short_name(), "S-1-5-21-1-2-3-1001");
    }
}
