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
    /// A list of split presets, `[[split_presets]]` tables each with a
    /// name, a size in bytes and an optional headroom in percent
    /// ([`crate::presets`]).
    SplitPresets,
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
            SettingKind::SplitPresets => {
                "a list of presets, each with a name, its size in bytes and the percent kept free"
                    .to_owned()
            }
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
    /// The viewer and the quick view (alpha.2).
    Viewer,
    /// Archives: the limits against hostile ones, and split presets
    /// (alpha.2).
    Archives,
    /// Search and the filter.
    Search,
    /// Comparing files (alpha.3).
    Compare,
    /// Plugins and components.
    Plugins,
}

impl SettingGroup {
    /// Every group, in order.
    pub const ALL: [SettingGroup; 10] = [
        SettingGroup::Panels,
        SettingGroup::Keys,
        SettingGroup::Speech,
        SettingGroup::Safety,
        SettingGroup::Look,
        SettingGroup::Viewer,
        SettingGroup::Archives,
        SettingGroup::Search,
        SettingGroup::Compare,
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
            SettingGroup::Viewer => "Viewer and quick view",
            SettingGroup::Archives => "Archives and split",
            SettingGroup::Search => "Search and filter",
            SettingGroup::Compare => "Compare",
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
    choice("auto", "as you type in the GUI, on Tab in a terminal"),
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

const HEX_ROWS: &[Choice] = &[
    choice(
        "auto",
        "eight bytes in screen reader mode, sixteen otherwise",
    ),
    choice("narrow", "eight bytes, a row within a Braille line"),
    choice("wide", "sixteen bytes"),
];

/// The default of `split_presets`, as TOML: the built-in table of
/// [`crate::presets::BUILTIN_SPLIT_PRESETS`], which a test keeps equal.
pub const SPLIT_PRESETS_DEFAULT: &str = r#"[{ name = "Email attachment", bytes = 18000000 }, { name = "GitHub file limit", bytes = 104857600 }, { name = "CD", bytes = 737280000, headroom_percent = 1 }, { name = "FAT32's largest file", bytes = 4294967295 }, { name = "DVD", bytes = 4700372992, headroom_percent = 1 }, { name = "Blu-ray", bytes = 25025314816, headroom_percent = 1 }, { name = "Floppy", bytes = 1457664 }]"#;

const COMPARE_ALGORITHMS: &[Choice] = &[
    choice("patience", "patience, lines that occur once matched first"),
    choice("histogram", "histogram, like patience with repeats"),
    choice("myers", "Myers, the fewest changes"),
];

const LARGE_FILES: &[Choice] = &[
    choice("ask", "ask each time"),
    choice("head", "compare their first part as text"),
    choice("bytes", "compare them byte by byte"),
];

const WEEK_STARTS: &[Choice] = &[
    choice("sunday", "Sunday"),
    choice("monday", "Monday"),
    choice("saturday", "Saturday"),
];

use SettingGroup as G;
use SettingKind as K;

/// Every setting, in Settings list order.
pub const SCHEMA: [SettingInfo; 91] = [
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
    // The F4 editor (the owner's answer 74): the setting first, then
    // $VISUAL, $EDITOR and the platform's editor. Added by W1d-2.
    SettingInfo {
        key: "editor",
        name: "Editor",
        group: G::Panels,
        kind: K::Text,
        default: "\"\"",
        help: "The program Edit opens a file with: a name on the path or a full path, with no arguments. Empty uses VISUAL, then EDITOR, then the system's own editor.",
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
        arrives: None,
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
        help: "When the candidates appear under a text field: as you type, or only when you ask for them. Auto lists them as you type in the graphical interface (GUI) and on request in a terminal.",
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
        key: "key_bar",
        name: "Key bar",
        group: G::Look,
        kind: K::Flag,
        default: "true",
        help: "Shows the common keys and what each one does on the last line of a terminal, in every mode. Say Key Bar reads it aloud.",
        arrives: None,
    },
    SettingInfo {
        key: "highlighting",
        name: "Code colors",
        group: G::Look,
        kind: SettingKind::Choice(HIGHLIGHTING),
        default: "\"auto\"",
        help: "Colors code in the viewer. Auto colors it except in screen reader mode.",
        arrives: None,
    },
    SettingInfo {
        key: "thumbnails",
        name: "Thumbnails",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "Shows pictures as small images in the GUI's thumbnail mode. The words stay.",
        arrives: Some("alpha.7"),
    },
    SettingInfo {
        key: "auto_hide_menu",
        name: "Hide the menu bar",
        group: G::Look,
        kind: K::Flag,
        default: "true",
        help: "In the GUI on Windows, keeps the menu bar hidden until the Menus command or Alt alone shows it.",
        arrives: Some("alpha.6"),
    },
    SettingInfo {
        key: "focus_mode",
        name: "Focus mode",
        group: G::Look,
        kind: K::Flag,
        default: "false",
        help: "In the GUI, fades the bars when the mouse rests, so the panels have the room. Visual only; nothing is announced.",
        arrives: Some("alpha.6"),
    },
    // The viewer (alpha.2, W2d; research/alpha2-plan.md part three and
    // R17's caps).
    SettingInfo {
        key: "viewer.wrap",
        name: "Wrap long lines",
        group: G::Viewer,
        kind: K::Flag,
        default: "true",
        help: "Continues a line longer than the screen on the next rows, each marked as continued, instead of cutting it off.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.wrap_width",
        name: "Wrap width",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 0,
            max: 1000,
            unit: "characters",
        },
        default: "0",
        help: "Where long lines wrap: 0 wraps at the width of the screen, and a number such as 40 wraps at that many characters, one Braille line.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.line_numbers",
        name: "Line numbers",
        group: G::Viewer,
        kind: K::Flag,
        default: "false",
        help: "Starts each line in the viewer with its number. The line's number is always said on request.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.follow",
        name: "Follow a growing file",
        group: G::Viewer,
        kind: K::Flag,
        default: "false",
        help: "Reads more of a file as it grows, as a log does, and stays at its end.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.hex_row",
        name: "Hex row width",
        group: G::Viewer,
        kind: SettingKind::Choice(HEX_ROWS),
        default: "\"auto\"",
        help: "How many bytes a row of hex shows. Eight fit a 40-cell Braille line with their offset and their letters.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.member_cache_mib",
        name: "Archive file memory",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 1,
            max: 4096,
            unit: "MiB",
        },
        default: "64",
        help: "How much of a file inside an archive the viewer keeps decoded, to move back and forth in it. Past that, the viewer says how much it shows.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.highlight_max_kib",
        name: "Code colors up to",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 1,
            max: 65_536,
            unit: "KiB",
        },
        default: "512",
        help: "Code colors stay off for a larger file, and the status line says so. A big file then opens at once.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.highlight_max_lines",
        name: "Code colors up to lines",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 100,
            max: 1_000_000,
            unit: "lines",
        },
        default: "10000",
        help: "Code colors stay off for a file with more lines than this.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.highlight_max_line_chars",
        name: "Code colors line limit",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 100,
            max: 100_000,
            unit: "characters",
        },
        default: "2000",
        help: "A line longer than this keeps no colors, so one huge line cannot stall the viewer.",
        arrives: None,
    },
    SettingInfo {
        key: "viewer.highlight_deadline_ms",
        name: "Code colors time limit",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 100,
            max: 60_000,
            unit: "milliseconds",
        },
        default: "2000",
        help: "How long finding the code colors may take. Past it they stop for the session, and the reason is said.",
        arrives: None,
    },
    // The quick view (alpha.2, W2e; plan section 5.9 and answer 15).
    SettingInfo {
        key: "quick_view.wait_ms",
        name: "Quick view wait",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 0,
            max: 5000,
            unit: "milliseconds",
        },
        default: "500",
        help: "How long the quick view waits after the cursor stops before it shows the file, so moving fast does not read every file on the way.",
        arrives: None,
    },
    SettingInfo {
        key: "quick_view.max_kib",
        name: "Quick view reads up to",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 16,
            max: 65_536,
            unit: "KiB",
        },
        default: "1024",
        help: "The most of a file the quick view reads. Past it, the quick view shows the start and says the file's size.",
        arrives: None,
    },
    SettingInfo {
        key: "quick_view.deadline_ms",
        name: "Quick view time limit",
        group: G::Viewer,
        kind: SettingKind::Number {
            min: 100,
            max: 60_000,
            unit: "milliseconds",
        },
        default: "2000",
        help: "How long reading a document may take. Past it, the quick view stops and shows the file's text instead.",
        arrives: None,
    },
    SettingInfo {
        key: "quick_view.follow_changes",
        name: "Quick view follows changes",
        group: G::Viewer,
        kind: K::Flag,
        default: "true",
        help: "Shows the file again when it changes on disk, such as a note being written in another program, and says it was updated.",
        arrives: None,
    },
    SettingInfo {
        key: "quick_view.line_numbers",
        name: "Quick view line numbers",
        group: G::Viewer,
        kind: K::Flag,
        default: "false",
        help: "Starts each line of plain text and code in the quick view with its number.",
        arrives: None,
    },
    // The archive limits (alpha.2, W2a; research/archives.md section 6,
    // R7's starting numbers, plan question 11 of R20).
    SettingInfo {
        key: "archives.max_member_gib",
        name: "Largest member",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 1_048_576,
            unit: "GiB",
        },
        default: "8",
        help: "Asks before reading one member larger than this, or one behind this much packed data in a tar.gz or tar.xz, which must be read from the start.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.ask_total_gib",
        name: "Ask above this total",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 1_048_576,
            unit: "GiB",
        },
        default: "32",
        help: "Asks before an unpack or a test that would write out more than this in all, since a small archive can expand to fill a drive.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_total_gib",
        name: "Refuse above this total",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 1_048_576,
            unit: "GiB",
        },
        default: "1024",
        help: "Refuses an operation that would expand to more than this in all. Past it, an archive is a bomb, not a backup.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_ratio",
        name: "Largest expansion",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 10,
            max: 1_000_000,
            unit: "times its packed size",
        },
        default: "1000",
        help: "Asks when one member grows more than this many times its packed size. Deflate cannot pass about 1,032 times, so more is a bomb or a run of zeros.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.ratio_floor_mib",
        name: "Expansion judged after",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 65_536,
            unit: "MiB",
        },
        default: "64",
        help: "How much one member writes before its expansion is judged, so a small file that packs well is never asked about.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.ask_dictionary_mib",
        name: "Ask above this dictionary",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 65_536,
            unit: "MiB",
        },
        default: "256",
        help: "Asks before opening a 7z or xz archive whose decoder needs a larger dictionary. 7-Zip's normal level uses 16 MiB, and xz's largest preset 64 MiB.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_dictionary_mib",
        name: "Largest dictionary",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 65_536,
            unit: "MiB",
        },
        default: "1536",
        help: "Refuses a larger dictionary, 7-Zip's own most, before any memory is taken for it.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_header_mib",
        name: "Largest header",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 65_536,
            unit: "MiB",
        },
        default: "256",
        help: "Refuses an archive whose header or list of contents is larger than this, which no real archive needs.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_index_mib",
        name: "Largest member list",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 65_536,
            unit: "MiB",
        },
        default: "256",
        help: "Refuses to list an archive whose members would take more memory than this to keep in a list.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_entries",
        name: "Most members",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1000,
            max: 100_000_000,
            unit: "members",
        },
        default: "1000000",
        help: "Refuses an archive with more members than this. A million covers the largest real archives.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_name_bytes",
        name: "Longest member name",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 255,
            max: 65_536,
            unit: "bytes",
        },
        default: "4096",
        help: "Skips a member whose full name is longer than this and says so, since no drive could store it.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_component_bytes",
        name: "Longest name part",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 64,
            max: 4096,
            unit: "bytes",
        },
        default: "255",
        help: "Skips a member with a folder or file name longer than this and says so: the limit of most drives.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_link_target_bytes",
        name: "Longest link target",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 255,
            max: 65_536,
            unit: "bytes",
        },
        default: "4096",
        help: "Skips a link inside an archive whose target is longer than this and says so.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_memory_mib",
        name: "Most decoder memory",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 16,
            max: 65_536,
            unit: "MiB",
        },
        default: "1024",
        help: "Refuses an operation whose decoder would need more memory than this.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.spool_memory_mib",
        name: "Inner archive in memory",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 4096,
            unit: "MiB",
        },
        default: "64",
        help: "Keeps an archive inside an archive in memory up to this size. A larger one goes to the spool folder in the state folder, never the system's temporary folder.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.spool_disk_gib",
        name: "Inner archive spool",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 1024,
            unit: "GiB",
        },
        default: "4",
        help: "The most one operation may write to the spool folder for archives inside archives, across every level, after a free-space check.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.max_nesting",
        name: "Archives inside archives",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 8,
            unit: "levels",
        },
        default: "4",
        help: "How many archives deep a place may go, such as a zip inside a zip. A level past this is refused in words.",
        arrives: None,
    },
    // The archive provider's own settings beside the limits (P2b, W2a's
    // request 3): the app passes them to the provider's builder.
    SettingInfo {
        key: "archives.ask_read_ahead_mib",
        name: "Ask before reading ahead",
        group: G::Archives,
        kind: SettingKind::Number {
            min: 1,
            max: 1_048_576,
            unit: "MiB",
        },
        default: "256",
        help: "Asks before one member of a tar.gz, a tar.xz or a solid 7z is read on its own when more than this must be read first, since those archives are read from the start.",
        arrives: None,
    },
    SettingInfo {
        key: "archives.spool_folder",
        name: "Spool folder",
        group: G::Archives,
        kind: K::Text,
        default: "\"\"",
        help: "Where an archive inside an archive is kept while it is open, when it is too large for memory: a full path. Empty uses the spool folder in the state folder, never the system's temporary folder.",
        arrives: None,
    },
    // Split and join (alpha.2, W2c; plan section 5.4, research/alpha2-plan.md
    // part 5.3 with its sources).
    SettingInfo {
        key: "split_presets",
        name: "Split presets",
        group: G::Archives,
        kind: K::SplitPresets,
        default: SPLIT_PRESETS_DEFAULT,
        help: "The sizes Split offers, each a use and its size in bytes. Presets you add as [[split_presets]] tables follow these, and one with a built-in name replaces it. Email attachment: 18,000,000 bytes, a 25 MB mail limit after base64 adds about 37 percent (RFC 2045). GitHub file limit: 104,857,600 bytes, its 100 MiB limit. CD: 737,280,000 bytes, an 80-minute CD-R. FAT32's largest file: 4,294,967,295 bytes, 4 GiB less one byte. DVD: 4,700,372,992 bytes, a single-layer DVD+R. Blu-ray: 25,025,314,816 bytes, a single layer. Floppy: 1,457,664 bytes, the data area of a formatted 1.44 MB disk. The disc sizes keep 1 percent free for the file system.",
        arrives: None,
    },
    SettingInfo {
        key: "filter_case",
        name: "Filter case",
        group: G::Search,
        kind: SettingKind::Choice(FILTER_CASES),
        default: "\"smart\"",
        help: "Whether case matters in filters and searches. Smart: only when the text has a capital letter.",
        arrives: None,
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
        arrives: None,
    },
    SettingInfo {
        key: "week_starts_on",
        name: "First day of the week",
        group: G::Search,
        kind: SettingKind::Choice(WEEK_STARTS),
        default: "\"sunday\"",
        help: "The day this week and last week start on in filters and searches.",
        arrives: None,
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
        arrives: None,
    },
    SettingInfo {
        key: "search_respect_ignore",
        name: "Search skips ignored files",
        group: G::Search,
        kind: K::Flag,
        default: "false",
        help: "Leaves out files that .gitignore and similar files name.",
        arrives: None,
    },
    // Compare files (alpha.3, W3a from W3d's notes; research/diffing.md).
    SettingInfo {
        key: "compare.ignore_line_endings",
        name: "Ignore line endings",
        group: G::Compare,
        kind: K::Flag,
        default: "true",
        help: "Compares lines without regard to how they end, so a file from Windows and the same file from Linux are the same text.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.ignore_trailing_space",
        name: "Ignore trailing spaces",
        group: G::Compare,
        kind: K::Flag,
        default: "false",
        help: "Leaves out spaces and tabs at the end of a line when comparing.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.ignore_space_amount",
        name: "Ignore how much space",
        group: G::Compare,
        kind: K::Flag,
        default: "false",
        help: "Counts any run of spaces and tabs as one space when comparing.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.ignore_all_space",
        name: "Ignore all spaces",
        group: G::Compare,
        kind: K::Flag,
        default: "false",
        help: "Leaves out every space and tab when comparing.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.ignore_blank_lines",
        name: "Ignore blank lines",
        group: G::Compare,
        kind: K::Flag,
        default: "false",
        help: "Leaves out lines that are empty or only spaces when comparing.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.ignore_case",
        name: "Ignore case",
        group: G::Compare,
        kind: K::Flag,
        default: "false",
        help: "Compares capital and small letters as the same.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.algorithm",
        name: "Line matching",
        group: G::Compare,
        kind: SettingKind::Choice(COMPARE_ALGORITHMS),
        default: "\"patience\"",
        help: "How the lines of two files are matched. Patience gives changes that read best.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.deadline_ms",
        name: "Compare time limit",
        group: G::Compare,
        kind: SettingKind::Number {
            min: 100,
            max: 60_000,
            unit: "milliseconds",
        },
        default: "2000",
        help: "How long a compare may look for the fewest changes before it settles for a longer list, and says so.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.text_limit_mib",
        name: "Text compare limit",
        group: G::Compare,
        kind: SettingKind::Number {
            min: 1,
            max: 1024,
            unit: "MiB",
        },
        default: "16",
        help: "The largest file compared as text in full; a larger one follows Larger files.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.context",
        name: "Lines around a change",
        group: G::Compare,
        kind: SettingKind::Number {
            min: 0,
            max: 9,
            unit: "lines",
        },
        default: "2",
        help: "How many same lines the unified view shows around each change.",
        arrives: None,
    },
    SettingInfo {
        key: "compare.large_files",
        name: "Larger files",
        group: G::Compare,
        kind: SettingKind::Choice(LARGE_FILES),
        default: "\"ask\"",
        help: "What a compare does with a file over the text compare limit.",
        arrives: None,
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
