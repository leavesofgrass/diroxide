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
//!
//! The trash as a list (R25 3.3, commit 85; answer 98) is here too:
//! [`SystemTrash`] lists the trash newest first, restores an item where it
//! was (never over a taken name) and deletes an item for good, through the
//! trash crate's `os_limited` calls on Windows and the freedesktop
//! systems, each on a worker thread that catches a panic and gives up
//! after a limit. On macOS the trash cannot be listed, and every call says
//! so ([`TRASH_NOT_LISTED`]). The calls sit behind a small backend, so the
//! tests run the real listing, checks and words against a test-only bin.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use diroxide_core::clock::describe_time;
use diroxide_core::format::{count, human_size};
use diroxide_core::{Clock, Place, Timestamp, names};

use crate::cancel::OpContext;
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
            Timestamp::from_system_time(time).unix_seconds()
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

// ----- the trash as a list: listing, restore and purge (R25 3.3) -----

/// The sentence where the trash cannot be listed (macOS: the Finder keeps
/// its own records), said whole by the trash browser (answer 98).
pub const TRASH_NOT_LISTED: &str = "The trash cannot be listed on this system.";

/// How long a listing of the trash may take before it is given up: a
/// freedesktop trash on a network mount that never answers never holds
/// the browser.
pub const TRASH_LIST_LIMIT: Duration = Duration::from_secs(30);

/// How many items a listing hands over at once, so the first rows can be
/// shown while the rest are read.
pub const TRASH_BATCH: usize = 500;

/// The trash's own id of one item: on Windows the shell's parsing name of
/// the item in the recycle bin, on the freedesktop systems the path of its
/// `.trashinfo` file. Opaque: it is only handed back.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrashId(OsString);

impl TrashId {
    /// An id as the trash gave it.
    pub fn new(id: impl Into<OsString>) -> Self {
        Self(id.into())
    }

    /// The id as the trash gave it.
    pub fn as_os_str(&self) -> &std::ffi::OsStr {
        &self.0
    }
}

/// The trash crate's own fields of an item, kept so a restore and a
/// purge hand back exactly what the listing gave.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Raw {
    name: OsString,
    parent: PathBuf,
    time: i64,
}

/// One item in the trash, for the trash browser: its name, where it was
/// and when it was deleted, so its row reads "report.pdf, deleted
/// Tuesday, D:\work".
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrashedItem {
    /// The trash's id of it.
    pub id: TrashId,
    /// Its name as it was.
    pub name: String,
    /// Where it was, its name included: where a restore puts it back.
    pub original: Place,
    /// When it was deleted, when the trash says.
    pub deleted: Option<Timestamp>,
    /// A file's size in bytes, once measured.
    pub size: Option<u64>,
    /// A folder's entries (not counting what is inside them), once
    /// measured.
    pub entries: Option<u64>,
    /// Whether it is a folder, once measured.
    pub folder: bool,
    raw: Option<Raw>,
}

impl TrashedItem {
    /// An item with an id, where it was and when it was deleted; its size
    /// is not measured.
    pub fn new(id: TrashId, original: Place, deleted: Option<Timestamp>) -> Self {
        let name = original
            .name()
            .map_or_else(|| original.to_string(), str::to_owned);
        Self {
            id,
            name,
            original,
            deleted,
            size: None,
            entries: None,
            folder: false,
            raw: None,
        }
    }

    /// The same item measured: a file's bytes, or a folder's entries.
    pub fn measured(mut self, size: Option<u64>, entries: Option<u64>) -> Self {
        self.folder = entries.is_some();
        self.size = size;
        self.entries = entries;
        self
    }

    /// The row in words, its name first: "report.pdf, deleted Tuesday,
    /// D:\work"; "notes, deleted today, 2:05 PM, /home/ada".
    pub fn words(&self, clock: &dyn Clock) -> String {
        let name = names::display_name(&self.name);
        let when = self.deleted.map_or_else(
            || "deleted at a time not known".to_owned(),
            |time| format!("deleted {}", describe_time(time, clock)),
        );
        let from = self
            .original
            .parent()
            .map_or_else(String::new, |folder| format!(", {folder}"));
        format!("{name}, {when}{from}")
    }

    /// The details line: "File, 12 KB, from D:\work\report.pdf"; "Folder,
    /// 3 entries, from /home/ada/notes".
    pub fn detail(&self) -> String {
        let what = match (self.folder, self.size, self.entries) {
            (true, _, Some(entries)) => format!("Folder, {}", count(entries, "entry", "entries")),
            (_, Some(size), _) => format!("File, {}", human_size(size)),
            _ => "Size not measured".to_owned(),
        };
        format!("{what}, from {}", self.original)
    }

    /// The trash crate's item, rebuilt from what the listing gave, or from
    /// the original place for an item made elsewhere.
    fn to_crate(&self) -> Option<::trash::TrashItem> {
        let (name, parent, time) = match &self.raw {
            Some(raw) => (raw.name.clone(), raw.parent.clone(), raw.time),
            None => {
                let path = PathBuf::from(self.original.local_path()?);
                let name = path.file_name()?.to_os_string();
                let parent = path.parent()?.to_path_buf();
                (
                    name,
                    parent,
                    self.deleted.map_or(0, Timestamp::unix_seconds),
                )
            }
        };
        Some(::trash::TrashItem {
            id: self.id.0.clone(),
            name,
            original_parent: parent,
            time_deleted: time,
        })
    }
}

/// What the check after a purge found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PurgeCheck {
    /// The item is gone from the trash.
    Gone,
    /// The trash cannot be looked into for this item; the call said yes.
    NotChecked,
}

/// The trash as a list (R25 3.3; answer 98): listing, restore and delete
/// for good, on a worker thread. The job engine plans and runs restores
/// and purges through it; the app's trash browser lists through it. The
/// local provider's is [`SystemTrash`]; tests use a test-only bin.
pub trait TrashBin: Send + Sync + fmt::Debug {
    /// Lists the trash, newest first, in batches of up to
    /// [`TRASH_BATCH`]; `batch` returns `Break` to stop. Sizes are not
    /// measured. Unsupported where the trash cannot be listed, with
    /// [`TRASH_NOT_LISTED`] as its sentence.
    fn list(
        &self,
        ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<TrashedItem>) -> ControlFlow<()>,
    ) -> Result<(), VfsError>;

    /// Measures one item: a file's bytes, a folder's entries.
    fn measure(&self, item: &TrashedItem, ctx: &OpContext) -> Result<TrashedItem, VfsError> {
        let _ = ctx;
        Ok(item.clone())
    }

    /// Puts one item back where it was. Never over a name that is taken:
    /// `AlreadyExists`, and nothing moves; the trash restores only to the
    /// original place, so no other name is offered.
    fn restore(&self, item: &TrashedItem, ctx: &OpContext) -> Result<(), VfsError>;

    /// Deletes one item from the trash for good, and checks it is gone.
    fn purge(&self, item: &TrashedItem, ctx: &OpContext) -> Result<PurgeCheck, VfsError>;
}

/// Lists a whole trash into one list, newest first: for a plan, which
/// needs every item before it is shown.
pub fn list_all(bin: &dyn TrashBin, ctx: &OpContext) -> Result<Vec<TrashedItem>, VfsError> {
    let mut all = Vec::new();
    bin.list(ctx, &mut |items| {
        all.extend(items);
        ControlFlow::Continue(())
    })?;
    Ok(all)
}

/// The operating system's calls behind [`SystemTrash`]: the trash crate's
/// `os_limited` on Windows and the freedesktop systems; a test-only bin in
/// the tests.
trait Backend: Send + Sync {
    fn list(&self) -> Result<Vec<::trash::TrashItem>, VfsError>;
    fn metadata(&self, item: &::trash::TrashItem) -> Result<(Option<u64>, Option<u64>), VfsError>;
    fn restore(&self, item: ::trash::TrashItem) -> Result<(), VfsError>;
    fn purge(&self, item: ::trash::TrashItem) -> Result<(), VfsError>;
    /// When an item was deleted, in Unix seconds.
    fn deleted(&self, item: &::trash::TrashItem) -> Option<i64>;
}

/// The trash crate's own calls, where it can list.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
struct OsBackend;

#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
impl Backend for OsBackend {
    fn list(&self) -> Result<Vec<::trash::TrashItem>, VfsError> {
        ::trash::os_limited::list().map_err(from_trash_error)
    }

    fn metadata(&self, item: &::trash::TrashItem) -> Result<(Option<u64>, Option<u64>), VfsError> {
        let metadata = ::trash::os_limited::metadata(item).map_err(from_trash_error)?;
        Ok((
            metadata.size.size(),
            metadata.size.entries().map(|n| n as u64),
        ))
    }

    fn restore(&self, item: ::trash::TrashItem) -> Result<(), VfsError> {
        ::trash::os_limited::restore_all([item]).map_err(from_trash_error)
    }

    fn purge(&self, item: ::trash::TrashItem) -> Result<(), VfsError> {
        ::trash::os_limited::purge_all([item]).map_err(from_trash_error)
    }

    fn deleted(&self, item: &::trash::TrashItem) -> Option<i64> {
        Some(deleted_at(item)).filter(|&time| time != i64::MIN)
    }
}

/// The local provider's trash as a list ([`TrashBin`]): on Windows and
/// the freedesktop systems through the trash crate's listing, restore and
/// purge, each call on a worker thread that catches a panic; on macOS
/// every call answers [`TRASH_NOT_LISTED`].
///
/// - **Restore** never writes over a taken name: the name is checked
///   first, and the crate refuses a collision itself. The crate checks on
///   Windows just before it moves the item, so a file made in that moment
///   can still be replaced; on the freedesktop systems it holds the name
///   with a new empty file first, so it cannot.
/// - **Purge** is checked after: where the item's id is a path the
///   trash holds, it must be gone.
#[derive(Clone)]
pub struct SystemTrash {
    backend: Option<Arc<dyn Backend>>,
}

impl fmt::Debug for SystemTrash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemTrash")
            .field("can_list", &self.can_list())
            .finish()
    }
}

impl Default for SystemTrash {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemTrash {
    /// The system's trash.
    pub fn new() -> Self {
        #[cfg(any(
            windows,
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "ios"),
                not(target_os = "android")
            )
        ))]
        let backend: Option<Arc<dyn Backend>> = Some(Arc::new(OsBackend));
        #[cfg(not(any(
            windows,
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "ios"),
                not(target_os = "android")
            )
        )))]
        let backend: Option<Arc<dyn Backend>> = None;
        Self { backend }
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    /// Whether this system's trash can be listed (not on macOS).
    pub fn can_list(&self) -> bool {
        self.backend.is_some()
    }

    fn backend(&self) -> Result<&Arc<dyn Backend>, VfsError> {
        self.backend
            .as_ref()
            .ok_or_else(|| VfsError::unsupported("file", "list the trash"))
    }

    /// An item of the crate's listing as diroxide's, or `None` when its
    /// former place is not one diroxide can name (a name that is not
    /// Unicode).
    fn item_of(backend: &dyn Backend, raw: ::trash::TrashItem) -> Option<TrashedItem> {
        let path = raw.original_path();
        let original = Place::parse(path.to_str()?).ok()?;
        let deleted = backend
            .deleted(&raw)
            .map(|seconds| Timestamp::from_unix(seconds, 0));
        let mut item = TrashedItem::new(TrashId(raw.id.clone()), original, deleted);
        item.raw = Some(Raw {
            name: raw.name,
            parent: raw.original_parent,
            time: raw.time_deleted,
        });
        Some(item)
    }
}

/// Runs a call on a worker thread, waiting at most `limit` while the
/// token is not cancelled; a panic inside is an error, never a crash. A
/// call that does not return in time keeps only its own thread.
fn on_worker_within<T: Send + 'static>(
    ctx: &OpContext,
    limit: Duration,
    call: impl FnOnce() -> Result<T, VfsError> + Send + 'static,
) -> Result<T, VfsError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("diroxide-trash".to_owned())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(call));
            let _ = sender.send(outcome);
        })
        .map_err(VfsError::from)?;
    let deadline = Instant::now() + limit;
    loop {
        ctx.check()?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(VfsError::Io {
                kind: std::io::ErrorKind::TimedOut,
                message: "the system trash did not answer in time".to_owned(),
            });
        }
        match receiver.recv_timeout(left.min(Duration::from_millis(50))) {
            Ok(Ok(result)) => return result,
            Ok(Err(_)) => {
                return Err(VfsError::Io {
                    kind: std::io::ErrorKind::Other,
                    message: "the system trash failed unexpectedly".to_owned(),
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(VfsError::Io {
                    kind: std::io::ErrorKind::Other,
                    message: "the system trash failed unexpectedly".to_owned(),
                });
            }
        }
    }
}

/// How long one restore, purge or measure may take.
const TRASH_ITEM_LIMIT: Duration = Duration::from_secs(60);

impl TrashBin for SystemTrash {
    fn list(
        &self,
        ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<TrashedItem>) -> ControlFlow<()>,
    ) -> Result<(), VfsError> {
        let backend = Arc::clone(self.backend()?);
        let listed = Arc::clone(&backend);
        let raw = on_worker_within(ctx, TRASH_LIST_LIMIT, move || listed.list())?;
        let mut items: Vec<TrashedItem> = raw
            .into_iter()
            .filter_map(|raw| Self::item_of(&*backend, raw))
            .collect();
        items.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.name.cmp(&b.name)));
        let mut items = items.into_iter().peekable();
        while items.peek().is_some() {
            ctx.check()?;
            let next: Vec<TrashedItem> = items.by_ref().take(TRASH_BATCH).collect();
            if batch(next).is_break() {
                break;
            }
        }
        Ok(())
    }

    fn measure(&self, item: &TrashedItem, ctx: &OpContext) -> Result<TrashedItem, VfsError> {
        let backend = Arc::clone(self.backend()?);
        let raw = item.to_crate().ok_or(VfsError::NotFound)?;
        let (size, entries) =
            on_worker_within(ctx, TRASH_ITEM_LIMIT, move || backend.metadata(&raw))?;
        Ok(item.clone().measured(size, entries))
    }

    fn restore(&self, item: &TrashedItem, ctx: &OpContext) -> Result<(), VfsError> {
        let backend = Arc::clone(self.backend()?);
        let target = item
            .original
            .local_path()
            .map(PathBuf::from)
            .ok_or_else(|| VfsError::unsupported("file", "restore a place that is not local"))?;
        // Never over a taken name: checked here, and again by the crate.
        if fs::symlink_metadata(&target).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        let raw = item.to_crate().ok_or(VfsError::NotFound)?;
        ctx.check()?;
        on_worker_within(ctx, TRASH_ITEM_LIMIT, move || backend.restore(raw))?;
        if fs::symlink_metadata(&target).is_err() {
            return Err(VfsError::Io {
                kind: std::io::ErrorKind::NotFound,
                message: "it was restored but is not at its place".to_owned(),
            });
        }
        Ok(())
    }

    fn purge(&self, item: &TrashedItem, ctx: &OpContext) -> Result<PurgeCheck, VfsError> {
        let backend = Arc::clone(self.backend()?);
        let raw = item.to_crate().ok_or(VfsError::NotFound)?;
        // The id is a path in the trash on both systems: the item in the
        // recycle bin, or its .trashinfo file.
        let held = PathBuf::from(&item.id.0);
        let checkable = held.is_absolute() && fs::symlink_metadata(&held).is_ok();
        ctx.check()?;
        on_worker_within(ctx, TRASH_ITEM_LIMIT, move || backend.purge(raw))?;
        if !checkable {
            return Ok(PurgeCheck::NotChecked);
        }
        if fs::symlink_metadata(&held).is_ok() {
            return Err(VfsError::Io {
                kind: std::io::ErrorKind::Other,
                message: "the system trash kept it".to_owned(),
            });
        }
        Ok(PurgeCheck::Gone)
    }
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

    // ----- the trash as a list, against a test-only bin -----

    use diroxide_core::{FixedClock, UtcOffset};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// A test-only bin standing in for the system's: items are files in a
    /// folder under the test's scratch folder, listed with the trash
    /// crate's own item type, so the listing, the checks and the words
    /// are the real ones. The real trash is never touched.
    #[derive(Default)]
    struct TestBin {
        folder: PathBuf,
        /// The id (the file in the bin) of each item, with where it was
        /// and when it was deleted.
        items: Mutex<BTreeMap<PathBuf, (PathBuf, i64)>>,
        /// A purge that says yes and leaves the item.
        purge_lies: bool,
        /// A listing that panics, as a COM failure inside the crate does.
        list_panics: bool,
    }

    impl TestBin {
        fn new(folder: &Path) -> Self {
            let bin = folder.join("test-bin");
            fs::create_dir_all(&bin).unwrap();
            Self {
                folder: bin,
                ..Self::default()
            }
        }

        /// Moves a file into the bin, as a trash call would.
        fn take(&self, path: &Path, deleted: i64) {
            let mut items = self.items.lock().unwrap();
            let held = self.folder.join(format!("item-{}", items.len() + 1));
            fs::rename(path, &held).unwrap();
            items.insert(held, (path.to_path_buf(), deleted));
        }
    }

    impl Backend for TestBin {
        fn list(&self) -> Result<Vec<::trash::TrashItem>, VfsError> {
            assert!(!self.list_panics, "the trash's COM call failed");
            Ok(self
                .items
                .lock()
                .unwrap()
                .iter()
                .map(|(held, (original, time))| ::trash::TrashItem {
                    id: held.clone().into_os_string(),
                    name: original.file_name().unwrap().to_os_string(),
                    original_parent: original.parent().unwrap().to_path_buf(),
                    time_deleted: *time,
                })
                .collect())
        }

        fn metadata(
            &self,
            item: &::trash::TrashItem,
        ) -> Result<(Option<u64>, Option<u64>), VfsError> {
            let metadata = fs::symlink_metadata(&item.id)?;
            Ok(if metadata.is_dir() {
                (None, Some(fs::read_dir(&item.id)?.count() as u64))
            } else {
                (Some(metadata.len()), None)
            })
        }

        fn restore(&self, item: ::trash::TrashItem) -> Result<(), VfsError> {
            let original = item.original_path();
            if original.exists() {
                return Err(from_trash_error(::trash::Error::RestoreCollision {
                    path: original,
                    remaining_items: vec![item],
                }));
            }
            let held = PathBuf::from(&item.id);
            fs::rename(&held, &original)?;
            self.items.lock().unwrap().remove(&held);
            Ok(())
        }

        fn purge(&self, item: ::trash::TrashItem) -> Result<(), VfsError> {
            if self.purge_lies {
                return Ok(());
            }
            let held = PathBuf::from(&item.id);
            fs::remove_file(&held)?;
            self.items.lock().unwrap().remove(&held);
            Ok(())
        }

        fn deleted(&self, item: &::trash::TrashItem) -> Option<i64> {
            Some(item.time_deleted)
        }
    }

    /// A clock on Friday, October 23, 2026 at noon, UTC.
    fn friday() -> FixedClock {
        FixedClock::new(Timestamp::from_unix(1_792_756_800, 0), UtcOffset::UTC)
    }

    fn place_of(path: &Path) -> Place {
        Place::parse(path.to_str().unwrap()).unwrap()
    }

    fn bin_with_two(folder: &Path) -> (Arc<TestBin>, PathBuf, PathBuf) {
        let bin = Arc::new(TestBin::new(folder));
        let report = folder.join("report.pdf");
        let notes = folder.join("notes.txt");
        fs::write(&report, b"pdf").unwrap();
        fs::write(&notes, b"notes!").unwrap();
        // Tuesday, then Friday morning.
        bin.take(&report, 1_792_756_800 - 3 * 86_400);
        bin.take(&notes, 1_792_756_800 - 3_600);
        (bin, report, notes)
    }

    #[test]
    fn the_trash_lists_newest_first_and_each_row_says_its_name_first() {
        let folder = scratch();
        let (bin, report, _) = bin_with_two(folder.path());
        let trash = SystemTrash::with_backend(bin);
        let ctx = OpContext::new();
        let items = list_all(&trash, &ctx).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "notes.txt", "newest first");
        let clock = friday();
        let row = items[1].words(&clock);
        let from = folder.path().to_str().unwrap();
        assert_eq!(row, format!("report.pdf, deleted Tuesday, {from}"));
        assert!(diroxide_core::format::within_cells(
            &row,
            "report.pdf, deleted Tuesday"
        ));
        assert_eq!(
            items[0].words(&clock),
            format!("notes.txt, deleted today, 11:00 AM, {from}")
        );
        let measured = trash.measure(&items[1], &ctx).unwrap();
        assert_eq!(measured.size, Some(3));
        assert_eq!(
            measured.detail(),
            format!("File, 3 bytes, from {}", report.display())
        );
    }

    #[test]
    fn a_restore_puts_an_item_back_and_never_over_a_taken_name() {
        let folder = scratch();
        let (bin, report, notes) = bin_with_two(folder.path());
        let trash = SystemTrash::with_backend(Arc::clone(&bin) as Arc<dyn Backend>);
        let ctx = OpContext::new();
        let items = list_all(&trash, &ctx).unwrap();
        // The collision case: a new report.pdf took the name.
        fs::write(&report, b"new").unwrap();
        let old_report = items.iter().find(|i| i.name == "report.pdf").unwrap();
        assert_eq!(
            trash.restore(old_report, &ctx),
            Err(VfsError::AlreadyExists)
        );
        assert_eq!(fs::read(&report).unwrap(), b"new", "the new file is kept");
        assert_eq!(bin.items.lock().unwrap().len(), 2, "nothing moved");
        let old_notes = items.iter().find(|i| i.name == "notes.txt").unwrap();
        trash.restore(old_notes, &ctx).unwrap();
        assert_eq!(fs::read(&notes).unwrap(), b"notes!");
        assert_eq!(list_all(&trash, &ctx).unwrap().len(), 1);
        assert_eq!(old_notes.original, place_of(&notes));
    }

    #[test]
    fn a_purge_is_checked_and_a_bin_that_keeps_the_item_is_said() {
        let folder = scratch();
        let (bin, _, _) = bin_with_two(folder.path());
        let trash = SystemTrash::with_backend(Arc::clone(&bin) as Arc<dyn Backend>);
        let ctx = OpContext::new();
        let items = list_all(&trash, &ctx).unwrap();
        assert_eq!(trash.purge(&items[0], &ctx), Ok(PurgeCheck::Gone));
        assert_eq!(list_all(&trash, &ctx).unwrap().len(), 1);
        let liar = Arc::new(TestBin {
            purge_lies: true,
            ..TestBin::new(&folder.path().join("liar"))
        });
        let kept = folder.path().join("kept.txt");
        fs::write(&kept, b"k").unwrap();
        liar.take(&kept, 1);
        let trash = SystemTrash::with_backend(liar);
        let item = list_all(&trash, &ctx).unwrap().remove(0);
        let refused = trash.purge(&item, &ctx).unwrap_err();
        assert_eq!(refused.to_string(), "the system trash kept it");
        // An id the trash gave that is not a path cannot be looked for.
        let mut odd = item.clone();
        odd.id = TrashId::new("::{645FF040-5081-101B-9F08-00AA002F954E}\\x");
        assert_eq!(trash.purge(&odd, &ctx), Ok(PurgeCheck::NotChecked));
    }

    #[test]
    fn a_trash_that_cannot_be_listed_says_so_and_a_panic_is_an_error() {
        let ctx = OpContext::new();
        let unlisted = SystemTrash { backend: None };
        assert!(!unlisted.can_list());
        assert!(matches!(
            list_all(&unlisted, &ctx),
            Err(VfsError::Unsupported { .. })
        ));
        assert!(diroxide_core::format::within_cells(
            TRASH_NOT_LISTED,
            "The trash cannot be listed"
        ));
        let folder = scratch();
        let panics = Arc::new(TestBin {
            list_panics: true,
            ..TestBin::new(folder.path())
        });
        let trash = SystemTrash::with_backend(panics);
        assert_eq!(
            list_all(&trash, &ctx).unwrap_err().to_string(),
            "the system trash failed unexpectedly"
        );
        let cancelled = OpContext::new();
        cancelled.cancel.cancel();
        assert_eq!(list_all(&trash, &cancelled), Err(VfsError::Cancelled));
    }

    #[test]
    fn a_large_trash_comes_in_batches_and_can_stop_early() {
        let folder = scratch();
        let bin = Arc::new(TestBin::new(folder.path()));
        {
            let mut items = bin.items.lock().unwrap();
            for n in 0..1_200_i64 {
                items.insert(
                    folder.path().join(format!("held-{n}")),
                    (folder.path().join(format!("f{n}.txt")), n),
                );
            }
        }
        let trash = SystemTrash::with_backend(bin);
        let mut sizes = Vec::new();
        trash
            .list(&OpContext::new(), &mut |items| {
                sizes.push(items.len());
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(sizes, [TRASH_BATCH, TRASH_BATCH, 200]);
        let mut first = Vec::new();
        trash
            .list(&OpContext::new(), &mut |items| {
                first = items;
                ControlFlow::Break(())
            })
            .unwrap();
        assert_eq!(first[0].name, "f1199.txt", "the newest first");
    }
}
