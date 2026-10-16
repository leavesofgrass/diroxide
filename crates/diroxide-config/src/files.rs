//! The other configuration files: keymap, menus, file types, banks,
//! conversions and tools.

use std::collections::BTreeMap;

use diroxide_core::{CommandId, CoreError, Key, KeyColumn, Layer};
use serde::{Deserialize, Serialize};
use toml_edit::{Document, Item};

use crate::error::ConfigError;
use crate::paths::Paths;
use crate::store::{ConfigProblem, line_of};

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
    /// The line in the file, counted from 1; 0 when the line came from
    /// code rather than a file.
    pub line: usize,
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
///
/// Reading never stops at a mistake: every line that can be used is
/// used, and every one that cannot is said in [`KeymapConfig::problems`],
/// meaning first, with its line ("Unknown key: hyper+q for app.quit, line
/// 4."). Whether a command id names a real command is the app's check,
/// since the catalog lives there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapConfig {
    /// The lines, in file order.
    pub overrides: Vec<KeyOverride>,
    /// What could not be used, in words, in file order.
    pub problems: Vec<ConfigProblem>,
}

fn key_problem(error: &CoreError, text: &str, command: &str) -> String {
    match error {
        CoreError::KeySequence(_) => {
            format!("Two keys in a row: {text} for {command}; use one key")
        }
        _ => format!("Unknown key: {text} for {command}"),
    }
}

impl KeymapConfig {
    /// The value that removes a command's binding.
    pub const UNBIND: &'static str = "none";

    /// Reads `keymap.toml` text. Only text that is not TOML is an error;
    /// every other mistake is collected in words and the rest is used.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let file = Paths::KEYMAP;
        let document = Document::parse(text).map_err(|e| ConfigError::parse(file, e))?;
        let mut config = Self::default();
        let problem =
            |line: usize, what: String| ConfigProblem::new(file, format!("{what}, line {line}."));
        let line_at =
            |span: Option<std::ops::Range<usize>>| span.map_or(0, |s| line_of(text, s.start));
        for (column_name, layers) in document.as_table().iter() {
            let column_line = line_at(
                document
                    .as_table()
                    .get_key_value(column_name)
                    .and_then(|(key, _)| key.span()),
            );
            let Some(column) = KeyColumn::ALL.into_iter().find(|c| c.word() == column_name) else {
                config.problems.push(problem(
                    column_line,
                    format!("Unknown column: [{column_name}]; use terminal or window"),
                ));
                continue;
            };
            let Some(layers) = layers.as_table_like() else {
                config.problems.push(problem(
                    column_line,
                    format!("No layers in [{column_name}]: write [{column_name}.panel]"),
                ));
                continue;
            };
            for (layer_name, lines) in layers.iter() {
                let layer_line = line_at(
                    layers
                        .get_key_value(layer_name)
                        .and_then(|(key, _)| key.span()),
                );
                let Some(layer) = Layer::ALL.into_iter().find(|l| l.word() == layer_name) else {
                    config.problems.push(problem(
                        layer_line,
                        format!("Unknown layer: [{column_name}.{layer_name}]"),
                    ));
                    continue;
                };
                let Some(lines) = lines.as_table_like() else {
                    config.problems.push(problem(
                        layer_line,
                        format!("No command lines in [{column_name}.{layer_name}]"),
                    ));
                    continue;
                };
                for (command_text, value) in lines.iter() {
                    let line = line_at(
                        lines
                            .get_key_value(command_text)
                            .and_then(|(key, _)| key.span()),
                    );
                    let Ok(command) = CommandId::new(command_text) else {
                        config
                            .problems
                            .push(problem(line, format!("Not a command id: {command_text}")));
                        continue;
                    };
                    let texts: Vec<Option<&str>> = match value {
                        Item::Value(toml_edit::Value::String(s)) => vec![Some(s.value().as_str())],
                        Item::Value(toml_edit::Value::Array(items)) => {
                            items.iter().map(|i| i.as_str()).collect()
                        }
                        _ => {
                            config.problems.push(problem(
                                line,
                                format!("No key for {command_text}: write \"f5\" or \"none\""),
                            ));
                            continue;
                        }
                    };
                    let mut keys = Vec::new();
                    let mut usable = true;
                    for text in texts {
                        let Some(text) = text else {
                            config.problems.push(problem(
                                line,
                                format!("No key for {command_text}: write \"f5\" or \"none\""),
                            ));
                            usable = false;
                            continue;
                        };
                        if text.trim() == Self::UNBIND {
                            continue;
                        }
                        match Key::parse(text) {
                            Ok(key) => keys.push(key),
                            Err(e) => {
                                config.problems.push(problem(
                                    line,
                                    key_problem(&e, text.trim(), command_text),
                                ));
                                usable = false;
                            }
                        }
                    }
                    if usable || !keys.is_empty() {
                        config.overrides.push(KeyOverride {
                            column,
                            layer,
                            command,
                            keys,
                            line,
                        });
                    }
                }
            }
        }
        Ok(config)
    }

    /// The lines of one column.
    pub fn column(&self, column: KeyColumn) -> impl Iterator<Item = &KeyOverride> {
        self.overrides.iter().filter(move |o| o.column == column)
    }

    /// Writes key lines as `keymap.toml` text, a table per column and
    /// layer, in the order given. `diro keys print` and the reference
    /// beside the owner's file are written this way.
    pub fn render(lines: &[KeyOverride], header: &str) -> String {
        let mut text = String::new();
        for line in header.lines() {
            text.push_str("# ");
            text.push_str(line);
            text.push('\n');
        }
        let mut tables: Vec<(KeyColumn, Layer)> = Vec::new();
        for line in lines {
            if !tables.contains(&(line.column, line.layer)) {
                tables.push((line.column, line.layer));
            }
        }
        for (column, layer) in tables {
            text.push_str(&format!("\n[{}.{}]\n", column.word(), layer.word()));
            for line in lines
                .iter()
                .filter(|l| l.column == column && l.layer == layer)
            {
                let keys: Vec<String> = line.keys.iter().map(key_text_for_file).collect();
                let value = match keys.as_slice() {
                    [] => format!("\"{}\"", Self::UNBIND),
                    [one] => format!("\"{one}\""),
                    many => format!(
                        "[{}]",
                        many.iter()
                            .map(|k| format!("\"{k}\""))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                text.push_str(&format!("\"{}\" = {value}\n", line.command));
            }
        }
        text
    }
}

/// A key as `keymap.toml` writes it: lowercase words joined by plus signs,
/// `ctrl+f5`, `shift+tab`, `kp-plus`, `alt+period`.
pub fn key_text_for_file(key: &Key) -> String {
    use diroxide_core::KeyCode;
    let mut parts: Vec<String> = Vec::new();
    if key.mods.ctrl {
        parts.push("ctrl".to_owned());
    }
    if key.mods.alt {
        parts.push("alt".to_owned());
    }
    if key.mods.shift {
        parts.push("shift".to_owned());
    }
    if key.mods.meta {
        parts.push("meta".to_owned());
    }
    let code = match key.code {
        KeyCode::Char(' ') => "space".to_owned(),
        KeyCode::Char('+') => "plus".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Enter => "enter".to_owned(),
        KeyCode::Escape => "escape".to_owned(),
        KeyCode::Tab => "tab".to_owned(),
        KeyCode::Backspace => "backspace".to_owned(),
        KeyCode::Delete => "delete".to_owned(),
        KeyCode::Insert => "insert".to_owned(),
        KeyCode::Home => "home".to_owned(),
        KeyCode::End => "end".to_owned(),
        KeyCode::PageUp => "pageup".to_owned(),
        KeyCode::PageDown => "pagedown".to_owned(),
        KeyCode::Up => "up".to_owned(),
        KeyCode::Down => "down".to_owned(),
        KeyCode::Left => "left".to_owned(),
        KeyCode::Right => "right".to_owned(),
        KeyCode::Menu => "menu".to_owned(),
        KeyCode::KeypadPlus => "kp-plus".to_owned(),
        KeyCode::KeypadMinus => "kp-minus".to_owned(),
        KeyCode::KeypadMultiply => "kp-multiply".to_owned(),
        KeyCode::KeypadDivide => "kp-divide".to_owned(),
    };
    parts.push(code);
    parts.join("+")
}

/// One item of a menu in `menus.toml`: a command, or a submenu.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuItemConfig {
    /// The command it runs, by id.
    pub command: Option<String>,
    /// The submenu it opens, by the submenu's id.
    pub menu: Option<String>,
    /// The label; the command's or the submenu's name when left out.
    pub label: Option<String>,
    /// The access key, one letter or digit, unique in its menu.
    pub access: Option<String>,
    /// Shown only when the place has this capability: `writable`,
    /// `trash`, `no_trash`, `restore`, `links` or `attributes`.
    pub when: Option<String>,
}

/// One menu in `menus.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuConfig {
    /// A stable id, such as `file`; a submenu is opened by it.
    pub id: String,
    /// The title, said and shown.
    pub title: String,
    /// The access key, opened with Alt plus it from the menu bar.
    pub access: Option<String>,
    /// Whether the menu is on the menu bar (a submenu is not).
    pub bar: bool,
    /// The items, as `[[menu.item]]` tables, in order.
    #[serde(rename = "item")]
    pub items: Vec<MenuItemConfig>,
}

impl Default for MenuConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            access: None,
            bar: true,
            items: Vec::new(),
        }
    }
}

/// `menus.toml`: the menus, with the same command ids the keymap and the
/// banks use. When the file is missing the built-in menus are used; the
/// built-in menus are written beside it as `menus-defaults.toml`, to copy
/// from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenusConfig {
    /// The menus, as `[[menu]]` tables, in menu bar order.
    #[serde(rename = "menu")]
    pub menus: Vec<MenuConfig>,
}

impl MenusConfig {
    /// Reads `menus.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(Paths::MENUS, e))
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
    /// The commands it runs, in order, each in the command text form: an
    /// id, then `name=value` arguments (`file.copy`, `go.path
    /// place="D:\Backup"`).
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
    use diroxide_core::format::within_cells;

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
        assert_eq!(config.problems, Vec::new());
        assert_eq!(config.overrides.len(), 4);
        let trash = config
            .column(KeyColumn::Terminal)
            .find(|o| o.command.as_str() == "file.trash")
            .unwrap();
        assert_eq!(trash.layer, Layer::Panel);
        assert_eq!(trash.keys.len(), 2);
        assert_eq!(trash.keys[1].to_string(), "F8");
        assert_eq!(trash.line, 3);
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
    fn every_mistake_is_said_and_the_rest_is_used() {
        let text = "[terminal.nowhere]
\"app.quit\" = \"f1\"
[tv.panel]
\"app.quit\" = \"f1\"
[terminal.panel]
\"app.quit\" = \"hyper+q\"
\"go.find\" = \"ctrl+x ctrl+f\"
\"File.Copy\" = \"f5\"
\"file.view\" = 3
\"file.move\" = [\"f6\", 7]
\"file.copy\" = \"f5\"
[window]
x = 1
";
        let config = KeymapConfig::from_toml(text).unwrap();
        let said: Vec<&str> = config.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "Unknown layer: [terminal.nowhere], line 1.",
                "Unknown key: hyper+q for app.quit, line 6.",
                "Two keys in a row: ctrl+x ctrl+f for go.find; use one key, line 7.",
                "Not a command id: File.Copy, line 8.",
                "No key for file.view: write \"f5\" or \"none\", line 9.",
                "No key for file.move: write \"f5\" or \"none\", line 10.",
                "Unknown column: [tv]; use terminal or window, line 3.",
                "Unknown layer: [window.x], line 13."
            ]
        );
        for problem in &config.problems {
            let meaning = problem.text.split(':').next().unwrap();
            assert!(within_cells(&problem.text, meaning), "{}", problem.text);
        }
        let used: Vec<&str> = config
            .overrides
            .iter()
            .map(|o| o.command.as_str())
            .collect();
        assert_eq!(used, ["file.move", "file.copy"]);
        assert!(KeymapConfig::from_toml("[terminal").is_err());
    }

    #[test]
    fn rendered_lines_read_back_the_same() {
        let config = KeymapConfig::from_toml(
            "[terminal.panel]\n\"file.copy\" = \"f5\"\n\"file.trash\" = [\"delete\", \"f8\"]\n\"view.hidden\" = \"alt+.\"\n\"select.pattern\" = \"+\"\n[window.global]\n\"app.palette\" = [\"ctrl+shift+p\", \"ctrl+p\"]\n\"app.help\" = \"none\"\n",
        )
        .unwrap();
        let text = KeymapConfig::render(&config.overrides, "Keys.\nSecond line.");
        assert!(text.starts_with("# Keys.\n# Second line.\n\n[terminal.panel]\n"));
        assert!(text.contains("\"select.pattern\" = \"plus\"\n"));
        let again = KeymapConfig::from_toml(&text).unwrap();
        assert_eq!(again.problems, Vec::new());
        let strip = |c: &KeymapConfig| -> Vec<(KeyColumn, Layer, String, Vec<Key>)> {
            c.overrides
                .iter()
                .map(|o| (o.column, o.layer, o.command.to_string(), o.keys.clone()))
                .collect()
        };
        assert_eq!(strip(&again), strip(&config));
    }

    #[test]
    fn menus_parse_and_refuse_unknown_fields() {
        let menus = MenusConfig::from_toml(
            "[[menu]]\nid = \"file\"\ntitle = \"File\"\naccess = \"f\"\n[[menu.item]]\ncommand = \"file.view\"\naccess = \"v\"\n[[menu.item]]\ncommand = \"file.trash\"\nwhen = \"trash\"\n[[menu]]\nid = \"sort\"\ntitle = \"Sort\"\nbar = false\n",
        )
        .unwrap();
        assert_eq!(menus.menus.len(), 2);
        assert!(menus.menus[0].bar);
        assert!(!menus.menus[1].bar);
        assert_eq!(menus.menus[0].items[1].when.as_deref(), Some("trash"));
        assert!(MenusConfig::from_toml("[[menu]]\nid = \"x\"\ncolour = 1\n").is_err());
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
