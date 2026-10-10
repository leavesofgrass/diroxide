//! `config.toml`: the settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths::Paths;

/// How a panel sorts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortKey {
    /// By name, naturally ("file2" before "file10").
    #[default]
    Name,
    /// By size.
    Size,
    /// By modification time.
    Modified,
    /// By file type.
    Type,
    /// By extension.
    Extension,
}

/// Whether the terminal frontend behaves for a screen reader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenReaderMode {
    /// Detect a running screen reader.
    #[default]
    Auto,
    /// Always behave for a screen reader.
    On,
    /// Never.
    Off,
}

/// The plugins section of `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PluginSettings {
    /// Plugins the user enabled, by name. Nothing is enabled by default.
    pub enabled: Vec<String>,
    /// How long a plugin may take to answer, in milliseconds.
    pub timeout_ms: u64,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            timeout_ms: 10_000,
        }
    }
}

/// The settings in `config.toml`. Every field has a default, so an empty
/// file is valid; keys this version does not know are kept in `unknown`
/// and written back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    /// Show hidden entries.
    pub show_hidden: bool,
    /// List folders before files.
    pub folders_first: bool,
    /// The sort key for new panels.
    pub sort: SortKey,
    /// The columns of a panel row, in order, after the name.
    pub columns: Vec<String>,
    /// Single-letter shortcuts (WCAG 2.1.4 lets them be turned off).
    pub single_letter_keys: bool,
    /// Screen reader behavior of the terminal frontend.
    pub screen_reader: ScreenReaderMode,
    /// Draw as little as possible, for screen readers that read changes.
    pub quiet_screen: bool,
    /// The theme: `plain` (no box drawing) or `lines`.
    pub theme: String,
    /// The shortest time between two progress sentences, in milliseconds.
    pub progress_interval_ms: u64,
    /// The plugins section.
    pub plugins: PluginSettings,
    /// Keys this version does not know, kept as they were.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, toml::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            folders_first: true,
            sort: SortKey::Name,
            columns: vec!["type".to_owned(), "size".to_owned(), "modified".to_owned()],
            single_letter_keys: true,
            screen_reader: ScreenReaderMode::Auto,
            quiet_screen: false,
            theme: "plain".to_owned(),
            progress_interval_ms: 500,
            plugins: PluginSettings::default(),
            unknown: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Reads settings from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::SETTINGS, e))
    }

    /// Writes settings as TOML text.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(|e| ConfigError::parse(Paths::SETTINGS, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_gives_the_defaults() {
        assert_eq!(Settings::from_toml("").unwrap(), Settings::default());
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let text = "show-hidden = true\nfuture-option = 3\n[plugins]\nenabled = [\"thumbs\"]\n";
        let settings = Settings::from_toml(text).unwrap();
        assert!(settings.show_hidden);
        assert_eq!(settings.plugins.enabled, vec!["thumbs".to_owned()]);
        assert_eq!(
            settings.unknown.get("future-option"),
            Some(&toml::Value::Integer(3))
        );
        let again = Settings::from_toml(&settings.to_toml().unwrap()).unwrap();
        assert_eq!(again, settings);
    }

    #[test]
    fn a_wrong_type_is_an_error_with_words() {
        let err = Settings::from_toml("show-hidden = \"yes\"").unwrap_err();
        assert!(err.to_string().contains("could not be read"));
    }
}
