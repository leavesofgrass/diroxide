//! Moving to the trash and restoring from it, behind the gate (plan
//! section 5.12; spikes four and six; ADR-0008). The `trash` crate's Ok
//! never means "in the trash": on Windows it removes a file for good, with
//! no dialog, on a removable drive and for a path of 260 units or more.
//! So the provider asks the gate before every call, runs the call on a
//! worker thread that catches a panic (the crate unwraps a COM creation),
//! and checks after it: the original is gone, and the trash holds it.
//!
//! The post-check, by platform:
//!
//! - Windows: the item's file id, read before the call, must now resolve
//!   inside the volume's `$Recycle.Bin`; an id that resolves nowhere means
//!   the item is gone for good.
//! - Linux and the other freedesktop systems: the trash's own listing must
//!   hold an item from the original path, deleted since the call began.
//! - macOS: the item's identity must be found in the home trash or the
//!   volume's `.Trashes` folder.
//!
//! The call and the check are passed in, so the tests run the whole flow
//! against a test-only bin folder and never touch the real trash.

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use crate::error::VfsError;
use crate::sys::{self, Identity};
use crate::trash::TrashCheck;

/// What is known of an item before the trash call.
#[derive(Clone, Debug)]
pub(crate) struct Before {
    /// The path as the trash is given it.
    pub(crate) path: PathBuf,
    /// Its identity, when the platform gives one.
    pub(crate) identity: Option<Identity>,
    /// How many items the trash held from this path before the call, where
    /// the trash can be listed (freedesktop): one more after the call means
    /// this one arrived. Counting needs no deletion times, which the
    /// `trash` crate gives on Linux only with its `chrono` feature.
    #[cfg_attr(
        not(all(unix, not(target_os = "macos"))),
        allow(dead_code, reason = "read by the freedesktop look only")
    )]
    pub(crate) count: Option<usize>,
}

/// The trash call: moves one item to the trash, or says why not.
pub(crate) type TrashCall<'a> = &'a (dyn Fn(&Path) -> Result<(), VfsError> + Sync);

/// Counts the items the trash holds from a path, where it can.
pub(crate) type TrashCount<'a> = &'a dyn Fn(&Path) -> Option<usize>;

/// The post-check's look into the trash: `Some(true)` when the trash holds
/// the item, `Some(false)` when it does not, `None` when it cannot tell.
pub(crate) type TrashLook<'a> = &'a dyn Fn(&Before) -> Option<bool>;

/// Whether the freedesktop trash would copy an item on the volume at
/// `root` to another drive (R8; [`crate::trash::copies_across_drives`]):
/// the volume is not the home folder's, and it has no trash of its own the
/// `trash` crate can use or make (`.Trash/UID` or `.Trash-UID` at its top,
/// or a top it may write to). The user's id is read from the home folder's
/// owner, which needs no system call beyond a stat.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn copies_across_drives(root: Option<&Path>) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Some(root), Some(home)) = (root, dirs::home_dir()) else {
        return false;
    };
    let home_root = super::volumes::facts_at(&home).root;
    let own_trash_usable = match fs::metadata(&home).map(|m| m.uid()) {
        Ok(uid) => {
            root.join(".Trash").join(uid.to_string()).is_dir()
                || root.join(format!(".Trash-{uid}")).is_dir()
                || rustix::fs::access(root, rustix::fs::Access::WRITE_OK).is_ok()
        }
        // The id is unknown: take the volume's trash as usable, so nothing
        // asks without cause.
        Err(_) => true,
    };
    crate::trash::copies_across_drives(root, home_root.as_deref(), own_trash_usable)
}

/// Whether the trash would copy an item to another drive: never on Windows
/// (the gate refuses drives without a recycle bin) or macOS (each volume
/// keeps its own `.Trashes`).
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(crate) fn copies_across_drives(root: Option<&Path>) -> bool {
    let _ = root;
    false
}

/// Runs the trash call on a worker thread; a panic inside becomes an
/// error, never a crash of the job.
fn call_on_worker(path: &Path, call: TrashCall<'_>) -> Result<(), VfsError> {
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(|| catch_unwind(AssertUnwindSafe(|| call(path))))
            .join()
    });
    match outcome {
        Ok(Ok(result)) => result,
        _ => Err(VfsError::Io {
            kind: std::io::ErrorKind::Other,
            message: "the system trash failed unexpectedly".to_owned(),
        }),
    }
}

/// Moves one item to the trash with `call`, then checks with `look`. The
/// gate has already allowed it. An item still in its place is
/// `StillThere` (or the call's own error, when it failed); an item gone is
/// what the look finds: `InTrash`, `GoneNotInTrash` (the loud failure), or
/// `NotChecked`.
pub(crate) fn trash_with(
    path: &Path,
    call: TrashCall<'_>,
    count: TrashCount<'_>,
    look: TrashLook<'_>,
) -> Result<TrashCheck, VfsError> {
    let before = Before {
        path: path.to_path_buf(),
        identity: sys::identity(path).ok(),
        count: count(path),
    };
    let called = call_on_worker(path, call);
    let still_here = match fs::symlink_metadata(path) {
        Ok(_) => before.identity.is_none() || sys::identity(path).ok() == before.identity,
        Err(_) => false,
    };
    if still_here {
        return called.map(|()| TrashCheck::StillThere);
    }
    Ok(match look(&before) {
        Some(true) => TrashCheck::InTrash,
        Some(false) => TrashCheck::GoneNotInTrash,
        None => TrashCheck::NotChecked,
    })
}

/// The `trash` crate's errors in diroxide's words; the crate's own text is
/// never shown.
fn from_trash_error(error: ::trash::Error) -> VfsError {
    const ACCESS_DENIED: i32 = 0x8007_0005_u32 as i32;
    const SHARING_VIOLATION: i32 = 0x8007_0020_u32 as i32;
    match error {
        ::trash::Error::TargetedRoot => VfsError::Unsafe {
            why: "it is the top of a drive".to_owned(),
        },
        ::trash::Error::CouldNotAccess { .. } => VfsError::PermissionDenied,
        ::trash::Error::CanonicalizePath { .. } => VfsError::NotFound,
        ::trash::Error::ConvertOsString { .. } => VfsError::NameNotUnicode,
        ::trash::Error::RestoreCollision { .. } => VfsError::AlreadyExists,
        ::trash::Error::Os { code, .. } if code == ACCESS_DENIED => VfsError::PermissionDenied,
        ::trash::Error::Os { code, .. } if code == SHARING_VIOLATION => VfsError::InUse,
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        ::trash::Error::FileSystem { source, .. } => source.into(),
        _ => VfsError::Io {
            kind: std::io::ErrorKind::Other,
            message: "the system trash refused it".to_owned(),
        },
    }
}

/// The real trash call.
pub(crate) fn system_trash(path: &Path) -> Result<(), VfsError> {
    ::trash::delete(path).map_err(from_trash_error)
}

/// The real count of a path's items in the trash: the freedesktop trash
/// is listed; elsewhere the look needs no count.
pub(crate) fn system_count(path: &Path) -> Option<usize> {
    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    {
        let items = ::trash::os_limited::list().ok()?;
        Some(
            items
                .iter()
                .filter(|item| item.original_path() == path)
                .count(),
        )
    }
    #[cfg(not(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )))]
    {
        let _ = path;
        None
    }
}

/// The real look into the trash, by platform.
pub(crate) fn system_look(before: &Before) -> Option<bool> {
    #[cfg(windows)]
    {
        let identity = before.identity?;
        let folder = before.path.parent()?;
        sys::in_recycle_bin(folder, identity)
    }
    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    {
        let before_count = before.count?;
        let after_count = system_count(&before.path)?;
        Some(after_count > before_count)
    }
    #[cfg(target_os = "macos")]
    {
        mac_look(before)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = before;
        None
    }
}

/// macOS: the item's identity in the home trash or in its volume's
/// `.Trashes` folder for this user.
#[cfg(target_os = "macos")]
fn mac_look(before: &Before) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;
    let identity = before.identity?;
    let home = dirs::home_dir()?;
    let uid = fs::metadata(&home).ok()?.uid();
    let mut folders = vec![home.join(".Trash")];
    if let Some(root) = crate::local::volumes::facts_at(&before.path).root {
        folders.push(root.join(".Trashes").join(uid.to_string()));
    }
    let found = folders.iter().any(|folder| {
        fs::read_dir(folder).is_ok_and(|items| {
            items
                .flatten()
                .any(|item| sys::identity(&item.path()).ok() == Some(identity))
        })
    });
    Some(found)
}

/// Restores the newest trashed item whose former path is `original`,
/// never over an entry that took its name. On a worker thread, as the
/// trash call is.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub(crate) fn restore_newest(original: &Path) -> Result<(), VfsError> {
    let original = original.to_path_buf();
    call_on_worker(&original, &|original: &Path| {
        let same = |a: &Path, b: &Path| {
            if cfg!(windows) {
                a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
            } else {
                a == b
            }
        };
        let newest = ::trash::os_limited::list()
            .map_err(from_trash_error)?
            .into_iter()
            .filter(|item| same(&item.original_path(), original))
            .max_by_key(deleted_at)
            .ok_or(VfsError::NotFound)?;
        ::trash::os_limited::restore_all([newest]).map_err(from_trash_error)
    })
}

/// When an item was trashed, for choosing the newest: the time the trash
/// recorded on Windows; on the freedesktop trash the time its
/// `.trashinfo` file was written (the item's id), since the crate gives
/// deletion times there only with its `chrono` feature.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn deleted_at(item: &::trash::TrashItem) -> i64 {
    if cfg!(windows) {
        return item.time_deleted;
    }
    fs::metadata(&item.id)
        .and_then(|metadata| metadata.modified())
        .map_or(i64::MIN, |time| {
            diroxide_core::Timestamp::from_system_time(time).unix_seconds()
        })
}

/// No restore on this platform (macOS: the Finder's Put Back).
#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
pub(crate) fn restore_newest(_original: &Path) -> Result<(), VfsError> {
    Err(VfsError::unsupported("file", "restore from the trash here"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::scratch;

    /// A test-only bin: the call moves the item into it; the look finds it
    /// there by identity.
    fn bin_in(folder: &Path) -> PathBuf {
        let bin = folder.join("test-bin");
        fs::create_dir_all(&bin).unwrap();
        bin
    }

    /// The test bin needs no count.
    fn no_count(_: &Path) -> Option<usize> {
        None
    }

    fn look_in(bin: &Path) -> impl Fn(&Before) -> Option<bool> + '_ {
        move |before: &Before| {
            let identity = before.identity?;
            Some(
                fs::read_dir(bin)
                    .unwrap()
                    .flatten()
                    .any(|item| sys::identity(&item.path()).ok() == Some(identity)),
            )
        }
    }

    #[test]
    fn an_item_moved_into_the_bin_is_confirmed() {
        let folder = scratch();
        let bin = bin_in(folder.path());
        let item = folder.path().join("old.txt");
        fs::write(&item, b"o").unwrap();
        let call =
            |path: &Path| -> Result<(), VfsError> { Ok(fs::rename(path, bin.join("old.txt"))?) };
        let check = trash_with(&item, &call, &no_count, &look_in(&bin)).unwrap();
        assert_eq!(check, TrashCheck::InTrash);
        assert!(!item.exists());
    }

    #[test]
    fn an_ok_that_removed_for_good_is_the_loud_failure() {
        // The crate's behavior on a USB stick and past 259 units (spike
        // six): Ok, and the file gone for good.
        let folder = scratch();
        let bin = bin_in(folder.path());
        let item = folder.path().join("lost.txt");
        fs::write(&item, b"l").unwrap();
        let call = |path: &Path| -> Result<(), VfsError> { Ok(fs::remove_file(path)?) };
        let check = trash_with(&item, &call, &no_count, &look_in(&bin)).unwrap();
        assert_eq!(check, TrashCheck::GoneNotInTrash);
        assert_eq!(check.problem(), Some("Deleted but not found in the trash."));
    }

    #[test]
    fn an_ok_that_did_nothing_is_still_there() {
        let folder = scratch();
        let bin = bin_in(folder.path());
        let item = folder.path().join("stays.txt");
        fs::write(&item, b"s").unwrap();
        let call = |_: &Path| -> Result<(), VfsError> { Ok(()) };
        assert_eq!(
            trash_with(&item, &call, &no_count, &look_in(&bin)).unwrap(),
            TrashCheck::StillThere
        );
        let refused = |_: &Path| -> Result<(), VfsError> { Err(VfsError::InUse) };
        assert_eq!(
            trash_with(&item, &refused, &no_count, &look_in(&bin)),
            Err(VfsError::InUse)
        );
    }

    #[test]
    fn a_panic_in_the_call_is_an_error_and_the_item_stays() {
        let folder = scratch();
        let bin = bin_in(folder.path());
        let item = folder.path().join("calm.txt");
        fs::write(&item, b"c").unwrap();
        let call = |_: &Path| -> Result<(), VfsError> { panic!("a COM creation failed") };
        let result = trash_with(&item, &call, &no_count, &look_in(&bin));
        assert!(matches!(result, Err(VfsError::Io { .. })));
        assert!(item.exists());
    }

    #[test]
    fn a_look_that_cannot_tell_is_not_checked() {
        let folder = scratch();
        let item = folder.path().join("unknown.txt");
        fs::write(&item, b"u").unwrap();
        let call = |path: &Path| -> Result<(), VfsError> { Ok(fs::remove_file(path)?) };
        let blind = |_: &Before| None;
        assert_eq!(
            trash_with(&item, &call, &no_count, &blind).unwrap(),
            TrashCheck::NotChecked
        );
    }

    #[test]
    fn crate_errors_read_in_words() {
        assert!(matches!(
            from_trash_error(::trash::Error::TargetedRoot),
            VfsError::Unsafe { .. }
        ));
        let text = from_trash_error(::trash::Error::Unknown {
            description: "Error during a `trash` operation: Some operations were aborted".into(),
        })
        .to_string();
        assert_eq!(text, "the system trash refused it");
        assert_eq!(
            from_trash_error(::trash::Error::Os {
                code: 0x8007_0020_u32 as i32,
                description: String::new(),
            }),
            VfsError::InUse
        );
    }
}
