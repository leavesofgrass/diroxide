//! Changing attributes, times and owners, and answering the inverse
//! (research/permissions.md 9.1 to 9.4): the change is refused whole
//! before anything is touched when part of it cannot be done here, the
//! owner goes first, the times next and the permissions last (so a mode
//! that takes away read or write never blocks the other steps, and a
//! change of owner never clears a set-id bit asked for), and a failure
//! part way puts back what was already changed.
//!
//! The Unix mode is written whole, all twelve bits; `set_readonly` is
//! never used. A link's own permissions are not changed (on Unix they are
//! its target's), its times and owner are.

use std::fs::{self, Metadata};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diroxide_core::{Attributes, Timestamp};

use crate::error::VfsError;
use crate::provider::{AttributeChange, Principal};
use crate::sys;

/// The Windows bits a change can set, less the archive bit, which backup
/// programs clear and writes set: the bits compared by `expect`.
const WINDOWS_COMPARED: u32 = (0x1 | 0x2 | 0x4 | 0x100 | 0x1000 | 0x2000) & !0x20;

/// A timestamp as the platform's time.
pub(crate) fn system_time(stamp: Timestamp) -> SystemTime {
    let secs = Duration::from_secs(stamp.unix_seconds().unsigned_abs());
    let base = if stamp.unix_seconds() >= 0 {
        UNIX_EPOCH.checked_add(secs)
    } else {
        UNIX_EPOCH.checked_sub(secs)
    };
    base.and_then(|time| time.checked_add(Duration::from_nanos(u64::from(stamp.subsec_nanos()))))
        .unwrap_or(UNIX_EPOCH)
}

/// The attributes metadata holds, in the platform's form.
pub(crate) fn current_attributes(metadata: &Metadata) -> Attributes {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Attributes::Windows(metadata.file_attributes())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Attributes::Unix(metadata.mode() & 0o7777)
    }
}

/// Whether two attribute values agree for `expect`: the twelve mode bits,
/// or the settable Windows bits other than archive.
pub(crate) fn same_attributes(a: Attributes, b: Attributes) -> bool {
    match (a, b) {
        (Attributes::Unix(a), Attributes::Unix(b)) => a & 0o7777 == b & 0o7777,
        (Attributes::Windows(a), Attributes::Windows(b)) => {
            a & WINDOWS_COMPARED == b & WINDOWS_COMPARED
        }
        _ => false,
    }
}

/// Refuses, before anything changes, a change that cannot be made here.
fn check_possible(change: &AttributeChange, is_link: bool) -> Result<(), VfsError> {
    match change.attributes {
        Some(Attributes::Unix(_)) if cfg!(windows) => {
            return Err(VfsError::unsupported(
                "file",
                "set Unix permissions on Windows",
            ));
        }
        Some(Attributes::Windows(_)) if !cfg!(windows) => {
            return Err(VfsError::unsupported("file", "set Windows attributes here"));
        }
        Some(Attributes::Unix(_)) if is_link => {
            return Err(VfsError::Unsafe {
                why: "a link's permissions are its target's".to_owned(),
            });
        }
        _ => {}
    }
    if change.created.is_some() {
        if !cfg!(any(windows, target_os = "macos")) {
            return Err(VfsError::unsupported(
                "file",
                "set the created time on this system",
            ));
        }
        if is_link && !cfg!(windows) {
            return Err(VfsError::unsupported(
                "file",
                "set the created time of a link",
            ));
        }
    }
    if cfg!(windows) && (change.owner.is_some() || change.group.is_some()) {
        return Err(VfsError::unsupported(
            "file",
            "change the owner on Windows yet",
        ));
    }
    Ok(())
}

/// The number in a principal: its id, or its name looked up.
#[cfg_attr(windows, allow(dead_code, reason = "owners are Unix's"))]
fn principal_number(principal: &Principal, group: bool) -> Result<u32, VfsError> {
    if let Ok(number) = principal.id.parse::<u32>() {
        return Ok(number);
    }
    #[cfg(unix)]
    if let Some(name) = principal.name.as_deref() {
        let owners = crate::local::owners::Owners::shared();
        let found = if group {
            owners.group_number(name)
        } else {
            owners.user_number(name)
        };
        if let Some(number) = found {
            return Ok(number);
        }
    }
    let _ = group;
    Err(VfsError::NameInvalid)
}

/// The principal an entry has now, for the inverse.
#[cfg(unix)]
fn principal_of(metadata: &Metadata, group: bool) -> Principal {
    use std::os::unix::fs::MetadataExt;
    let owners = crate::local::owners::Owners::shared();
    let (id, name) = if group {
        (metadata.gid(), owners.group(metadata.gid()))
    } else {
        (metadata.uid(), owners.user(metadata.uid()))
    };
    let id = id.to_string();
    let name = (name != id).then_some(name);
    Principal { id, name }
}

/// Applies a change, step by step, recording in `inverse` the old value
/// of each field it has changed. Stops at the first failure.
fn apply(
    path: &Path,
    before: &Metadata,
    change: &AttributeChange,
    inverse: &mut AttributeChange,
) -> Result<(), VfsError> {
    #[cfg(unix)]
    if change.owner.is_some() || change.group.is_some() {
        let owner = change
            .owner
            .as_ref()
            .map(|p| principal_number(p, false))
            .transpose()?;
        let group = change
            .group
            .as_ref()
            .map(|p| principal_number(p, true))
            .transpose()?;
        sys::set_owner(path, owner, group)?;
        if owner.is_some() {
            inverse.owner = Some(principal_of(before, false));
        }
        if group.is_some() {
            inverse.group = Some(principal_of(before, true));
        }
    }
    if change.created.is_some() || change.modified.is_some() || change.accessed.is_some() {
        sys::set_times(
            path,
            change.created.map(system_time),
            change.modified.map(system_time),
            change.accessed.map(system_time),
        )?;
        let old = |time: std::io::Result<SystemTime>| time.ok().map(Timestamp::from_system_time);
        if change.created.is_some() {
            inverse.created = old(before.created());
        }
        if change.modified.is_some() {
            inverse.modified = old(before.modified());
        }
        if change.accessed.is_some() {
            inverse.accessed = old(before.accessed());
        }
    }
    if let Some(attributes) = change.attributes {
        let old = current_attributes(before);
        match attributes {
            #[cfg(unix)]
            Attributes::Unix(mode) => sys::set_mode(path, mode)?,
            #[cfg(windows)]
            Attributes::Windows(flags) => {
                let Attributes::Windows(now) = old else {
                    return Err(VfsError::unsupported("file", "set these attributes"));
                };
                let kept = now & !sys::SETTABLE_ATTRIBUTES;
                sys::set_os_attributes(path, kept | (flags & sys::SETTABLE_ATTRIBUTES))?;
            }
            _ => return Err(VfsError::unsupported("file", "set these attributes")),
        }
        inverse.attributes = Some(old);
    }
    Ok(())
}

/// Changes attributes, times and owners at `path`, and returns the
/// inverse: the old values of exactly the fields changed. With `expect`
/// set, nothing changes unless the entry still has those attributes.
pub(crate) fn set_attributes(
    path: &Path,
    change: &AttributeChange,
) -> Result<AttributeChange, VfsError> {
    let before = fs::symlink_metadata(path)?;
    let is_link = before.file_type().is_symlink();
    if let Some(expected) = change.expect
        && !same_attributes(current_attributes(&before), expected)
    {
        return Err(VfsError::Changed);
    }
    check_possible(change, is_link)?;
    let mut inverse = AttributeChange::default();
    if let Err(error) = apply(path, &before, change, &mut inverse) {
        if !inverse.is_empty()
            && let Ok(now) = fs::symlink_metadata(path)
        {
            // Put back what was changed before the failure, best effort.
            let _ = apply(path, &now, &inverse, &mut AttributeChange::default());
        }
        return Err(error);
    }
    Ok(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_and_system_times_agree() {
        // Windows keeps time in 100-nanosecond steps, so the samples do.
        for stamp in [
            Timestamp::from_unix(0, 0),
            Timestamp::from_unix(1_760_000_000, 123_456_700),
            Timestamp::from_unix(-86_400, 500),
        ] {
            assert_eq!(Timestamp::from_system_time(system_time(stamp)), stamp);
        }
    }

    #[test]
    fn expect_compares_what_a_change_can_set() {
        assert!(same_attributes(
            Attributes::Unix(0o100_644),
            Attributes::Unix(0o644)
        ));
        assert!(!same_attributes(
            Attributes::Unix(0o4755),
            Attributes::Unix(0o755)
        ));
        assert!(same_attributes(
            Attributes::Windows(0x20 | 0x1),
            Attributes::Windows(0x1)
        ));
        assert!(!same_attributes(
            Attributes::Windows(0x2),
            Attributes::Windows(0x0)
        ));
        assert!(!same_attributes(
            Attributes::Unix(0),
            Attributes::Windows(0)
        ));
    }

    #[test]
    fn impossible_changes_are_refused_before_anything_moves() {
        let created = AttributeChange {
            created: Some(Timestamp::from_unix(1, 0)),
            ..AttributeChange::default()
        };
        assert_eq!(
            check_possible(&created, false).is_ok(),
            cfg!(any(windows, target_os = "macos"))
        );
        let other = if cfg!(windows) {
            Attributes::Unix(0o644)
        } else {
            Attributes::Windows(0x1)
        };
        let wrong = AttributeChange {
            attributes: Some(other),
            ..AttributeChange::default()
        };
        assert!(matches!(
            check_possible(&wrong, false),
            Err(VfsError::Unsupported { .. })
        ));
    }
}
