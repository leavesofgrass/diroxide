//! `plugin.toml`: what a plugin is, what it provides, and what it may do.
//!
//! ```toml
//! name = "thumbs"
//! version = "0.1.0"
//! description = "Makes thumbnails of images"
//! program = ["python3", "thumbs.py"]
//! capabilities = ["fs", "tools"]
//! hooks = ["after-copy"]
//!
//! [[command]]
//! id = "thumbs.make"
//! name = "Make thumbnails"
//! key = "ctrl+alt+t"
//! ```

use diroxide_core::CommandId;
use serde::{Deserialize, Serialize};

use crate::error::PluginError;

/// What a plugin may ask the host to do. Shown in words on first enable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Say things and ask questions.
    Ui,
    /// Read and change the panels.
    Panel,
    /// Read, write, copy, move and delete files through the job engine.
    Fs,
    /// Run registered tools.
    Tools,
    /// Read settings.
    Config,
}

impl Capability {
    /// The capability as the enable dialog says it.
    pub const fn describe(self) -> &'static str {
        match self {
            Capability::Ui => "speak and ask questions",
            Capability::Panel => "read and change the panels",
            Capability::Fs => "read, write and delete files",
            Capability::Tools => "run external tools",
            Capability::Config => "read settings",
        }
    }
}

/// An event a plugin can hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookKind {
    /// Before a copy runs.
    BeforeCopy,
    /// After a copy ran.
    AfterCopy,
    /// Before a delete or trash runs.
    BeforeDelete,
    /// After a delete or trash ran.
    AfterDelete,
    /// Before a rename runs.
    BeforeRename,
    /// After a rename ran.
    AfterRename,
    /// A panel entered a folder.
    EnterFolder,
    /// The selection changed.
    Select,
}

/// A command a plugin adds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CommandSpec {
    /// The command's id, such as `thumbs.make`.
    pub id: String,
    /// Its name in menus and the palette.
    pub name: String,
    /// One sentence for F1 help.
    #[serde(default)]
    pub description: String,
    /// A default key, such as `ctrl+alt+t`.
    #[serde(default)]
    pub key: Option<String>,
}

/// A plugin's manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    /// The plugin's name: letters, digits, `-` and `_`.
    pub name: String,
    /// Its version.
    pub version: String,
    /// One sentence about it.
    #[serde(default)]
    pub description: String,
    /// The program and its arguments, run in the plugin's folder.
    pub program: Vec<String>,
    /// What it may ask the host to do.
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// The commands it adds, as `[[command]]` tables.
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandSpec>,
    /// Schemes it serves as a provider, such as `s3`.
    #[serde(default)]
    pub providers: Vec<String>,
    /// File type ids it previews.
    #[serde(default)]
    pub previewers: Vec<String>,
    /// Conversion recipe names it runs.
    #[serde(default)]
    pub converters: Vec<String>,
    /// Events it hooks.
    #[serde(default)]
    pub hooks: Vec<HookKind>,
}

impl Manifest {
    /// The manifest's file name in a plugin's folder.
    pub const FILE: &'static str = "plugin.toml";

    /// Reads and checks a manifest.
    pub fn from_toml(text: &str) -> Result<Self, PluginError> {
        let bad = |message: String| PluginError::Manifest {
            plugin: String::new(),
            message,
        };
        let manifest: Self = toml::from_str(text).map_err(|e| bad(e.message().to_owned()))?;
        let plain = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if manifest.name.is_empty() || !manifest.name.chars().all(plain) {
            return Err(bad(format!("the name \"{}\" is not plain", manifest.name)));
        }
        if manifest.program.is_empty() {
            return Err(bad("program is empty".to_owned()));
        }
        for command in &manifest.commands {
            if !CommandId::is_valid(&command.id) {
                return Err(bad(format!("\"{}\" is not a command id", command.id)));
            }
        }
        Ok(manifest)
    }

    /// The capabilities in words, for the enable dialog: "This plugin can
    /// read, write and delete files; run external tools."
    pub fn capabilities_in_words(&self) -> String {
        if self.capabilities.is_empty() {
            return "This plugin asks for nothing beyond its own commands.".to_owned();
        }
        let words: Vec<&str> = self.capabilities.iter().map(|c| c.describe()).collect();
        format!("This plugin can {}.", words.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THUMBS: &str = r#"
name = "thumbs"
version = "0.1.0"
program = ["python3", "thumbs.py"]
capabilities = ["fs", "tools"]
hooks = ["after-copy"]

[[command]]
id = "thumbs.make"
name = "Make thumbnails"
key = "ctrl+alt+t"
"#;

    #[test]
    fn a_manifest_reads() {
        let manifest = Manifest::from_toml(THUMBS).unwrap();
        assert_eq!(manifest.commands[0].name, "Make thumbnails");
        assert_eq!(manifest.hooks, vec![HookKind::AfterCopy]);
        assert_eq!(
            manifest.capabilities_in_words(),
            "This plugin can read, write and delete files; run external tools."
        );
    }

    #[test]
    fn bad_manifests_are_refused() {
        assert!(Manifest::from_toml("name = \"x\"").is_err());
        let spaced = THUMBS.replace("name = \"thumbs\"", "name = \"my thumbs\"");
        assert!(Manifest::from_toml(&spaced).is_err());
        let bad_id = THUMBS.replace("thumbs.make", "Thumbs Make");
        assert!(Manifest::from_toml(&bad_id).is_err());
    }
}
