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
use diroxide_plugins::manifest::{PlaceInfoSpec, ProviderSpec};

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

/// The scheme of the plugin's provider that claims a place inside a
/// container by the container's extension (PD's request 6): the disk
/// plugin's `disk` for `D:\x\disk.iso!EFI`. `None` for any other place.
pub fn claimed_scheme<'a>(place: &Place, providers: &'a [ProviderSpec]) -> Option<&'a str> {
    let (container, _) = place.archive()?;
    let name = container.name()?.to_lowercase();
    providers
        .iter()
        .flat_map(|p| p.extensions.iter().map(move |e| (p, e)))
        .filter(|(_, extension)| {
            let extension = extension.to_lowercase();
            name.strip_suffix(extension.as_str())
                .is_some_and(|stem| !stem.is_empty())
        })
        .max_by_key(|(_, extension)| extension.len())
        .map(|(provider, _)| provider.scheme.as_str())
}

/// Whether a table fits a place: by the place's own scheme, or, for a
/// place inside a container one of the plugin's providers claims, by that
/// provider's scheme, so the disk plugin describes a place in an image.
pub fn fits(spec: &PlaceInfoSpec, providers: &[ProviderSpec], place: &Place) -> bool {
    scheme_fits(spec, place)
        || spec
            .scheme
            .as_deref()
            .is_some_and(|s| claimed_scheme(place, providers) == Some(s))
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
/// its scheme fits (by the place, or by a provider of the plugin's that
/// claims the place's container), and its marker, when it names one, is
/// found.
pub fn matches(specs: &[PlaceInfoSpec], providers: &[ProviderSpec], place: &Place) -> bool {
    specs.iter().any(|spec| {
        fits(spec, providers, place)
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
    /// Its providers, whose claimed extensions let it describe places
    /// inside the containers it reads.
    pub providers: Vec<ProviderSpec>,
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
        if !matches(&describer.specs, &describer.providers, place) {
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

    /// PD's request 6: a place inside a container a plugin's provider
    /// claims fits that plugin's `[[place_info]]` by the provider's
    /// scheme, nested in a zip too; a place in a zip it does not claim,
    /// and a folder, do not.
    #[test]
    fn a_place_inside_a_claimed_container_fits_its_plugin() {
        let disk = ProviderSpec {
            scheme: "disk".to_owned(),
            name: "Disk images".to_owned(),
            separator: "/".to_owned(),
            flags: std::collections::BTreeSet::new(),
            max_name_length: None,
            reserved_names: Vec::new(),
            methods: std::collections::BTreeSet::new(),
            extensions: vec![".iso".to_owned(), ".vhdx".to_owned()],
            wants: Vec::new(),
        };
        let providers = [disk];
        let spec = spec(Some("disk"), None);
        let at = |text: &str| {
            let text = if cfg!(windows) {
                format!("D:{}", text.replace('/', "\\"))
            } else {
                text.to_owned()
            };
            Place::parse(&text).unwrap()
        };
        let inside = at("/x/disk.iso").into_archive().join("EFI").unwrap();
        assert_eq!(claimed_scheme(&inside, &providers), Some("disk"));
        assert!(fits(&spec, &providers, &inside));
        assert!(matches(std::slice::from_ref(&spec), &providers, &inside));
        let nested = at("/x/bundle.zip")
            .into_archive()
            .join("Disk.ISO")
            .unwrap()
            .into_archive();
        assert!(fits(&spec, &providers, &nested));
        let zip = at("/x/bundle.zip").into_archive();
        assert_eq!(claimed_scheme(&zip, &providers), None);
        assert!(!fits(&spec, &providers, &zip));
        assert!(!fits(&spec, &providers, &at("/x")));
        assert!(!fits(&spec, &[], &inside), "no provider claims it");
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
        assert!(matches(&git, &[], &place(&repo)));
        assert!(matches(&git, &[], &place(&deep)));
        assert!(!matches(&git, &[], &place(dir.path())));
        assert!(matches(
            &[spec(Some("file"), None)],
            &[],
            &place(dir.path())
        ));
        assert!(!matches(&[spec(Some("sftp"), None)], &[], &place(&repo)));
        assert!(!matches(
            &[spec(Some("sftp"), Some(".git"))],
            &[],
            &place(&repo)
        ));
    }
}
