//! The host: finds plugins, starts them on demand, routes calls, and
//! serves their requests through [`HostServices`], which the app
//! implements.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use diroxide_config::PluginSettings;
use diroxide_core::{Message, Place, Side};
use serde_json::Value;

use crate::error::PluginError;
use crate::manifest::Manifest;

/// A plugin found on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    /// Its manifest.
    pub manifest: Manifest,
    /// Its folder.
    pub folder: PathBuf,
    /// Whether the user enabled it.
    pub enabled: bool,
}

/// A panel as a plugin sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelSnapshot {
    /// Which panel.
    pub side: Side,
    /// Its place.
    pub place: Place,
    /// The entry under the cursor.
    pub cursor: Option<Place>,
    /// The selected entries.
    pub selection: Vec<Place>,
}

/// What the host offers plugins, implemented by the app over its own state,
/// dialogs and job engine, so a plugin's operations get the same safety,
/// trash, progress and undo as built-in ones.
pub trait HostServices: Send + Sync {
    /// `ui/announce`: says a message.
    fn announce(&self, message: Message);

    /// `ui/ask`: asks a yes-or-no question through an accessible dialog.
    /// `None` when the user cancels.
    fn ask_yes_no(&self, question: &str) -> Option<bool>;

    /// `ui/ask`: asks for text, with a label. `None` when cancelled.
    fn ask_text(&self, label: &str, initial: &str) -> Option<String>;

    /// `panel/get`: a panel's state; `None` for the active one.
    fn panel(&self, side: Option<Side>) -> PanelSnapshot;
}

/// The plugin host.
#[derive(Debug)]
pub struct Host {
    plugins: Vec<PluginInfo>,
    enabled: Vec<String>,
    timeout: Duration,
}

impl Host {
    /// A host with the user's plugin settings and no plugins found yet.
    pub fn new(settings: &PluginSettings) -> Self {
        Self {
            plugins: Vec::new(),
            enabled: settings.enabled.clone(),
            timeout: Duration::from_millis(settings.timeout_ms),
        }
    }

    /// How long a plugin may take to answer.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Finds plugins: every folder under `plugins_dir` with a readable
    /// `plugin.toml`. Folders whose manifest cannot be read are returned as
    /// errors beside the plugins found; a missing folder finds nothing.
    pub fn discover(&mut self, plugins_dir: &Path) -> Vec<PluginError> {
        self.plugins.clear();
        let mut problems = Vec::new();
        let Ok(folders) = fs::read_dir(plugins_dir) else {
            return problems;
        };
        for folder in folders
            .flatten()
            .map(|item| item.path())
            .filter(|p| p.is_dir())
        {
            let label = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(text) = fs::read_to_string(folder.join(Manifest::FILE)) else {
                continue;
            };
            match Manifest::from_toml(&text) {
                Ok(manifest) => {
                    let enabled = self.enabled.contains(&manifest.id);
                    self.plugins.push(PluginInfo {
                        manifest,
                        folder,
                        enabled,
                    });
                }
                Err(PluginError::Manifest { message, .. }) => {
                    problems.push(PluginError::Manifest {
                        plugin: label,
                        message,
                    })
                }
                Err(other) => problems.push(other),
            }
        }
        self.plugins
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        problems
    }

    /// The plugins found, by name.
    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugins
    }

    /// A plugin by id.
    pub fn plugin(&self, id: &str) -> Option<&PluginInfo> {
        self.plugins.iter().find(|p| p.manifest.id == id)
    }

    /// Calls a method on a plugin, starting it if needed. Phase 0: checks
    /// that the plugin exists and is enabled, then says the call is not
    /// built.
    pub fn call(
        &mut self,
        plugin: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, PluginError> {
        let info = self
            .plugin(plugin)
            .ok_or_else(|| PluginError::Unknown(plugin.to_owned()))?;
        if !info.enabled {
            return Err(PluginError::NotEnabled(plugin.to_owned()));
        }
        let _ = (method, params);
        Err(PluginError::NotBuilt("calling plugins".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_plugins_folder_finds_nothing() {
        let mut host = Host::new(&PluginSettings::default());
        let nowhere = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-plugins-folder");
        assert!(host.discover(&nowhere).is_empty());
        assert!(host.plugins().is_empty());
        let err = host.call("thumbs", "command/run", Value::Null).unwrap_err();
        assert_eq!(err.to_string(), "no plugin is called thumbs");
        assert_eq!(host.timeout(), Duration::from_secs(10));
    }
}
