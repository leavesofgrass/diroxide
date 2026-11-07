//! The hot-folder tests' scratch folders: every one under the build's own
//! folder, never the system's temporary folder or the owner's folders.

use std::fs;

/// A scratch folder under the build's folder, removed when dropped.
pub(crate) fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe.parent().unwrap().parent().unwrap().join("hot-unit-tmp");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("hot-")
        .tempdir_in(&base)
        .unwrap()
}
