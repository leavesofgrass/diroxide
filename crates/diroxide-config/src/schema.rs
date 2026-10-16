//! The settings schema: every setting's key, name, kind, default, range
//! and help, in words. It checks `config.toml` as it is read, generates
//! `docs/settings-reference.md` (`cargo xtask regen`), and feeds the
//! Settings list with its F1 help.
//!
//! Settings marked with a later alpha are reserved now, so their names
//! and defaults are fixed before the code that reads them arrives; they
//! are listed and kept, and do nothing yet.

use crate::columns::ColumnKind;

/// One choice of a [`SettingKind::Choice`] setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The value as written in `config.toml`.
    pub value: &'static str,
    /// The value in words, for the Settings list.
    pub words: &'static str,
}

const fn choice(value: &'static str, words: &'static str) -> Choice {
    Choice { value, words }
}

/// What a setting holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// True or false.
    Flag,
    /// A whole number in a range, with its unit in words.
    Number {
        /// The smallest value.
        min: i64,
        /// The largest value.
        max: i64,
        /// The unit, such as "seconds".
        unit: &'static str,
    },
    /// One of a list of words.
    Choice(&'static [Choice]),
    /// Free text.
    Text,
    /// A theme's name: a built-in one or a file in the themes folder.
    Theme,
    /// A list of column kinds ([`ColumnKind`]).
    Columns,
    /// A list of drive kinds: fixed, removable, network.
    DriveKinds,
    /// A table of screen roles and their colors, `role = "#rrggbb"`.
    Colors,
    /// A list of names.
    Names,
}

impl SettingKind {
    /// The kind in words, for the reference page.
    pub fn words(&self) -> String {
        match self {
            SettingKind::Flag => "true or false".to_owned(),
            SettingKind::Number { min, max, unit } => format!("a number of {unit}, {min} to {max}"),
            SettingKind::Choice(choices) => {
                let values: Vec<&str> = choices.iter().map(|c| c.value).collect();
                format!("one of {}", list_words(&values, "or"))
            }
            SettingKind::Text => "text".to_owned(),
            SettingKind::Theme => "a theme's name".to_owned(),
            SettingKind::Columns => {
                let names: Vec<&str> = ColumnKind::ALL.iter().map(|c| c.name()).collect();
                format!("a list of columns: {}", list_words(&names, "and"))
            }
            SettingKind::DriveKinds => {
                "a list of drive kinds: fixed, removable and network".to_owned()
            }
            SettingKind::Colors => {
                "a table of screen roles, each with a color as #rrggbb".to_owned()
            }
            SettingKind::Names => "a list of names".to_owned(),
        }
    }
}

/// Words joined as said: "a", "a or b", "a, b or c".
pub fn list_words(items: &[&str], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., final_item] => format!("{} {last} {final_item}", rest.join(", ")),
    }
}

/// The groups of the Settings list, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SettingGroup {
    /// The panels: layout, sorting, columns, hidden files.
    Panels,
    /// Keys and letters.
    Keys,
    /// Speech, Braille and the screen reader.
    Speech,
    /// Confirmations, the trash and safe names.
    Safety,
    /// Themes and colors.
    Look,
    /// Search and the filter.
    Search,
    /// Plugins and components.
    Plugins,
}

impl SettingGroup {
    /// Every group, in order.
    pub const ALL: [SettingGroup; 7] = [
        SettingGroup::Panels,
        SettingGroup::Keys,
        SettingGroup::Speech,
        SettingGroup::Safety,
        SettingGroup::Look,
        SettingGroup::Search,
        SettingGroup::Plugins,
    ];

    /// The group's title.
    pub const fn title(self) -> &'static str {
        match self {
            SettingGroup::Panels => "Panels",
            SettingGroup::Keys => "Keys",
            SettingGroup::Speech => "Speech and Braille",
            SettingGroup::Safety => "Safety",
            SettingGroup::Look => "Look",
            SettingGroup::Search => "Search and filter",
            SettingGroup::Plugins => "Plugins",
        }
    }
}

/// One setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingInfo {
    /// The key in `config.toml`; a dot means a table (`sort.key` is `key`
    /// under `[sort]`).
    pub key: &'static str,
    /// The name in the Settings list, meaning first.
    pub name: &'static str,
    /// Where it is listed.
    pub group: SettingGroup,
    /// What it holds.
    pub kind: SettingKind,
    /// The default, as written in TOML.
    pub default: &'static str,
    /// F1 help: one or two plain sentences.
    pub help: &'static str,
    /// `None` when the setting is used now; else the alpha that starts
    /// using it ("alpha.3"). Reserved settings are kept and listed.
    pub arrives: Option<&'static str>,
}

impl SettingInfo {
    /// The default as a TOML value.
    pub fn default_value(&self) -> Option<toml::Value> {
        let text = format!("value = {}", self.default);
        let table: toml::Table = toml::from_str(&text).ok()?;
        table.get("value").cloned()
    }

    /// The setting's row in the Settings list, meaning first: "Layout:
    /// side by side".
    pub fn row_text(&self, value: &str) -> String {
        format!("{}: {value}", self.name)
    }
}

const LAYOUTS: &[Choice] = &[
    choice("side_by_side", "side by side"),
    choice("stacked", "stacked"),
    choice("single", "single panel"),
];

const SORT_KEYS: &[Choice] = &[
    choice("name", "name"),
    choice("extension", "extension"),
    choice("size", "size"),
    choice("modified", "modified time"),
    choice("created", "created time"),
    choice("accessed", "accessed time"),
    choice("kind", "kind"),
    choice("attributes", "attributes"),
    choice("owner", "owner"),
    choice("label", "label"),
    choice("unsorted", "the folder's own order"),
];

const DIRECTIONS: &[Choice] = &[
    choice("ascending", "A to Z, smallest or oldest first"),
    choice("descending", "Z to A, largest or newest first"),
];

const FOLDER_ORDERS: &[Choice] = &[
    choice("first", "folders first"),
    choice("mixed", "folders among files"),
    choice("last", "folders last"),
];

const CASES: &[Choice] = &[
    choice("ignore", "capitals sort with small letters"),
    choice("respect", "capitals sort apart"),
];

const SINGLE_LETTERS: &[Choice] = &[
    choice("type_ahead", "letters jump to names"),
    choice("commands", "letters run commands"),
    choice("off", "single keys do nothing"),
];

const READER_MODES: &[Choice] = &[
    choice("auto", "on when a screen reader runs"),
    choice("on", "on"),
    choice("off", "off"),
];

const COMPLETION_LISTS: &[Choice] = &[
    choice("auto", "as you type in the window, on Tab in a terminal"),
    choice("as_you_type", "as you type"),
    choice("on_tab", "on Tab"),
];

const SECRET_ECHOES: &[Choice] = &[
    choice("asterisks", "an asterisk per character"),
    choice("silent", "nothing"),
];

const HIGHLIGHTING: &[Choice] = &[
    choice("auto", "colors, except in screen reader mode"),
    choice("on", "colors"),
    choice("off", "no colors"),
];

const FILTER_CASES: &[Choice] = &[
    choice("smart", "case matters only with a capital"),
    choice("ignore", "case never matters"),
    choice("match", "case always matters"),
];

const WEEK_STARTS: &[Choice] = &[
    choice("sunday", "Sunday"),
    choice("monday", "Monday"),
    choice("saturday", "Saturday"),
];

use SettingGroup as G;
use SettingKind as K;

/// Every setting, in Settings list order.
pub const SCHEMA: [SettingInfo; 43] = [
    SettingInfo {
        key: "layout",
        name: "Layout",
        group: G::Panels,
        kind: SettingKind::Choice(LAYOUTS),
        default: "\"side_by_side\"",
        help: "How the two panels share the screen: side by side, stacked one above the other, or one panel at a time.",
        arrives: None,
    },
    SettingInfo {
        key: "show_hidden",
        name: "Hidden files",
        group: G::Panels,
        kind: K::Flag,
        default: "false",
        help: "Lists files and folders that are hidden by their name or their attributes.",
        arrives: None,
    },
    SettingInfo {
        key: "sort.key",
        name: "Sort by",
        group: G::Panels,
        kind: SettingKind::Choice(SORT_KEYS),
        default: "\"name\"",
        help: "What a new panel sorts by. The sort commands and the Sort list change it for one panel.",
        arrives: None,
    },
    SettingInfo {
        key: "sort.direction",
        name: "Sort direction",
        group: G::Panels,
        kind: SettingKind::Choice(DIRECTIONS),
        default: "\"ascending\"",
        help: "Which way the sort runs. Left out, each key starts its own way: names A to Z, sizes largest first, dates newest first.",
        arrives: None,
    },
    SettingInfo {
        key: "sort.folders",
        name: "Folders in the sort",
        group: G::Panels,
        kind: SettingKind::Choice(FOLDER_ORDERS),
        default: "\"first\"",
        help: "Where folders go: before the files, among them, or after them.",
        arrives: None,
    },
    SettingInfo {
        key: "sort.case",
        name: "Capitals in the sort",
        group: G::Panels,
        kind: SettingKind::Choice(CASES),
        default: "\"ignore\"",
        help: "Whether names with capital letters sort apart from the same names in small letters.",
        arrives: None,
    },
    SettingInfo {
        key: "sort.natural",
        name: "Numbers in names",
        group: G::Panels,
        kind: K::Flag,
        default: "true",
        help: "Sorts numbers in names by their value, so file2 comes before file10.",
        arrives: None,
    },
    SettingInfo {
        key: "columns",
        name: "Columns",
        group: G::Panels,
        kind: K::Columns,
        default: "[\"type\", \"size\", \"modified\"]",
        help: "The details said after each name, in this order. Permissions read in words; mode and octal give the Unix forms.",
        arrives: None,
    },
    SettingInfo {
        key: "single_letters",
        name: "Single letters",
        group: G::Keys,
        kind: SettingKind::Choice(SINGLE_LETTERS),
        default: "\"type_ahead\"",
        help: "What a key without Control or Alt does in a panel: jump to a name, run a command, or nothing. Off turns every such key off; the Filter command still filters.",
        arrives: None,
    },
    SettingInfo {
        key: "preset",
        name: "Key preset",
        group: G::Keys,
        kind: K::Text,
        default: "\"diroxide\"",
        help: "The set of keys under your own keys in keymap.toml: diroxide, or a preset modeled on another file manager.",
        arrives: Some("alpha.3"),
    },
    SettingInfo {
        key: "screen_reader_mode",
        name: "Screen reader mode",
        group: G::Speech,
        kind: SettingKind::Choice(READER_MODES),
        default: "\"auto\"",
        help: "Behaves for a screen reader: says changes once, keeps the cursor on the focused row, and spaces out progress.",
        arrives: None,
    },
    SettingInfo {
        key: "quiet_screen",
        name: "Quiet screen",
        group: G::Speech,
        kind: K::Flag,
        default: "false",
        help: "Draws as little as possible, so a screen reader that reads every change stays quiet.",
        arrives: None,
    },
    SettingInfo {
        key: "progress_interval_seconds",
        name: "Progress interval",
        group: G::Speech,
        kind: SettingKind::Number {
            min: 1,
            max: 600,
            unit: "seconds",
        },
        default: "10",
        help: "In screen reader mode, the shortest time between two progress lines. A job's start, its end and its questions are said at once.",
        arrives: None,
    },
    SettingInfo {
        key: "cursor_fallback",
        name: "Cursor on the status line",
        group: G::Speech,
        kind: K::Flag,
        default: "false",
        help: "Puts the terminal's cursor on the status line instead of the focused row, for a screen reader that follows it better there.",
        arrives: None,
    },
    SettingInfo {
        key: "dialog_settle_ms",
        name: "Dialog settle time",
        group: G::Speech,
        kind: SettingKind::Number {
            min: 0,
            max: 2000,
            unit: "milliseconds",
        },
        default: "250",
        help: "How long a new dialog waits before it takes keys, so a key pressed as it opens is not taken as an answer.",
        arrives: None,
    },
    SettingInfo {
        key: "completion_list",
        name: "Completion list",
        group: G::Speech,
        kind: SettingKind::Choice(COMPLETION_LISTS),
        default: "\"auto\"",
        help: "When the candidates appear under a text field: as you type, or only when you ask for them. Auto lists them as you type in the window and on request in a terminal.",
        arrives: None,
    },
    SettingInfo {
        key: "secret_echo",
        name: "Password echo",
        group: G::Speech,
        kind: SettingKind::Choice(SECRET_ECHOES),
        default: "\"asterisks\"",
        help: "What a password field shows for each typed character: an asterisk, which proves a key registered, or nothing.",
        arrives: None,
    },
    SettingInfo {
        key: "secret_show_toggle",
        name: "Passwords shown",
        group: G::Speech,
        kind: K::Flag,
        default: "false",
        help: "Starts password fields with the typed text shown. Anyone who can see or hear the screen can read it.",
        arrives: None,
    },
    SettingInfo {
        key: "confirm.copy",
        name: "Ask before copying",
        group: G::Safety,
        kind: K::Flag,
        default: "true",
        help: "Asks before a copy, after saying what will be copied. Every question starts on No.",
        arrives: None,
    },
    SettingInfo {
        key: "confirm.move",
        name: "Ask before moving",
        group: G::Safety,
        kind: K::Flag,
        default: "true",
        help: "Asks before a move, after listing what will be moved. Every question starts on No.",
        arrives: None,
    },
    SettingInfo {
        key: "confirm.trash",
        name: "Ask before the trash",
        group: G::Safety,
        kind: K::Flag,
        default: "true",
        help: "Asks before moving items to the trash, after listing them. Deleting for good always asks.",
        arrives: None,
    },
    SettingInfo {
        key: "confirm.quit_with_jobs",
        name: "Ask before quitting",
        group: G::Safety,
        kind: K::Flag,
        default: "true",
        help: "Asks before quitting while a job is still running.",
        arrives: None,
    },
    SettingInfo {
        key: "trash",
        name: "Trash",
        group: G::Safety,
        kind: K::Text,
        default: "\"system\"",
        help: "Where Delete sends things: system for the system's trash, or a folder you name, for tests and portable use.",
        arrives: None,
    },
    SettingInfo {
        key: "trash_drives",
        name: "Drives with a trash",
        group: G::Safety,
        kind: K::DriveKinds,
        default: "[\"fixed\"]",
        help: "Drive kinds where Delete uses the system trash. Elsewhere Delete lists the items and asks before deleting them for good. On Windows only fixed drives keep trashed files safely.",
        arrives: None,
    },
    SettingInfo {
        key: "replace_to_trash",
        name: "Replaced files to the trash",
        group: G::Safety,
        kind: K::Flag,
        default: "true",
        help: "Replacing a file sends the old one to the trash first, where the place has one, so Undo can bring it back.",
        arrives: None,
    },
    SettingInfo {
        key: "portable_names",
        name: "Portable names",
        group: G::Safety,
        kind: K::Flag,
        default: "true",
        help: "Refuses names that other systems cannot store, such as a colon or a trailing space, when diroxide makes or renames an entry.",
        arrives: None,
    },
    SettingInfo {
        key: "theme",
        name: "Theme",
        group: G::Look,
        kind: K::Theme,
        default: "\"galaxy\"",
        help: "The colors. Every theme also marks each state with bold, underline or a word, never by color alone, and its contrast is said when chosen.",
        arrives: None,
    },
    SettingInfo {
        key: "follow_system_appearance",
        name: "Follow the system",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "Switches between the theme and its light or dark partner as the system does.",
        arrives: None,
    },
    SettingInfo {
        key: "colors",
        name: "Your colors",
        group: G::Look,
        kind: K::Colors,
        default: "{}",
        help: "Your own color for any part of the screen, by its role, such as folder = \"#ffaa00\". Each is checked for contrast and the result said in words.",
        arrives: None,
    },
    SettingInfo {
        key: "borders",
        name: "Borders",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "Draws lines around the panels in a terminal, for sighted users. Off by default, because a screen reader reads them as noise.",
        arrives: None,
    },
    SettingInfo {
        key: "compact",
        name: "Compact",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "Hides the key bar and the title line in a terminal. The status and jobs lines stay.",
        arrives: None,
    },
    SettingInfo {
        key: "highlighting",
        name: "Code colors",
        group: G::Look,
        kind: SettingKind::Choice(HIGHLIGHTING),
        default: "\"auto\"",
        help: "Colors code in the viewer. Auto colors it except in screen reader mode.",
        arrives: Some("alpha.2"),
    },
    SettingInfo {
        key: "thumbnails",
        name: "Thumbnails",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "Shows pictures as small images in the window's thumbnail mode. The words stay.",
        arrives: Some("alpha.7"),
    },
    SettingInfo {
        key: "auto_hide_menu",
        name: "Hide the menu bar",
        group: G::Look,
        kind: K::Flag,
        default: "true",
        help: "In the window on Windows, keeps the menu bar hidden until the Menus command or Alt alone shows it.",
        arrives: Some("alpha.6"),
    },
    SettingInfo {
        key: "focus_mode",
        name: "Focus mode",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "In the window, fades the bars when the mouse rests, so the panels have the room. Visual only; nothing is announced.",
        arrives: Some("alpha.6"),
    },
    SettingInfo {
        key: "filter_case",
        name: "Filter case",
        group: G::Search,
        kind: SettingKind::Choice(FILTER_CASES),
        default: "\"smart\"",
        help: "Whether case matters in filters and searches. Smart: only when the text has a capital letter.",
        arrives: Some("alpha.3"),
    },
    SettingInfo {
        key: "filter_pause_ms",
        name: "Filter pause",
        group: G::Search,
        kind: SettingKind::Number {
            min: 0,
            max: 5000,
            unit: "milliseconds",
        },
        default: "400",
        help: "How long the filter waits after the last key before it says the count.",
        arrives: Some("alpha.3"),
    },
    SettingInfo {
        key: "week_starts_on",
        name: "First day of the week",
        group: G::Search,
        kind: SettingKind::Choice(WEEK_STARTS),
        default: "\"sunday\"",
        help: "The day this week and last week start on in filters and searches.",
        arrives: Some("alpha.3"),
    },
    SettingInfo {
        key: "search_row_cap",
        name: "Search result limit",
        group: G::Search,
        kind: SettingKind::Number {
            min: 1000,
            max: 10_000_000,
            unit: "rows",
        },
        default: "100000",
        help: "The most rows a search lists. The search says when it stopped at the limit.",
        arrives: Some("alpha.3"),
    },
    SettingInfo {
        key: "search_respect_ignore",
        name: "Search skips ignored files",
        group: G::Search,
        kind: K::Flag,
        default: "false",
        help: "Leaves out files that .gitignore and similar files name.",
        arrives: Some("alpha.3"),
    },
    SettingInfo {
        key: "plugins.enabled",
        name: "Plugins in use",
        group: G::Plugins,
        kind: K::Names,
        default: "[]",
        help: "The plugins you turned on, by name. None is on until you turn it on.",
        arrives: Some("alpha.4"),
    },
    SettingInfo {
        key: "plugins.timeout_ms",
        name: "Plugin time limit",
        group: G::Plugins,
        kind: SettingKind::Number {
            min: 1000,
            max: 600_000,
            unit: "milliseconds",
        },
        default: "10000",
        help: "How long a plugin may take to answer before it is reported and left.",
        arrives: Some("alpha.4"),
    },
    SettingInfo {
        key: "components_source",
        name: "Components list",
        group: G::Plugins,
        kind: K::Text,
        default: "\"\"",
        help: "Where the list of optional components is read from. Empty means the public list.",
        arrives: Some("alpha.4"),
    },
];

/// A setting by key.
pub fn setting(key: &str) -> Option<&'static SettingInfo> {
    SCHEMA.iter().find(|s| s.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn every_setting_reads_well_and_its_default_parses() {
        let mut keys = std::collections::HashSet::new();
        for s in &SCHEMA {
            assert!(keys.insert(s.key), "duplicate {}", s.key);
            assert!(s.default_value().is_some(), "{}: {}", s.key, s.default);
            assert!(s.help.ends_with('.'), "{}", s.key);
            assert!(s.name.chars().count() <= 30, "{}", s.name);
            assert!(
                s.key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'),
                "{}",
                s.key
            );
            if let SettingKind::Choice(choices) = s.kind {
                let default = s.default.trim_matches('"');
                assert!(choices.iter().any(|c| c.value == default), "{}", s.key);
            }
            if let SettingKind::Number { min, max, .. } = s.kind {
                let value = s.default_value().and_then(|v| v.as_integer()).unwrap();
                assert!((min..=max).contains(&value), "{}", s.key);
            }
            assert!(within_cells(&s.row_text("on"), s.name));
        }
    }

    #[test]
    fn kinds_are_said_in_words() {
        assert_eq!(list_words(&["a", "b", "c"], "or"), "a, b or c");
        assert_eq!(list_words(&["a"], "or"), "a");
        assert_eq!(
            setting("layout").unwrap().kind.words(),
            "one of side_by_side, stacked or single"
        );
        assert_eq!(
            setting("progress_interval_seconds").unwrap().kind.words(),
            "a number of seconds, 1 to 600"
        );
    }
}
