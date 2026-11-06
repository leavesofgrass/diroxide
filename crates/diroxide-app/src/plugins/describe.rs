//! A plugin's words about a folder for the title line (`place/describe`,
//! alpha 4's host-pieces lane, R30's commit 23): which plugins may
//! describe a folder, and asking them, off the input thread.
//!
//! - **Who is asked:** an enabled plugin whose manifest declares a
//!   `[[place_info]]` table that matches the folder: its `scheme` (`file`
//!   for a folder on this machine or in an archive, as hooks say it) and
//!   its `marker`, a file or folder found in the folder or one above it,
//!   such as `.git`. A plugin whose manifest declares no `[[place_info]]`
//!   is never asked.
//! - **The first answer wins:** the plugins are asked in turn and the
//!   first that gives words describes the folder; one that answers
//!   nothing or fails is passed over.
//! - The walk up for a marker reads the disk, so it runs here, on the
//!   describing thread, never on the input thread.

use std::path::Path;
use std::sync::Arc;

use diroxide_core::Place;
use diroxide_plugins::host::Host;
use diroxide_plugins::manifest::PlaceInfoSpec;

/// The most folders a marker is looked for in, the folder and those above
/// it.
const MARKER_DEPTH: usize = 64;

/// The scheme a place's `[[place_info]]` `scheme` is matched against:
/// the remote place's own, else `file`, as hooks say it.
pub fn scheme_of(place: &Place) -> &str {
    place.remote().map_or("file", |r| r.scheme())
}

/// Whether a table's scheme, when it names one, is the place's: the check
/// the app makes before asking, with no disk read.
pub fn scheme_fits(spec: &PlaceInfoSpec, place: &Place) -> bool {
    spec.scheme.as_deref().is_none_or(|s| s == scheme_of(place))
}

/// Whether a marker is in a local folder or one above it.
fn marker_found(place: &Place, marker: &str) -> bool {
    let Some(path) = place.local_path() else {
        return false;
    };
    Path::new(path)
        .ancestors()
        .take(MARKER_DEPTH)
        .any(|folder| folder.join(marker).symlink_metadata().is_ok())
}

/// Whether any of a plugin's `[[place_info]]` tables matches a folder:
/// its scheme fits, and its marker, when it names one, is found.
pub fn matches(specs: &[PlaceInfoSpec], place: &Place) -> bool {
    specs.iter().any(|spec| {
        scheme_fits(spec, place)
            && spec
                .marker
                .as_deref()
                .is_none_or(|marker| marker_found(place, marker))
    })
}

/// A plugin that may describe a folder: its id, its name, its host and
/// its `[[place_info]]` tables.
pub struct Describer {
    /// The plugin's id.
    pub id: String,
    /// Its name, said after its words.
    pub name: String,
    /// Its host.
    pub host: Arc<Host>,
    /// Its `[[place_info]]` tables.
    pub specs: Vec<PlaceInfoSpec>,
}

/// What the describers said about a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Described {
    /// A plugin's words: its id, its name and the words.
    Words {
        /// The plugin's id.
        plugin: String,
        /// Its name.
        name: String,
        /// The words, cleaned by the host.
        text: String,
    },
    /// No plugin matched, or those asked gave no words.
    Nothing,
    /// Every plugin asked failed: the title stays as it was.
    Failed,
}

/// Asks the describers that match a folder, in turn, until one gives
/// words. Runs on the describing thread.
pub fn ask(describers: &[Describer], place: &Place) -> Described {
    let mut answered = false;
    let mut failed = false;
    for describer in describers {
        if !matches(&describer.specs, place) {
            continue;
        }
        match describer.host.describe_place(&describer.id, place) {
            Ok(Some(text)) => {
                return Described::Words {
                    plugin: describer.id.clone(),
                    name: describer.name.clone(),
                    text,
                };
            }
            Ok(None) => answered = true,
            Err(_) => failed = true,
        }
    }
    if failed && !answered {
        Described::Failed
    } else {
        Described::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(scheme: Option<&str>, marker: Option<&str>) -> PlaceInfoSpec {
        PlaceInfoSpec {
            scheme: scheme.map(str::to_owned),
            marker: marker.map(str::to_owned),
        }
    }

    #[test]
    fn a_marker_is_found_in_the_folder_or_above_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let deep = repo.join("src").join("app");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let place = |p: &Path| Place::parse(&p.display().to_string()).unwrap();
        let git = [spec(None, Some(".git"))];
        assert!(matches(&git, &place(&repo)));
        assert!(matches(&git, &place(&deep)));
        assert!(!matches(&git, &place(dir.path())));
        assert!(matches(&[spec(Some("file"), None)], &place(dir.path())));
        assert!(!matches(&[spec(Some("sftp"), None)], &place(&repo)));
        assert!(!matches(&[spec(Some("sftp"), Some(".git"))], &place(&repo)));
    }
}
