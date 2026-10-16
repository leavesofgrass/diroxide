//! The other configuration files: keymap, file types, banks, conversions
//! and tools.

use std::collections::BTreeMap;

use diroxide_core::{CommandId, Key, KeyColumn, Layer};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::error::ConfigError;
use crate::paths::Paths;

/// One line of `keymap.toml`: a command's keys in one column and layer.
/// The keys replace the command's default keys there; an empty list
/// ("none") unbinds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyOverride {
    /// The column: the terminal's keys or the window's.
    pub column: KeyColumn,
    /// The layer the binding applies in.
    pub layer: Layer,
    /// The command.
    pub command: CommandId,
    /// Its keys, each one key with its modifiers; empty to unbind.
    pub keys: Vec<Key>,
}

/// `keymap.toml`: command to key, one line per command, in a table per
/// column and layer (ADR-0005). The value is one key, a list of keys, or
/// `none` to unbind. Key sequences are refused.
///
/// ```toml
/// [terminal.panel]
/// "file.copy" = "f5"
/// "file.trash" = ["delete", "f8"]
/// "view.sort-list" = "none"
///
/// [window.panel]
/// "view.sort-list" = "ctrl+shift+s"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapConfig {
    /// The lines, in file order within each table.
    pub overrides: Vec<KeyOverride>,
}

impl KeymapConfig {
    /// The value that removes a command's binding.
    pub const UNBIND: &'static str = "none";

    /// Reads `keymap.toml` text. Phase 0 stops at the first mistake; W1c
    /// reports every unknown command or key in words and drops none.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let bad = |what: String| ConfigError::bad_entry(Paths::KEYMAP, what);
        let table: Table =
            toml::from_str(text).map_err(|e| ConfigError::parse(Paths::KEYMAP, e))?;
        let mut overrides = Vec::new();
        for (column_name, layers) in &table {
            let column = KeyColumn::ALL
                .into_iter()
                .find(|column| column.word() == column_name)
                .ok_or_else(|| bad(format!("unknown column [{column_name}]")))?;
            let layers = layers
                .as_table()
                .ok_or_else(|| bad(format!("[{column_name}] is not a table of layers")))?;
            for (layer_name, lines) in layers {
                let where_ = format!("[{column_name}.{layer_name}]");
                let layer = Layer::ALL
                    .into_iter()
                    .find(|layer| layer.word() == layer_name)
                    .ok_or_else(|| bad(format!("unknown layer {where_}")))?;
                let lines = lines
                    .as_table()
                    .ok_or_else(|| bad(format!("{where_} is not a table of commands")))?;
                for (command, value) in lines {
                    let command = CommandId::new(command.clone())
                        .map_err(|e| bad(format!("{where_} {e}")))?;
                    let texts: Vec<&str> = match value {
                        Value::String(text) => vec![text.as_str()],
                        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
                        _ => return Err(bad(format!("{where_} {command} has no key text"))),
                    };
                    let mut keys = Vec::new();
                    for text in texts {
                        if text.trim() == Self::UNBIND {
                            continue;
                        }
                        keys.push(Key::parse(text).map_err(|e| bad(format!("{where_} {e}")))?);
                    }
                    overrides.push(KeyOverride {
                        column,
                        layer,
                        command,
                        keys,
                    });
                }
            }
        }
        Ok(Self { overrides })
    }

    /// The lines of one column.
    pub fn column(&self, column: KeyColumn) -> impl Iterator<Item = &KeyOverride> {
        self.overrides.iter().filter(move |o| o.column == column)
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
    /// The media type, such as `application/pdf`.
    pub mime: Option<String>,
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
    fn keymap_lines_are_command_to_key() {
        let config = KeymapConfig::from_toml(
            "[terminal.panel]
\"file.copy\" = \"f5\"
\"file.trash\" = [\"delete\", \"f8\"]
\"app.help\" = \"none\"
[window.panel]
\"app.help\" = \"f1\"
",
        )
        .unwrap();
        assert_eq!(config.overrides.len(), 4);
        let trash = config
            .column(KeyColumn::Terminal)
            .find(|o| o.command.as_str() == "file.trash")
            .unwrap();
        assert_eq!(trash.layer, Layer::Panel);
        assert_eq!(trash.keys.len(), 2);
        assert_eq!(trash.keys[1].to_string(), "F8");
        let help: Vec<&KeyOverride> = config
            .overrides
            .iter()
            .filter(|o| o.command.as_str() == "app.help")
            .collect();
        assert!(
            help.iter()
                .any(|o| o.column == KeyColumn::Terminal && o.keys.is_empty())
        );
        assert!(
            help.iter()
                .any(|o| o.column == KeyColumn::Window && o.keys.len() == 1)
        );
    }

    #[test]
    fn keymap_mistakes_are_named() {
        let err = KeymapConfig::from_toml(
            "[terminal.nowhere]
\"app.quit\" = \"f1\"
",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "keymap.toml has a bad entry: unknown layer [terminal.nowhere]"
        );
        let err = KeymapConfig::from_toml(
            "[tv.panel]
\"app.quit\" = \"f1\"
",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "keymap.toml has a bad entry: unknown column [tv]"
        );
        let err = KeymapConfig::from_toml(
            "[terminal.panel]
\"app.quit\" = \"hyper+q\"
",
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("keymap.toml has a bad entry: [terminal.panel]")
        );
        let err = KeymapConfig::from_toml(
            "[terminal.panel]
\"go.find\" = \"ctrl+x ctrl+f\"
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("two keys in a row"));
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
