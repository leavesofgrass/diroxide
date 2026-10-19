//! The Keys list: every command with its keys in both columns and its
//! state in words, in the list model like every other list
//! (research/alpha3-plan.md, "The Keys list").
//!
//! - The title says the preset and the count of changes: "Keys, preset
//!   diroxide defaults, 0 changed".
//! - One row per command, name first, then the keys, then a state word:
//!   "Copy to other panel, F5 in both"; "Rename, Shift+F6 in both,
//!   changed" for the owner's own line, "preset" for a preset's;
//!   "Paste, Ctrl+V in both, check terminal" when a terminal may keep a
//!   terminal key; "Compare files, no key".
//! - Typing narrows it: words match names; a key's name ("ctrl+f5")
//!   matches by key; "unbound", "changed", "preset" and "conflicts" show
//!   those rows.
//! - Enter on a row opens its actions: Change key, Change the GUI key
//!   (typed, since a terminal cannot send the keys only the GUI has),
//!   Add a second key, Remove a key, Reset this key, Test keys.
//!
//! A dialog's own commands (Yes, No, Accept) are not listed: their keys
//! are the ones the questions name.

use diroxide_config::KeymapConfig;
use diroxide_core::{Key, KeyColumn, Layer, ListModel, ListRow};

use super::conflicts::{self, terminal_concern};
use super::{Keymap, written};
use crate::commands::{CATALOG, CommandInfo};

/// What the Keys list shows: both columns' keymaps as they apply (the
/// defaults, the preset, then the owner's lines, every key whatever the
/// single-letter setting), the owner's lines and the preset's.
#[derive(Clone, Copy, Debug)]
pub struct KeysView<'a> {
    /// The terminal column.
    pub terminal: &'a Keymap,
    /// The GUI column.
    pub gui: &'a Keymap,
    /// The column this frontend can capture: the terminal's in a
    /// terminal.
    pub here: KeyColumn,
    /// The owner's `keymap.toml` lines.
    pub owner: &'a KeymapConfig,
    /// The preset's lines; empty for the diroxide defaults.
    pub preset: &'a KeymapConfig,
    /// The preset's name, said in the title: "diroxide defaults".
    pub preset_name: &'a str,
}

/// The actions of a row, by id.
pub mod actions {
    /// Change key: capture the new key in this frontend's column.
    pub const CHANGE: &str = "change";
    /// Change the other column's key, by typing its name.
    pub const OTHER: &str = "other";
    /// Add a second key in this column.
    pub const ADD: &str = "add";
    /// Remove a key.
    pub const REMOVE: &str = "remove";
    /// Take the owner's lines for the command away.
    pub const RESET: &str = "reset";
    /// The key test.
    pub const TEST: &str = "test";
}

impl KeysView<'_> {
    /// One column's keymap.
    pub fn column(&self, column: KeyColumn) -> &Keymap {
        match column {
            KeyColumn::Terminal => self.terminal,
            KeyColumn::Gui => self.gui,
        }
    }

    /// A command's keys in one column, in its own layer.
    pub fn keys(&self, column: KeyColumn, info: &CommandInfo) -> Vec<Key> {
        self.column(column).keys_in(info.layer, &info.id)
    }

    fn keys_written(&self, column: KeyColumn, info: &CommandInfo) -> String {
        let keymap = self.column(column);
        self.keys(column, info)
            .iter()
            .map(|k| written(k, keymap.platform()))
            .collect::<Vec<_>>()
            .join(" or ")
    }

    /// A command's keys in words, the terminal's first: "F5 in both",
    /// "Shift+F1 or ? in a terminal, F1 in the GUI", "no key".
    pub fn keys_words(&self, info: &CommandInfo) -> String {
        let terminal = self.keys_written(KeyColumn::Terminal, info);
        let gui = self.keys_written(KeyColumn::Gui, info);
        if terminal == gui {
            return if terminal.is_empty() {
                "no key".to_owned()
            } else {
                format!("{terminal} in both")
            };
        }
        let side = |keys: &str, place: &str| {
            if keys.is_empty() {
                format!("no key {place}")
            } else {
                format!("{keys} {place}")
            }
        };
        format!(
            "{}, {}",
            side(&terminal, "in a terminal"),
            side(&gui, "in the GUI")
        )
    }

    /// Whether the owner's `keymap.toml` has a line for the command.
    pub fn changed(&self, info: &CommandInfo) -> bool {
        self.owner.overrides.iter().any(|o| o.command == info.id)
    }

    /// Whether the preset has a line for the command.
    pub fn from_preset(&self, info: &CommandInfo) -> bool {
        self.preset.overrides.iter().any(|o| o.command == info.id)
    }

    /// Whether a terminal may keep one of the command's terminal keys, or
    /// send it as another, on this system.
    pub fn check_terminal(&self, info: &CommandInfo) -> bool {
        let platform = self.terminal.platform();
        self.keys(KeyColumn::Terminal, info)
            .iter()
            .any(|k| terminal_concern(k, platform).is_some())
    }

    /// Whether one of the command's keys in this frontend's column meets
    /// another command's binding where its keys are looked up.
    pub fn shadowed(&self, info: &CommandInfo) -> bool {
        let keymap = self.column(self.here);
        self.keys(self.here, info)
            .iter()
            .any(|k| conflicts::holder(keymap, info.layer, k, &info.id).is_some())
    }

    /// The state words after the keys: "changed" or "preset", then "check
    /// terminal".
    pub fn states(&self, info: &CommandInfo) -> Vec<String> {
        let mut states = Vec::new();
        if self.changed(info) {
            states.push("changed".to_owned());
        } else if self.from_preset(info) {
            states.push("preset".to_owned());
        }
        if self.check_terminal(info) {
            states.push("check terminal".to_owned());
        }
        states
    }

    /// The row of a command: "Copy to other panel, F5 in both".
    pub fn row(&self, info: &CommandInfo) -> ListRow {
        let mut details = vec![self.keys_words(info)];
        details.extend(self.states(info));
        ListRow::new(info.id.as_str(), info.name).with_details(details)
    }

    /// How many commands the owner's lines change.
    pub fn changed_count(&self) -> usize {
        listed().filter(|info| self.changed(info)).count()
    }

    /// The list's title: "Keys, preset diroxide defaults, 0 changed".
    pub fn title(&self) -> String {
        format!(
            "Keys, preset {}, {} changed",
            self.preset_name,
            self.changed_count()
        )
    }

    fn matches(&self, info: &CommandInfo, query: &str) -> bool {
        let wanted = query.trim().to_lowercase();
        if wanted.is_empty() {
            return true;
        }
        match wanted.as_str() {
            "unbound" => return self.keys(self.here, info).is_empty(),
            "changed" => return self.changed(info),
            "preset" => return self.from_preset(info),
            "conflicts" => return self.check_terminal(info) || self.shadowed(info),
            _ => {}
        }
        let name = info.name.to_lowercase();
        let by_name = wanted.split_whitespace().all(|word| name.contains(word));
        let by_key = Key::parse(&wanted).is_ok_and(|key| {
            KeyColumn::ALL
                .iter()
                .any(|column| self.keys(*column, info).contains(&key))
        });
        by_name || by_key
    }

    /// The Keys list for what was typed, in the catalog's order, the
    /// focus on the first row.
    pub fn list(&self, query: &str) -> ListModel {
        self.list_with(query, Vec::new())
    }

    /// The Keys list with the list's own commands first while nothing is
    /// typed (Choose a preset, Reset all keys, Single letters), so they
    /// are reached from inside it.
    pub fn list_with(&self, query: &str, own: Vec<ListRow>) -> ListModel {
        let mut rows: Vec<ListRow> = if query.trim().is_empty() {
            own
        } else {
            Vec::new()
        };
        rows.extend(
            listed()
                .filter(|info| self.matches(info, query))
                .map(|info| self.row(info)),
        );
        let mut list = ListModel::new(self.title()).with_rows(rows);
        list.set_empty_text("No command matches");
        list.set_help(
            "Type to narrow by a name, by a key such as ctrl+f5, or by unbound, changed, preset or conflicts. Down enters the list, Enter opens a command's actions, Escape closes.",
        );
        list
    }

    /// A row's actions, Change key first.
    pub fn actions(&self, info: &CommandInfo) -> ListModel {
        let other = match self.here {
            KeyColumn::Terminal => ("Change the GUI key", "type its name"),
            KeyColumn::Gui => ("Change the terminal key", "type its name"),
        };
        let mut rows = vec![
            ListRow::new(actions::CHANGE, "Change key").with_details(["press the new key"]),
            ListRow::new(actions::OTHER, other.0).with_details([other.1]),
            ListRow::new(actions::ADD, "Add a second key").with_details(["press it"]),
        ];
        if !self.keys(self.here, info).is_empty() {
            rows.push(ListRow::new(actions::REMOVE, "Remove a key"));
        }
        if self.changed(info) {
            rows.push(
                ListRow::new(actions::RESET, "Reset this key").with_details(["back to the preset"]),
            );
        }
        rows.push(
            ListRow::new(actions::TEST, "Test keys").with_details(["say what each key runs"]),
        );
        let mut list =
            ListModel::new(format!("{}, {}", info.name, self.keys_words(info))).with_rows(rows);
        list.set_help("Up and Down choose, Enter does it, Escape goes back to the Keys list.");
        list
    }
}

/// The commands the Keys list shows: every one but a dialog's own.
pub fn listed() -> impl Iterator<Item = &'static CommandInfo> {
    CATALOG.iter().filter(|info| info.layer != Layer::Dialog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{self, ids};
    use crate::keymap::names::Platform;
    use diroxide_core::format::within_cells;

    fn info(id: &diroxide_core::CommandId) -> &'static CommandInfo {
        commands::info(id).unwrap()
    }

    fn view<'a>(
        terminal: &'a Keymap,
        gui: &'a Keymap,
        owner: &'a KeymapConfig,
        preset: &'a KeymapConfig,
    ) -> KeysView<'a> {
        KeysView {
            terminal,
            gui,
            here: KeyColumn::Terminal,
            owner,
            preset,
            preset_name: "diroxide defaults",
        }
    }

    #[test]
    fn each_row_says_the_name_the_keys_and_the_state() {
        let terminal = Keymap::defaults().for_platform(Platform::Windows);
        let gui = Keymap::defaults_for(KeyColumn::Gui).for_platform(Platform::Windows);
        let owner = KeymapConfig::from_toml("[terminal.panel]\n\"file.rename\" = \"shift+f6\"\n[gui.panel]\n\"file.rename\" = \"shift+f6\"\n").unwrap();
        let none = KeymapConfig::default();
        let terminal_changed = terminal.clone().with_overrides(&owner);
        let gui_changed = gui.clone().with_overrides(&owner);
        let v = view(&terminal_changed, &gui_changed, &owner, &none);
        assert_eq!(v.title(), "Keys, preset diroxide defaults, 1 changed");
        let said = |id: diroxide_core::CommandId| v.row(info(&id)).spoken();
        assert_eq!(said(ids::COPY), "Copy to other panel, F5 in both");
        // Some terminals send Shift with a function key as F13 to F24.
        assert_eq!(
            said(ids::RENAME),
            "Rename, Shift+F6 in both, changed, check terminal"
        );
        assert_eq!(
            said(ids::CLIPBOARD_PASTE),
            "Paste, Ctrl+V in both, check terminal"
        );
        assert_eq!(said(ids::SORT_KIND), "Sort by kind, no key");
        assert_eq!(
            said(ids::HELP),
            "Help, Shift+F1 or ? in a terminal, F1 in the GUI, check terminal"
        );
        assert_eq!(
            said(ids::SETTINGS),
            "Settings, no key in a terminal, Ctrl+Comma in the GUI"
        );
        // Ctrl+F5 is a macOS key only on macOS.
        assert_eq!(
            said(ids::SORT_MODIFIED),
            "Sort by modified, Ctrl+F5 in both"
        );
        for row in v.list("").rows() {
            assert!(within_cells(&row.spoken(), &row.label), "{}", row.spoken());
        }
        let v = view(&terminal, &gui, &none, &owner);
        assert_eq!(
            v.row(info(&ids::RENAME)).spoken(),
            "Rename, F2 in both, preset"
        );
    }

    #[test]
    fn typing_narrows_by_name_by_key_and_by_state() {
        let terminal = Keymap::defaults();
        let gui = Keymap::defaults_for(KeyColumn::Gui);
        let owner = KeymapConfig::from_toml("[terminal.panel]\n\"view.sort-size\" = \"ctrl+f9\"\n")
            .unwrap();
        let none = KeymapConfig::default();
        let terminal = terminal.with_overrides(&owner);
        let v = view(&terminal, &gui, &owner, &none);
        let ids_of = |query: &str| -> Vec<String> {
            v.list(query).rows().iter().map(|r| r.id.clone()).collect()
        };
        let copy = ids_of("copy");
        assert_eq!(copy[0], "file.copy-path");
        assert!(copy.contains(&"file.copy".to_owned()));
        assert!(copy.iter().all(|id| {
            info(&diroxide_core::CommandId::new(id.clone()).unwrap())
                .name
                .to_lowercase()
                .contains("copy")
        }));
        assert_eq!(ids_of("ctrl+f9"), ["view.sort-size"]);
        assert_eq!(ids_of("F5"), ["file.copy"]);
        assert_eq!(ids_of("changed"), ["view.sort-size"]);
        assert!(ids_of("unbound").contains(&"view.sort-kind".to_owned()));
        assert!(ids_of("conflicts").contains(&"clipboard.paste".to_owned()));
        assert!(ids_of("zzz").is_empty());
        assert_eq!(v.list("zzz").announce_focus(), "No command matches");
        assert!(!ids_of("").iter().any(|id| id.starts_with("dialog.")));
        assert_eq!(
            v.list("").title(),
            "Keys, preset diroxide defaults, 1 changed"
        );
    }

    #[test]
    fn a_rows_actions_start_with_change_key() {
        let terminal = Keymap::defaults();
        let gui = Keymap::defaults_for(KeyColumn::Gui);
        let owner = KeymapConfig::from_toml("[terminal.panel]\n\"view.sort-size\" = \"ctrl+f9\"\n")
            .unwrap();
        let none = KeymapConfig::default();
        let terminal = terminal.with_overrides(&owner);
        let v = view(&terminal, &gui, &owner, &none);
        let list = v.actions(info(&ids::SORT_SIZE));
        assert_eq!(
            list.title(),
            "Sort by size, Ctrl+F9 in a terminal, Ctrl+F6 in the GUI"
        );
        let rows: Vec<String> = list.rows().iter().map(|r| r.spoken()).collect();
        assert_eq!(
            rows,
            [
                "Change key, press the new key",
                "Change the GUI key, type its name",
                "Add a second key, press it",
                "Remove a key",
                "Reset this key, back to the preset",
                "Test keys, say what each key runs"
            ]
        );
        let kind: Vec<String> = v
            .actions(info(&ids::SORT_KIND))
            .rows()
            .iter()
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(kind, ["change", "other", "add", "test"]);
    }
}
