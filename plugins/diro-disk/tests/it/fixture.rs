//! The real host over the plugin's program, as the app builds it: the
//! host reads the images through its own local provider (`fs/read`), and
//! a registry with the plugin's claim sends `IMAGE!...` places to it.

// Shared helpers: each reader's tests use some of them.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_config::PluginSettings;
use diroxide_core::{Entry, Place};
use diroxide_plugins::host::QUIT_BUDGET;
use diroxide_plugins::provider::PluginProvider;
use diroxide_plugins::tools::ToolBox;
use diroxide_plugins::{Host, Manifest, PluginInfo};
use diroxide_vfs::{OpContext, Provider, Registry, VfsError};

use crate::{plugin_exe, plugin_folder, scratch};

/// A host with the plugin enabled, and a folder for the images.
pub struct Fixture {
    /// The test's folder: `images/` holds the images.
    pub root: tempfile::TempDir,
    /// The host.
    pub host: Arc<Host>,
    /// The registry as the app builds it, with the plugin's claim.
    pub registry: Registry,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.shutdown_all(QUIT_BUDGET);
    }
}

impl Fixture {
    /// A host whose tools are found on the process's PATH.
    pub fn new() -> Self {
        Self::with_tools(ToolBox::default())
    }

    /// A host whose tools are found in `tools`.
    pub fn with_tools(tools: ToolBox) -> Self {
        let root = scratch();
        let folder = plugin_folder(root.path(), &plugin_exe());
        let manifest =
            Manifest::from_toml(&std::fs::read_to_string(folder.join("plugin.toml")).unwrap())
                .unwrap();
        let mut host = Host::new(&PluginSettings::default())
            .with_state_dir(root.path().join("state"))
            .with_registry(Arc::new(Registry::with_local()))
            .with_tools(tools);
        host.add(PluginInfo {
            manifest,
            folder,
            enabled: true,
        });
        let host = Arc::new(host);
        let mut registry = Registry::with_local();
        for (scheme, provider) in PluginProvider::all(&host) {
            registry.register(&scheme, provider);
        }
        std::fs::create_dir_all(root.path().join("images")).unwrap();
        Self {
            root,
            host,
            registry,
        }
    }

    /// Where an image of this name goes.
    pub fn image_path(&self, name: &str) -> PathBuf {
        self.root.path().join("images").join(name)
    }

    /// Writes an image and gives its root's place: `IMAGE!`.
    pub fn image(&self, name: &str, bytes: &[u8]) -> Place {
        let path = self.image_path(name);
        std::fs::write(&path, bytes).unwrap();
        root_of(&path)
    }

    /// The provider that serves a place, by the claim route.
    pub fn provider(&self, place: &Place) -> Arc<dyn Provider> {
        self.registry.resolve(place).unwrap()
    }

    /// The names a place lists, in the image's order.
    pub fn names(&self, place: &Place) -> Vec<String> {
        self.list(place)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    /// The entries a place lists.
    pub fn list(&self, place: &Place) -> Result<Vec<Entry>, VfsError> {
        self.provider(place).list(place, &OpContext::new())
    }

    /// One entry's details.
    pub fn stat(&self, place: &Place) -> Result<Entry, VfsError> {
        self.provider(place).stat(place, &OpContext::new())
    }

    /// A file's bytes, read whole through the host's reader.
    pub fn read(&self, place: &Place) -> Result<Vec<u8>, VfsError> {
        let mut bytes = Vec::new();
        self.provider(place)
            .read(place, &OpContext::new())?
            .read_to_end(&mut bytes)
            .map_err(|e| VfsError::Io {
                kind: e.kind(),
                message: e.to_string(),
            })?;
        Ok(bytes)
    }
}

/// The root place of an image file: `D:\x\fedora.iso!`.
pub fn root_of(path: &Path) -> Place {
    Place::parse(&path.display().to_string())
        .unwrap()
        .into_archive()
}

/// A place under another, by names.
pub fn at(place: &Place, names: &[&str]) -> Place {
    let mut place = place.clone();
    for name in names {
        place = place.join(name).unwrap();
    }
    place
}

/// The words of an error, as the host's line says them.
pub fn words(error: &VfsError) -> String {
    diroxide_core::Message::from_error("open", "the image", error).text
}

/// A tool box that finds 7-Zip: on PATH, or, on Windows, in its usual
/// install folder; `None` when this system has none (the test then says
/// so and passes, as the host's own git test does).
pub fn seven_zip_tools() -> Option<ToolBox> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut folders: Vec<PathBuf> = std::env::split_paths(&path).collect();
    if cfg!(windows) {
        folders.push(PathBuf::from(r"C:\Program Files\7-Zip"));
    }
    let names: &[&str] = if cfg!(windows) {
        &["7z.exe", "7za.exe", "7zz.exe"]
    } else {
        &["7z", "7za", "7zz"]
    };
    let found = folders
        .iter()
        .any(|folder| names.iter().any(|name| folder.join(name).is_file()));
    if !found {
        eprintln!("skipped: no 7-Zip on this system");
        return None;
    }
    let joined = std::env::join_paths(folders).ok()?;
    Some(ToolBox::with_path(Some(joined)))
}
