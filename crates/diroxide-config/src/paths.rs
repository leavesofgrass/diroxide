//! Where the configuration and the state live.
//!
//! The config folder holds what the owner writes: `config.toml`,
//! `keymap.toml`, `menus.toml`, `filetypes.toml`, `banks.toml`,
//! `conversions.toml`, `tools.toml`, the `themes` and `plugins` folders,
//! and the two read-only references diroxide writes beside the owner's
//! files (`keymap-defaults.toml`, `menus-defaults.toml`). The state folder
//! holds what diroxide remembers: the session, the prompt histories, the
//! undo journal, the hot folders' logs and the optional components.
//!
//! The folders come from the `dirs` crate: `%APPDATA%\diroxide` and
//! `%LOCALAPPDATA%\diroxide` on Windows, `~/Library/Application
//! Support/diroxide` on macOS, the XDG config and state folders elsewhere.
//! `--portable` keeps both beside the program instead, and tests pass
//! their own folders to [`Paths::new`], so no test touches the real ones.

use std::env;
use std::path::{Path, PathBuf};

/// The config and state folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// Settings, keymap, menus, file types, banks, conversions, tools,
    /// themes and plugins.
    pub config_dir: PathBuf,
    /// The session, histories, the undo journal, plugin logs and
    /// components.
    pub state_dir: PathBuf,
}

impl Paths {
    /// The folder's name under the platform's config and state folders.
    pub const FOLDER: &'static str = "diroxide";
    /// The settings file.
    pub const SETTINGS: &'static str = "config.toml";
    /// The owner's keys.
    pub const KEYMAP: &'static str = "keymap.toml";
    /// The built-in keys, written beside `keymap.toml` as a read-only
    /// reference.
    pub const KEYMAP_DEFAULTS: &'static str = "keymap-defaults.toml";
    /// The owner's menus.
    pub const MENUS: &'static str = "menus.toml";
    /// The built-in menus, written beside `menus.toml` as a read-only
    /// reference.
    pub const MENUS_DEFAULTS: &'static str = "menus-defaults.toml";
    /// The file types.
    pub const FILETYPES: &'static str = "filetypes.toml";
    /// The button banks.
    pub const BANKS: &'static str = "banks.toml";
    /// The conversion recipes.
    pub const CONVERSIONS: &'static str = "conversions.toml";
    /// The external tool overrides.
    pub const TOOLS: &'static str = "tools.toml";
    /// The hot folders' rules (reserved now; alpha.7 reads it).
    pub const HOTFOLDERS: &'static str = "hotfolders.toml";
    /// The folder of plugins, under the config folder.
    pub const PLUGINS: &'static str = "plugins";
    /// The folder of the owner's theme files, under the config folder.
    pub const THEMES: &'static str = "themes";
    /// The session restored at the next start, in the state folder.
    pub const SESSION: &'static str = "session.toml";
    /// The prompts' histories, one file per prompt, in the state folder.
    pub const HISTORY: &'static str = "history";
    /// The hot folders' logs and queue, in the state folder (reserved now).
    pub const HOT: &'static str = "hot";
    /// Optional components, in the state folder (reserved now; alpha.4).
    pub const COMPONENTS: &'static str = "components";
    /// The archive provider's spool for inner archives too large for
    /// memory, in the state folder (alpha.2; `archives.spool_folder`).
    pub const SPOOL: &'static str = "spool";
    /// The config folder's name in the `--portable` layout.
    pub const PORTABLE_CONFIG: &'static str = "config";
    /// The state folder's name in the `--portable` layout.
    pub const PORTABLE_STATE: &'static str = "state";

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
        Self::new(
            beside.join(Self::PORTABLE_CONFIG),
            beside.join(Self::PORTABLE_STATE),
        )
    }

    /// The `--portable` layout beside the running program, or `None` when
    /// the program's own folder cannot be found.
    pub fn beside_program() -> Option<Self> {
        let program = env::current_exe().ok()?;
        program.parent().map(Self::portable)
    }

    /// The platform's folders, from the `dirs` crate: the config folder,
    /// and the state folder where the platform has one (Linux), else the
    /// local data folder (Windows, macOS). `None` when the system does not
    /// say where they are.
    pub fn from_environment() -> Option<Self> {
        Self::from_bases(
            dirs::config_dir(),
            dirs::state_dir().or_else(dirs::data_local_dir),
        )
    }

    /// The `diroxide` folders under two base folders; `None` when either is
    /// missing. [`Paths::from_environment`] passes the platform's.
    pub fn from_bases(config: Option<PathBuf>, state: Option<PathBuf>) -> Option<Self> {
        Some(Self::new(
            config?.join(Self::FOLDER),
            state?.join(Self::FOLDER),
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

    /// The owner's themes folder.
    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join(Self::THEMES)
    }

    /// The prompts' history folder.
    pub fn history_dir(&self) -> PathBuf {
        self.state_dir.join(Self::HISTORY)
    }

    /// The hot folders' state folder.
    pub fn hot_dir(&self) -> PathBuf {
        self.state_dir.join(Self::HOT)
    }

    /// The optional components' folder.
    pub fn components_dir(&self) -> PathBuf {
        self.state_dir.join(Self::COMPONENTS)
    }

    /// The archive provider's default spool folder, the state folder's
    /// `spool`: never the system's temporary folder (W2a's request 3).
    pub fn spool_dir(&self) -> PathBuf {
        self.state_dir.join(Self::SPOOL)
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
        assert_eq!(
            paths.themes_dir(),
            Path::new("program").join("config").join("themes")
        );
        assert_eq!(
            paths.history_dir(),
            Path::new("program").join("state").join("history")
        );
    }

    #[test]
    fn the_folders_are_named_diroxide_under_their_bases() {
        let paths =
            Paths::from_bases(Some(PathBuf::from("cfg")), Some(PathBuf::from("st"))).unwrap();
        assert_eq!(paths.config_dir, Path::new("cfg").join("diroxide"));
        assert_eq!(paths.state_dir, Path::new("st").join("diroxide"));
        assert_eq!(Paths::from_bases(None, Some(PathBuf::from("st"))), None);
        assert_eq!(Paths::from_bases(Some(PathBuf::from("cfg")), None), None);
    }

    #[test]
    fn every_reserved_name_is_plain() {
        for name in [
            Paths::SETTINGS,
            Paths::KEYMAP,
            Paths::KEYMAP_DEFAULTS,
            Paths::MENUS,
            Paths::MENUS_DEFAULTS,
            Paths::FILETYPES,
            Paths::BANKS,
            Paths::CONVERSIONS,
            Paths::TOOLS,
            Paths::HOTFOLDERS,
            Paths::PLUGINS,
            Paths::THEMES,
            Paths::SESSION,
            Paths::HISTORY,
            Paths::HOT,
            Paths::COMPONENTS,
            Paths::SPOOL,
        ] {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c)),
                "{name}"
            );
        }
    }
}
