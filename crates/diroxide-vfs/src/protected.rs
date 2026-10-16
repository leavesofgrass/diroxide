//! Protected places: the folders a recursive removal (a delete, a trash,
//! a move that removes its source) must never take as a whole (R14, R16;
//! the anchor refusals of W1b). They are the tops of drives and shares,
//! folders a drive is mounted on, the home folder and the folders of its
//! own (Desktop, Documents, Downloads and the rest), and the system's own
//! folders. On September 26, 2026 a delete aimed at one junk folder
//! removed most of the owner's profile; this list is the line the engine
//! will not cross. Work inside these folders is not refused: only the
//! folder itself, as the root of a recursive removal.
//!
//! Asking reads the home folder's location and the volume a place is on,
//! so it runs off the input thread like any provider call.

use std::path::{Path, PathBuf};

use diroxide_core::Place;

use crate::local::volumes;

/// What makes a place protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Protection {
    /// The top of a drive, a share, or `/`.
    Top,
    /// A folder another drive is mounted on.
    VolumeRoot,
    /// The home folder.
    Home,
    /// One of the home folder's own folders: Desktop, Documents,
    /// Downloads, Pictures, Music, Videos, Public, and the folders that hold
    /// programs' settings and data.
    UserFolder,
    /// One of the system's own folders.
    SystemFolder,
}

impl Protection {
    /// What it is, in words: "the home folder".
    pub const fn words(self) -> &'static str {
        match self {
            Protection::Top => "the top of a drive",
            Protection::VolumeRoot => "a folder a drive is mounted on",
            Protection::Home => "the home folder",
            Protection::UserFolder => "one of the home folder's own folders",
            Protection::SystemFolder => "a system folder",
        }
    }

    /// The refusal, meaning first: "Never removed whole: the home folder".
    pub fn refusal(self) -> String {
        format!("Never removed whole: {}", self.words())
    }
}

/// The system folders of Unix-like systems, Linux's and macOS's.
const UNIX_SYSTEM_FOLDERS: [&str; 39] = [
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/media",
    "/mnt",
    "/opt",
    "/proc",
    "/root",
    "/run",
    "/sbin",
    "/srv",
    "/sys",
    "/tmp",
    "/usr",
    "/usr/bin",
    "/usr/lib",
    "/usr/local",
    "/usr/sbin",
    "/usr/share",
    "/var",
    "/var/lib",
    "/var/log",
    "/Applications",
    "/Library",
    "/System",
    "/Users",
    "/Volumes",
    "/private",
    "/private/etc",
    "/private/tmp",
    "/private/var",
    "/cores",
    "/nix",
];

/// The environment variables that name Windows's system folders.
const WINDOWS_SYSTEM_VARIABLES: [&str; 6] = [
    "SystemRoot",
    "windir",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "ProgramData",
];

/// Whether a place and a path name the same folder (ignoring case on
/// Windows, where places do).
fn same(place: &Place, path: &Path) -> bool {
    path.to_str()
        .and_then(|text| Place::parse(text).ok())
        .is_some_and(|other| place.is_within(&other) && other.is_within(place))
}

fn user_folders() -> Vec<PathBuf> {
    [
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::picture_dir(),
        dirs::audio_dir(),
        dirs::video_dir(),
        dirs::public_dir(),
        dirs::template_dir(),
        dirs::config_dir(),
        dirs::data_dir(),
        dirs::data_local_dir(),
        dirs::cache_dir(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn system_folders() -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut folders: Vec<PathBuf> = WINDOWS_SYSTEM_VARIABLES
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect();
        // The folder that holds every user's home: C:\Users.
        if let Some(users) = dirs::home_dir().as_deref().and_then(Path::parent) {
            folders.push(users.to_path_buf());
        }
        folders
    } else {
        UNIX_SYSTEM_FOLDERS.iter().map(PathBuf::from).collect()
    }
}

/// What protects a place from being removed whole, or `None`. Only the
/// place itself is protected, never what is inside it.
pub fn protection(place: &Place) -> Option<Protection> {
    if place.is_root() {
        return Some(Protection::Top);
    }
    let path = place.local_path()?;
    if dirs::home_dir().is_some_and(|home| same(place, &home)) {
        return Some(Protection::Home);
    }
    if user_folders().iter().any(|folder| same(place, folder)) {
        return Some(Protection::UserFolder);
    }
    if system_folders().iter().any(|folder| same(place, folder)) {
        return Some(Protection::SystemFolder);
    }
    let path = Path::new(path);
    if std::fs::metadata(path).is_ok_and(|m| m.is_dir())
        && volumes::facts_at(path)
            .root
            .is_some_and(|root| same(place, &root))
    {
        return Some(Protection::VolumeRoot);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn tops_and_the_home_are_protected() {
        let top = Place::parse(if cfg!(windows) { "D:\\" } else { "/" }).unwrap();
        assert_eq!(protection(&top), Some(Protection::Top));
        if let Some(home) = dirs::home_dir().and_then(|h| Place::parse(h.to_str()?).ok()) {
            assert_eq!(protection(&home), Some(Protection::Home));
            // Inside the home, a plain folder is not protected.
            let inside = home.join("diroxide-no-such-folder").unwrap();
            assert_eq!(protection(&inside), None);
        }
    }

    #[test]
    fn system_folders_are_protected() {
        let system = if cfg!(windows) {
            std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned())
        } else {
            "/usr".to_owned()
        };
        let place = Place::parse(&system).unwrap();
        assert_eq!(protection(&place), Some(Protection::SystemFolder));
        if cfg!(windows) {
            let lower = Place::parse(&system.to_lowercase()).unwrap();
            assert_eq!(protection(&lower), Some(Protection::SystemFolder));
        }
    }

    #[test]
    fn this_crate_folder_is_not_protected() {
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert_eq!(protection(&here), None);
    }

    #[test]
    fn refusals_say_the_fact_first() {
        for protection in [
            Protection::Top,
            Protection::VolumeRoot,
            Protection::Home,
            Protection::UserFolder,
            Protection::SystemFolder,
        ] {
            assert!(within_cells(&protection.refusal(), "Never removed whole"));
        }
        assert_eq!(
            Protection::Home.refusal(),
            "Never removed whole: the home folder"
        );
    }
}
