//! Menus and button banks, from the catalog and the keymap, so no menu or
//! button can show a key that is not bound.
//!
//! Wave 1 (W1c) owns this module.

use diroxide_config::BanksConfig;
use diroxide_core::{CommandId, ListModel, ListRow};

use crate::commands::{CATALOG, Menu, name_of};
use crate::keymap::Keymap;

/// A menu as a list: each row a command's name, then its key.
pub fn menu_list(menu: Menu, keymap: &Keymap) -> ListModel {
    let rows = CATALOG
        .iter()
        .filter(|info| info.menu == Some(menu))
        .map(|info| {
            let key = keymap.key_text(&info.id);
            let row = ListRow::new(info.id.as_str(), info.name);
            if key.is_empty() {
                row
            } else {
                row.with_details([key])
            }
        })
        .collect();
    let mut list = ListModel::new(format!("{} menu", menu.title())).with_rows(rows);
    list.set_help("Up and Down choose, Enter runs, Escape closes, Left and Right change menus.");
    list
}

/// The menu bar: every menu, in order.
pub fn menu_bar(keymap: &Keymap) -> Vec<(Menu, ListModel)> {
    Menu::ALL
        .iter()
        .map(|menu| (*menu, menu_list(*menu, keymap)))
        .collect()
}

/// A button in a bank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    /// The label, said and shown.
    pub label: String,
    /// The commands it runs, in order.
    pub commands: Vec<CommandId>,
}

/// A named bank of buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bank {
    /// The bank's name.
    pub name: String,
    /// Its buttons.
    pub buttons: Vec<Button>,
}

impl Bank {
    /// The bank as a list: each row a button's label, then the key of its
    /// first command when it has one.
    pub fn list(&self, keymap: &Keymap) -> ListModel {
        let rows = self
            .buttons
            .iter()
            .enumerate()
            .map(|(i, button)| {
                let key = button
                    .commands
                    .first()
                    .map(|c| keymap.key_text(c))
                    .unwrap_or_default();
                let row = ListRow::new(i.to_string(), button.label.clone());
                if key.is_empty() {
                    row
                } else {
                    row.with_details([key])
                }
            })
            .collect();
        ListModel::new(format!("{} bank", self.name)).with_rows(rows)
    }
}

/// The banks from `banks.toml`; command ids that are not valid are left
/// out. With no banks configured, one bank of the file commands.
pub fn banks(config: &BanksConfig) -> Vec<Bank> {
    if config.banks.is_empty() {
        let main = [
            crate::commands::ids::VIEW,
            crate::commands::ids::EDIT,
            crate::commands::ids::COPY,
            crate::commands::ids::MOVE,
            crate::commands::ids::MAKE_FOLDER,
            crate::commands::ids::TRASH,
        ];
        return vec![Bank {
            name: "Main".to_owned(),
            buttons: main
                .iter()
                .map(|id| Button {
                    label: name_of(id),
                    commands: vec![id.clone()],
                })
                .collect(),
        }];
    }
    config
        .banks
        .iter()
        .map(|bank| Bank {
            name: bank.name.clone(),
            buttons: bank
                .buttons
                .iter()
                .map(|button| Button {
                    label: button.label.clone(),
                    commands: button
                        .commands
                        .iter()
                        .filter_map(|c| CommandId::new(c.clone()).ok())
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_show_bound_keys() {
        let keymap = Keymap::defaults();
        let edit = menu_list(Menu::Edit, &keymap);
        assert_eq!(edit.title(), "Edit menu");
        assert_eq!(edit.announce_focus(), "Copy to other panel, F5, 1 of 6");
        assert_eq!(menu_bar(&keymap).len(), 8);
    }

    #[test]
    fn the_default_bank_lists_file_commands() {
        let keymap = Keymap::defaults();
        let bank = &banks(&BanksConfig::default())[0];
        assert_eq!(bank.list(&keymap).announce_focus(), "View, F3, 1 of 6");
    }
}
