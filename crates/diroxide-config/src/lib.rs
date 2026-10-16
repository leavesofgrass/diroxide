//! The configuration of diroxide: plain TOML files a person can read and
//! diff, in the `diroxide` folder under the platform's config directory.
//!
//! - `config.toml`: [`Settings`], including the plugins section.
//! - `keymap.toml`: [`KeymapConfig`], the user's keys: command to key, in a
//!   table per column (terminal, window) and layer.
//! - `filetypes.toml`: [`FileTypesConfig`].
//! - `banks.toml`: [`BanksConfig`], the button banks.
//! - `conversions.toml`: [`ConversionsConfig`], the conversion recipes.
//! - `tools.toml`: [`ToolsConfig`], overrides for external tools.
//! - `session.toml`, in the state folder: [`SessionState`], what the next
//!   start restores.
//!
//! Missing files mean defaults. Unknown keys in `config.toml` are kept and
//! written back. Writes are atomic and happen only on an explicit change.
//!
//! Phase 0: the shapes are the contract; loading and saving are simple.
//! Wave 1 (W1c) owns this crate, adds the settings schema and the generated
//! reference page, and keeps unknown keys in every file.

pub mod error;
pub mod files;
pub mod paths;
pub mod session;
pub mod settings;

pub use error::ConfigError;
pub use files::{
    ActionConfig, BankConfig, BanksConfig, ButtonConfig, ConversionsConfig, FileTypeConfig,
    FileTypesConfig, KeyOverride, KeymapConfig, RecipeConfig, ToolOverride, ToolsConfig,
};
pub use paths::Paths;
pub use session::{Layout, PanelState, SessionState, load_session, save_session};
pub use settings::{PluginSettings, ScreenReaderMode, Settings, SortKey};

use std::fs;
use std::io;
use std::path::Path;

/// Everything read from the config folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// `config.toml`.
    pub settings: Settings,
    /// `keymap.toml`.
    pub keymap: KeymapConfig,
    /// `filetypes.toml`.
    pub filetypes: FileTypesConfig,
    /// `banks.toml`.
    pub banks: BanksConfig,
    /// `conversions.toml`.
    pub conversions: ConversionsConfig,
    /// `tools.toml`.
    pub tools: ToolsConfig,
}

/// Reads every file in the config folder; a missing file gives defaults.
pub fn load(paths: &Paths) -> Result<Config, ConfigError> {
    Ok(Config {
        settings: read_optional(paths, Paths::SETTINGS, Settings::from_toml)?,
        keymap: read_optional(paths, Paths::KEYMAP, KeymapConfig::from_toml)?,
        filetypes: read_optional(paths, Paths::FILETYPES, FileTypesConfig::from_toml)?,
        banks: read_optional(paths, Paths::BANKS, BanksConfig::from_toml)?,
        conversions: read_optional(paths, Paths::CONVERSIONS, ConversionsConfig::from_toml)?,
        tools: read_optional(paths, Paths::TOOLS, ToolsConfig::from_toml)?,
    })
}

/// Writes `config.toml` atomically: to a temporary file beside it, then a
/// rename over the old one.
pub fn save_settings(paths: &Paths, settings: &Settings) -> Result<(), ConfigError> {
    let text = settings.to_toml()?;
    write_atomic(&paths.config_file(Paths::SETTINGS), Paths::SETTINGS, &text)
}

fn read_optional<T: Default>(
    paths: &Paths,
    name: &str,
    parse: impl Fn(&str) -> Result<T, ConfigError>,
) -> Result<T, ConfigError> {
    match fs::read_to_string(paths.config_file(name)) {
        Ok(text) => parse(&text).map_err(|e| e.in_file(name)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(ConfigError::io(name, &e)),
    }
}

fn write_atomic(path: &Path, name: &str, text: &str) -> Result<(), ConfigError> {
    let temporary = path.with_extension("toml.new");
    fs::write(&temporary, text).map_err(|e| ConfigError::io(name, &e))?;
    fs::rename(&temporary, path).map_err(|e| ConfigError::io(name, &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_folder_gives_defaults() {
        let nowhere = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-config-folder");
        let paths = Paths::new(nowhere.clone(), nowhere);
        assert_eq!(load(&paths).unwrap(), Config::default());
    }
}
