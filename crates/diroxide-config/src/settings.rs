//! `config.toml`: the settings.
//!
//! Every setting has a default, so an empty file is valid. Reading never
//! fails on a value: a value the schema refuses keeps its default and is
//! said in words ("layout cannot be \"diagonal\": use side_by_side,
//! stacked or single."); a key this version does not know is kept as it
//! is, written back untouched, and said once ("Unknown setting: foo. Kept
//! as it is."). Only a file that is not TOML at all is refused, and then
//! the store backs it up (see [`crate::store`]).
//!
//! Saving writes only the settings that changed, one value at a time,
//! through `toml_edit`, so the owner's comments and order survive.

use std::collections::BTreeMap;

use diroxide_core::{SortKey, SortSpec};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::columns::ColumnKind;
use crate::error::ConfigError;
use crate::paths::Paths;
use crate::presets::{
    SplitPreset, builtin_split_presets, check_preset, merge_split_presets, unknown_preset_keys,
};
use crate::schema::{SCHEMA, SettingInfo, SettingKind};
use crate::session::Layout;
use crate::store::{ConfigProblem, Loaded};

/// Whether the frontends behave for a screen reader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenReaderMode {
    /// On when a running screen reader is detected.
    #[default]
    Auto,
    /// Always behave for a screen reader.
    On,
    /// Never.
    Off,
}

/// What a key without Control, Alt or Command does in a panel (WCAG
/// 2.1.4 lets the user turn such keys off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleLetters {
    /// Letters and digits jump to names (type-ahead); the plus, minus,
    /// star and question mark keys run their commands.
    #[default]
    TypeAhead,
    /// Letters run the commands the keymap gives them (a vim-style
    /// preset); nothing jumps.
    Commands,
    /// No single-character key does anything; Ctrl+S filters.
    Off,
}

impl SingleLetters {
    /// Whether single-character bindings stay in the keymap.
    pub const fn keeps_bindings(self) -> bool {
        !matches!(self, SingleLetters::Off)
    }

    /// Whether letters and digits jump to names.
    pub const fn types_ahead(self) -> bool {
        matches!(self, SingleLetters::TypeAhead)
    }
}

/// When a text field's candidates are listed (plan section 5.19).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionList {
    /// As you type in the window, on Tab in a terminal.
    #[default]
    Auto,
    /// As you type.
    AsYouType,
    /// On Tab.
    OnTab,
}

/// What a password field shows for each typed character (ADR-0009).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretEcho {
    /// One asterisk per character.
    #[default]
    Asterisks,
    /// Nothing.
    Silent,
}

/// Whether code is colored in the viewer (alpha.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Highlighting {
    /// Colored, except in screen reader mode.
    #[default]
    Auto,
    /// Colored.
    On,
    /// Not colored.
    Off,
}

/// Whether case matters in filters and searches (alpha.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterCase {
    /// Only when the text has a capital letter.
    #[default]
    Smart,
    /// Never.
    Ignore,
    /// Always.
    Match,
}

/// The first day of the week, for "this week" and "last week" (alpha.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeekStart {
    /// Sunday.
    #[default]
    Sunday,
    /// Monday.
    Monday,
    /// Saturday.
    Saturday,
}

/// The `[confirm]` table: which questions are asked. Every question starts
/// on No, and deleting for good always asks; neither is a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfirmSettings {
    /// Before a copy.
    pub copy: bool,
    /// Before a move.
    #[serde(rename = "move")]
    pub move_items: bool,
    /// Before moving to the trash.
    pub trash: bool,
    /// Before quitting while a job runs.
    pub quit_with_jobs: bool,
}

impl Default for ConfirmSettings {
    fn default() -> Self {
        Self {
            copy: true,
            move_items: true,
            trash: true,
            quit_with_jobs: true,
        }
    }
}

/// How many bytes a row of hex shows in the viewer (alpha.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexRow {
    /// Eight in screen reader mode, sixteen otherwise.
    #[default]
    Auto,
    /// Eight: the offset, the bytes and their letters fit 40 cells.
    Narrow,
    /// Sixteen.
    Wide,
}

impl HexRow {
    /// The bytes a row shows, in or out of screen reader mode.
    pub const fn bytes_per_row(self, screen_reader: bool) -> usize {
        match self {
            HexRow::Narrow => 8,
            HexRow::Wide => 16,
            HexRow::Auto if screen_reader => 8,
            HexRow::Auto => 16,
        }
    }
}

/// The `[viewer]` table (alpha.2, W2d): wrapping, line numbers, follow,
/// hex rows, the decoded-bytes cache for files inside archives, and the
/// highlighting caps of R17.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerSettings {
    /// Wrap long lines, each continuation marked.
    pub wrap: bool,
    /// Where lines wrap, in characters; 0 for the screen's width.
    pub wrap_width: u64,
    /// Start each line with its number.
    pub line_numbers: bool,
    /// Read more of a growing file and stay at its end.
    pub follow: bool,
    /// The bytes of a hex row.
    pub hex_row: HexRow,
    /// How much of a file inside an archive is kept decoded, in MiB.
    pub member_cache_mib: u64,
    /// No code colors for a file larger than this, in KiB.
    pub highlight_max_kib: u64,
    /// No code colors for a file with more lines than this.
    pub highlight_max_lines: u64,
    /// No code colors on a line longer than this, in characters.
    pub highlight_max_line_chars: u64,
    /// How long highlighting may take before it stops for the session,
    /// in milliseconds.
    pub highlight_deadline_ms: u64,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            wrap: true,
            wrap_width: 0,
            line_numbers: false,
            follow: false,
            hex_row: HexRow::Auto,
            member_cache_mib: 64,
            highlight_max_kib: 512,
            highlight_max_lines: 10_000,
            highlight_max_line_chars: 2_000,
            highlight_deadline_ms: 2_000,
        }
    }
}

/// The `[quick_view]` table (alpha.2, W2e): the wait after the cursor
/// stops, how much is read and for how long, following changes on disk,
/// and line numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickViewSettings {
    /// The wait after the cursor stops, in milliseconds.
    pub wait_ms: u64,
    /// The most of a file read, in KiB.
    pub max_kib: u64,
    /// How long reading a document may take, in milliseconds.
    pub deadline_ms: u64,
    /// Show the file again when it changes on disk.
    pub follow_changes: bool,
    /// Line numbers for plain text and code.
    pub line_numbers: bool,
}

impl Default for QuickViewSettings {
    fn default() -> Self {
        Self {
            wait_ms: 500,
            max_kib: 1_024,
            deadline_ms: 2_000,
            follow_changes: true,
            line_numbers: false,
        }
    }
}

/// One mebibyte, for the `_mib` settings.
pub const MIB: u64 = 1 << 20;
/// One gibibyte, for the `_gib` settings.
pub const GIB: u64 = 1 << 30;

/// The `[archives]` table (alpha.2, W2a): the limits that keep a hostile
/// archive from filling the disk or the memory, with the starting numbers
/// of research/archives.md section 6. Each field is one field of the
/// archive provider's `Limits`, in the unit its name ends with; the app
/// builds the provider's limits from these, through
/// [`ArchiveSettings::mib`] and [`ArchiveSettings::gib`].
/// "Ask" limits are questions asked once per job; the others refuse with
/// the reason in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchiveSettings {
    /// Ask before one member larger than this, in GiB.
    pub max_member_gib: u64,
    /// Ask before an operation writes out more than this in all, in GiB.
    pub ask_total_gib: u64,
    /// Refuse an operation that expands to more than this, in GiB.
    pub max_total_gib: u64,
    /// Ask when a member grows more than this many times its packed size.
    pub max_ratio: u64,
    /// What a member writes before its ratio is judged, in MiB.
    pub ratio_floor_mib: u64,
    /// Ask before a dictionary larger than this, in MiB.
    pub ask_dictionary_mib: u64,
    /// Refuse a dictionary larger than this, in MiB.
    pub max_dictionary_mib: u64,
    /// Refuse a header or central directory larger than this, in MiB.
    pub max_header_mib: u64,
    /// Refuse an index that takes more memory than this, in MiB.
    pub max_index_mib: u64,
    /// Refuse an archive with more members than this.
    pub max_entries: u64,
    /// Skip a member whose name is longer than this, in bytes.
    pub max_name_bytes: u64,
    /// Skip a member with a name part longer than this, in bytes.
    pub max_component_bytes: u64,
    /// Skip a link whose target is longer than this, in bytes.
    pub max_link_target_bytes: u64,
    /// Refuse an operation that needs more memory than this, in MiB.
    pub max_memory_mib: u64,
    /// Spool an inner archive in memory up to this, in MiB.
    pub spool_memory_mib: u64,
    /// Spool on disk up to this per operation, in GiB.
    pub spool_disk_gib: u64,
    /// How many archives deep a place may go.
    pub max_nesting: u64,
}

impl Default for ArchiveSettings {
    fn default() -> Self {
        Self {
            max_member_gib: 8,
            ask_total_gib: 32,
            max_total_gib: 1_024,
            max_ratio: 1_000,
            ratio_floor_mib: 64,
            ask_dictionary_mib: 256,
            max_dictionary_mib: 1_536,
            max_header_mib: 256,
            max_index_mib: 256,
            max_entries: 1_000_000,
            max_name_bytes: 4_096,
            max_component_bytes: 255,
            max_link_target_bytes: 4_096,
            max_memory_mib: 1_024,
            spool_memory_mib: 64,
            spool_disk_gib: 4,
            max_nesting: 4,
        }
    }
}

impl ArchiveSettings {
    /// A count of MiB in bytes, saturating.
    pub const fn mib(units: u64) -> u64 {
        units.saturating_mul(MIB)
    }

    /// A count of GiB in bytes, saturating.
    pub const fn gib(units: u64) -> u64 {
        units.saturating_mul(GIB)
    }
}

/// The plugins section of `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

/// The settings in `config.toml`. The schema ([`SCHEMA`]) gives each one
/// its name, kind, range and help; keys this version does not know are
/// kept in `unknown` and written back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How the panels share the screen.
    pub layout: Layout,
    /// Show hidden entries.
    pub show_hidden: bool,
    /// How a new panel sorts: the key, its direction, where folders go
    /// and how names compare.
    pub sort: SortSpec,
    /// The columns of a panel row, in order, after the name, by
    /// [`ColumnKind`] name.
    pub columns: Vec<String>,
    /// The program the Edit command opens a file with; empty for
    /// `$VISUAL`, then `$EDITOR`, then the platform's editor.
    pub editor: String,
    /// What single-character keys do.
    pub single_letters: SingleLetters,
    /// The key preset under `keymap.toml` (alpha.3).
    pub preset: String,
    /// Screen reader behavior.
    pub screen_reader_mode: ScreenReaderMode,
    /// Draw as little as possible.
    pub quiet_screen: bool,
    /// The shortest time between two progress lines in screen reader mode.
    pub progress_interval_seconds: u64,
    /// Park the terminal's cursor on the status line.
    pub cursor_fallback: bool,
    /// How long a new dialog waits before it takes keys.
    pub dialog_settle_ms: u64,
    /// When completion candidates are listed.
    pub completion_list: CompletionList,
    /// What a password field shows per character.
    pub secret_echo: SecretEcho,
    /// Whether password fields start with the text shown.
    pub secret_show_toggle: bool,
    /// Which questions are asked.
    pub confirm: ConfirmSettings,
    /// `system`, or a folder used as the trash.
    pub trash: String,
    /// Drive kinds where the system trash is used.
    pub trash_drives: Vec<String>,
    /// Send a replaced file to the trash first.
    pub replace_to_trash: bool,
    /// Refuse names other systems cannot store.
    pub portable_names: bool,
    /// The theme's name.
    pub theme: String,
    /// Follow the system's light or dark appearance.
    pub follow_system_appearance: bool,
    /// The owner's colors by screen role, `#rrggbb`.
    pub colors: BTreeMap<String, String>,
    /// Box drawing around the panels in a terminal.
    pub borders: bool,
    /// Hide the key bar and the title line in a terminal.
    pub compact: bool,
    /// Code colors in the viewer (alpha.2).
    pub highlighting: Highlighting,
    /// Thumbnails in the window (alpha.7).
    pub thumbnails: bool,
    /// Hide the window's menu bar until Alt or F10 (alpha.6).
    pub auto_hide_menu: bool,
    /// Fade the window's bars while the mouse rests (alpha.6).
    pub focus_mode: bool,
    /// The viewer (alpha.2).
    pub viewer: ViewerSettings,
    /// The quick view (alpha.2).
    pub quick_view: QuickViewSettings,
    /// The archive limits (alpha.2).
    pub archives: ArchiveSettings,
    /// The split presets: the built-in ones with the user's merged in
    /// ([`merge_split_presets`]) (alpha.2).
    pub split_presets: Vec<SplitPreset>,
    /// Whether case matters in filters (alpha.3).
    pub filter_case: FilterCase,
    /// The filter's pause before it says the count (alpha.3).
    pub filter_pause_ms: u64,
    /// The first day of the week (alpha.3).
    pub week_starts_on: WeekStart,
    /// The most rows a search lists (alpha.3).
    pub search_row_cap: u64,
    /// Whether searches skip ignored files (alpha.3).
    pub search_respect_ignore: bool,
    /// Where the components list is read from; empty for the public list
    /// (alpha.4).
    pub components_source: String,
    /// The plugins section.
    pub plugins: PluginSettings,
    /// Keys this version does not know, kept as they were.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            layout: Layout::SideBySide,
            show_hidden: false,
            sort: SortSpec::default(),
            columns: vec!["type".to_owned(), "size".to_owned(), "modified".to_owned()],
            editor: String::new(),
            single_letters: SingleLetters::TypeAhead,
            preset: "diroxide".to_owned(),
            screen_reader_mode: ScreenReaderMode::Auto,
            quiet_screen: false,
            progress_interval_seconds: 10,
            cursor_fallback: false,
            dialog_settle_ms: 250,
            completion_list: CompletionList::Auto,
            secret_echo: SecretEcho::Asterisks,
            secret_show_toggle: false,
            confirm: ConfirmSettings::default(),
            trash: "system".to_owned(),
            trash_drives: vec!["fixed".to_owned()],
            replace_to_trash: true,
            portable_names: true,
            theme: diroxide_theme::DEFAULT_THEME.to_owned(),
            follow_system_appearance: false,
            colors: BTreeMap::new(),
            borders: false,
            compact: false,
            highlighting: Highlighting::Auto,
            thumbnails: false,
            auto_hide_menu: true,
            focus_mode: false,
            viewer: ViewerSettings::default(),
            quick_view: QuickViewSettings::default(),
            archives: ArchiveSettings::default(),
            split_presets: builtin_split_presets(),
            filter_case: FilterCase::Smart,
            filter_pause_ms: 400,
            week_starts_on: WeekStart::Sunday,
            search_row_cap: 100_000,
            search_respect_ignore: false,
            components_source: String::new(),
            plugins: PluginSettings::default(),
            unknown: BTreeMap::new(),
        }
    }
}

/// The drive kinds `trash_drives` may name.
pub const DRIVE_KINDS: [&str; 3] = ["fixed", "removable", "network"];

/// A value at a dotted key in a table.
fn get_dotted<'a>(table: &'a Table, key: &str) -> Option<&'a Value> {
    let mut parts = key.split('.');
    let first = parts.next()?;
    let mut value = table.get(first)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

/// Removes the value at a dotted key; returns it.
fn remove_dotted(table: &mut Table, key: &str) -> Option<Value> {
    match key.split_once('.') {
        None => table.remove(key),
        Some((first, rest)) => table
            .get_mut(first)
            .and_then(Value::as_table_mut)
            .and_then(|inner| remove_dotted(inner, rest)),
    }
}

/// A value as written in `config.toml`, for messages.
fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => format!("\"{text}\""),
        other => other.to_string(),
    }
}

/// Checks one value against its setting's kind; the error is the reason
/// in words, after the key ("cannot be 0: use 1 to 600.").
fn check_value(info: &SettingInfo, value: &Value) -> Result<(), String> {
    let key = info.key;
    match info.kind {
        SettingKind::Flag => match value {
            Value::Boolean(_) => Ok(()),
            other => Err(format!("{key} needs true or false, not {}.", shown(other))),
        },
        SettingKind::Number { min, max, .. } => match value.as_integer() {
            Some(n) if (min..=max).contains(&n) => Ok(()),
            Some(n) => Err(format!("{key} cannot be {n}: use {min} to {max}.")),
            None => Err(format!("{key} needs a whole number, not {}.", shown(value))),
        },
        SettingKind::Choice(choices) => match value.as_str() {
            Some(text) if choices.iter().any(|c| c.value == text) => Ok(()),
            _ => {
                let values: Vec<&str> = choices.iter().map(|c| c.value).collect();
                Err(format!(
                    "{key} cannot be {}: use {}.",
                    shown(value),
                    crate::schema::list_words(&values, "or")
                ))
            }
        },
        SettingKind::Text | SettingKind::Theme => match value {
            Value::String(_) => Ok(()),
            other => Err(format!("{key} needs text in quotes, not {}.", shown(other))),
        },
        SettingKind::Columns => match value.as_array() {
            Some(items) => {
                for item in items {
                    let known = item.as_str().and_then(ColumnKind::from_name);
                    if known.is_none() {
                        return Err(format!(
                            "{key} cannot hold {}: no column has that name.",
                            shown(item)
                        ));
                    }
                }
                Ok(())
            }
            None => Err(format!("{key} needs a list, not {}.", shown(value))),
        },
        SettingKind::DriveKinds => match value.as_array() {
            Some(items) => {
                for item in items {
                    if !item.as_str().is_some_and(|t| DRIVE_KINDS.contains(&t)) {
                        return Err(format!(
                            "{key} cannot hold {}: use fixed, removable or network.",
                            shown(item)
                        ));
                    }
                }
                Ok(())
            }
            None => Err(format!("{key} needs a list, not {}.", shown(value))),
        },
        SettingKind::Names => match value.as_array() {
            Some(items) if items.iter().all(Value::is_str) => Ok(()),
            _ => Err(format!("{key} needs a list of names in quotes.")),
        },
        SettingKind::Colors => match value.as_table() {
            Some(table) if table.values().all(Value::is_str) => Ok(()),
            _ => Err(format!("{key} needs a table such as folder = \"#ffaa00\".")),
        },
        SettingKind::SplitPresets => match value.as_array() {
            Some(items) => items
                .iter()
                .enumerate()
                .try_for_each(|(at, item)| check_preset(key, at + 1, item)),
            None => Err(format!(
                "{key} needs a list of presets, as [[{key}]] tables with a name and bytes."
            )),
        },
    }
}

/// Unknown keys in a table, with their dotted paths.
fn unknown_keys(table: &Table, prefix: &str, out: &mut Vec<String>) {
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let known = SCHEMA.iter().any(|s| s.key == dotted);
        let parent = SCHEMA
            .iter()
            .any(|s| s.key.starts_with(&format!("{dotted}.")));
        if known {
            continue;
        }
        match value.as_table() {
            Some(inner) if parent => unknown_keys(inner, &dotted, out),
            _ => out.push(dotted),
        }
    }
}

impl Settings {
    /// Reads settings from TOML text: every value the schema refuses keeps
    /// its default and is said in words; unknown keys are kept and said.
    /// Only text that is not TOML is an error.
    pub fn read(text: &str) -> Result<Loaded<Self>, ConfigError> {
        let file = Paths::SETTINGS;
        let mut table: Table = toml::from_str(text).map_err(|e| ConfigError::parse(file, e))?;
        let mut problems = Vec::new();
        for parent in [
            "sort",
            "confirm",
            "plugins",
            "viewer",
            "quick_view",
            "archives",
        ] {
            if table.get(parent).is_some_and(|v| !v.is_table()) {
                problems.push(ConfigProblem::new(
                    file,
                    format!("{parent} needs a table, as [{parent}]. The defaults are in use."),
                ));
                table.remove(parent);
            }
        }
        for info in &SCHEMA {
            let Some(value) = get_dotted(&table, info.key) else {
                continue;
            };
            if let Err(reason) = check_value(info, value) {
                problems.push(ConfigProblem::new(file, reason));
                remove_dotted(&mut table, info.key);
            }
        }
        let mut unknown = Vec::new();
        unknown_keys(&table, "", &mut unknown);
        for key in unknown {
            problems.push(ConfigProblem::new(
                file,
                format!("Unknown setting: {key}. Kept as it is."),
            ));
        }
        if let Some(presets) = table.get("split_presets") {
            for (name, key) in unknown_preset_keys(presets) {
                problems.push(ConfigProblem::new(
                    file,
                    format!("Unknown key in split_presets, {name}: {key}. Kept as it is."),
                ));
            }
        }
        let direction_given = get_dotted(&table, "sort.direction").is_some();
        let presets_given = table.contains_key("split_presets");
        let mut settings: Settings = Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::parse(file, e))?;
        if !direction_given {
            settings.sort.direction = settings.sort.key.first_direction();
        }
        if presets_given {
            settings.split_presets =
                merge_split_presets(std::mem::take(&mut settings.split_presets));
        }
        Ok(Loaded {
            value: settings,
            problems,
        })
    }

    /// Reads settings from TOML text, setting aside the problems; see
    /// [`Settings::read`].
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        Self::read(text).map(|loaded| loaded.value)
    }

    /// Writes every setting as TOML text, for a reference or a test. Saving
    /// to the owner's file goes through [`crate::save_settings`], which
    /// writes only what changed.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(|e| ConfigError::parse(Paths::SETTINGS, e))
    }

    /// Every schema setting's value, by dotted key.
    pub fn values(&self) -> Result<BTreeMap<&'static str, Value>, ConfigError> {
        let table = Value::try_from(self).map_err(|e| ConfigError::parse(Paths::SETTINGS, e))?;
        let mut values = BTreeMap::new();
        if let Value::Table(table) = table {
            for info in &SCHEMA {
                if let Some(value) = get_dotted(&table, info.key) {
                    values.insert(info.key, value.clone());
                }
            }
        }
        Ok(values)
    }

    /// The value of one setting in words, for the Settings list: a choice
    /// in its words ("side by side"), a flag as "on" or "off", a list
    /// joined with commas.
    pub fn value_words(&self, key: &str) -> String {
        let Some(info) = crate::schema::setting(key) else {
            return String::new();
        };
        let Ok(values) = self.values() else {
            return String::new();
        };
        let Some(value) = values.get(info.key) else {
            return String::new();
        };
        match (info.kind, value) {
            (SettingKind::SplitPresets, _) => crate::schema::list_words(
                &self
                    .split_presets
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>(),
                "and",
            ),
            (SettingKind::Flag, Value::Boolean(true)) => "on".to_owned(),
            (SettingKind::Flag, Value::Boolean(false)) => "off".to_owned(),
            (SettingKind::Choice(choices), Value::String(text)) => choices
                .iter()
                .find(|c| c.value == text)
                .map_or_else(|| text.clone(), |c| c.words.to_owned()),
            (SettingKind::Number { unit, .. }, Value::Integer(n)) => format!("{n} {unit}"),
            (_, Value::String(text)) if text.is_empty() => "none".to_owned(),
            (_, Value::String(text)) => text.clone(),
            (_, Value::Array(items)) if items.is_empty() => "none".to_owned(),
            (_, Value::Array(items)) => items
                .iter()
                .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_owned))
                .collect::<Vec<_>>()
                .join(", "),
            (_, Value::Table(table)) if table.is_empty() => "none".to_owned(),
            (_, Value::Table(table)) => crate::schema::list_words(
                &table.keys().map(String::as_str).collect::<Vec<_>>(),
                "and",
            ),
            (_, other) => other.to_string(),
        }
    }

    /// The columns as kinds, in order; names the schema refused are already
    /// gone.
    pub fn column_kinds(&self) -> Vec<ColumnKind> {
        self.columns
            .iter()
            .filter_map(|c| ColumnKind::from_name(c))
            .collect()
    }

    /// The sort for a new panel.
    pub fn sort_spec(&self) -> SortSpec {
        self.sort.clone()
    }

    /// Whether the sort's key is one a panel offers (not a plugin column).
    pub fn sorts_by_panel_key(&self) -> bool {
        SortKey::panel_keys().contains(&self.sort.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::{FolderOrder, SortDirection};

    #[test]
    fn an_empty_file_gives_the_defaults() {
        assert_eq!(Settings::from_toml("").unwrap(), Settings::default());
    }

    #[test]
    fn every_schema_default_is_the_struct_default() {
        let mut text = String::new();
        let mut tables: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for setting in &SCHEMA {
            match setting.key.split_once('.') {
                Some((table, key)) => tables
                    .entry(table)
                    .or_default()
                    .push(format!("{key} = {}", setting.default)),
                None => text.push_str(&format!("{} = {}\n", setting.key, setting.default)),
            }
        }
        for (table, lines) in tables {
            text.push_str(&format!("[{table}]\n{}\n", lines.join("\n")));
        }
        let loaded = Settings::read(&text).unwrap();
        assert_eq!(loaded.problems, Vec::new());
        assert_eq!(loaded.value, Settings::default());
        let values = Settings::default().values().unwrap();
        for setting in &SCHEMA {
            assert_eq!(
                values.get(setting.key),
                setting.default_value().as_ref(),
                "{}",
                setting.key
            );
        }
    }

    #[test]
    fn unknown_keys_survive_a_round_trip_and_are_said() {
        let text = "show_hidden = true\nfuture_option = 3\n[plugins]\nenabled = [\"thumbs\"]\n[sort]\nkey = \"size\"\nshape = \"round\"\n";
        let loaded = Settings::read(text).unwrap();
        let settings = loaded.value;
        assert!(settings.show_hidden);
        assert_eq!(settings.plugins.enabled, vec!["thumbs".to_owned()]);
        assert_eq!(
            settings.unknown.get("future_option"),
            Some(&Value::Integer(3))
        );
        let said: Vec<&str> = loaded.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "Unknown setting: future_option. Kept as it is.",
                "Unknown setting: sort.shape. Kept as it is."
            ]
        );
        let again = Settings::from_toml(&settings.to_toml().unwrap()).unwrap();
        assert_eq!(again, settings);
    }

    #[test]
    fn a_wrong_value_keeps_its_default_and_is_said_in_words() {
        let text = "show_hidden = \"yes\"\nlayout = \"diagonal\"\nprogress_interval_seconds = 0\ncolumns = [\"type\", \"colour\"]\ntrash_drives = [\"fixed\", \"floppy\"]\n[confirm]\ncopy = 1\n";
        let loaded = Settings::read(text).unwrap();
        assert_eq!(loaded.value, Settings::default());
        let said: Vec<&str> = loaded.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "layout cannot be \"diagonal\": use side_by_side, stacked or single.",
                "show_hidden needs true or false, not \"yes\".",
                "columns cannot hold \"colour\": no column has that name.",
                "progress_interval_seconds cannot be 0: use 1 to 600.",
                "confirm.copy needs true or false, not 1.",
                "trash_drives cannot hold \"floppy\": use fixed, removable or network."
            ]
        );
        for problem in &loaded.problems {
            let key = problem.text.split(' ').next().unwrap();
            assert!(within_cells(&problem.text, key), "{}", problem.text);
        }
        assert!(Settings::from_toml("show_hidden = ").is_err());
    }

    #[test]
    fn the_sort_starts_in_its_keys_first_direction() {
        let settings = Settings::from_toml("[sort]\nkey = \"size\"\nfolders = \"last\"\n").unwrap();
        assert_eq!(settings.sort.key, SortKey::Size);
        assert_eq!(settings.sort.direction, SortDirection::Descending);
        assert_eq!(settings.sort.folders, FolderOrder::Last);
        let settings =
            Settings::from_toml("[sort]\nkey = \"size\"\ndirection = \"ascending\"\n").unwrap();
        assert_eq!(settings.sort.direction, SortDirection::Ascending);
        assert!(settings.sorts_by_panel_key());
    }

    #[test]
    fn the_split_presets_default_is_the_built_in_table_with_its_sources() {
        let info = crate::schema::setting("split_presets").unwrap();
        let default = info.default_value().unwrap();
        let expected = Value::try_from(builtin_split_presets()).unwrap();
        assert_eq!(default, expected);
        for builtin in &crate::presets::BUILTIN_SPLIT_PRESETS {
            let figure = diroxide_core::format::grouped(builtin.bytes);
            assert!(
                info.help
                    .contains(&format!("{}: {figure} bytes", builtin.name)),
                "{}: {figure}",
                builtin.name
            );
        }
        assert_eq!(
            Settings::default().value_words("split_presets"),
            "Email attachment, GitHub file limit, CD, FAT32's largest file, DVD, Blu-ray and Floppy"
        );
    }

    #[test]
    fn a_users_preset_round_trips_and_its_unknown_key_is_said() {
        let text = "[[split_presets]]\nname = \"Zip disk\"\nbytes = 100663296\nlabel = \"blue\"\n\n[[split_presets]]\nname = \"dvd\"\nbytes = 4700372992\nheadroom_percent = 2\n";
        let loaded = Settings::read(text).unwrap();
        let said: Vec<&str> = loaded.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            ["Unknown key in split_presets, Zip disk: label. Kept as it is."]
        );
        let presets = &loaded.value.split_presets;
        assert_eq!(presets.len(), 8, "seven built-in ones and the user's");
        assert_eq!(presets[4].name, "dvd", "the built-in DVD replaced in place");
        assert_eq!(presets[4].headroom_percent, 2);
        assert_eq!(presets[7].name, "Zip disk");
        assert_eq!(presets[7].part_bytes(), 100_663_296);
        assert_eq!(
            presets[7].unknown.get("label"),
            Some(&Value::String("blue".into()))
        );
        let again = Settings::read(&loaded.value.to_toml().unwrap()).unwrap();
        assert_eq!(again.value, loaded.value);
    }

    #[test]
    fn a_bad_preset_keeps_the_built_in_ones_and_is_said_in_words() {
        let cases = [
            (
                "[[split_presets]]\nname = \"Zip disk\"\nbytes = 0\n",
                "split_presets preset Zip disk needs bytes, a whole number above 0.",
            ),
            (
                "[[split_presets]]\nbytes = 10\n",
                "split_presets preset 1 needs a name in quotes.",
            ),
            (
                "[[split_presets]]\nname = \"Tape\"\nbytes = 10\nheadroom_percent = 80\n",
                "split_presets preset Tape cannot keep 80 percent free: use 0 to 50.",
            ),
            (
                "split_presets = \"big\"\n",
                "split_presets needs a list of presets, as [[split_presets]] tables with a name and bytes.",
            ),
        ];
        for (text, sentence) in cases {
            let loaded = Settings::read(text).unwrap();
            assert_eq!(
                loaded.value.split_presets,
                builtin_split_presets(),
                "{text}"
            );
            let said: Vec<&str> = loaded.problems.iter().map(|p| p.text.as_str()).collect();
            assert_eq!(said, [sentence]);
            assert!(within_cells(sentence, "split_presets"));
        }
    }

    #[test]
    fn the_archive_limits_start_at_r7s_numbers_and_say_unknown_keys() {
        let archives = ArchiveSettings::default();
        assert_eq!(ArchiveSettings::gib(archives.max_member_gib), 8 << 30);
        assert_eq!(ArchiveSettings::gib(archives.ask_total_gib), 32 << 30);
        assert_eq!(
            ArchiveSettings::gib(archives.max_total_gib),
            1 << 40,
            "1 TiB"
        );
        assert_eq!(ArchiveSettings::mib(archives.ask_dictionary_mib), 256 << 20);
        assert_eq!(
            ArchiveSettings::mib(archives.max_dictionary_mib),
            1_536 << 20
        );
        assert_eq!(ArchiveSettings::mib(archives.max_index_mib), 256 << 20);
        assert_eq!(archives.max_entries, 1_000_000);
        assert_eq!(archives.max_nesting, 4);
        assert_eq!(ArchiveSettings::gib(u64::MAX), u64::MAX, "saturates");
        let loaded = Settings::read(
            "[archives]\nmax_entries = 500000\nmax_size = 3\n[viewer]\nwrap = false\n",
        )
        .unwrap();
        assert_eq!(loaded.value.archives.max_entries, 500_000);
        assert!(!loaded.value.viewer.wrap);
        let said: Vec<&str> = loaded.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(said, ["Unknown setting: archives.max_size. Kept as it is."]);
        let refused = Settings::read("[archives]\nmax_nesting = 9\n").unwrap();
        assert_eq!(refused.value.archives.max_nesting, 4);
        assert_eq!(
            refused.problems[0].text,
            "archives.max_nesting cannot be 9: use 1 to 8."
        );
        let not_a_table = Settings::read("viewer = 3\n").unwrap();
        assert_eq!(
            not_a_table.problems[0].text,
            "viewer needs a table, as [viewer]. The defaults are in use."
        );
        assert_eq!(HexRow::Auto.bytes_per_row(true), 8);
        assert_eq!(HexRow::Auto.bytes_per_row(false), 16);
        assert_eq!(HexRow::Wide.bytes_per_row(true), 16);
        assert_eq!(QuickViewSettings::default().wait_ms, 500);
        assert_eq!(ViewerSettings::default().highlight_max_kib, 512);
    }

    #[test]
    fn values_are_said_in_words() {
        let settings = Settings::default();
        assert_eq!(settings.value_words("layout"), "side by side");
        assert_eq!(settings.value_words("show_hidden"), "off");
        assert_eq!(
            settings.value_words("progress_interval_seconds"),
            "10 seconds"
        );
        assert_eq!(settings.value_words("columns"), "type, size, modified");
        assert_eq!(settings.value_words("colors"), "none");
        assert_eq!(settings.value_words("components_source"), "none");
        assert_eq!(settings.value_words("theme"), "galaxy");
        assert_eq!(settings.value_words("nothing"), "");
        assert_eq!(
            settings.column_kinds(),
            [ColumnKind::Type, ColumnKind::Size, ColumnKind::Modified]
        );
        assert!(SingleLetters::TypeAhead.types_ahead());
        assert!(!SingleLetters::Off.keeps_bindings());
    }
}
