//! Where the configuration and the state live.

use std::env;
use std::path::{Path, PathBuf};

/// The config and state folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// Settings, keymap, file types, banks, conversions, tools and plugins.
    pub config_dir: PathBuf,
    /// Bookmarks, history, labels, collections, the undo log and plugin
    /// logs.
    pub state_dir: PathBuf,
}

impl Paths {
    /// The folder's name under the platform's config and state folders.
    pub const FOLDER: &'static str = "diroxide";
    /// The settings file.
    pub const SETTINGS: &'static str = "config.toml";
    /// The keymap overrides.
    pub const KEYMAP: &'static str = "keymap.toml";
    /// The file types.
    pub const FILETYPES: &'static str = "filetypes.toml";
    /// The button banks.
    pub const BANKS: &'static str = "banks.toml";
    /// The conversion recipes.
    pub const CONVERSIONS: &'static str = "conversions.toml";
    /// The external tool overrides.
    pub const TOOLS: &'static str = "tools.toml";
    /// The folder of plugins, under the config folder.
    pub const PLUGINS: &'static str = "plugins";

    /// Paths from two folders, as tests and `--config` give them.
    pub fn new(config_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            config_dir,
            state_dir,
        }
    }

    /// The `--portable` layout: `config` and `state` folders beside the
    /// program.
    pub fn portable(beside: &Path) -> Self {
        Self::new(beside.join("config"), beside.join("state"))
    }

    /// The platform's folders: `%APPDATA%\diroxide` and
    /// `%LOCALAPPDATA%\diroxide` on Windows, `~/Library/Application
    /// Support/diroxide` on macOS, the XDG folders elsewhere. `None` when
    /// the environment does not say. Phase 0 reads the environment
    /// directly; W1c moves this to the `dirs` crate.
    pub fn from_environment() -> Option<Self> {
        let var = |name: &str| {
            env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let (config, state) = if cfg!(windows) {
            (var("APPDATA")?, var("LOCALAPPDATA")?)
        } else if cfg!(target_os = "macos") {
            let support = var("HOME")?.join("Library").join("Application Support");
            (support.clone(), support)
        } else {
            let home = var("HOME");
            let config =
                var("XDG_CONFIG_HOME").or_else(|| home.clone().map(|h| h.join(".config")))?;
            let state =
                var("XDG_STATE_HOME").or_else(|| home.map(|h| h.join(".local").join("state")))?;
            (config, state)
        };
        Some(Self::new(
            config.join(Self::FOLDER),
            state.join(Self::FOLDER),
        ))
    }

    /// A file in the config folder.
    pub fn config_file(&self, name: &str) -> PathBuf {
        self.config_dir.join(name)
    }

    /// A file in the state folder.
    pub fn state_file(&self, name: &str) -> PathBuf {
        self.state_dir.join(name)
    }

    /// The plugins folder.
    pub fn plugins_dir(&self) -> PathBuf {
        self.config_dir.join(Self::PLUGINS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_paths_sit_beside_the_program() {
        let paths = Paths::portable(Path::new("program"));
        assert_eq!(
            paths.config_file("config.toml"),
            Path::new("program").join("config").join("config.toml")
        );
        assert_eq!(
            paths.plugins_dir(),
            Path::new("program").join("config").join("plugins")
        );
        assert_eq!(
            paths.state_file("undo.log"),
            Path::new("program").join("state").join("undo.log")
        );
    }
}
