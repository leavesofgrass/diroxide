//! A plugin's settings (R22 commit 40): each setting its manifest declares
//! shows under the plugin's name, its value in words and its description
//! as F1 help; a change is written to `settings.toml` in the plugin's
//! state folder (never into the plugin's own folder, whose files the
//! consent hashes) and reaches the plugin at once as `settings/changed`.
//!
//! Values are the manifest's kinds: `bool` flips, `choice` offers its
//! choices, `int` takes a whole number, `string` and `path` take text, and
//! `argv` takes words separated by spaces.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use diroxide_core::{ListModel, ListRow};
use diroxide_plugins::manifest::SettingSpec;
use serde_json::{Map, Value};

/// The file of a plugin's values, in its state folder.
pub const SETTINGS_FILE: &str = "settings.toml";

/// Where a plugin's values are kept: `<plugins state>/<id>/settings.toml`.
pub fn settings_path(plugins_state: &Path, id: &str) -> PathBuf {
    plugins_state.join(id).join(SETTINGS_FILE)
}

fn to_json(value: &toml::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The owner's values for a plugin, as JSON; none when the file is
/// missing or cannot be read.
pub fn read_values(path: &Path) -> Map<String, Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Map::new();
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        return Map::new();
    };
    table.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()
}

fn to_toml(value: &Value) -> Option<toml::Value> {
    Some(match value {
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64()?),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => toml::Value::Array(items.iter().filter_map(to_toml).collect()),
        _ => return None,
    })
}

/// Writes a plugin's values, whole, through a temporary file and a
/// rename.
pub fn write_values(path: &Path, values: &Map<String, Value>) -> io::Result<()> {
    let table: toml::Table = values
        .iter()
        .filter_map(|(k, v)| to_toml(v).map(|t| (k.clone(), t)))
        .collect();
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let text = toml::to_string(&table).map_err(io::Error::other)?;
    let header = "# The owner's values for this plugin's settings. diroxide writes\n# this file from the plugin's Settings list.\n\n";
    let temporary = path.with_extension("toml.new");
    fs::write(&temporary, format!("{header}{text}"))?;
    fs::rename(&temporary, path)
}

/// A value in words: "on", "off", "none", "1000", "a b c".
pub fn value_words(value: &Value) -> String {
    match value {
        Value::Bool(true) => "on".to_owned(),
        Value::Bool(false) => "off".to_owned(),
        Value::Null => "none".to_owned(),
        Value::String(s) if s.is_empty() => "none".to_owned(),
        Value::String(s) => s.clone(),
        Value::Array(items) if items.is_empty() => "none".to_owned(),
        Value::Array(items) => items
            .iter()
            .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_owned))
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

/// A setting's name in words, from its key: "max_lines" is "Max lines".
pub fn setting_name(key: &str) -> String {
    let words = key.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The current value of a setting: the owner's, or the default.
pub fn current(spec: &SettingSpec, values: &Map<String, Value>) -> Value {
    values
        .get(&spec.key)
        .cloned()
        .unwrap_or_else(|| to_json(&spec.default))
}

/// The plugin's Settings list, titled with its name: one row a setting,
/// its name then its value in words ("Max lines, 1000"); each row's id is
/// the setting's key.
pub fn settings_list(
    plugin: &str,
    specs: &[SettingSpec],
    values: &Map<String, Value>,
) -> ListModel {
    let rows = specs
        .iter()
        .map(|s| {
            ListRow::new(s.key.clone(), setting_name(&s.key))
                .with_details([value_words(&current(s, values))])
        })
        .collect();
    let mut list = ListModel::new(format!("Settings of {plugin}")).with_rows(rows);
    list.set_empty_text("This plugin has no settings");
    list.set_help("Up and Down choose, Enter changes, F1 says what a setting does, Escape closes.");
    list
}

/// F1 help for a plugin's setting: its description, its kind and its
/// default, the plugin's name last.
pub fn setting_help(plugin: &str, spec: &SettingSpec) -> String {
    let kind = match spec.kind.as_str() {
        "bool" => "on or off".to_owned(),
        "int" => "a whole number".to_owned(),
        "choice" => format!("one of {}", spec.choices.join(", ")),
        "path" => "a place".to_owned(),
        "argv" => "words separated by spaces".to_owned(),
        _ => "text".to_owned(),
    };
    let description = spec.description.trim().trim_end_matches('.');
    let description = if description.is_empty() {
        "No description".to_owned()
    } else {
        description.to_owned()
    };
    format!(
        "{}: {description}. Holds {kind}. Default: {}. ({plugin})",
        setting_name(&spec.key),
        value_words(&to_json(&spec.default))
    )
}

/// A typed text as a setting's value, or why it does not fit.
pub fn parse_value(spec: &SettingSpec, text: &str) -> Result<Value, String> {
    let text = text.trim();
    match spec.kind.as_str() {
        "bool" => match text {
            "on" | "true" | "yes" => Ok(Value::Bool(true)),
            "off" | "false" | "no" => Ok(Value::Bool(false)),
            _ => Err("Type on or off.".to_owned()),
        },
        "int" => text
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| "Type a whole number.".to_owned()),
        "choice" => {
            if spec.choices.iter().any(|c| c == text) {
                Ok(Value::String(text.to_owned()))
            } else {
                Err(format!("Choose one of {}.", spec.choices.join(", ")))
            }
        }
        "argv" => Ok(Value::Array(
            text.split_whitespace()
                .map(|w| Value::String(w.to_owned()))
                .collect(),
        )),
        _ => Ok(Value::String(text.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(key: &str, kind: &str, default: toml::Value) -> SettingSpec {
        SettingSpec {
            key: key.into(),
            kind: kind.into(),
            default,
            description: "How many lines to count at most.".into(),
            choices: vec!["fast".into(), "exact".into()],
        }
    }

    #[test]
    fn settings_read_in_words_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path(), "line-counter");
        let max = spec("max_lines", "int", toml::Value::Integer(1000));
        let mode = spec("mode", "choice", toml::Value::String("fast".into()));
        let specs = vec![max.clone(), mode.clone()];
        let list = settings_list("Line counter", &specs, &read_values(&path));
        assert_eq!(list.title(), "Settings of Line counter");
        assert_eq!(list.announce_focus(), "Max lines, 1000, 1 of 2");
        let mut values = Map::new();
        values.insert("max_lines".into(), parse_value(&max, "25").unwrap());
        values.insert("mode".into(), parse_value(&mode, "exact").unwrap());
        write_values(&path, &values).unwrap();
        let back = read_values(&path);
        assert_eq!(back, values);
        assert_eq!(current(&max, &back), Value::from(25));
        assert_eq!(
            parse_value(&max, "many").unwrap_err(),
            "Type a whole number."
        );
        assert_eq!(
            parse_value(&mode, "slow").unwrap_err(),
            "Choose one of fast, exact."
        );
        assert_eq!(
            setting_help("Line counter", &max),
            "Max lines: How many lines to count at most. Holds a whole number. Default: 1000. (Line counter)"
        );
    }
}
