//! The access details view in words (alpha.7; R29 4.2, R11 6.1, 6.7 and
//! 8): the rows a provider's [`AccessFacts`] read as, meaning first, each
//! within 40 cells where the names allow.
//!
//! Unix: "You can: read and write", "Owner can read and write (ada)",
//! "Group can read (staff)", "Others can read", "Special: runs as its
//! owner", "Mode: 644, rw-r--r--", then the Linux access list ("Access
//! list: 2 extra entries", one row per named user or group, "Mask limits
//! them to read"), the flags ("Locked: yes, nobody can change it") and
//! the SELinux label ("Label: user_home_t (SELinux)"). Windows: "You can:
//! read and write", "Owner: BUILTIN\Administrators", then one row per
//! access-list entry in the order Windows checks them: "Users can read
//! and run (inherited)", "Administrators have full control", "Denied:
//! Guests cannot write (inherited)".
//!
//! "You can" is advice, not a promise (a file another program holds, or a
//! disk that turns read only, still stops a write): the help text says so
//! ([`YOU_CAN_HELP`]), never the row.

use diroxide_core::{DetailGroup, DetailRow};
use diroxide_vfs::access::{
    AccessEntry, AccessFacts, AccessSource, AclEntry, AclTag, EntryEffect, FileFlags, ListUnread,
    PosixAcl, Rights, WindowsAccess, WindowsList, inherit,
};

/// The help text of the "You can" row: what it is, and what it is not.
pub const YOU_CAN_HELP: &str = "You can: what this account may do, as the system answers now. \
     It is advice: another program using the file, or a disk turning read only, can still \
     stop a change.";

/// The help text of the access list's rows on Linux: the mask surprise.
pub const MASK_HELP: &str = "With an access list, the group digit of the mode is the mask. \
     Changing the group's permissions changes the mask, which limits every named user and \
     group too.";

/// The row of a place that keeps no permissions.
pub const NO_PERMISSIONS: &str = "This place has no permissions";

/// The row of a link.
pub const LINK_HAS_NONE: &str = "Permissions: a link has none of its own";

/// A right's or a class's words on a file or a folder (R11 6.1): "read
/// and write"; on a folder "list, enter and change contents"; "no
/// access".
pub fn rights_phrase(rights: Rights, folder: bool) -> &'static str {
    match (rights.bits(), folder) {
        (0o4, false) => "read",
        (0o6, false) => "read and write",
        (0o5, false) => "read and run",
        (0o7, false) => "read, write and run",
        (0o1, false) => "run only",
        (0o3, false) => "write and run",
        (0o2, false) => "write only",
        (0o4, true) => "list names only",
        (0o6, true) => "list names only (changing needs enter)",
        (0o5, true) => "list and enter",
        (0o7, true) => "list, enter and change contents",
        (0o1, true) => "enter only, not list",
        (0o3, true) => "enter and change contents, not list",
        (0o2, true) => "no usable access (changing needs enter)",
        _ => "no access",
    }
}

/// "Owner can read and write (ada)"; "Others have no access".
fn class_line(who: &str, plural: bool, rights: Rights, folder: bool, name: Option<&str>) -> String {
    let line = if rights.is_none() {
        format!("{who} {} no access", if plural { "have" } else { "has" })
    } else {
        format!("{who} can {}", rights_phrase(rights, folder))
    };
    match name {
        Some(name) => format!("{line} ({name})"),
        None => line,
    }
}

fn row(key: &str, label: &str, value: impl Into<String>, group: DetailGroup) -> DetailRow {
    DetailRow::new(key, label, value, group)
}

/// A sentence row: no label, the whole line its value.
fn line(key: &str, value: impl Into<String>, group: DetailGroup) -> DetailRow {
    DetailRow::new(key, "", value, group)
}

/// The "You can" row: "You can: read and write"; "You have no access".
pub fn you_can_row(rights: Rights, folder: bool) -> DetailRow {
    if rights.is_none() {
        line("access.you-can", "You have no access", DetailGroup::Flags)
    } else {
        row(
            "access.you-can",
            "You can",
            rights_phrase(rights, folder),
            DetailGroup::Flags,
        )
    }
}

/// The special bits in words: "runs as its owner"; on a folder "new
/// items get its group", "only owners delete inside".
pub fn special_words(mode: u32, folder: bool) -> Vec<&'static str> {
    let mut words = Vec::new();
    if mode & 0o4000 != 0 {
        words.push(if folder {
            "the setuid mark (no effect)"
        } else {
            "runs as its owner"
        });
    }
    if mode & 0o2000 != 0 {
        words.push(if folder {
            "new items get its group"
        } else {
            "runs as its group"
        });
    }
    if mode & 0o1000 != 0 {
        words.push(if folder {
            "only owners delete inside"
        } else {
            "the sticky mark (no effect)"
        });
    }
    words
}

/// The rows of the access details view for one entry's facts, in the
/// order said: who you are first, the owner, the group and the others,
/// the special bits, the mode, the access lists, the flags, the labels.
/// Windows facts give the owner and the access list's entries instead
/// of the mode.
pub fn access_rows(facts: &AccessFacts) -> Vec<DetailRow> {
    let mut rows = Vec::new();
    match (facts.source, facts.kind) {
        (AccessSource::None, _) => {
            rows.push(line("access.none", NO_PERMISSIONS, DetailGroup::Flags));
            return rows;
        }
        (_, Some(diroxide_core::EntryKind::Link)) => {
            rows.push(line("access.link", LINK_HAS_NONE, DetailGroup::Flags));
            return rows;
        }
        _ => {}
    }
    let folder = facts.is_folder();
    if let Some(you) = facts.you_can {
        rows.push(you_can_row(you, folder));
    }
    if let Some(windows) = &facts.windows {
        windows_rows(windows, folder, &mut rows);
        return rows;
    }
    if let Some(mode) = facts.mode {
        unix_rows(facts, mode, folder, &mut rows);
    }
    match facts.list_unread {
        Some(ListUnread::NotOnThisSystem) => rows.push(row(
            "access.list",
            "Access list",
            "not read on this system",
            DetailGroup::Flags,
        )),
        Some(ListUnread::Damaged) => rows.push(row(
            "access.list",
            "Access list",
            "could not be read",
            DetailGroup::Flags,
        )),
        _ => {}
    }
    if let Some(flags) = &facts.flags {
        flag_rows(flags, &mut rows);
    }
    if let Some(label) = &facts.label {
        rows.push(row(
            "access.label",
            "Label",
            label_words(label),
            DetailGroup::System,
        ));
    }
    for (name, value) in &facts.security {
        rows.push(row(
            &format!("access.security.{name}"),
            name,
            value.clone(),
            DetailGroup::System,
        ));
    }
    rows
}

/// The owner, group and others rows, the special bits, the mode, and the
/// Linux access lists.
fn unix_rows(facts: &AccessFacts, mode: u32, folder: bool, rows: &mut Vec<DetailRow>) {
    let list = facts.access_list.as_ref();
    let owner = list
        .and_then(|list| list.find(AclTag::Owner))
        .map_or(Rights::from_bits(mode >> 6 & 0o7), |entry| entry.rights);
    // With a mask, the mode's group digit is the mask; the owning group's
    // own rights are its entry under the mask.
    let group = match list.and_then(|list| list.find(AclTag::OwningGroup).map(|e| (list, e))) {
        Some((list, entry)) => list.effective(entry),
        None => Rights::from_bits(mode >> 3 & 0o7),
    };
    let others = Rights::from_bits(mode & 0o7);
    let flags = DetailGroup::Flags;
    rows.push(line(
        "access.owner",
        class_line("Owner", false, owner, folder, facts.owner.as_deref()),
        flags,
    ));
    rows.push(line(
        "access.group",
        class_line("Group", false, group, folder, facts.group.as_deref()),
        flags,
    ));
    rows.push(line(
        "access.others",
        class_line("Others", true, others, folder, None),
        flags,
    ));
    let special = special_words(mode, folder);
    if !special.is_empty() {
        rows.push(row("access.special", "Special", special.join(", "), flags));
    }
    let attributes = diroxide_core::Attributes::Unix(mode);
    rows.push(row(
        "access.mode",
        "Mode",
        format!("{}, {}", attributes.octal(), attributes.describe()),
        flags,
    ));
    if let Some(list) = list {
        list_rows(list, folder, false, rows);
    }
    if let Some(list) = &facts.default_list {
        list_rows(list, true, true, rows);
    }
}

/// "ada", "Ada Example (ada)", or the number when no name is known.
fn named(entry: &AclEntry) -> String {
    let name = entry
        .name
        .clone()
        .or_else(|| entry.id.map(|id| id.to_string()))
        .unwrap_or_default();
    match &entry.full_name {
        Some(full) => format!("{full} ({name})"),
        None => name,
    }
}

/// A named entry's line: "User ada can read and write", "Ada Example
/// (ada) can read and write", "Group dev can read", "User 1001 has no
/// access". The rights as stored; the mask row says what it takes away.
pub fn acl_entry_line(entry: &AclEntry, folder: bool) -> String {
    let who = match (entry.tag, &entry.full_name) {
        (AclTag::User, Some(_)) => named(entry),
        (AclTag::User, None) => format!("User {}", named(entry)),
        (AclTag::Group, _) => format!("Group {}", named(entry)),
        (AclTag::Owner, _) => "Owner".to_owned(),
        (AclTag::OwningGroup, _) => "Group".to_owned(),
        (AclTag::Mask, _) => "Mask".to_owned(),
        (AclTag::Other, _) => "Others".to_owned(),
    };
    class_line(&who, entry.tag == AclTag::Other, entry.rights, folder, None)
}

/// "Access list: 2 extra entries" and a row per named user and group,
/// the mask's limit where it takes something away, and the note on the
/// mode's group bits; the default list's rows start "Default".
fn list_rows(list: &PosixAcl, folder: bool, default: bool, rows: &mut Vec<DetailRow>) {
    let flags = DetailGroup::Flags;
    let named: Vec<&AclEntry> = list.named().collect();
    let count = diroxide_core::format::count(named.len() as u64, "extra entry", "extra entries");
    let (key, label) = if default {
        ("access.default-list", "Default access list")
    } else {
        ("access.list", "Access list")
    };
    rows.push(row(key, label, count, flags));
    for (index, entry) in named.iter().enumerate() {
        let text = acl_entry_line(entry, folder);
        let text = if default {
            format!("Default: {text}")
        } else {
            text
        };
        let mut entry_row = line(&format!("{key}.{index}"), text, flags);
        if entry.full_name.is_some() {
            entry_row = entry_row.private();
        }
        rows.push(entry_row);
    }
    if let Some(mask) = list.mask() {
        let limited = list
            .entries
            .iter()
            .filter(|e| matches!(e.tag, AclTag::User | AclTag::OwningGroup | AclTag::Group))
            .any(|e| list.effective(e) != e.rights);
        if limited {
            let text = mask_line(mask, folder);
            let text = if default {
                format!("Default: {text}")
            } else {
                text
            };
            rows.push(line(&format!("{key}.mask"), text, flags));
        }
        if !default {
            rows.push(line("access.mask-note", MASK_NOTE, flags));
        }
    }
}

/// The note on the mode's group digit when an access list has a mask.
pub const MASK_NOTE: &str = "Mode's group bits cap named users";

/// "Mask limits them to read"; "Mask leaves them no access".
pub fn mask_line(mask: Rights, folder: bool) -> String {
    if mask.is_none() {
        "Mask leaves them no access".to_owned()
    } else {
        format!("Mask limits them to {}", rights_phrase(mask, folder))
    }
}

/// The flag rows: "Locked: yes, nobody can change it" (or "Locked: no"),
/// "Append only: yes, it can only grow", and the rest in one row.
fn flag_rows(flags: &FileFlags, rows: &mut Vec<DetailRow>) {
    let group = DetailGroup::Flags;
    rows.push(row(
        "access.locked",
        "Locked",
        if flags.locked {
            "yes, nobody can change it"
        } else {
            "no"
        },
        group,
    ));
    if flags.append_only {
        rows.push(row(
            "access.append-only",
            "Append only",
            "yes, it can only grow",
            group,
        ));
    }
    let mut others = Vec::new();
    if flags.no_backup {
        others.push("no backup");
    }
    if flags.compressed {
        others.push("compressed");
    }
    if flags.encrypted {
        others.push("encrypted");
    }
    if flags.no_copy_on_write {
        others.push("no copy on write");
    }
    if !others.is_empty() {
        rows.push(row("access.flags", "Flags", others.join(", "), group));
    }
}

/// The SELinux label by its type, the part that matters: "user_home_t
/// (SELinux)"; a label without four parts whole.
pub fn label_words(label: &str) -> String {
    let parts: Vec<&str> = label.split(':').collect();
    match parts.get(2) {
        Some(kind) if parts.len() >= 3 && !kind.is_empty() => format!("{kind} (SELinux)"),
        _ => format!("{label} (SELinux)"),
    }
}

// Windows.

const FILE_READ_DATA: u32 = 0x1;
const FILE_WRITE_DATA: u32 = 0x2;
const FILE_APPEND_DATA: u32 = 0x4;
const FILE_READ_EA: u32 = 0x8;
const FILE_WRITE_EA: u32 = 0x10;
const FILE_EXECUTE: u32 = 0x20;
const FILE_DELETE_CHILD: u32 = 0x40;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
const DELETE: u32 = 0x1_0000;
const READ_CONTROL: u32 = 0x2_0000;
const WRITE_DAC: u32 = 0x4_0000;
const WRITE_OWNER: u32 = 0x8_0000;
const SYNCHRONIZE: u32 = 0x10_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_EXECUTE: u32 = 0x2000_0000;
const GENERIC_ALL: u32 = 0x1000_0000;

/// Full control (`FILE_ALL_ACCESS`, icacls `F`).
pub const FULL_CONTROL: u32 = 0x1F_01FF;
/// The read bundle (`FILE_GENERIC_READ`, icacls `R`).
const READ: u32 = FILE_READ_DATA | FILE_READ_EA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE;
/// The write bundle (icacls `W`).
const WRITE: u32 =
    FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE;
/// The run bundle (`FILE_GENERIC_EXECUTE`).
const RUN: u32 = FILE_EXECUTE | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE;

/// Generic rights as a file's own (the file generic mapping).
pub fn map_generic(mask: u32) -> u32 {
    let mut mapped = mask & !(GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE | GENERIC_ALL);
    if mask & GENERIC_READ != 0 {
        mapped |= READ;
    }
    if mask & GENERIC_WRITE != 0 {
        mapped |= WRITE | READ_CONTROL;
    }
    if mask & GENERIC_EXECUTE != 0 {
        mapped |= RUN;
    }
    if mask & GENERIC_ALL != 0 {
        mapped |= FULL_CONTROL;
    }
    mapped
}

/// A Windows rights mask in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RightsWords {
    /// Full control.
    Full,
    /// Bundles Explorer has boxes for: "read and run", "read, write, run
    /// and delete".
    Bundles(String),
    /// Anything else: the rights one by one.
    Special(String),
}

/// A mask as Explorer's bundles (read, write, run, delete; full control),
/// or the rights one by one. Read permissions and synchronize, which
/// every bundle carries, are not said on their own when a bundle is
/// there.
pub fn windows_rights(mask: u32) -> RightsWords {
    let mask = map_generic(mask);
    if mask & FULL_CONTROL == FULL_CONTROL {
        return RightsWords::Full;
    }
    let bundles = [
        (READ, "read"),
        (WRITE, "write"),
        (RUN, "run"),
        (DELETE, "delete"),
    ];
    let mut covered = 0u32;
    let mut words = Vec::new();
    for (bits, word) in bundles {
        // Explorer's bundles carry synchronize; an entry made without it
        // still reads as the bundle.
        if (mask | SYNCHRONIZE) & bits == bits {
            covered |= bits;
            words.push(word);
        }
    }
    let rest = mask & !covered & !(SYNCHRONIZE | READ_CONTROL);
    if !words.is_empty() && rest == 0 {
        return RightsWords::Bundles(and_list(&words));
    }
    RightsWords::Special(special_rights(mask))
}

/// Each right in words, for a mask that is no bundle.
fn special_rights(mask: u32) -> String {
    let names = [
        (FILE_READ_DATA, "read data"),
        (FILE_WRITE_DATA, "write data"),
        (FILE_APPEND_DATA, "append data"),
        (FILE_READ_EA, "read extended attributes"),
        (FILE_WRITE_EA, "write extended attributes"),
        (FILE_EXECUTE, "run"),
        (FILE_DELETE_CHILD, "delete inside"),
        (FILE_READ_ATTRIBUTES, "read attributes"),
        (FILE_WRITE_ATTRIBUTES, "write attributes"),
        (DELETE, "delete"),
        (READ_CONTROL, "read permissions"),
        (WRITE_DAC, "change permissions"),
        (WRITE_OWNER, "take ownership"),
    ];
    let known: u32 = names
        .iter()
        .map(|(bit, _)| bit)
        .fold(SYNCHRONIZE, |a, b| a | b);
    let mut words: Vec<String> = names
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect();
    let other = mask & !known;
    if other != 0 {
        words.push(format!("other rights {other:#x}"));
    }
    if words.is_empty() {
        "nothing".to_owned()
    } else {
        words.join(", ")
    }
}

fn and_list(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Where a folder's entry applies, when not the usual "this folder,
/// subfolders and files": "this folder only", "subfolders and files
/// only". A file's entries apply to the file; one meant only for what
/// is inside "does not apply here".
pub fn applies_to(flags: u8, folder: bool) -> Option<&'static str> {
    let files = flags & inherit::FILES != 0;
    let folders = flags & inherit::FOLDERS != 0;
    let inside_only = flags & inherit::INSIDE_ONLY != 0;
    let one_level = flags & inherit::ONE_LEVEL != 0;
    if !folder {
        return inside_only.then_some("does not apply here");
    }
    Some(match (inside_only, files, folders, one_level) {
        (false, true, true, false) => return None,
        (false, true, true, true) => "this folder and one level down",
        (false, false, false, _) => "this folder only",
        (false, false, true, false) => "this folder and subfolders",
        (false, false, true, true) => "this folder and its subfolders only",
        (false, true, false, false) => "this folder and files",
        (false, true, false, true) => "this folder and its files only",
        (true, true, true, false) => "subfolders and files only",
        (true, true, true, true) => "its subfolders and files only",
        (true, false, true, false) => "subfolders only",
        (true, false, true, true) => "its subfolders only",
        (true, true, false, false) => "files only",
        (true, true, false, true) => "its files only",
        (true, false, false, _) => "nothing",
    })
}

/// A folder Windows names as an entry's source, without a trailing
/// separator: "D:\data".
fn folder_words(folder: &str) -> &str {
    let trimmed = folder.trim_end_matches('\\');
    if trimmed.ends_with(':') {
        // A drive's top keeps its separator: "D:\".
        &folder[..trimmed.len() + 1]
    } else {
        trimmed
    }
}

/// One access-list entry in words, who first: "Users can read and run
/// (inherited)", "Administrators have full control", "Denied: Guests
/// cannot write (inherited from D:\data)", "Users have special
/// permissions: read data, write attributes".
pub fn windows_entry_line(entry: &AccessEntry, folder: bool) -> String {
    let who = if entry.who.name.is_none() && entry.who.sid.is_empty() {
        "Someone unknown"
    } else {
        entry.who.short_name()
    };
    let plural = entry.who.group && who.ends_with(['s', 'S']);
    let has = if plural { "have" } else { "has" };
    let rights = windows_rights(entry.mask);
    let mut text = match (entry.effect, &rights) {
        (EntryEffect::Allow, RightsWords::Full) => format!("{who} {has} full control"),
        (EntryEffect::Allow, RightsWords::Bundles(words)) => format!("{who} can {words}"),
        (EntryEffect::Allow, RightsWords::Special(words)) => {
            format!("{who} {has} special permissions: {words}")
        }
        (EntryEffect::Deny, RightsWords::Full) => format!("Denied: {who} cannot do anything"),
        (EntryEffect::Deny, RightsWords::Bundles(words) | RightsWords::Special(words)) => {
            format!("Denied: {who} cannot {words}")
        }
        (EntryEffect::Other(kind), _) => {
            let what = match kind {
                9 | 10 => "A conditional entry",
                5..=8 | 11..=16 => "An object entry",
                _ => "An entry of another kind",
            };
            format!("{what} for {who}, type {kind}")
        }
        _ => format!("An entry of another kind for {who}"),
    };
    let mut notes: Vec<String> = Vec::new();
    if let Some(scope) = applies_to(entry.flags, folder) {
        notes.push(scope.to_owned());
    }
    if entry.inherited() {
        notes.push(match &entry.inherited_from {
            Some(source) => format!("inherited from {}", folder_words(source)),
            None => "inherited".to_owned(),
        });
    }
    if !notes.is_empty() {
        text.push_str(&format!(" ({})", notes.join(", ")));
    }
    text
}

/// The entries as Explorer shows them: on a folder, Windows often keeps
/// one grant as two entries, one for "this folder only" and one for
/// "subfolders and files only" (an inherited Full control, or a Users
/// grant with its generic rights for what is inside). Spike AS1 found
/// every inherited entry of a folder split so. Such a pair, for the same
/// account with the same rights and the same source, and nothing of the
/// other kind between them, reads as one entry for "this folder,
/// subfolders and files", at the first one's place.
pub fn merged_entries(entries: &[AccessEntry], folder: bool) -> Vec<AccessEntry> {
    use inherit::{FILES, FOLDERS, INHERITED, INSIDE_ONLY, ONE_LEVEL};
    let both = FILES | FOLDERS;
    let mut used = vec![false; entries.len()];
    let mut out = Vec::with_capacity(entries.len());
    for (i, first) in entries.iter().enumerate() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let here_only = first.flags & (both | INSIDE_ONLY) == 0;
        let inside_only = first.flags & INSIDE_ONLY != 0 && first.flags & both == both;
        if !folder || !(here_only || inside_only) {
            out.push(first.clone());
            continue;
        }
        let partner = entries
            .iter()
            .enumerate()
            .skip(i + 1)
            .take_while(|(_, between)| between.effect == first.effect)
            .find(|(j, other)| {
                let other_here = other.flags & (both | INSIDE_ONLY) == 0;
                let other_inside = other.flags & INSIDE_ONLY != 0 && other.flags & both == both;
                !used[*j]
                    && other.who.sid == first.who.sid
                    && other.who.name == first.who.name
                    && map_generic(other.mask) == map_generic(first.mask)
                    && other.inherited() == first.inherited()
                    && other.inherited_from == first.inherited_from
                    && ((here_only && other_inside) || (inside_only && other_here))
            })
            .map(|(j, other)| (j, other.flags));
        match partner {
            Some((j, flags)) => {
                used[j] = true;
                let mut one = first.clone();
                one.mask = map_generic(first.mask);
                one.flags = (first.flags & INHERITED) | both | ((first.flags | flags) & ONE_LEVEL);
                out.push(one);
            }
            None => out.push(first.clone()),
        }
    }
    out
}

/// Whether entries are in the order Windows itself writes: explicit
/// denials, explicit grants, inherited denials, inherited grants.
pub fn canonical(entries: &[AccessEntry]) -> bool {
    let rank = |entry: &AccessEntry| {
        let inherited = u8::from(entry.inherited()) * 2;
        match entry.effect {
            EntryEffect::Deny => inherited,
            _ => inherited + 1,
        }
    };
    entries
        .windows(2)
        .all(|pair| rank(&pair[0]) <= rank(&pair[1]))
}

/// The Windows rows: the owner, the entries in their order, inheritance
/// turned off, an order Windows would not write.
fn windows_rows(windows: &WindowsAccess, folder: bool, rows: &mut Vec<DetailRow>) {
    let flags = DetailGroup::Flags;
    if let Some(owner) = &windows.owner {
        let name = owner.name.clone().unwrap_or_else(|| owner.sid.clone());
        rows.push(row("access.owner", "Owner", name, DetailGroup::System));
    }
    match &windows.list {
        WindowsList::Missing => rows.push(row(
            "access.list",
            "Access list",
            "none, everyone has full control",
            flags,
        )),
        WindowsList::Unreadable => rows.push(row(
            "access.list",
            "Access list",
            "you cannot read it",
            flags,
        )),
        WindowsList::Entries(entries) if entries.is_empty() => rows.push(row(
            "access.list",
            "Access list",
            "empty, nobody has access",
            flags,
        )),
        WindowsList::Entries(entries) => {
            for (index, entry) in merged_entries(entries, folder).iter().enumerate() {
                rows.push(line(
                    &format!("access.list.{index}"),
                    windows_entry_line(entry, folder),
                    flags,
                ));
            }
            if !canonical(entries) {
                rows.push(row(
                    "access.list-order",
                    "Access list order",
                    "not the usual one",
                    flags,
                ));
            }
        }
        _ => rows.push(row(
            "access.list",
            "Access list",
            "could not be read",
            flags,
        )),
    }
    if windows.protected {
        rows.push(row(
            "access.inherits",
            "Inherits from its folder",
            "no",
            flags,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::EntryKind;
    use diroxide_core::format::within_cells;
    use diroxide_vfs::access::Account;

    fn texts(rows: &[DetailRow]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                if row.label.is_empty() {
                    row.value.clone()
                } else {
                    row.spoken()
                }
            })
            .collect()
    }

    fn local(kind: EntryKind, mode: u32) -> AccessFacts {
        let mut facts = AccessFacts::from_entry(&{
            let mut entry = diroxide_core::Entry::new("x", kind);
            entry.attributes = Some(diroxide_core::Attributes::Unix(mode));
            entry.owner = Some("ada".to_owned());
            entry.group = Some("staff".to_owned());
            entry
        });
        facts.source = AccessSource::Local;
        facts
    }

    #[test]
    fn unix_rows_say_who_can_do_what_first() {
        let mut facts = local(EntryKind::File, 0o644);
        facts.you_can = Some(Rights::from_bits(6));
        facts.flags = Some(FileFlags::default());
        let lines = texts(&access_rows(&facts));
        assert_eq!(
            lines,
            [
                "You can: read and write",
                "Owner can read and write (ada)",
                "Group can read (staff)",
                "Others can read",
                "Mode: 644, rw-r--r--",
                "Locked: no",
            ]
        );
        for (text, cells) in [
            ("You can: read and write", 23),
            ("Owner can read and write (ada)", 30),
            ("Group can read (staff)", 22),
            ("Others can read", 15),
            ("Mode: 644, rw-r--r--", 20),
        ] {
            assert_eq!(text.chars().count(), cells, "{text}");
        }
        let mut facts = local(EntryKind::File, 0o4750);
        facts.you_can = Some(Rights::NONE);
        let mut flags = FileFlags::default();
        flags.locked = true;
        flags.append_only = true;
        flags.no_backup = true;
        facts.flags = Some(flags);
        let lines = texts(&access_rows(&facts));
        assert_eq!(
            lines,
            [
                "You have no access",
                "Owner can read, write and run (ada)",
                "Group can read and run (staff)",
                "Others have no access",
                "Special: runs as its owner",
                "Mode: 4750, rwsr-x---",
                "Locked: yes, nobody can change it",
                "Append only: yes, it can only grow",
                "Flags: no backup",
            ]
        );
        assert_eq!("Locked: yes, nobody can change it".len(), 33);
        assert_eq!("Special: runs as its owner".len(), 26);
        let folder = local(EntryKind::Folder, 0o3775);
        let lines = texts(&access_rows(&folder));
        assert_eq!(lines[0], "Owner can list, enter and change contents (ada)");
        assert!(within_cells(&lines[0], "Owner can list, enter and change"));
        assert_eq!(
            lines[3],
            "Special: new items get its group, only owners delete inside"
        );
        assert!(within_cells(&lines[3], "Special: new items get its group"));
    }

    #[test]
    fn places_without_permissions_and_links_say_so() {
        let none = access_rows(&AccessFacts::none());
        assert_eq!(texts(&none), [NO_PERMISSIONS]);
        assert_eq!(NO_PERMISSIONS.len(), 29);
        let mut link = local(EntryKind::Link, 0o777);
        link.mode = None;
        assert_eq!(texts(&access_rows(&link)), [LINK_HAS_NONE]);
        assert!(LINK_HAS_NONE.len() <= 40);
        // A listing (SFTP, an archive): no "You can", numbers as given.
        let mut sftp = AccessFacts::from_entry(&{
            let mut entry = diroxide_core::Entry::new("x", EntryKind::File);
            entry.attributes = Some(diroxide_core::Attributes::Unix(0o600));
            entry.owner = Some("1000".to_owned());
            entry.group = Some("1000".to_owned());
            entry
        });
        sftp.flags = None;
        let lines = texts(&access_rows(&sftp));
        assert_eq!(lines[0], "Owner can read and write (1000)");
        assert_eq!(lines[1], "Group has no access (1000)");
        assert!(!lines.iter().any(|l| l.starts_with("You")));
    }

    fn acl(entries: &[(AclTag, u32, Option<u32>, Option<&str>)]) -> PosixAcl {
        PosixAcl {
            entries: entries
                .iter()
                .map(|&(tag, bits, id, name)| {
                    let mut entry = AclEntry::new(tag, Rights::from_bits(bits), id);
                    entry.name = name.map(str::to_owned);
                    entry
                })
                .collect(),
        }
    }

    #[test]
    fn a_linux_access_list_says_its_entries_and_the_mask() {
        // setfacl -m u:ada:rw,g:dev:r on a 640 file whose mask was then
        // narrowed to read: the mode's group digit is the mask.
        let mut facts = local(EntryKind::File, 0o640);
        facts.owner = Some("bob".to_owned());
        facts.access_list = Some(acl(&[
            (AclTag::Owner, 6, None, None),
            (AclTag::User, 6, Some(1000), Some("ada")),
            (AclTag::OwningGroup, 4, None, None),
            (AclTag::Group, 4, Some(60), Some("dev")),
            (AclTag::Mask, 4, None, None),
            (AclTag::Other, 0, None, None),
        ]));
        let lines = texts(&access_rows(&facts));
        assert_eq!(
            lines,
            [
                "Owner can read and write (bob)",
                "Group can read (staff)",
                "Others have no access",
                "Mode: 640, rw-r-----",
                "Access list: 2 extra entries",
                "User ada can read and write",
                "Group dev can read",
                "Mask limits them to read",
                MASK_NOTE,
            ]
        );
        assert_eq!("Access list: 2 extra entries".len(), 28);
        assert_eq!(MASK_NOTE.len(), 33);
        // The mask makes the owning group's own read and write a read.
        facts.access_list = Some(acl(&[
            (AclTag::Owner, 6, None, None),
            (AclTag::OwningGroup, 6, None, None),
            (AclTag::Mask, 4, None, None),
            (AclTag::Other, 0, None, None),
        ]));
        let lines = texts(&access_rows(&facts));
        assert_eq!(lines[1], "Group can read (staff)");
        assert!(lines.contains(&"Access list: no extra entries".to_owned()));
        // A full name where the account gives one, and the row is private.
        let mut ada = AclEntry::new(AclTag::User, Rights::from_bits(6), Some(1000));
        ada.name = Some("ada".to_owned());
        ada.full_name = Some("Ada Example".to_owned());
        assert_eq!(
            acl_entry_line(&ada, false),
            "Ada Example (ada) can read and write"
        );
        let unknown = AclEntry::new(AclTag::User, Rights::NONE, Some(1001));
        assert_eq!(acl_entry_line(&unknown, false), "User 1001 has no access");
        assert_eq!(mask_line(Rights::NONE, false), "Mask leaves them no access");
    }

    #[test]
    fn a_folders_default_list_and_unread_lists_are_said() {
        let mut facts = local(EntryKind::Folder, 0o755);
        facts.default_list = Some(acl(&[
            (AclTag::Owner, 7, None, None),
            (AclTag::User, 7, Some(1000), Some("ada")),
            (AclTag::OwningGroup, 5, None, None),
            (AclTag::Mask, 7, None, None),
            (AclTag::Other, 5, None, None),
        ]));
        let lines = texts(&access_rows(&facts));
        assert!(lines.contains(&"Default access list: 1 extra entry".to_owned()));
        assert!(
            lines.contains(&"Default: User ada can list, enter and change contents".to_owned())
        );
        assert!(!lines.iter().any(|l| l.contains("Mask")), "{lines:?}");
        let mut mac = local(EntryKind::File, 0o644);
        mac.list_unread = Some(ListUnread::NotOnThisSystem);
        assert!(
            texts(&access_rows(&mac)).contains(&"Access list: not read on this system".to_owned())
        );
        assert_eq!("Access list: not read on this system".len(), 36);
        mac.list_unread = Some(ListUnread::Damaged);
        assert!(texts(&access_rows(&mac)).contains(&"Access list: could not be read".to_owned()));
    }

    #[test]
    fn the_selinux_label_is_said_by_its_type() {
        let mut facts = local(EntryKind::File, 0o644);
        facts.label = Some("unconfined_u:object_r:user_home_t:s0".to_owned());
        facts.security = vec![("security.ima".to_owned(), "34 bytes".to_owned())];
        let lines = texts(&access_rows(&facts));
        assert!(lines.contains(&"Label: user_home_t (SELinux)".to_owned()));
        assert_eq!("Label: user_home_t (SELinux)".len(), 28);
        assert!(lines.contains(&"security.ima: 34 bytes".to_owned()));
        assert_eq!(label_words("odd"), "odd (SELinux)");
    }

    fn account(name: &str, group: bool) -> Account {
        Account {
            name: Some(name.to_owned()),
            sid: "S-1-5-32-545".to_owned(),
            group,
        }
    }

    fn ace(effect: EntryEffect, who: Account, mask: u32, flags: u8) -> AccessEntry {
        AccessEntry {
            effect,
            who,
            mask,
            flags,
            inherited_from: None,
        }
    }

    #[test]
    fn windows_bundles_are_explorers() {
        let table = [
            (0x1F_01FF, RightsWords::Full),
            (
                0x13_01BF,
                RightsWords::Bundles("read, write, run and delete".into()),
            ),
            (0x12_00A9, RightsWords::Bundles("read and run".into())),
            (0x12_0089, RightsWords::Bundles("read".into())),
            (0x10_0116, RightsWords::Bundles("write".into())),
            (0x12_019F, RightsWords::Bundles("read and write".into())),
            (
                0x12_01BF,
                RightsWords::Bundles("read, write and run".into()),
            ),
            (0x1_0000, RightsWords::Bundles("delete".into())),
            (GENERIC_ALL, RightsWords::Full),
            (
                GENERIC_READ | GENERIC_EXECUTE,
                RightsWords::Bundles("read and run".into()),
            ),
            (GENERIC_WRITE, RightsWords::Bundles("write".into())),
            (
                FILE_READ_DATA | FILE_WRITE_ATTRIBUTES,
                RightsWords::Special("read data, write attributes".into()),
            ),
        ];
        for (mask, words) in table {
            assert_eq!(windows_rights(mask), words, "{mask:#x}");
        }
    }

    #[test]
    fn windows_entries_say_who_first() {
        let users = account("BUILTIN\\Users", true);
        let admins = account("BUILTIN\\Administrators", true);
        let guests = account("BUILTIN\\Guests", true);
        let system = account("NT AUTHORITY\\SYSTEM", true);
        let inherited = inherit::FILES | inherit::FOLDERS | inherit::INHERITED;
        let table = [
            (
                ace(EntryEffect::Allow, users.clone(), 0x12_00A9, inherited),
                false,
                "Users can read and run (inherited)",
            ),
            (
                ace(EntryEffect::Allow, admins, FULL_CONTROL, 0),
                false,
                "Administrators have full control",
            ),
            (
                ace(EntryEffect::Deny, guests, 0x10_0116, inherited),
                false,
                "Denied: Guests cannot write (inherited)",
            ),
            (
                ace(EntryEffect::Allow, system, GENERIC_ALL, 0),
                false,
                "SYSTEM has full control",
            ),
            (
                ace(
                    EntryEffect::Allow,
                    users.clone(),
                    0x12_0089,
                    inherit::FILES | inherit::FOLDERS | inherit::INSIDE_ONLY,
                ),
                true,
                "Users can read (subfolders and files only)",
            ),
            (
                ace(EntryEffect::Allow, users.clone(), 0x12_0089, 0),
                true,
                "Users can read (this folder only)",
            ),
            (
                ace(
                    EntryEffect::Allow,
                    users.clone(),
                    0x12_0089,
                    inherit::FILES | inherit::FOLDERS,
                ),
                true,
                "Users can read",
            ),
        ];
        for (entry, folder, text) in table {
            assert_eq!(windows_entry_line(&entry, folder), text);
        }
        for (text, cells) in [
            ("Users can read and run (inherited)", 34),
            ("Administrators have full control", 32),
            ("Denied: Guests cannot write (inherited)", 39),
        ] {
            assert_eq!(text.len(), cells);
        }
        let mut from = ace(EntryEffect::Allow, users.clone(), 0x12_00A9, inherited);
        from.inherited_from = Some("D:\\data\\".to_owned());
        assert_eq!(
            windows_entry_line(&from, false),
            "Users can read and run (inherited from D:\\data)"
        );
        from.inherited_from = Some("D:\\".to_owned());
        assert!(windows_entry_line(&from, false).ends_with("(inherited from D:\\)"));
        let special = ace(
            EntryEffect::Allow,
            users,
            FILE_READ_DATA | FILE_WRITE_ATTRIBUTES,
            0,
        );
        let text = windows_entry_line(&special, false);
        assert_eq!(
            text,
            "Users have special permissions: read data, write attributes"
        );
        assert!(within_cells(&text, "Users have special permissions"));
    }

    #[test]
    fn windows_rows_give_the_owner_and_the_lists_state() {
        let mut facts = local(EntryKind::File, 0o644);
        facts.mode = None;
        facts.you_can = Some(Rights::from_bits(6));
        let users = account("BUILTIN\\Users", true);
        let inherited = inherit::INHERITED;
        let entries = vec![
            ace(EntryEffect::Allow, users.clone(), 0x12_00A9, inherited),
            ace(EntryEffect::Deny, users, 0x10_0116, inherited),
        ];
        facts.windows = Some(WindowsAccess::new(
            Some(account("BUILTIN\\Administrators", true)),
            WindowsList::Entries(entries),
            true,
        ));
        let lines = texts(&access_rows(&facts));
        assert_eq!(
            lines,
            [
                "You can: read and write",
                "Owner: BUILTIN\\Administrators",
                "Users can read and run (inherited)",
                "Denied: Users cannot write (inherited)",
                "Access list order: not the usual one",
                "Inherits from its folder: no",
            ]
        );
        assert_eq!("Owner: BUILTIN\\Administrators".len(), 29);
        for (list, text) in [
            (
                WindowsList::Missing,
                "Access list: none, everyone has full control",
            ),
            (WindowsList::Unreadable, "Access list: you cannot read it"),
            (
                WindowsList::Entries(Vec::new()),
                "Access list: empty, nobody has access",
            ),
        ] {
            facts.windows = Some(WindowsAccess::new(None, list, false));
            let lines = texts(&access_rows(&facts));
            assert_eq!(lines[1], text);
            assert!(within_cells(text, "Access list"));
        }
    }

    #[test]
    fn a_folders_split_pairs_read_as_one_entry() {
        use inherit::{FILES, FOLDERS, INHERITED, INSIDE_ONLY};
        let admins = account("BUILTIN\\Administrators", true);
        let users = account("BUILTIN\\Users", true);
        let guests = account("BUILTIN\\Guests", true);
        let inside = FILES | FOLDERS | INSIDE_ONLY | INHERITED;
        // As AS1 read a folder under D:\, in icacls's order.
        let entries = vec![
            ace(EntryEffect::Allow, admins.clone(), FULL_CONTROL, INHERITED),
            ace(EntryEffect::Allow, admins.clone(), FULL_CONTROL, inside),
            ace(EntryEffect::Allow, users.clone(), 0x12_00A9, INHERITED),
            ace(
                EntryEffect::Allow,
                users.clone(),
                GENERIC_READ | GENERIC_EXECUTE,
                inside,
            ),
        ];
        let lines: Vec<String> = merged_entries(&entries, true)
            .iter()
            .map(|e| windows_entry_line(e, true))
            .collect();
        assert_eq!(
            lines,
            [
                "Administrators have full control (inherited)",
                "Users can read and run (inherited)",
            ]
        );
        // A file's entries, and pairs with a denial between, stay apart.
        assert_eq!(merged_entries(&entries, false).len(), 4);
        let apart = vec![
            ace(EntryEffect::Allow, admins.clone(), FULL_CONTROL, 0),
            ace(EntryEffect::Deny, guests, 0x10_0116, 0),
            ace(
                EntryEffect::Allow,
                admins,
                FULL_CONTROL,
                FILES | FOLDERS | INSIDE_ONLY,
            ),
        ];
        assert_eq!(merged_entries(&apart, true).len(), 3);
        // Different rights stay two entries.
        let differ = vec![
            ace(EntryEffect::Allow, users.clone(), 0x12_0089, 0),
            ace(
                EntryEffect::Allow,
                users,
                FULL_CONTROL,
                FILES | FOLDERS | INSIDE_ONLY,
            ),
        ];
        assert_eq!(merged_entries(&differ, true).len(), 2);
    }

    #[test]
    fn where_a_folders_entry_applies_is_said_unless_usual() {
        use inherit::{FILES, FOLDERS, INSIDE_ONLY, ONE_LEVEL};
        assert_eq!(applies_to(FILES | FOLDERS, true), None);
        assert_eq!(applies_to(0, true), Some("this folder only"));
        assert_eq!(
            applies_to(FOLDERS, true),
            Some("this folder and subfolders")
        );
        assert_eq!(applies_to(FILES, true), Some("this folder and files"));
        assert_eq!(
            applies_to(FILES | FOLDERS | INSIDE_ONLY, true),
            Some("subfolders and files only")
        );
        assert_eq!(
            applies_to(FOLDERS | INSIDE_ONLY, true),
            Some("subfolders only")
        );
        assert_eq!(applies_to(FILES | INSIDE_ONLY, true), Some("files only"));
        assert_eq!(
            applies_to(FILES | FOLDERS | ONE_LEVEL, true),
            Some("this folder and one level down")
        );
        assert_eq!(applies_to(FILES | FOLDERS, false), None);
        assert_eq!(applies_to(INSIDE_ONLY, false), Some("does not apply here"));
    }
}
