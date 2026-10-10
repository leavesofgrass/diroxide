//! The command catalog: every built-in command's id, name, help, menu and
//! default keys, in one place, so menus, the palette, banks, F1 help and
//! `docs/keyboard.md` all read the same names and keys.
//!
//! Wave 1 (W1c) owns this module. Insert is never bound: it is the NVDA
//! and JAWS modifier key.

use std::fmt;

use diroxide_core::{CommandId, Layer};

/// The menus, in menu bar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Menu {
    /// File.
    File,
    /// Edit.
    Edit,
    /// View.
    View,
    /// Go.
    Go,
    /// Select.
    Select,
    /// Tools.
    Tools,
    /// Plugins.
    Plugins,
    /// Help.
    Help,
}

impl Menu {
    /// Every menu, in order.
    pub const ALL: [Menu; 8] = [
        Menu::File,
        Menu::Edit,
        Menu::View,
        Menu::Go,
        Menu::Select,
        Menu::Tools,
        Menu::Plugins,
        Menu::Help,
    ];

    /// The menu's title.
    pub const fn title(self) -> &'static str {
        match self {
            Menu::File => "File",
            Menu::Edit => "Edit",
            Menu::View => "View",
            Menu::Go => "Go",
            Menu::Select => "Select",
            Menu::Tools => "Tools",
            Menu::Plugins => "Plugins",
            Menu::Help => "Help",
        }
    }
}

impl fmt::Display for Menu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

/// One built-in command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    /// The id.
    pub id: CommandId,
    /// The name in menus and the palette, meaning first.
    pub name: &'static str,
    /// One sentence for F1 help.
    pub help: &'static str,
    /// The menu it is listed in, if any.
    pub menu: Option<Menu>,
    /// Default keys, as written in `keymap.toml`, with their layers.
    pub keys: &'static [(Layer, &'static str)],
}

/// Well-known command ids.
pub mod ids {
    use diroxide_core::CommandId;

    /// Leave diroxide.
    pub const QUIT: CommandId = CommandId::from_static("app.quit");
    /// Say help for where the focus is.
    pub const HELP: CommandId = CommandId::from_static("app.help");
    /// Say where the focus is.
    pub const SAY_STATUS: CommandId = CommandId::from_static("app.say-status");
    /// Open the command palette.
    pub const PALETTE: CommandId = CommandId::from_static("app.palette");
    /// Open the menus.
    pub const MENU: CommandId = CommandId::from_static("app.menu");
    /// Make the other panel active.
    pub const SWITCH_PANEL: CommandId = CommandId::from_static("panel.switch");
    /// Open the focused entry.
    pub const ENTER: CommandId = CommandId::from_static("panel.enter");
    /// Go up one folder.
    pub const UP: CommandId = CommandId::from_static("panel.up");
    /// List the place again.
    pub const RELOAD: CommandId = CommandId::from_static("panel.reload");
    /// Type a path to go to.
    pub const GO_PATH: CommandId = CommandId::from_static("go.path");
    /// Focus up one row.
    pub const LIST_UP: CommandId = CommandId::from_static("list.up");
    /// Focus down one row.
    pub const LIST_DOWN: CommandId = CommandId::from_static("list.down");
    /// Focus up one page.
    pub const LIST_PAGE_UP: CommandId = CommandId::from_static("list.page-up");
    /// Focus down one page.
    pub const LIST_PAGE_DOWN: CommandId = CommandId::from_static("list.page-down");
    /// Focus the first row.
    pub const LIST_HOME: CommandId = CommandId::from_static("list.home");
    /// Focus the last row.
    pub const LIST_END: CommandId = CommandId::from_static("list.end");
    /// Select or unselect the focused row and move down.
    pub const SELECT_TOGGLE: CommandId = CommandId::from_static("select.toggle");
    /// Select every row.
    pub const SELECT_ALL: CommandId = CommandId::from_static("select.all");
    /// Clear the selection.
    pub const SELECT_NONE: CommandId = CommandId::from_static("select.none");
    /// Invert the selection.
    pub const SELECT_INVERT: CommandId = CommandId::from_static("select.invert");
    /// Select by a pattern.
    pub const SELECT_PATTERN: CommandId = CommandId::from_static("select.pattern");
    /// View the focused file.
    pub const VIEW: CommandId = CommandId::from_static("file.view");
    /// Edit the focused file.
    pub const EDIT: CommandId = CommandId::from_static("file.edit");
    /// Copy to the other panel.
    pub const COPY: CommandId = CommandId::from_static("file.copy");
    /// Move to the other panel.
    pub const MOVE: CommandId = CommandId::from_static("file.move");
    /// Make a folder.
    pub const MAKE_FOLDER: CommandId = CommandId::from_static("folder.make");
    /// Move to the trash.
    pub const TRASH: CommandId = CommandId::from_static("file.trash");
    /// Delete for good, after listing every path.
    pub const DELETE_PERMANENTLY: CommandId = CommandId::from_static("file.delete-permanently");
    /// Rename the focused entry.
    pub const RENAME: CommandId = CommandId::from_static("file.rename");
    /// Undo the last job's renames, moves and trashings.
    pub const UNDO: CommandId = CommandId::from_static("edit.undo");
    /// Show or hide hidden entries.
    pub const TOGGLE_HIDDEN: CommandId = CommandId::from_static("view.hidden");
    /// Find files.
    pub const FIND: CommandId = CommandId::from_static("search.find");
    /// List the plugins.
    pub const PLUGINS: CommandId = CommandId::from_static("plugins.list");
}

use Layer::{Global, List, Panel};

/// Every built-in command.
pub static CATALOG: [CommandInfo; 33] = [
    CommandInfo {
        id: ids::QUIT,
        name: "Quit",
        help: "Leaves diroxide.",
        menu: Some(Menu::File),
        keys: &[(Global, "ctrl+q"), (Global, "f10")],
    },
    CommandInfo {
        id: ids::HELP,
        name: "Help",
        help: "Says help for where the focus is.",
        menu: Some(Menu::Help),
        keys: &[(Global, "f1")],
    },
    CommandInfo {
        id: ids::SAY_STATUS,
        name: "Say status",
        help: "Says the panel, the count, the selection and the focus.",
        menu: Some(Menu::Help),
        keys: &[(Global, "ctrl+shift+s")],
    },
    CommandInfo {
        id: ids::PALETTE,
        name: "Command palette",
        help: "Finds any command by name.",
        menu: Some(Menu::Tools),
        keys: &[(Global, "ctrl+p")],
    },
    CommandInfo {
        id: ids::MENU,
        name: "Menus",
        help: "Opens the menu bar.",
        menu: None,
        keys: &[(Global, "f9")],
    },
    CommandInfo {
        id: ids::SWITCH_PANEL,
        name: "Switch panels",
        help: "Makes the other panel active.",
        menu: Some(Menu::Go),
        keys: &[(Panel, "tab")],
    },
    CommandInfo {
        id: ids::ENTER,
        name: "Open",
        help: "Opens the focused folder, archive or file.",
        menu: Some(Menu::File),
        keys: &[(Panel, "enter")],
    },
    CommandInfo {
        id: ids::UP,
        name: "Up one folder",
        help: "Goes to the folder above.",
        menu: Some(Menu::Go),
        keys: &[(Panel, "backspace")],
    },
    CommandInfo {
        id: ids::RELOAD,
        name: "Reload",
        help: "Lists the folder again.",
        menu: Some(Menu::View),
        keys: &[(Panel, "ctrl+r")],
    },
    CommandInfo {
        id: ids::GO_PATH,
        name: "Go to a path",
        help: "Asks for a path and goes there.",
        menu: Some(Menu::Go),
        keys: &[(Global, "ctrl+l")],
    },
    CommandInfo {
        id: ids::LIST_UP,
        name: "Previous row",
        help: "Moves the focus up one row.",
        menu: None,
        keys: &[(List, "up")],
    },
    CommandInfo {
        id: ids::LIST_DOWN,
        name: "Next row",
        help: "Moves the focus down one row.",
        menu: None,
        keys: &[(List, "down")],
    },
    CommandInfo {
        id: ids::LIST_PAGE_UP,
        name: "Previous page",
        help: "Moves the focus up one page.",
        menu: None,
        keys: &[(List, "pageup")],
    },
    CommandInfo {
        id: ids::LIST_PAGE_DOWN,
        name: "Next page",
        help: "Moves the focus down one page.",
        menu: None,
        keys: &[(List, "pagedown")],
    },
    CommandInfo {
        id: ids::LIST_HOME,
        name: "First row",
        help: "Moves the focus to the first row.",
        menu: None,
        keys: &[(List, "home")],
    },
    CommandInfo {
        id: ids::LIST_END,
        name: "Last row",
        help: "Moves the focus to the last row.",
        menu: None,
        keys: &[(List, "end")],
    },
    CommandInfo {
        id: ids::SELECT_TOGGLE,
        name: "Select or unselect",
        help: "Selects or unselects the focused row and moves down.",
        menu: Some(Menu::Select),
        keys: &[(Panel, "space")],
    },
    CommandInfo {
        id: ids::SELECT_ALL,
        name: "Select all",
        help: "Selects every row.",
        menu: Some(Menu::Select),
        keys: &[(Panel, "kp-plus"), (Panel, "ctrl+a")],
    },
    CommandInfo {
        id: ids::SELECT_NONE,
        name: "Select none",
        help: "Clears the selection.",
        menu: Some(Menu::Select),
        keys: &[(Panel, "kp-minus"), (Panel, "ctrl+shift+a")],
    },
    CommandInfo {
        id: ids::SELECT_INVERT,
        name: "Invert selection",
        help: "Selects what was not selected, and the reverse.",
        menu: Some(Menu::Select),
        keys: &[(Panel, "kp-multiply"), (Panel, "ctrl+i")],
    },
    CommandInfo {
        id: ids::SELECT_PATTERN,
        name: "Select by pattern",
        help: "Selects the rows whose names match a pattern.",
        menu: Some(Menu::Select),
        keys: &[(Panel, "kp-divide"), (Panel, "ctrl+shift+p")],
    },
    CommandInfo {
        id: ids::VIEW,
        name: "View",
        help: "Opens the focused file in the viewer.",
        menu: Some(Menu::File),
        keys: &[(Panel, "f3")],
    },
    CommandInfo {
        id: ids::EDIT,
        name: "Edit",
        help: "Opens the focused file in the editor.",
        menu: Some(Menu::File),
        keys: &[(Panel, "f4")],
    },
    CommandInfo {
        id: ids::COPY,
        name: "Copy to other panel",
        help: "Copies the selection to the other panel's folder.",
        menu: Some(Menu::Edit),
        keys: &[(Panel, "f5")],
    },
    CommandInfo {
        id: ids::MOVE,
        name: "Move to other panel",
        help: "Moves the selection to the other panel's folder, after listing it.",
        menu: Some(Menu::Edit),
        keys: &[(Panel, "f6")],
    },
    CommandInfo {
        id: ids::MAKE_FOLDER,
        name: "Make folder",
        help: "Asks for a name and makes a folder.",
        menu: Some(Menu::File),
        keys: &[(Panel, "f7")],
    },
    CommandInfo {
        id: ids::TRASH,
        name: "Move to trash",
        help: "Moves the selection to the trash, after listing it.",
        menu: Some(Menu::Edit),
        keys: &[(Panel, "f8"), (Panel, "delete")],
    },
    CommandInfo {
        id: ids::DELETE_PERMANENTLY,
        name: "Delete permanently",
        help: "Deletes the selection for good, after listing every path.",
        menu: Some(Menu::Edit),
        keys: &[(Panel, "shift+delete")],
    },
    CommandInfo {
        id: ids::RENAME,
        name: "Rename",
        help: "Asks for a new name for the focused entry.",
        menu: Some(Menu::Edit),
        keys: &[(Panel, "f2")],
    },
    CommandInfo {
        id: ids::UNDO,
        name: "Undo",
        help: "Reverses the last job's renames, moves and trashings.",
        menu: Some(Menu::Edit),
        keys: &[(Global, "ctrl+z")],
    },
    CommandInfo {
        id: ids::TOGGLE_HIDDEN,
        name: "Show or hide hidden files",
        help: "Shows or hides hidden entries.",
        menu: Some(Menu::View),
        keys: &[(Panel, "ctrl+h")],
    },
    CommandInfo {
        id: ids::FIND,
        name: "Find files",
        help: "Searches by name and content.",
        menu: Some(Menu::Tools),
        keys: &[(Panel, "alt+f7")],
    },
    CommandInfo {
        id: ids::PLUGINS,
        name: "Plugins",
        help: "Lists the plugins and what they may do.",
        menu: Some(Menu::Plugins),
        keys: &[],
    },
];

/// A built-in command by id.
pub fn info(id: &CommandId) -> Option<&'static CommandInfo> {
    CATALOG.iter().find(|info| &info.id == id)
}

/// A command's name, or its id when it is not built in.
pub fn name_of(id: &CommandId) -> String {
    info(id).map_or_else(|| id.to_string(), |info| info.name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{KeyChord, KeyCode};
    use std::collections::HashSet;

    #[test]
    fn every_id_is_valid_and_unique() {
        let mut seen = HashSet::new();
        for info in &CATALOG {
            assert!(CommandId::is_valid(info.id.as_str()), "{}", info.id);
            assert!(seen.insert(info.id.clone()), "duplicate {}", info.id);
            assert!(
                !info.name.is_empty() && info.help.ends_with('.'),
                "{}",
                info.id
            );
        }
    }

    #[test]
    fn default_keys_parse_and_never_use_insert() {
        for info in &CATALOG {
            for (_, keys) in info.keys {
                let chord = KeyChord::parse(keys).unwrap_or_else(|e| panic!("{}: {e}", info.id));
                assert!(
                    chord.keys().iter().all(|k| k.code != KeyCode::Insert),
                    "{}",
                    info.id
                );
            }
        }
    }

    #[test]
    fn names_fit_a_braille_line() {
        for info in &CATALOG {
            assert!(info.name.chars().count() <= 30, "{}", info.name);
        }
        assert_eq!(name_of(&ids::COPY), "Copy to other panel");
        assert_eq!(name_of(&CommandId::from_static("x.y")), "x.y");
    }
}
