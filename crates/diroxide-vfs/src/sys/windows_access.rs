//! Windows: who may do what with one entry (R29 4.2; R11 3.7 and 6.7),
//! for the focused row only. The owner comes from the security
//! descriptor (`GetNamedSecurityInfoW`) and its name from
//! `LookupAccountSidW`; the access list's entries are read in their
//! stored order, which is the order Windows checks them in, each with the
//! folder it was inherited from (`GetInheritanceSourceW`); and "you can"
//! is an Authz access check for the running process's own token
//! (`AuthzInitializeContextFromToken`, `AuthzAccessCheck`), so groups the
//! token only denies (an administrator without elevation) count as they
//! do for a real open.
//!
//! Never `GetEffectiveRightsFromAcl` (documented as possibly unavailable,
//! and blind to the owner's implied rights) and never `icacls` text
//! (translated, so unreadable to a program). Every `unsafe` block says
//! why it is sound; every buffer and handle the calls give back is freed
//! or closed by a guard that drops on every path: the security descriptor
//! and the security-id text (`LocalFree`), the inheritance array
//! (`FreeInheritedFromArray`), the Authz context and resource manager
//! (`AuthzFreeContext`, `AuthzFreeResourceManager`), and the process
//! token (`OwnedHandle`, `CloseHandle`).

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::Metadata;
use std::io;
use std::mem::size_of;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};

use diroxide_core::EntryKind;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS, HANDLE, LUID, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    AUTHZ_ACCESS_REPLY, AUTHZ_ACCESS_REQUEST, AUTHZ_CLIENT_CONTEXT_HANDLE,
    AUTHZ_RESOURCE_MANAGER_HANDLE, AUTHZ_RM_FLAG_NO_AUDIT, AuthzAccessCheck, AuthzFreeContext,
    AuthzFreeResourceManager, AuthzInitializeContextFromToken, AuthzInitializeResourceManager,
    ConvertSidToStringSidW, FreeInheritedFromArray, GetInheritanceSourceW, GetNamedSecurityInfoW,
    INHERITED_FROMW, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION, GetAce,
    GetAclInformation, GetSecurityDescriptorControl, IsValidSid, LookupAccountSidW,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SID_NAME_USE,
    SidTypeAlias, SidTypeGroup, SidTypeWellKnownGroup, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_ATTRIBUTE_READONLY, FILE_EXECUTE, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_DATA, FILE_WRITE_DATA, SYNCHRONIZE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::windows::wide;
use crate::access::{
    AccessEntry, AccessFacts, AccessSource, Account, EntryEffect, Rights, WindowsAccess,
    WindowsList,
};
use crate::error::VfsError;

/// The access a check asks for when it wants to know everything granted
/// (`MAXIMUM_ALLOWED`, `winnt.h`; its windows-sys module is not one this
/// crate turns on).
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
/// An entry that grants (`ACCESS_ALLOWED_ACE_TYPE`).
const ALLOW: u8 = 0;
/// An entry that denies (`ACCESS_DENIED_ACE_TYPE`).
const DENY: u8 = 1;
/// The conditional entries, laid out as the plain ones up to the security
/// id (`ACCESS_ALLOWED_CALLBACK_ACE_TYPE`, `ACCESS_DENIED_CALLBACK_ACE_TYPE`).
const CALLBACK_TYPES: [u8; 2] = [9, 10];

/// How generic rights map to a file's own (`FILE_GENERIC_*`).
const FILE_MAPPING: GENERIC_MAPPING = GENERIC_MAPPING {
    GenericRead: FILE_GENERIC_READ,
    GenericWrite: FILE_GENERIC_WRITE,
    GenericExecute: FILE_GENERIC_EXECUTE,
    GenericAll: FILE_ALL_ACCESS,
};

/// A buffer Windows allocated for the caller, freed with `LocalFree`.
struct LocalBuffer(*mut c_void);

impl Drop for LocalBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a call documented to allocate
            // with LocalAlloc (GetNamedSecurityInfoW,
            // ConvertSidToStringSidW), and is freed once, here.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// An entry's security descriptor: its owner, group and access list,
/// pointers into one buffer that lives as long as this does.
struct Descriptor {
    buffer: LocalBuffer,
    owner: PSID,
    dacl: *mut ACL,
}

impl Descriptor {
    /// Reads the owner, the group and the access list of the entry at a
    /// path (the group too, since an access check needs a whole
    /// descriptor).
    fn read(path: &[u16]) -> io::Result<Self> {
        let mut owner: PSID = null_mut();
        let mut group: PSID = null_mut();
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `path` is NUL-terminated; the out pointers are valid
        // locals. On success the descriptor is a LocalAlloc buffer that the
        // guard frees, and the three pointers point into it.
        let error = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &raw mut owner,
                &raw mut group,
                &raw mut dacl,
                null_mut(),
                &raw mut descriptor,
            )
        };
        let buffer = LocalBuffer(descriptor);
        if error != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(Self {
            buffer,
            owner,
            dacl,
        })
    }

    /// Whether inheritance from the folder above is turned off.
    fn protected(&self) -> bool {
        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: the descriptor is valid while `self.buffer` lives; the
        // out pointers are valid locals.
        let ok = unsafe {
            GetSecurityDescriptorControl(self.buffer.0, &raw mut control, &raw mut revision)
        };
        ok != 0 && control & SE_DACL_PROTECTED != 0
    }

    /// The number of entries in the access list, `None` with no list.
    fn entry_count(&self) -> Option<u32> {
        if self.dacl.is_null() {
            return None;
        }
        let mut info = ACL_SIZE_INFORMATION::default();
        // SAFETY: `dacl` points into the live descriptor; `info` is a
        // writable struct of the size given.
        let ok = unsafe {
            GetAclInformation(
                self.dacl,
                (&raw mut info).cast::<c_void>(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        Some(if ok == 0 { 0 } else { info.AceCount })
    }

    /// The entries of the access list, in the stored order.
    fn entries(&self, path: &[u16], folder: bool) -> WindowsList {
        let Some(count) = self.entry_count() else {
            return WindowsList::Missing;
        };
        let sources = inheritance_sources(path, folder, self.dacl, count);
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut ace: *mut c_void = null_mut();
            // SAFETY: `index` is below the list's entry count; `ace` is a
            // valid out pointer, set to an entry inside the live list.
            if unsafe { GetAce(self.dacl, index, &raw mut ace) } == 0 || ace.is_null() {
                continue;
            }
            // SAFETY: every entry starts with an ACE_HEADER, and GetAce
            // returned a pointer to a whole entry.
            let header = unsafe { *ace.cast::<ACE_HEADER>() };
            let plain = header.AceType == ALLOW
                || header.AceType == DENY
                || CALLBACK_TYPES.contains(&header.AceType);
            let fits = usize::from(header.AceSize) >= size_of::<ACCESS_ALLOWED_ACE>();
            let (mask, who) = if plain && fits {
                let entry = ace.cast::<ACCESS_ALLOWED_ACE>();
                // SAFETY: the entry is a plain or conditional one at least
                // the size of ACCESS_ALLOWED_ACE, whose layout it shares up
                // to and including the start of its security id; the id is
                // checked before it is read.
                let (mask, sid) =
                    unsafe { ((*entry).Mask, (&raw const (*entry).SidStart).cast_mut()) };
                (mask, account(sid.cast::<c_void>()))
            } else {
                let mask = if usize::from(header.AceSize) >= size_of::<ACE_HEADER>() + 4 {
                    // SAFETY: every entry type keeps its rights in the four
                    // bytes after the header, which the size covers.
                    unsafe {
                        ace.cast::<u8>()
                            .add(size_of::<ACE_HEADER>())
                            .cast::<u32>()
                            .read_unaligned()
                    }
                } else {
                    0
                };
                (mask, unknown_account())
            };
            let effect = match header.AceType {
                ALLOW => EntryEffect::Allow,
                DENY => EntryEffect::Deny,
                other => EntryEffect::Other(other),
            };
            entries.push(AccessEntry {
                effect,
                who,
                mask,
                flags: header.AceFlags,
                inherited_from: sources.get(index as usize).cloned().flatten(),
            });
        }
        WindowsList::Entries(entries)
    }
}

/// The inheritance array `GetInheritanceSourceW` filled, freed with
/// `FreeInheritedFromArray`.
struct Inherited(Vec<INHERITED_FROMW>);

impl Drop for Inherited {
    fn drop(&mut self) {
        let count = u16::try_from(self.0.len()).unwrap_or(u16::MAX);
        // SAFETY: the array was filled by a successful
        // GetInheritanceSourceW for this many entries (a guard is made only
        // then); its names are freed once, here, and the array itself is
        // the Vec's.
        unsafe { FreeInheritedFromArray(self.0.as_ptr(), count, null()) };
    }
}

/// The folder each entry was inherited from, in the list's order: `None`
/// for an entry set here, or when Windows cannot say.
fn inheritance_sources(
    path: &[u16],
    folder: bool,
    dacl: *mut ACL,
    count: u32,
) -> Vec<Option<String>> {
    let unknown = vec![None; count as usize];
    if count == 0 || count > u32::from(u16::MAX) {
        return unknown;
    }
    let mut array = vec![INHERITED_FROMW::default(); count as usize];
    // SAFETY: `path` is NUL-terminated; `dacl` is the entry's own list,
    // alive in its descriptor; the array holds one slot per entry, as the
    // call requires; no object types, no callbacks.
    let error = unsafe {
        GetInheritanceSourceW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            i32::from(folder),
            null(),
            0,
            dacl,
            null(),
            &FILE_MAPPING,
            array.as_mut_ptr(),
        )
    };
    if error != ERROR_SUCCESS {
        return unknown;
    }
    let filled = Inherited(array);
    filled
        .0
        .iter()
        .map(|item| {
            (item.GenerationGap > 0 && !item.AncestorName.is_null())
                // SAFETY: a filled name is a NUL-terminated wide string the
                // array owns until the guard frees it, after this read.
                .then(|| unsafe { wide_text(item.AncestorName) })
        })
        .collect()
}

/// A NUL-terminated wide string, as text.
///
/// # Safety
///
/// `text` must point to a NUL-terminated wide string that stays alive for
/// the call.
unsafe fn wide_text(text: *const u16) -> String {
    let mut len = 0usize;
    // SAFETY: the caller promises a NUL-terminated string; the loop stops
    // at the NUL.
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: the `len` units before the NUL were just read.
    let units = unsafe { std::slice::from_raw_parts(text, len) };
    let text = String::from_utf16_lossy(units);
    diroxide_core::place::strip_extended_prefix(&text).into_owned()
}

/// An account no entry names in a form read here.
fn unknown_account() -> Account {
    Account {
        name: None,
        sid: String::new(),
        group: false,
    }
}

/// The security id as text: `S-1-5-32-544`.
fn sid_text(sid: PSID) -> Option<String> {
    let mut text: *mut u16 = null_mut();
    // SAFETY: `sid` was checked valid by the caller; `text` is a valid out
    // pointer, set to a LocalAlloc string the guard frees.
    let ok = unsafe { ConvertSidToStringSidW(sid, &raw mut text) };
    let guard = LocalBuffer(text.cast::<c_void>());
    if ok == 0 || text.is_null() {
        return None;
    }
    // SAFETY: the string is NUL-terminated and lives until `guard` drops,
    // after this read.
    let text = unsafe { wide_text(text) };
    drop(guard);
    Some(text)
}

/// An account's name as `DOMAIN\name`, and whether it is a group.
type Named = (String, bool);

/// The names already looked up, by security id: a lookup of an account
/// another computer keeps can wait on the network, so it is made once.
fn names() -> &'static Mutex<HashMap<String, Option<Named>>> {
    static NAMES: OnceLock<Mutex<HashMap<String, Option<Named>>>> = OnceLock::new();
    NAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The account a security id names: its text, its name as
/// `DOMAIN\name` (or the name alone where there is no domain), and
/// whether it is a group.
fn account(sid: PSID) -> Account {
    // SAFETY: IsValidSid reads only the bytes the security id's own
    // header says it has; `sid` points into a live descriptor.
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return unknown_account();
    }
    let text = sid_text(sid).unwrap_or_default();
    let cached = {
        let names = names()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        names.get(&text).cloned()
    };
    let found = match cached {
        Some(found) => found,
        None => {
            let found = lookup(sid);
            names()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(text.clone(), found.clone());
            found
        }
    };
    let (name, group) = match found {
        Some((name, group)) => (Some(name), group),
        None => (None, false),
    };
    Account {
        name,
        sid: text,
        group,
    }
}

/// `LookupAccountSidW`, asked for the sizes, then the names.
fn lookup(sid: PSID) -> Option<Named> {
    let mut name_len = 0u32;
    let mut domain_len = 0u32;
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: the sizing call: no buffers and zero lengths, as documented;
    // it writes only the two lengths and the kind.
    unsafe {
        LookupAccountSidW(
            null(),
            sid,
            null_mut(),
            &raw mut name_len,
            null_mut(),
            &raw mut domain_len,
            &raw mut kind,
        )
    };
    if name_len == 0 {
        return None;
    }
    let mut name = vec![0u16; name_len as usize];
    let mut domain = vec![0u16; domain_len.max(1) as usize];
    let mut domain_len = domain.len() as u32;
    // SAFETY: both buffers are writable for the lengths given.
    let ok = unsafe {
        LookupAccountSidW(
            null(),
            sid,
            name.as_mut_ptr(),
            &raw mut name_len,
            domain.as_mut_ptr(),
            &raw mut domain_len,
            &raw mut kind,
        )
    };
    if ok == 0 {
        return None;
    }
    let name = String::from_utf16_lossy(&name[..(name_len as usize).min(name.len())]);
    let domain = String::from_utf16_lossy(&domain[..(domain_len as usize).min(domain.len())]);
    let whole = if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    };
    let group = [SidTypeGroup, SidTypeAlias, SidTypeWellKnownGroup].contains(&kind);
    Some((whole, group))
}

/// An Authz resource manager, freed on drop.
struct Manager(AUTHZ_RESOURCE_MANAGER_HANDLE);

impl Drop for Manager {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful
        // AuthzInitializeResourceManager and is freed once, here.
        unsafe { AuthzFreeResourceManager(self.0) };
    }
}

/// An Authz client context, freed on drop.
struct Context(AUTHZ_CLIENT_CONTEXT_HANDLE);

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful
        // AuthzInitializeContextFromToken and is freed once, here.
        unsafe { AuthzFreeContext(self.0) };
    }
}

/// The running process's own token, for reading only.
fn process_token() -> Option<OwnedHandle> {
    let mut token: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no
    // closing; `token` is a valid out pointer, set to a real handle on
    // success, which the OwnedHandle closes.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
    if ok == 0 || token.is_null() {
        return None;
    }
    // SAFETY: the handle is new, open and owned by nothing else.
    Some(unsafe { OwnedHandle::from_raw_handle(token) })
}

/// Every right the running process's token has to an entry with this
/// descriptor, by an Authz check that asks for the most allowed; `None`
/// when the check cannot be made.
fn granted(descriptor: &Descriptor) -> Option<u32> {
    let mut manager: AUTHZ_RESOURCE_MANAGER_HANDLE = null_mut();
    // SAFETY: no callbacks and no name; `manager` is a valid out pointer,
    // set to a handle the guard frees.
    let ok = unsafe {
        AuthzInitializeResourceManager(
            AUTHZ_RM_FLAG_NO_AUDIT,
            None,
            None,
            None,
            null(),
            &raw mut manager,
        )
    };
    if ok == 0 || manager.is_null() {
        return None;
    }
    let manager = Manager(manager);
    let token = process_token()?;
    let mut context: AUTHZ_CLIENT_CONTEXT_HANDLE = null_mut();
    // SAFETY: the token is open for querying for the length of the call;
    // no expiry and no dynamic groups; `context` is a valid out pointer,
    // set to a handle the guard frees (before the manager, which it
    // depends on, since locals drop in reverse order).
    let ok = unsafe {
        AuthzInitializeContextFromToken(
            0,
            token.as_raw_handle(),
            manager.0,
            null(),
            LUID::default(),
            null(),
            &raw mut context,
        )
    };
    if ok == 0 || context.is_null() {
        return None;
    }
    let context = Context(context);
    let request = AUTHZ_ACCESS_REQUEST {
        DesiredAccess: MAXIMUM_ALLOWED,
        ..AUTHZ_ACCESS_REQUEST::default()
    };
    let mut mask = 0u32;
    let mut error = 0u32;
    let mut reply = AUTHZ_ACCESS_REPLY {
        ResultListLength: 1,
        GrantedAccessMask: &raw mut mask,
        SaclEvaluationResults: null_mut(),
        Error: &raw mut error,
    };
    // SAFETY: the context and the descriptor are alive; the request asks
    // for one result and the reply points at one mask and one error, both
    // valid locals; no audit event, no extra descriptors, no result
    // handle kept.
    let ok = unsafe {
        AuthzAccessCheck(
            0,
            context.0,
            &raw const request,
            null_mut(),
            descriptor.buffer.0,
            null(),
            0,
            &raw mut reply,
            null_mut(),
        )
    };
    drop(context);
    drop(manager);
    (ok != 0).then_some(if error == ERROR_SUCCESS { mask } else { 0 })
}

/// The rights a granted mask gives, as a program's open asks for them. A
/// file is read, written and run with the generic bundles, which carry
/// synchronize: spike AS1 found a file whose list denies only write (which
/// Windows' write bundle makes include synchronize) refuses an open for
/// reading too, so read needs the whole read bundle, not only "read
/// data". A folder is listed with "list" and synchronize, changed with
/// "add file", entered with "traverse". A file with the read-only
/// attribute cannot be written, whatever its list says.
pub(crate) fn rights_of(mask: u32, folder: bool, attributes: u32) -> Rights {
    let all = |bits: u32| mask & bits == bits;
    if folder {
        return Rights {
            read: all(FILE_READ_DATA | SYNCHRONIZE),
            write: all(FILE_WRITE_DATA),
            run: all(FILE_EXECUTE),
        };
    }
    let read_only = attributes & FILE_ATTRIBUTE_READONLY != 0;
    Rights {
        read: all(FILE_GENERIC_READ),
        write: all(FILE_GENERIC_WRITE) && !read_only,
        run: all(FILE_GENERIC_EXECUTE),
    }
}

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

/// Who may do what with the entry at `path`, never through a final link
/// (a link's own list does not decide who reaches its target, so a link
/// is said to have no permissions of its own, as on Unix).
pub(crate) fn access_facts(path: &Path) -> Result<AccessFacts, VfsError> {
    let metadata = std::fs::symlink_metadata(path)?;
    let kind = kind_of(&metadata);
    let mut facts = AccessFacts {
        source: AccessSource::Local,
        kind: Some(kind),
        ..AccessFacts::default()
    };
    if kind == EntryKind::Link {
        return Ok(facts);
    }
    let path = wide(path);
    let descriptor = match Descriptor::read(&path) {
        Ok(descriptor) => descriptor,
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            facts.windows = Some(WindowsAccess::new(None, WindowsList::Unreadable, false));
            return Ok(facts);
        }
        Err(error) => return Err(error.into()),
    };
    let folder = kind == EntryKind::Folder;
    let owner = (!descriptor.owner.is_null()).then(|| account(descriptor.owner));
    facts.owner = owner
        .as_ref()
        .map(|owner| owner.name.clone().unwrap_or_else(|| owner.sid.clone()));
    let list = descriptor.entries(&path, folder);
    let protected = descriptor.protected();
    facts.you_can =
        granted(&descriptor).map(|mask| rights_of(mask, folder, metadata.file_attributes()));
    facts.windows = Some(WindowsAccess::new(owner, list, protected));
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_granted_mask_reads_as_rights_and_read_only_stops_a_write() {
        let all = FILE_ALL_ACCESS;
        assert_eq!(rights_of(all, false, 0), Rights::from_bits(7));
        assert_eq!(
            rights_of(all, false, FILE_ATTRIBUTE_READONLY),
            Rights::from_bits(5)
        );
        assert_eq!(
            rights_of(all, true, FILE_ATTRIBUTE_READONLY),
            Rights::from_bits(7),
            "Windows ignores read-only on folders"
        );
        assert_eq!(rights_of(FILE_GENERIC_READ, false, 0), Rights::from_bits(4));
        assert_eq!(rights_of(0, false, 0), Rights::NONE);
        // AS1: write denied takes synchronize with it, and with it every
        // open for reading.
        let no_synchronize = FILE_ALL_ACCESS & !SYNCHRONIZE & !FILE_WRITE_DATA;
        assert_eq!(rights_of(no_synchronize, false, 0), Rights::NONE);
        assert!(!rights_of(FILE_READ_DATA, false, 0).read, "read data alone");
        assert!(
            rights_of(FILE_READ_DATA | SYNCHRONIZE, true, 0).read,
            "a folder lists"
        );
    }

    #[test]
    fn our_own_file_has_an_owner_a_list_and_full_rights() {
        let dir = crate::test_support::scratch();
        let path = dir.path().join("mine.txt");
        std::fs::write(&path, b"x").expect("a test file");
        let facts = access_facts(&path).expect("facts of a file of ours");
        assert_eq!(facts.source, AccessSource::Local);
        assert_eq!(facts.kind, Some(EntryKind::File));
        let you = facts.you_can.expect("an Authz check");
        assert!(you.read && you.write, "{you:?}");
        let windows = facts.windows.expect("Windows facts");
        let owner = windows.owner.expect("an owner");
        assert!(owner.sid.starts_with("S-1-"), "{}", owner.sid);
        let WindowsList::Entries(entries) = windows.list else {
            panic!("a list on a test folder: {:?}", windows.list);
        };
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.who.sid.starts_with("S-1-")));
        let folder = access_facts(dir.path()).expect("facts of a folder");
        assert_eq!(folder.kind, Some(EntryKind::Folder));
        assert!(
            folder
                .you_can
                .is_some_and(|you| you.read && you.write && you.run)
        );
    }
}
