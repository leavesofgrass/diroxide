//! The configuration of diroxide: plain TOML files a person can read and
//! diff, in the `diroxide` folder under the platform's config directory
//! (the `dirs` crate), or beside the program with `--portable`.
//!
//! - `config.toml`: [`Settings`], checked against the [`schema`].
//! - `keymap.toml`: [`KeymapConfig`], the owner's keys: command to key, in
//!   a table per column (terminal, window) and layer.
//! - `menus.toml`: [`MenusConfig`], the menus; the built-in ones when the
//!   file is missing.
//! - `filetypes.toml`: [`FileTypesConfig`].
//! - `banks.toml`: [`BanksConfig`], the button banks.
//! - `conversions.toml`: [`ConversionsConfig`], the conversion recipes.
//! - `tools.toml`: [`ToolsConfig`], overrides for external tools.
//! - `themes/`: the owner's theme files ([`theme`]).
//! - `session.toml`, in the state folder: [`SessionState`], what the next
//!   start restores.
//!
//! Reading never stops the program: a missing file means the defaults; a
//! file that is not TOML is copied to `NAME.bad` and the defaults are
//! used; every other mistake is one sentence in [`Config::problems`],
//! meaning first, and the rest of the file is used. Unknown keys are kept.
//! Writes are atomic, happen only on an explicit change, and edit only
//! the values that changed, so the owner's comments survive ([`store`]).

pub mod columns;
pub mod error;
pub mod files;
pub mod paths;
pub mod schema;
pub mod session;
pub mod settings;
pub mod store;
pub mod theme;

pub use columns::ColumnKind;
pub use error::ConfigError;
pub use files::{
    ActionConfig, BankConfig, BanksConfig, ButtonConfig, ConversionsConfig, FileTypeConfig,
    FileTypesConfig, KeyOverride, KeymapConfig, MenuConfig, MenuItemConfig, MenusConfig,
    RecipeConfig, ToolOverride, ToolsConfig, key_text_for_file,
};
pub use paths::Paths;
pub use schema::{Choice, SCHEMA, SettingGroup, SettingInfo, SettingKind};
pub use session::{Layout, PanelState, SessionState, load_session, save_session};
pub use settings::{
    CompletionList, ConfirmSettings, DRIVE_KINDS, FilterCase, Highlighting, PluginSettings,
    ScreenReaderMode, SecretEcho, Settings, SingleLetters, WeekStart,
};
pub use store::{ConfigProblem, Loaded, problems_line, write_atomic, write_reference};

use std::path::Path;

use serde::de::DeserializeOwned;
use store::ReadText;

/// Everything read from the config folder, with what was wrong.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// `config.toml`.
    pub settings: Settings,
    /// `keymap.toml`.
    pub keymap: KeymapConfig,
    /// `menus.toml`; `None` when the file is missing or cannot be used, so
    /// the built-in menus apply.
    pub menus: Option<MenusConfig>,
    /// `filetypes.toml`.
    pub filetypes: FileTypesConfig,
    /// `banks.toml`.
    pub banks: BanksConfig,
    /// `conversions.toml`.
    pub conversions: ConversionsConfig,
    /// `tools.toml`.
    pub tools: ToolsConfig,
    /// Every problem found, in words, file by file. The app says
    /// [`problems_line`] at startup and lists these on request.
    pub problems: Vec<ConfigProblem>,
}

/// Reads a typed file: missing gives `None`; text that cannot be used is
/// backed up and said.
fn read_typed<T: DeserializeOwned>(
    paths: &Paths,
    name: &str,
    problems: &mut Vec<ConfigProblem>,
) -> Option<T> {
    let path = paths.config_file(name);
    match store::read_text(&path, name) {
        ReadText::Missing => None,
        ReadText::Failed(problem) => {
            problems.push(problem);
            None
        }
        ReadText::Text(text) => match store::parse_typed::<T>(name, &text) {
            Ok(value) => Some(value),
            Err(failure) => {
                let backup = store::back_up(&path, &text);
                problems.push(ConfigProblem::new(name, failure.sentence(&backup)));
                None
            }
        },
    }
}

/// Reads a file's text for a reader that checks it line by line; text that
/// is not TOML is backed up and said.
fn read_checked(paths: &Paths, name: &str, problems: &mut Vec<ConfigProblem>) -> Option<String> {
    let path = paths.config_file(name);
    match store::read_text(&path, name) {
        ReadText::Missing => None,
        ReadText::Failed(problem) => {
            problems.push(problem);
            None
        }
        ReadText::Text(text) => match store::parse_table(name, &text) {
            Ok(_) => Some(text),
            Err(failure) => {
                let backup = store::back_up(&path, &text);
                problems.push(ConfigProblem::new(name, failure.sentence(&backup)));
                None
            }
        },
    }
}

/// Reads `config.toml` alone, with its problems: the schema's checks, the
/// theme's name against the built-in themes and the owner's theme files,
/// and the `[colors]` overrides.
pub fn load_settings(paths: &Paths) -> Loaded<Settings> {
    let mut problems = Vec::new();
    let settings = match read_checked(paths, Paths::SETTINGS, &mut problems) {
        None => Settings::default(),
        Some(text) => match Settings::read(&text) {
            Ok(loaded) => {
                problems.extend(loaded.problems);
                loaded.value
            }
            Err(error) => {
                problems.push(ConfigProblem::new(Paths::SETTINGS, error.to_string()));
                Settings::default()
            }
        },
    };
    problems.extend(check_look(paths, &settings));
    Loaded {
        value: settings,
        problems,
    }
}

/// The theme's name and the colors, checked.
fn check_look(paths: &Paths, settings: &Settings) -> Vec<ConfigProblem> {
    let (registry, mut problems) = theme::registry(&paths.themes_dir(), Paths::SETTINGS);
    let (_, unknown) = theme::resolve(&registry, &settings.theme, Paths::SETTINGS);
    problems.extend(unknown);
    let (_, color_problems) = theme::read_overrides(&settings.colors, Paths::SETTINGS);
    problems.extend(color_problems);
    problems
}

/// Reads every file in the config folder. Never fails: what cannot be read
/// takes its defaults, and [`Config::problems`] says why.
pub fn load(paths: &Paths) -> Config {
    let mut problems = Vec::new();
    let settings = load_settings(paths);
    problems.extend(settings.problems);
    let keymap = match read_checked(paths, Paths::KEYMAP, &mut problems) {
        None => KeymapConfig::default(),
        Some(text) => match KeymapConfig::from_toml(&text) {
            Ok(keymap) => {
                problems.extend(keymap.problems.iter().cloned());
                keymap
            }
            Err(error) => {
                problems.push(ConfigProblem::new(Paths::KEYMAP, error.to_string()));
                KeymapConfig::default()
            }
        },
    };
    let menus = read_typed::<MenusConfig>(paths, Paths::MENUS, &mut problems);
    let filetypes = read_typed(paths, Paths::FILETYPES, &mut problems).unwrap_or_default();
    let banks = read_typed(paths, Paths::BANKS, &mut problems).unwrap_or_default();
    let conversions = read_typed(paths, Paths::CONVERSIONS, &mut problems).unwrap_or_default();
    let tools = read_typed(paths, Paths::TOOLS, &mut problems).unwrap_or_default();
    Config {
        settings: settings.value,
        keymap,
        menus,
        filetypes,
        banks,
        conversions,
        tools,
        problems,
    }
}

/// Saves the settings that differ from what `config.toml` holds now, one
/// value at a time, so comments, order and unknown keys survive; nothing
/// is written when nothing changed. A file that cannot be read is not
/// written over.
pub fn save_settings(paths: &Paths, settings: &Settings) -> Result<(), ConfigError> {
    let mut problems = Vec::new();
    let current = match read_checked(paths, Paths::SETTINGS, &mut problems) {
        None if problems.is_empty() => Settings::default(),
        None => {
            return Err(ConfigError::bad_entry(
                Paths::SETTINGS,
                "the file could not be read, so nothing was saved",
            ));
        }
        Some(text) => Settings::read(&text)?.value,
    };
    let before = current.values()?;
    let after = settings.values()?;
    let changes: Vec<(String, Option<toml::Value>)> = after
        .into_iter()
        .filter(|(key, value)| before.get(key) != Some(value))
        .map(|(key, value)| (key.to_owned(), Some(value)))
        .collect();
    store::edit_file(paths, Paths::SETTINGS, &changes)
}

/// Sets one setting by its dotted key, after the schema checks the value;
/// the refusal says why in words.
pub fn set_setting(paths: &Paths, key: &str, value: toml::Value) -> Result<(), ConfigError> {
    let Some(info) = schema::setting(key) else {
        return Err(ConfigError::bad_entry(
            Paths::SETTINGS,
            format!("no setting is named {key}"),
        ));
    };
    let mut probe = toml::Table::new();
    insert_dotted(&mut probe, info.key, value.clone());
    let loaded = Settings::read(&toml::to_string(&probe).unwrap_or_default())?;
    if let Some(problem) = loaded.problems.first() {
        return Err(ConfigError::bad_entry(
            Paths::SETTINGS,
            problem.text.trim_end_matches('.'),
        ));
    }
    store::edit_file(
        paths,
        Paths::SETTINGS,
        &[(info.key.to_owned(), Some(value))],
    )
}

/// Puts a setting back to its default by removing its line from
/// `config.toml`; the rest of the file is untouched.
pub fn reset_setting(paths: &Paths, key: &str) -> Result<(), ConfigError> {
    store::edit_file(paths, Paths::SETTINGS, &[(key.to_owned(), None)])
}

fn insert_dotted(table: &mut toml::Table, key: &str, value: toml::Value) {
    match key.split_once('.') {
        None => {
            table.insert(key.to_owned(), value);
        }
        Some((first, rest)) => {
            let inner = table
                .entry(first.to_owned())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let toml::Value::Table(inner) = inner {
                insert_dotted(inner, rest, value);
            }
        }
    }
}

/// Whether a folder looks like a diroxide config folder (it holds
/// `config.toml` or `keymap.toml`), for `--portable` and `--config`.
pub fn looks_like_config(folder: &Path) -> bool {
    folder.join(Paths::SETTINGS).is_file() || folder.join(Paths::KEYMAP).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        (dir, paths)
    }

    #[test]
    fn a_missing_folder_gives_defaults_and_no_problems() {
        let (_dir, paths) = paths();
        let config = load(&paths);
        assert_eq!(config, Config::default());
        assert!(!looks_like_config(&paths.config_dir));
    }

    #[test]
    fn changing_one_setting_keeps_the_owners_comments() {
        let (_dir, paths) = paths();
        fs::create_dir_all(&paths.config_dir).unwrap();
        let before = "# Chosen after the listening test.\nlayout = \"stacked\" # for NVDA\n\n# Later.\nfuture = 1\n";
        fs::write(paths.config_file(Paths::SETTINGS), before).unwrap();
        let mut settings = load(&paths).settings;
        assert_eq!(settings.layout, Layout::Stacked);
        settings.show_hidden = true;
        save_settings(&paths, &settings).unwrap();
        let after = fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap();
        assert_eq!(
            after,
            "# Chosen after the listening test.\nlayout = \"stacked\" # for NVDA\n\n# Later.\nfuture = 1\nshow_hidden = true\n"
        );
        save_settings(&paths, &settings).unwrap();
        assert_eq!(
            fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap(),
            after
        );
    }

    #[test]
    fn set_and_reset_one_setting() {
        let (_dir, paths) = paths();
        set_setting(&paths, "sort.key", toml::Value::String("size".into())).unwrap();
        assert_eq!(
            fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap(),
            "[sort]\nkey = \"size\"\n"
        );
        let err =
            set_setting(&paths, "layout", toml::Value::String("diagonal".into())).unwrap_err();
        assert_eq!(
            err.to_string(),
            "config.toml has a bad entry: layout cannot be \"diagonal\": use side_by_side, stacked or single"
        );
        assert!(set_setting(&paths, "no_such", toml::Value::Boolean(true)).is_err());
        reset_setting(&paths, "sort.key").unwrap();
        assert_eq!(load(&paths).settings, Settings::default());
    }

    #[test]
    fn a_corrupt_file_is_backed_up_and_said_and_the_rest_still_loads() {
        let (_dir, paths) = paths();
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::write(
            paths.config_file(Paths::SETTINGS),
            "theme = \"nord\"\nlayout = \n",
        )
        .unwrap();
        fs::write(
            paths.config_file(Paths::KEYMAP),
            "[terminal.panel]\n\"file.copy\" = \"hyper+c\"\n",
        )
        .unwrap();
        fs::write(paths.config_file(Paths::MENUS), "[[menu]]\nid = 3\n").unwrap();
        let config = load(&paths);
        assert_eq!(config.settings, Settings::default());
        let said: Vec<&str> = config.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "config.toml could not be read: line 2 is not valid TOML. A copy is kept as config.toml.bad; the defaults are in use.",
                "Unknown key: hyper+c for file.copy, line 2.",
                "menus.toml could not be read: line 2 has a value of the wrong kind. A copy is kept as menus.toml.bad; the defaults are in use."
            ]
        );
        assert!(config.menus.is_none());
        assert!(paths.config_file("config.toml.bad").exists());
        assert_eq!(
            problems_line(&config.problems),
            "config.toml: 1 problem, keymap.toml: 1 problem, menus.toml: 1 problem"
        );
        let changed = Settings {
            borders: true,
            ..Settings::default()
        };
        assert!(save_settings(&paths, &changed).is_err());
        assert_eq!(
            fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap(),
            "theme = \"nord\"\nlayout = \n"
        );
    }

    #[test]
    fn an_unknown_theme_and_bad_colors_are_said() {
        let (_dir, paths) = paths();
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::write(
            paths.config_file(Paths::SETTINGS),
            "theme = \"no-such\"\n[colors]\nfolder = \"orange\"\n",
        )
        .unwrap();
        let said: Vec<String> = load(&paths).problems.into_iter().map(|p| p.text).collect();
        assert_eq!(
            said,
            [
                "Unknown theme: no-such. Galaxy is in use.",
                "colors.folder cannot be \"orange\": write it as #rrggbb."
            ]
        );
    }
}
