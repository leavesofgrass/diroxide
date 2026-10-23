//! Sites and saved sessions (R24 part 3; answers 83 and 94): the sites
//! file, the model, the Sites list with the hosts of the owner's ssh
//! configuration, and the places the plugin offers.
//!
//! - **The file holds no secret.** `sites.toml` in the plugin's state
//!   folder names hosts, users, start folders and a secret mode; a file
//!   with a planted password field is refused and said ([`store`]).
//! - **Hosts from the ssh configuration** are rows with nothing saved:
//!   "nas, from ssh config". Only the owner's own file and the files its
//!   `Include` lines name are read (answer 94); ssh resolves the rest.
//! - **Every row's state is a word,** right after the name.

pub mod commands;
pub mod lister;
pub mod model;
pub mod store;
pub mod words;

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

pub use lister::{Row, RowOf};
pub use model::{SecretMode, Site, SiteError};
pub use store::{SitesFile, StoreError};

use crate::ssh::hosts::{ConfigPlaces, HostList, config_hosts};
use crate::ssh::target::Target;

/// The plugin's sites: where the file is, and where the owner's ssh
/// configuration is read from.
#[derive(Debug, Default)]
pub struct Sites {
    dir: Mutex<Option<PathBuf>>,
    config: Mutex<ConfigSource>,
}

/// Where the hosts of the ssh configuration come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConfigSource {
    /// The owner's own file, or the `ssh_config_file` setting's.
    #[default]
    Owner,
    /// The setting's file, read with the owner's home for `Include`.
    Setting(PathBuf),
    /// Fixed places (a test's fixture; never the owner's real files).
    Fixed(ConfigPlaces),
}

impl Sites {
    /// Sites with no state folder yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the state folder, from `initialize`.
    pub fn set_dir(&self, dir: Option<PathBuf>) {
        *self.dir.lock().unwrap_or_else(PoisonError::into_inner) = dir;
    }

    /// The state folder, when the host gave one.
    pub fn dir(&self) -> Option<PathBuf> {
        self.dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sets where the configuration's hosts come from: the setting's file
    /// when it names one, unless fixed places were set.
    pub fn set_config_file(&self, file: Option<PathBuf>) {
        let mut config = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*config, ConfigSource::Fixed(_)) {
            *config = file.map_or(ConfigSource::Owner, ConfigSource::Setting);
        }
    }

    /// Reads the configuration from fixed places from now on (tests).
    pub fn set_config_places(&self, places: ConfigPlaces) {
        *self.config.lock().unwrap_or_else(PoisonError::into_inner) = ConfigSource::Fixed(places);
    }

    /// The sites file, read now (the owner may have edited it by hand). A
    /// plugin with no state folder has an empty list in memory.
    pub fn file(&self) -> Result<SitesFile, StoreError> {
        match self.dir() {
            Some(dir) => SitesFile::load(&dir),
            None => SitesFile::parse("", None),
        }
    }

    /// The hosts of the ssh configuration; none when there is no home
    /// folder.
    pub fn config_hosts(&self) -> HostList {
        let source = self
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let places = match source {
            ConfigSource::Owner => ConfigPlaces::for_user(None),
            ConfigSource::Setting(file) => ConfigPlaces::for_user(Some(&file)),
            ConfigSource::Fixed(places) => Some(places),
        };
        places.map(|p| config_hosts(&p)).unwrap_or_default()
    }

    /// `provider/places`: the sites and the configuration's hosts for the
    /// go-to-place list, each with its state word; listing them never
    /// signs in. A refused sites file leaves the saved sites out and
    /// keeps the configuration's hosts.
    pub fn places(&self, provider: &crate::provider::Provider) -> serde_json::Value {
        let rows = match self.rows(|site| commands::state_of(provider, site)) {
            Ok((rows, _)) => rows,
            Err(_) => lister::rows(&[], &self.config_hosts().hosts, |_| words::SAVED),
        };
        lister::places(&rows)
    }

    /// The saved site a connection to `target` uses, when one matches.
    pub fn for_target(&self, target: &Target) -> Option<Site> {
        let file = self.file().ok()?;
        file.sites().iter().find(|s| s.matches(target)).cloned()
    }

    /// The rows of the Sites list, each with the state `state` gives it,
    /// and what was left out in words; a refused file gives its refusal.
    pub fn rows(
        &self,
        state: impl Fn(&Site) -> &'static str,
    ) -> Result<(Vec<Row>, Vec<String>), StoreError> {
        let file = self.file()?;
        let hosts = self.config_hosts();
        let rows = lister::rows(file.sites(), &hosts.hosts, state);
        let mut notes = Vec::new();
        if !file.skipped().is_empty() {
            notes.push(words::sites_skipped(file.skipped().len()));
        }
        if !hosts.skipped.is_empty() {
            notes.push(crate::lines::names_skipped(hosts.skipped.len()));
        }
        Ok((rows, notes))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn fixture_places() -> ConfigPlaces {
        let home =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sftp/ssh-config/home");
        ConfigPlaces {
            file: home.join(".ssh/config"),
            ssh_dir: home.join(".ssh"),
            home,
        }
    }

    #[test]
    fn rows_come_from_the_file_and_the_fixture_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(store::FILE_NAME),
            "[[site]]\nid = \"k3f9\"\nname = \"build\"\nhost = \"build.example\"\nuser = \"alice\"\n",
        )
        .unwrap();
        let sites = Sites::new();
        sites.set_dir(Some(dir.path().to_path_buf()));
        sites.set_config_places(fixture_places());
        let (rows, _) = sites.rows(|_| words::SAVED).unwrap();
        assert_eq!(rows[0].words(), "build, saved, alice@build.example");
        assert!(rows.len() > 1, "the fixture config names hosts");
        assert!(rows[1..].iter().all(|r| r.state == words::FROM_CONFIG));
        let target = Target::new("build.example", Some("alice"), None).unwrap();
        assert_eq!(sites.for_target(&target).unwrap().id, "k3f9");
        // A planted password: the list is refused, in words.
        std::fs::write(
            dir.path().join(store::FILE_NAME),
            "[[site]]\nid = \"k3f9\"\nname = \"build\"\nhost = \"b\"\npassword = \"x\"\n",
        )
        .unwrap();
        let refused = sites.rows(|_| words::SAVED).unwrap_err();
        assert_eq!(refused.lines()[1], "Remove password from build.");
        assert!(sites.for_target(&target).is_none());
    }

    #[test]
    fn no_state_folder_means_an_empty_list_in_memory() {
        let sites = Sites::new();
        sites.set_config_places(ConfigPlaces {
            file: PathBuf::from("no-such-config"),
            home: PathBuf::from("no-such-home"),
            ssh_dir: PathBuf::from("no-such-home"),
        });
        let (rows, notes) = sites.rows(|_| words::SAVED).unwrap();
        assert!(rows.is_empty());
        assert!(notes.is_empty());
    }
}
