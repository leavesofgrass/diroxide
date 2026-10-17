//! Keymap presets: sets of keys modeled on other file managers, as data
//! (research/alpha3-plan.md, "The keymap presets"; plan section 5.7).
//!
//! - **A preset is a layer.** The keys that apply are the diroxide
//!   defaults, then the chosen preset's lines, then the owner's
//!   `keymap.toml`. Applying a preset sets its name in `config.toml`
//!   (`preset`) and clears nothing, so the owner's own lines stay on top.
//! - **A preset is data.** One file per preset under
//!   `crates/diroxide-app/keys/presets/`, built into the program. A
//!   `[preset]` table gives its name, a description with the meaning
//!   first, the source manager and the single-letter mode
//!   (`type-ahead`, or `commands` for the vim-style set); the key lines
//!   are `keymap.toml`'s own tables (`[terminal.panel]`, `[window.global]`
//!   and the rest), and list only what the preset changes. Key sequences a
//!   source manager uses are listed under `[[held_back]]`, with the
//!   command each would run, since a key is one key here (question 56).
//! - **Every preset passes the same checks before it ships**
//!   ([`check`]): each command is reachable, the terminal column never
//!   uses a key a terminal cannot send and a key a terminal may keep has a
//!   menu as the sure way, no key runs two commands in one layer, no key a
//!   screen reader owns is bound, the critical set keeps its keys, no
//!   keypad key is a command's only key, and a type-ahead preset binds no
//!   letter or digit. `diro keys check` says the result for each preset.

use std::sync::OnceLock;

use diroxide_config::{KeymapConfig, SingleLetters};
use diroxide_core::{KeyColumn, Layer};
use serde::Deserialize;

use super::conflicts::CRITICAL;
use super::names::Platform;
use super::tables::{self, TerminalKind};
use super::{Keymap, written};
use crate::commands::{self, CATALOG};
use crate::menu::Menus;

/// The id of the diroxide defaults, the preset when none is chosen.
pub const DEFAULT_ID: &str = "diroxide";

/// The built-in presets' files, by id, in the chooser's order.
pub const FILES: [(&str, &str); 6] = [
    (DEFAULT_ID, include_str!("../../keys/presets/diroxide.toml")),
    (
        "total-commander",
        include_str!("../../keys/presets/total-commander.toml"),
    ),
    (
        "midnight-commander",
        include_str!("../../keys/presets/midnight-commander.toml"),
    ),
    (
        "explorer-finder",
        include_str!("../../keys/presets/explorer-finder.toml"),
    ),
    ("worker", include_str!("../../keys/presets/worker.toml")),
    (
        "vim-style",
        include_str!("../../keys/presets/vim-style.toml"),
    ),
];

/// A key sequence a source manager uses, held back.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldBack {
    /// The sequence, as the source writes it: "ctrl+x c".
    pub keys: String,
    /// What it runs there, in words.
    pub command: String,
    /// The command it would run here, by id, when there is one.
    #[serde(default)]
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeader {
    name: String,
    description: String,
    source: String,
    letters: String,
}

#[derive(Deserialize)]
struct RawPreset {
    preset: RawHeader,
    #[serde(default)]
    held_back: Vec<HeldBack>,
}

/// One preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// Its id, the file's name and the `preset` setting's value:
    /// "total-commander".
    pub id: String,
    /// Its name, said in the Keys list's title: "Total Commander".
    pub name: String,
    /// One line, the meaning first: "For people who know Total
    /// Commander's keys."
    pub description: String,
    /// Where its keys come from.
    pub source: String,
    /// What a key without Control or Alt does with it applied.
    pub letters: SingleLetters,
    /// Its key lines, as `keymap.toml` reads them.
    pub lines: KeymapConfig,
    /// The lines that hold on macOS only, from its `[macos.*]` tables,
    /// applied after [`Preset::lines`]: the Finder's Command keys. In them
    /// `meta` is Command.
    pub macos: KeymapConfig,
    /// The sequences held back.
    pub held_back: Vec<HeldBack>,
    /// What could not be read, in words; empty in a correct build.
    pub problems: Vec<String>,
}

/// The tables of a preset file that are not key lines for every system.
const OWN_TABLES: [&str; 3] = ["preset", "held_back", "macos"];

/// Reads key lines from text, keeping the problems that are not about a
/// preset's own tables, each with where it came from, and every line of
/// a command the catalog does not have as a problem too.
fn key_lines(text: &str, place: &str, problems: &mut Vec<String>) -> KeymapConfig {
    match KeymapConfig::from_toml(text) {
        Ok(mut lines) => {
            lines.problems.retain(|p| {
                !OWN_TABLES
                    .iter()
                    .any(|own| p.text.starts_with(&format!("Unknown column: [{own}]")))
            });
            problems.extend(
                lines
                    .problems
                    .iter()
                    .map(|p| format!("{place}: {}", p.text)),
            );
            for line in &lines.overrides {
                if commands::info(&line.command).is_none() {
                    problems.push(format!(
                        "{place}: unknown command {}, line {}",
                        line.command, line.line
                    ));
                }
            }
            lines
        }
        Err(e) => {
            problems.push(format!("{place}: {e}"));
            KeymapConfig::default()
        }
    }
}

/// The `[macos]` table's own tables as a document of their own, so the
/// keymap reader reads them as it reads any key lines.
fn macos_text(text: &str) -> String {
    let Ok(document) = text.parse::<toml_edit::DocumentMut>() else {
        return String::new();
    };
    let Some(macos) = document.get("macos").and_then(toml_edit::Item::as_table) else {
        return String::new();
    };
    let mut own = toml_edit::DocumentMut::new();
    for (name, item) in macos.iter() {
        own.insert(name, item.clone());
    }
    own.to_string()
}

impl Preset {
    /// Reads a preset file. Every mistake is collected in words; the
    /// lines that can be used are used.
    pub fn from_toml(id: &str, text: &str) -> Self {
        let mut problems = Vec::new();
        let file = format!("{id}.toml");
        let (name, description, source, letters, held_back) =
            match toml::from_str::<RawPreset>(text) {
                Ok(raw) => {
                    let letters = match raw.preset.letters.as_str() {
                        "type-ahead" => SingleLetters::TypeAhead,
                        "commands" => SingleLetters::Commands,
                        other => {
                            problems.push(format!(
                                "{file}: letters is {other}; write type-ahead or commands"
                            ));
                            SingleLetters::TypeAhead
                        }
                    };
                    (
                        raw.preset.name,
                        raw.preset.description,
                        raw.preset.source,
                        letters,
                        raw.held_back,
                    )
                }
                Err(e) => {
                    problems.push(format!("{file}: {e}"));
                    (
                        id.to_owned(),
                        String::new(),
                        String::new(),
                        SingleLetters::TypeAhead,
                        Vec::new(),
                    )
                }
            };
        let lines = key_lines(text, &file, &mut problems);
        let macos = key_lines(
            &macos_text(text),
            &format!("{file}, [macos]"),
            &mut problems,
        );
        Self {
            id: id.to_owned(),
            name,
            description,
            source,
            letters,
            lines,
            macos,
            held_back,
            problems,
        }
    }

    /// Whether this is the diroxide defaults.
    pub fn is_default(&self) -> bool {
        self.id == DEFAULT_ID
    }

    /// Its keys as a sentence names them: "Total Commander keys", "The
    /// diroxide default keys".
    pub fn keys_name(&self) -> String {
        if self.is_default() {
            "The diroxide default keys".to_owned()
        } else {
            format!("{} keys", self.name)
        }
    }

    /// The question before applying it, without its "? y or n": "Apply
    /// Total Commander keys", "Apply the diroxide default keys".
    pub fn question(&self) -> String {
        if self.is_default() {
            "Apply the diroxide default keys".to_owned()
        } else {
            format!("Apply {}", self.keys_name())
        }
    }

    /// One column's keys with this preset applied over the defaults for a
    /// system, before the owner's lines.
    pub fn keymap(&self, column: KeyColumn, platform: Platform) -> Keymap {
        let keymap = Keymap::defaults_for(column)
            .for_platform(platform)
            .with_overrides(&self.lines);
        if platform == Platform::MacOs {
            keymap.with_overrides(&self.macos)
        } else {
            keymap
        }
    }
}

/// The built-in presets, read once, in the chooser's order.
pub fn presets() -> &'static [Preset] {
    static PRESETS: OnceLock<Vec<Preset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        FILES
            .iter()
            .map(|(id, text)| Preset::from_toml(id, text))
            .collect()
    })
}

/// A preset by its id or its name, without regard to case.
pub fn preset(wanted: &str) -> Option<&'static Preset> {
    let wanted = wanted.trim().to_lowercase();
    presets()
        .iter()
        .find(|p| p.id == wanted || p.name.to_lowercase() == wanted)
}

/// The preset a setting names, or the diroxide defaults when it names
/// none; see [`unknown_preset`] for the sentence that says so.
pub fn chosen(setting: &str) -> &'static Preset {
    preset(setting)
        .or_else(|| preset(DEFAULT_ID))
        .unwrap_or_else(|| &presets()[0])
}

/// The startup sentence for a `preset` setting no preset has, or `None`.
pub fn unknown_preset(setting: &str) -> Option<String> {
    if setting.trim().is_empty() || preset(setting).is_some() {
        return None;
    }
    let ids: Vec<&str> = presets().iter().map(|p| p.id.as_str()).collect();
    Some(format!(
        "Unknown key preset: {}. The diroxide keys are in use; the presets are {}.",
        setting.trim(),
        crate::settings::list_words(&ids, "and")
    ))
}

/// The single-letter setting after applying a preset, when it changes: a
/// preset whose letters run commands turns type-ahead off; one whose
/// letters jump turns it back on after a preset that ran them. Off, the
/// owner's choice for WCAG 2.1.4, is never changed.
pub fn letters_after(preset: &Preset, now: SingleLetters) -> Option<SingleLetters> {
    match (preset.letters, now) {
        (SingleLetters::Commands, SingleLetters::TypeAhead) => Some(SingleLetters::Commands),
        (SingleLetters::TypeAhead, SingleLetters::Commands) => Some(SingleLetters::TypeAhead),
        _ => None,
    }
}

/// The single-letter setting's value in `config.toml`.
pub const fn letters_value(letters: SingleLetters) -> &'static str {
    match letters {
        SingleLetters::TypeAhead => "type_ahead",
        SingleLetters::Commands => "commands",
        SingleLetters::Off => "off",
    }
}

/// Writes a preset's choice to `config.toml`: `preset`, and
/// `single_letters` when [`letters_after`] changes it, which it returns.
/// `diro keys preset NAME` calls it; the app asks its runtime for the same
/// two settings. The error is one sentence.
pub fn save_choice(
    paths: &diroxide_config::Paths,
    preset: &Preset,
    now: SingleLetters,
) -> Result<Option<SingleLetters>, String> {
    let save = |key: &str, value: &str| {
        diroxide_config::set_setting(paths, key, toml::Value::String(value.to_owned()))
            .map_err(|e| format!("Could not apply {}: {e}.", preset.name))
    };
    save("preset", &preset.id)?;
    let letters = letters_after(preset, now);
    if let Some(letters) = letters {
        save("single_letters", letters_value(letters))?;
    }
    Ok(letters)
}

/// Both columns' keys for a system: the defaults, the preset, then the
/// owner's `keymap.toml`.
pub fn keymaps(owner: &KeymapConfig, preset: &Preset, platform: Platform) -> (Keymap, Keymap) {
    (
        preset
            .keymap(KeyColumn::Terminal, platform)
            .with_overrides(owner),
        preset
            .keymap(KeyColumn::Window, platform)
            .with_overrides(owner),
    )
}

/// Every check a preset must pass, in words, the preset's name first:
/// "Total Commander: Rename has no terminal key and no menu item." Empty
/// when it passes. The checks hold on every system.
pub fn check(preset: &Preset) -> Vec<String> {
    let mut found: Vec<String> = preset.problems.clone();
    let menus = Menus::builtin();
    let name = &preset.name;
    for platform in [Platform::Windows, Platform::MacOs, Platform::Other] {
        let terminal = preset.keymap(KeyColumn::Terminal, platform);
        let window = preset.keymap(KeyColumn::Window, platform);
        let mut say = |line: String| {
            if !found.contains(&line) {
                found.push(line);
            }
        };
        // Reachability: a key in a terminal, or a menu item.
        for info in CATALOG.iter().filter(|i| i.layer != Layer::Dialog) {
            if !info.palette_only
                && terminal.keys_for(&info.id).is_empty()
                && !menus.reaches(&info.id)
            {
                say(format!(
                    "{name}: {} has no terminal key and no menu item.",
                    info.name
                ));
            }
        }
        // The terminal column never uses a key a terminal cannot send; a
        // key a terminal may keep needs a menu as the sure way.
        for binding in terminal.bindings() {
            let Some(row) = tables::terminal_entry(&binding.key) else {
                continue;
            };
            let k = written(&binding.key, platform);
            let command = commands::name_of(&binding.command);
            let opens_a_menu = commands::Menu::ALL
                .iter()
                .any(|m| m.command() == binding.command);
            match row.kind {
                TerminalKind::Unsafe => {
                    say(format!(
                        "{name}: the terminal cannot send {k}, the key of {command}."
                    ));
                }
                _ if !opens_a_menu && !menus.reaches(&binding.command) => say(format!(
                    "{name}: a terminal may keep {k}, and {command} has no menu item."
                )),
                _ => {}
            }
        }
        for keymap in [&terminal, &window] {
            let column = keymap.column();
            for conflict in keymap.conflicts() {
                say(format!("{name}, {column} column: {conflict}"));
            }
            for conflict in keymap.screen_reader_conflicts() {
                say(format!("{name}, {column} column: {conflict}"));
            }
            for only in keymap.only_num_lock_keys() {
                say(format!("{name}, {column} column: {only}"));
            }
        }
        // The critical set keeps a terminal key.
        for id in &CRITICAL {
            if terminal.keys_for(id).is_empty() {
                say(format!(
                    "{name}: {} needs a terminal key.",
                    commands::name_of(id)
                ));
            }
        }
        // A command's only terminal key may not run another command in the
        // window, unless it has a window key of its own.
        for info in CATALOG.iter() {
            let keys = terminal.keys_in(info.layer, &info.id);
            if let [only] = keys.as_slice()
                && window.keys_in(info.layer, &info.id).is_empty()
                && let Some(other) = window.binding(info.layer, only)
                && other.command != info.id
            {
                say(format!(
                    "{name}: {}, the only terminal key of {}, runs {} in the window.",
                    written(only, platform),
                    info.name,
                    commands::name_of(&other.command)
                ));
            }
        }
    }
    // Letters: a type-ahead preset binds no letter or digit where typing
    // jumps to names.
    if preset.letters == SingleLetters::TypeAhead {
        for line in &preset.lines.overrides {
            if matches!(line.layer, Layer::Viewer | Layer::Preview | Layer::Dialog) {
                continue;
            }
            for key in &line.keys {
                let alnum =
                    matches!(key.code, diroxide_core::KeyCode::Char(c) if c.is_alphanumeric());
                if key.is_single_character() && alnum {
                    found.push(format!(
                        "{name}: {} is a letter, and letters jump to names in this preset.",
                        written(key, Platform::Other)
                    ));
                }
            }
        }
    }
    // The key test and the Keys list stay in the palette.
    for id in [commands::ids::KEY_TEST, commands::ids::KEYS_LIST] {
        if !crate::palette::reachable().any(|i| i.id == id) {
            found.push(format!(
                "{name}: the palette does not reach {}.",
                commands::name_of(&id)
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ids;

    fn read(text: &str) -> Preset {
        Preset::from_toml("test", text)
    }

    const HEADER: &str = "[preset]\nname = \"Test\"\ndescription = \"For tests.\"\nsource = \"none\"\nletters = \"type-ahead\"\n";

    #[test]
    fn the_built_in_presets_read_cleanly_and_pass_every_check() {
        assert!(!presets().is_empty());
        for preset in presets() {
            assert_eq!(preset.problems, Vec::<String>::new(), "{}", preset.id);
            assert_eq!(check(preset), Vec::<String>::new(), "{}", preset.id);
            assert!(preset.description.ends_with('.'), "{}", preset.id);
            assert!(!preset.source.is_empty(), "{}", preset.id);
            assert_eq!(
                preset.id,
                preset.id.to_lowercase(),
                "a plain lowercase name"
            );
        }
        let defaults = preset("diroxide").unwrap();
        assert!(defaults.is_default());
        assert_eq!(defaults.name, "diroxide defaults");
        assert!(defaults.lines.overrides.is_empty());
        assert_eq!(preset("DIROXIDE DEFAULTS").unwrap().id, "diroxide");
        assert_eq!(chosen("nothing").id, "diroxide");
    }

    #[test]
    fn each_preset_changes_what_its_source_does() {
        let keys = |id: &str, column: KeyColumn, platform: Platform, command| {
            preset(id)
                .unwrap()
                .keymap(column, platform)
                .keys_text(&command)
        };
        let (t, w, win, mac) = (
            KeyColumn::Terminal,
            KeyColumn::Window,
            Platform::Windows,
            Platform::MacOs,
        );
        let ids = [
            "diroxide",
            "total-commander",
            "midnight-commander",
            "explorer-finder",
            "worker",
            "vim-style",
        ];
        let listed: Vec<&str> = presets().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(listed, ids);
        // Total Commander: Shift+F6 renames, F2 reloads; the window quits
        // on Alt+F4 and Ctrl+Q is its quick view.
        assert_eq!(keys("total-commander", t, win, ids::RENAME), "Shift+F6");
        assert_eq!(keys("total-commander", t, win, ids::RELOAD), "F2 or Ctrl+R");
        assert_eq!(keys("total-commander", t, win, ids::QUIT), "Ctrl+Q");
        assert_eq!(keys("total-commander", w, win, ids::QUIT), "Alt+F4");
        assert_eq!(
            keys("total-commander", w, win, ids::QUICK_VIEW),
            "Ctrl+Q or Shift+F3"
        );
        assert_eq!(keys("total-commander", t, win, ids::PACK), "Alt+F5");
        // Midnight Commander: F10 quits in the window, its Alt keys.
        assert_eq!(
            keys("midnight-commander", w, win, ids::QUIT),
            "F10 or Ctrl+Q"
        );
        assert_eq!(keys("midnight-commander", w, win, ids::MENU), "F9");
        assert_eq!(
            keys("midnight-commander", t, win, ids::BACK),
            "Alt+Left or Alt+Y"
        );
        // Explorer and the Finder: Backspace goes back; the Finder's
        // Command keys on macOS only.
        assert_eq!(
            keys("explorer-finder", t, win, ids::BACK),
            "Alt+Left or Backspace"
        );
        assert_eq!(
            keys("explorer-finder", w, mac, ids::BACK),
            "Option+Left or Cmd+["
        );
        assert_eq!(keys("explorer-finder", w, win, ids::DETAILS), "Alt+Enter");
        assert_eq!(
            keys("explorer-finder", w, mac, ids::DETAILS),
            "Option+Enter or Cmd+I"
        );
        assert_eq!(keys("explorer-finder", t, win, ids::MENU_PLUGINS), "Alt+U");
        // Worker: plus selects all in a terminal; F1 is the palette in
        // the window, help moves to Shift+F1.
        assert_eq!(keys("worker", t, win, ids::SELECT_ALL), "Ctrl+A or Plus");
        assert_eq!(keys("worker", t, win, ids::SELECT_PATTERN), "");
        assert_eq!(keys("worker", w, win, ids::HELP), "Shift+F1");
        assert_eq!(
            keys("worker", w, win, ids::PALETTE),
            "Ctrl+Shift+P or Ctrl+P or F1"
        );
        // vim-style: h, j, k and l, letters running commands.
        assert_eq!(keys("vim-style", t, win, ids::LIST_DOWN), "Down or J");
        assert_eq!(
            keys("vim-style", t, win, ids::UP),
            "Left or Backspace or Alt+Up or H"
        );
        assert_eq!(
            preset("vim-style").unwrap().letters,
            SingleLetters::Commands
        );
        assert_eq!(keys("vim-style", t, win, ids::LIST_PAGE_UP), "Page Up");
        assert_eq!(
            keys("vim-style", w, win, ids::LIST_PAGE_UP),
            "Page Up or Ctrl+B"
        );
        for id in &ids[1..] {
            let p = preset(id).unwrap();
            assert!(!p.lines.overrides.is_empty(), "{id}");
            assert!(p.description.starts_with("For people who "), "{id}");
        }
        assert!(preset("midnight-commander").unwrap().held_back.len() >= 10);
        assert!(preset("worker").unwrap().held_back.len() >= 8);
    }

    #[test]
    fn a_preset_is_a_layer_under_the_owners_lines() {
        let tc = read(&format!(
            "{HEADER}[terminal.panel]\n\"file.rename\" = \"shift+f6\"\n\"panel.reload\" = [\"f2\", \"ctrl+r\"]\n"
        ));
        assert_eq!(tc.problems, Vec::<String>::new());
        let owner =
            KeymapConfig::from_toml("[terminal.panel]\n\"file.rename\" = \"ctrl+f9\"\n").unwrap();
        let (terminal, window) = keymaps(&owner, &tc, Platform::Windows);
        assert_eq!(
            terminal.key_text(&ids::RENAME),
            "Ctrl+F9",
            "the owner's line wins"
        );
        assert_eq!(terminal.keys_text(&ids::RELOAD), "F2 or Ctrl+R");
        assert_eq!(
            window.key_text(&ids::RENAME),
            "F2",
            "the window column is untouched"
        );
        let (terminal, _) = keymaps(&KeymapConfig::default(), &tc, Platform::Windows);
        assert_eq!(terminal.key_text(&ids::RENAME), "Shift+F6");
        assert_eq!(check(&tc), Vec::<String>::new());
    }

    #[test]
    fn the_checks_say_each_problem_in_words() {
        let bad = read(&format!(
            "{HEADER}[terminal.global]\n\"app.say-status\" = \"none\"\n\"app.quit\" = \"ctrl+enter\"\n[terminal.panel]\n\"view.sort-size\" = \"w\"\n\"view.sort-name\" = \"ctrl+alt+n\"\n\"file.copi\" = \"f5\"\n\"file.view\" = \"ctrl+x ctrl+v\"\n[window.panel]\n\"select.all\" = \"kp-plus\"\n[[held_back]]\nkeys = \"ctrl+x c\"\ncommand = \"Change permissions\"\nid = \"file.permissions\"\n"
        ));
        assert_eq!(bad.held_back.len(), 1);
        let said = check(&bad);
        for expected in [
            "test.toml: unknown command file.copi, line 12",
            "test.toml: Two keys in a row: ctrl+x ctrl+v for file.view; use one key, line 13.",
            "Test: Say status needs a terminal key.",
            "Test: the terminal cannot send Ctrl+Enter, the key of Quit.",
            "Test, terminal column: Screen readers keep Ctrl+Alt+N: NVDA. (Sort by name)",
            "Test, window column: Select all has only keypad keys.",
            "Test: W is a letter, and letters jump to names in this preset.",
        ] {
            assert!(said.iter().any(|s| s == expected), "{expected}\n{said:#?}");
        }
        for line in &said {
            let meaning = line.split(':').next().unwrap_or(line);
            assert!(diroxide_core::format::within_cells(line, meaning), "{line}");
        }
        let commands_mode = read(
            "[preset]\nname = \"Vim\"\ndescription = \"For tests.\"\nsource = \"none\"\nletters = \"commands\"\n[terminal.list]\n\"list.down\" = [\"down\", \"j\"]\n",
        );
        assert_eq!(commands_mode.letters, SingleLetters::Commands);
        assert_eq!(check(&commands_mode), Vec::<String>::new());
    }

    #[test]
    fn an_unknown_preset_setting_is_said_once_and_the_defaults_apply() {
        assert_eq!(unknown_preset("diroxide"), None);
        assert_eq!(unknown_preset(""), None);
        let said = unknown_preset("norton").unwrap();
        assert!(said.starts_with("Unknown key preset: norton. "), "{said}");
        assert_eq!(chosen("norton").id, DEFAULT_ID);
    }
}
