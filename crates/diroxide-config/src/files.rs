//! The other configuration files: keymap, file types, banks, conversions
//! and tools.

use std::collections::BTreeMap;

use diroxide_core::{CommandId, KeyChord, Layer};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths::Paths;

/// One override in `keymap.toml`: a chord in a layer bound to a command,
/// or unbound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyOverride {
    /// The layer the binding applies in.
    pub layer: Layer,
    /// The key or two-key chord.
    pub chord: KeyChord,
    /// The command, or `None` to remove the default binding.
    pub command: Option<CommandId>,
}

/// `keymap.toml`: one table per layer, chord to command id; the command
/// `none` removes a default binding.
///
/// ```toml
/// [panel]
/// "ctrl+x ctrl+f" = "go.find"
/// "f1" = "none"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapConfig {
    /// The overrides, in file order within each layer.
    pub overrides: Vec<KeyOverride>,
}

impl KeymapConfig {
    /// The command that removes a binding.
    pub const UNBIND: &'static str = "none";

    /// Reads `keymap.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let tables: BTreeMap<String, BTreeMap<String, String>> =
            toml::from_str(text).map_err(|e| ConfigError::parse(Paths::KEYMAP, e))?;
        let mut overrides = Vec::new();
        for (layer_name, bindings) in tables {
            let layer = Layer::ALL
                .into_iter()
                .find(|layer| layer.word() == layer_name)
                .ok_or_else(|| {
                    ConfigError::bad_entry(Paths::KEYMAP, format!("unknown layer [{layer_name}]"))
                })?;
            for (keys, command) in bindings {
                let chord = KeyChord::parse(&keys).map_err(|e| {
                    ConfigError::bad_entry(Paths::KEYMAP, format!("[{layer_name}] {e}"))
                })?;
                let command = if command == Self::UNBIND {
                    None
                } else {
                    Some(CommandId::new(command).map_err(|e| {
                        ConfigError::bad_entry(Paths::KEYMAP, format!("[{layer_name}] {e}"))
                    })?)
                };
                overrides.push(KeyOverride {
                    layer,
                    chord,
                    command,
                });
            }
        }
        Ok(Self { overrides })
    }
}

/// An action of a file type: a command, a registered tool, or a plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ActionConfig {
    /// The action's name, as listed: "Open", "View", "Make thumbnail".
    pub name: String,
    /// A command id to run.
    pub command: Option<String>,
    /// A tool id from the tools registry, run with `args`.
    pub tool: Option<String>,
    /// Arguments for the tool, with placeholders such as `{file}`.
    pub args: Vec<String>,
    /// A plugin to call.
    pub plugin: Option<String>,
}

/// One file type in `filetypes.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FileTypeConfig {
    /// A stable id, such as `pdf`.
    pub id: String,
    /// The name said in details: "PDF document".
    pub name: String,
    /// The short name for rows: "PDF".
    pub short: String,
    /// The type this one refines, for subtypes.
    pub parent: Option<String>,
    /// Extensions, without the dot, lowercase.
    pub extensions: Vec<String>,
    /// Name patterns (globs).
    pub patterns: Vec<String>,
    /// Higher priorities are checked first.
    pub priority: i32,
    /// The actions, the first being the default.
    pub actions: Vec<ActionConfig>,
}

/// `filetypes.toml`: user file types, added to the built-in ones.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileTypesConfig {
    /// The types, as `[[type]]` tables.
    #[serde(rename = "type")]
    pub types: Vec<FileTypeConfig>,
}

impl FileTypesConfig {
    /// Reads `filetypes.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::FILETYPES, e))
    }
}

/// One button in a bank.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ButtonConfig {
    /// The label, said and shown.
    pub label: String,
    /// The command ids it runs, in order.
    pub commands: Vec<String>,
}

/// A named bank of buttons.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct BankConfig {
    /// The bank's name.
    pub name: String,
    /// The buttons, as `[[bank.button]]` tables.
    #[serde(rename = "button")]
    pub buttons: Vec<ButtonConfig>,
}

/// `banks.toml`: the button banks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BanksConfig {
    /// The banks, as `[[bank]]` tables.
    #[serde(rename = "bank")]
    pub banks: Vec<BankConfig>,
}

impl BanksConfig {
    /// Reads `banks.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::BANKS, e))
    }
}

/// One conversion recipe.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RecipeConfig {
    /// The recipe's name, as listed: "PNG to JPEG".
    pub name: String,
    /// The extensions it takes, lowercase, without the dot.
    pub from: Vec<String>,
    /// The extension it makes.
    pub to: String,
    /// The tool id that runs it.
    pub tool: String,
    /// The arguments, with `{input}` and `{output}` placeholders.
    pub args: Vec<String>,
}

/// `conversions.toml`: the user's conversion recipes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversionsConfig {
    /// The recipes, as `[[recipe]]` tables.
    #[serde(rename = "recipe")]
    pub recipes: Vec<RecipeConfig>,
}

impl ConversionsConfig {
    /// Reads `conversions.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::CONVERSIONS, e))
    }
}

/// An override for one external tool.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ToolOverride {
    /// The program's full path, instead of looking on PATH.
    pub path: Option<String>,
    /// Never use this tool.
    pub disabled: bool,
}

/// `tools.toml`: one table per tool id.
///
/// ```toml
/// [pandoc]
/// path = "D:\\tools\\pandoc.exe"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolsConfig {
    /// The overrides by tool id.
    pub tools: BTreeMap<String, ToolOverride>,
}

impl ToolsConfig {
    /// Reads `tools.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::TOOLS, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_overrides_parse_and_unbind() {
        let config =
            KeymapConfig::from_toml("[panel]\n\"ctrl+x ctrl+f\" = \"go.find\"\nf1 = \"none\"\n")
                .unwrap();
        assert_eq!(config.overrides.len(), 2);
        let find = config
            .overrides
            .iter()
            .find(|o| o.command.is_some())
            .unwrap();
        assert_eq!(find.layer, Layer::Panel);
        assert_eq!(find.chord.to_string(), "Ctrl+X Ctrl+F");
        assert!(config.overrides.iter().any(|o| o.command.is_none()));
    }

    #[test]
    fn keymap_mistakes_are_named() {
        let err = KeymapConfig::from_toml("[nowhere]\nf1 = \"app.quit\"\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "keymap.toml has a bad entry: unknown layer [nowhere]"
        );
        let err = KeymapConfig::from_toml("[panel]\n\"hyper+q\" = \"app.quit\"\n").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("keymap.toml has a bad entry: [panel]")
        );
    }

    #[test]
    fn the_other_files_parse() {
        let types = FileTypesConfig::from_toml(
            "[[type]]\nid = \"md\"\nname = \"Markdown\"\nshort = \"MD\"\nextensions = [\"md\"]\n[[type.actions]]\nname = \"View\"\ncommand = \"file.view\"\n",
        )
        .unwrap();
        assert_eq!(
            types.types[0].actions[0].command.as_deref(),
            Some("file.view")
        );
        let banks = BanksConfig::from_toml(
            "[[bank]]\nname = \"Main\"\n[[bank.button]]\nlabel = \"Copy\"\ncommands = [\"file.copy\"]\n",
        )
        .unwrap();
        assert_eq!(banks.banks[0].buttons[0].label, "Copy");
        let tools = ToolsConfig::from_toml("[pandoc]\npath = \"/opt/pandoc\"\n").unwrap();
        assert_eq!(tools.tools["pandoc"].path.as_deref(), Some("/opt/pandoc"));
        let recipes = ConversionsConfig::from_toml(
            "[[recipe]]\nname = \"PNG to JPEG\"\nfrom = [\"png\"]\nto = \"jpg\"\ntool = \"magick\"\nargs = [\"{input}\", \"{output}\"]\n",
        )
        .unwrap();
        assert_eq!(recipes.recipes[0].to, "jpg");
    }
}
