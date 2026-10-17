//! Menus and button banks, read from data and shown with keys from the
//! live keymap, so no menu or button can show a key that is not bound.
//!
//! The menus are `menus.toml` (plan section 5.7): the built-in file is
//! [`BUILTIN_MENUS_TOML`], used when the owner has none and written beside
//! the owner's file as `menus-defaults.toml`. Every item names a command
//! by the same id the keymap and the banks use. Access keys are checked
//! for conflicts, unknown items are said in words and left out of the
//! menu (never out of the file), labels fit 40 cells, and an item with a
//! condition shows only where the place allows it, so a place without a
//! trash never offers Move to trash.
//!
//! The banks are `banks.toml`: named rows of buttons, each running
//! commands in the command text form the banks share with `diro run`. The
//! default bank is the function keys, and the key bar is its one line,
//! labels first.

use diroxide_config::{BanksConfig, ConfigProblem, MenusConfig, Paths};
use diroxide_core::format::BRAILLE_CELLS;
use diroxide_core::{Command, CommandId, ListModel, ListRow};
use diroxide_vfs::Capabilities;

use crate::commands::{self, ids, parse_command_text};
use crate::keymap::Keymap;

/// The built-in `menus.toml`.
pub const BUILTIN_MENUS_TOML: &str = include_str!("../keys/menus.toml");

/// The built-in menus as text, written beside the owner's file as
/// `menus-defaults.toml`, a read-only reference.
pub fn defaults_reference() -> String {
    BUILTIN_MENUS_TOML.to_owned()
}

/// When an item is shown, from the place's capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum When {
    /// The place can be changed.
    Writable,
    /// The place has a trash.
    Trash,
    /// The place can be changed but has no trash.
    NoTrash,
    /// Trashed items can be restored.
    Restore,
    /// Links can be made.
    Links,
    /// Permissions or attributes can be changed.
    Attributes,
}

impl When {
    /// Every condition, by its name in `menus.toml`.
    pub const ALL: [(When, &'static str); 6] = [
        (When::Writable, "writable"),
        (When::Trash, "trash"),
        (When::NoTrash, "no_trash"),
        (When::Restore, "restore"),
        (When::Links, "links"),
        (When::Attributes, "attributes"),
    ];

    /// A condition by name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, n)| *n == name.trim())
            .map(|(w, _)| *w)
    }

    /// Whether the place allows it.
    pub fn holds(self, place: &Capabilities) -> bool {
        match self {
            When::Writable => place.writable,
            When::Trash => place.trash,
            When::NoTrash => place.writable && !place.trash,
            When::Restore => place.restore,
            When::Links => place.links,
            When::Attributes => place.attributes,
        }
    }
}

/// What an item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemAction {
    /// Runs a command.
    Command(CommandId),
    /// Opens a submenu, by its id.
    Submenu(String),
}

/// One item of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// What it does.
    pub action: ItemAction,
    /// The label, said and shown.
    pub label: String,
    /// The access key, lowercase.
    pub access: Option<char>,
    /// When it is shown; always when `None`.
    pub when: Option<When>,
}

impl MenuItem {
    /// The row's id: the command's id, or `menu:` and the submenu's id.
    pub fn row_id(&self) -> String {
        match &self.action {
            ItemAction::Command(id) => id.to_string(),
            ItemAction::Submenu(id) => format!("menu:{id}"),
        }
    }
}

/// One menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuDef {
    /// Its id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its access key on the menu bar, lowercase.
    pub access: Option<char>,
    /// Whether it is on the menu bar.
    pub bar: bool,
    /// Its items, in order.
    pub items: Vec<MenuItem>,
}

/// The menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menus {
    menus: Vec<MenuDef>,
}

fn access_key(text: Option<&str>, problems: &mut Vec<String>, place: &str) -> Option<char> {
    let text = text?;
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_alphanumeric() => c.to_lowercase().next(),
        _ => {
            problems.push(format!(
                "Access key must be one letter: \"{text}\" in {place}."
            ));
            None
        }
    }
}

impl Menus {
    /// The built-in menus.
    pub fn builtin() -> Self {
        let config: MenusConfig = toml::from_str(BUILTIN_MENUS_TOML).unwrap_or_default();
        Self::from_config(&config).0
    }

    /// The owner's menus when `menus.toml` was read, else the built-in
    /// ones, with the problems in words.
    pub fn load(config: Option<&MenusConfig>) -> (Self, Vec<ConfigProblem>) {
        match config {
            Some(config) => Self::from_config(config),
            None => (Self::builtin(), Vec::new()),
        }
    }

    /// Menus from `menus.toml`'s content, checked: unknown commands and
    /// submenus, access keys that are not one letter or are used twice,
    /// unknown conditions, labels longer than 40 cells. Every problem is a
    /// sentence; an item that cannot work is left out of the menu.
    pub fn from_config(config: &MenusConfig) -> (Self, Vec<ConfigProblem>) {
        let mut problems: Vec<String> = Vec::new();
        let mut menus: Vec<MenuDef> = Vec::new();
        let known_ids: Vec<&str> = config.menus.iter().map(|m| m.id.as_str()).collect();
        for (index, menu) in config.menus.iter().enumerate() {
            let title = if menu.title.trim().is_empty() {
                problems.push(format!("Menu {} has no title.", index + 1));
                menu.id.clone()
            } else {
                menu.title.clone()
            };
            if menus.iter().any(|m| m.id == menu.id) {
                problems.push(format!(
                    "Menu id {} is used twice; the first one is kept.",
                    menu.id
                ));
                continue;
            }
            let place = format!("menu {title}");
            let access = access_key(menu.access.as_deref(), &mut problems, &place);
            let mut items = Vec::new();
            for (n, item) in menu.items.iter().enumerate() {
                let action = match (&item.command, &item.menu) {
                    (Some(command), None) => match CommandId::new(command.clone()) {
                        Ok(id) if commands::info(&id).is_some() => ItemAction::Command(id),
                        _ => {
                            problems.push(format!("Unknown command in {place}: {command}."));
                            continue;
                        }
                    },
                    (None, Some(sub)) => {
                        if known_ids.contains(&sub.as_str()) && sub != &menu.id {
                            ItemAction::Submenu(sub.clone())
                        } else {
                            problems.push(format!("Unknown submenu in {place}: {sub}."));
                            continue;
                        }
                    }
                    _ => {
                        problems.push(format!(
                            "Item {} of {place} needs a command or a menu, not both.",
                            n + 1
                        ));
                        continue;
                    }
                };
                let when = match item.when.as_deref() {
                    None => None,
                    Some(name) => match When::from_name(name) {
                        Some(when) => Some(when),
                        None => {
                            problems.push(format!(
                                "Unknown condition in {place}: {name}. Use writable, trash, no_trash, restore, links or attributes."
                            ));
                            continue;
                        }
                    },
                };
                let label = match (&item.label, &action) {
                    (Some(label), _) if !label.trim().is_empty() => label.trim().to_owned(),
                    (_, ItemAction::Command(id)) => commands::name_of(id),
                    (_, ItemAction::Submenu(sub)) => config
                        .menus
                        .iter()
                        .find(|m| &m.id == sub)
                        .map_or_else(|| sub.clone(), |m| m.title.clone()),
                };
                if label.chars().count() > BRAILLE_CELLS {
                    problems.push(format!(
                        "Label too long in {place}: {} cells, at most 40.",
                        label.chars().count()
                    ));
                }
                let access = access_key(item.access.as_deref(), &mut problems, &place);
                items.push(MenuItem {
                    action,
                    label,
                    access,
                    when,
                });
            }
            for (i, a) in items.iter().enumerate() {
                for b in &items[i + 1..] {
                    if let (Some(x), Some(y)) = (a.access, b.access)
                        && x == y
                    {
                        problems.push(format!(
                            "Access key {} twice in {place}: {} and {}.",
                            x.to_uppercase(),
                            a.label,
                            b.label
                        ));
                    }
                }
            }
            menus.push(MenuDef {
                id: menu.id.clone(),
                title,
                access,
                bar: menu.bar,
                items,
            });
        }
        let bar: Vec<&MenuDef> = menus.iter().filter(|m| m.bar).collect();
        for (i, a) in bar.iter().enumerate() {
            for b in &bar[i + 1..] {
                if let (Some(x), Some(y)) = (a.access, b.access)
                    && x == y
                {
                    problems.push(format!(
                        "Access key {} twice on the menu bar: {} and {}.",
                        x.to_uppercase(),
                        a.title,
                        b.title
                    ));
                }
            }
        }
        let problems = problems
            .into_iter()
            .map(|text| ConfigProblem::new(Paths::MENUS, text))
            .collect();
        (Self { menus }, problems)
    }

    /// Every menu, bar menus and submenus, in file order.
    pub fn menus(&self) -> &[MenuDef] {
        &self.menus
    }

    /// The menus on the menu bar, in order.
    pub fn bar(&self) -> impl Iterator<Item = &MenuDef> {
        self.menus.iter().filter(|m| m.bar)
    }

    /// A menu by id.
    pub fn menu(&self, id: &str) -> Option<&MenuDef> {
        self.menus.iter().find(|m| m.id == id)
    }

    /// Whether some menu runs a command.
    pub fn reaches(&self, command: &CommandId) -> bool {
        self.menus.iter().any(|m| {
            m.items
                .iter()
                .any(|i| i.action == ItemAction::Command(command.clone()))
        })
    }

    /// The title of the menu whose item runs a command, the first in file
    /// order: "View" for Switch bank, "Sort" for Sort by size.
    pub fn holder_of(&self, command: &CommandId) -> Option<&str> {
        self.menus
            .iter()
            .find(|m| {
                m.items
                    .iter()
                    .any(|i| i.action == ItemAction::Command(command.clone()))
            })
            .map(|m| m.title.as_str())
    }

    /// The item an access key chooses in a menu, where the place allows it.
    pub fn item_by_access(
        &self,
        menu: &str,
        letter: char,
        place: &Capabilities,
    ) -> Option<&MenuItem> {
        let letter = letter.to_lowercase().next()?;
        self.menu(menu)?
            .items
            .iter()
            .find(|i| i.access == Some(letter) && i.when.is_none_or(|w| w.holds(place)))
    }

    /// The menu bar as a list: each row a menu's title, then its key from
    /// the live keymap ("File, Alt+F").
    pub fn bar_list(&self, keymap: &Keymap) -> ListModel {
        let rows = self
            .bar()
            .map(|menu| {
                let key = commands::Menu::ALL
                    .iter()
                    .find(|m| m.id() == menu.id)
                    .map(|m| keymap.key_text(&m.command()))
                    .unwrap_or_default();
                let row = ListRow::new(format!("menu:{}", menu.id), menu.title.clone());
                if key.is_empty() {
                    row
                } else {
                    row.with_details([key])
                }
            })
            .collect();
        let mut list = ListModel::new("Menu bar").with_rows(rows);
        list.set_help("Left and Right choose a menu, Enter or Down opens it, Escape closes.");
        list
    }

    /// One menu as a list for a place: each row an item's label, then its
    /// key from the live keymap ("Copy to other panel, F5"); a submenu says
    /// "submenu". Items the place does not allow are left out.
    pub fn list(&self, id: &str, keymap: &Keymap, place: &Capabilities) -> Option<ListModel> {
        let menu = self.menu(id)?;
        let rows = menu
            .items
            .iter()
            .filter(|item| item.when.is_none_or(|w| w.holds(place)))
            .map(|item| {
                let row = ListRow::new(item.row_id(), item.label.clone());
                match &item.action {
                    ItemAction::Command(command) => {
                        let key = keymap.key_text(command);
                        if key.is_empty() {
                            row
                        } else {
                            row.with_details([key])
                        }
                    }
                    ItemAction::Submenu(_) => row.with_details(["submenu"]),
                }
            })
            .collect();
        let mut list = ListModel::new(format!("{} menu", menu.title)).with_rows(rows);
        list.set_empty_text("Nothing here for this place");
        list.set_help(
            "Up and Down choose, Enter runs, a letter runs its item, Escape closes, Left and Right change menus.",
        );
        Some(list)
    }
}

/// A button in a bank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    /// The label, said and shown.
    pub label: String,
    /// The commands it runs, in order, with their arguments.
    pub commands: Vec<Command>,
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
                    .map(|c| keymap.key_text(&c.id))
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

    /// The key bar: the bank's buttons as pairs on one line, each key
    /// first and its label after, two spaces between pairs, as the
    /// orthodox managers draw it: "F2 Rename  F3 View  F4 Edit". Keys come
    /// from the live keymap; a button whose command has no key is left
    /// out, since the bar names keys (answer 49).
    pub fn key_bar(&self, keymap: &Keymap) -> String {
        self.pairs(keymap).join(KEY_BAR_GAP)
    }

    /// The key bar cut to `cells`: whole pairs only, the first ones first,
    /// so the most common operations always show; the rest are said by
    /// Say Key Bar. The gap between pairs narrows to one space before a
    /// pair is left out, so the whole default bank fits 80 columns.
    pub fn key_bar_within(&self, keymap: &Keymap, cells: usize) -> String {
        let pairs = self.pairs(keymap);
        let wide = fit(&pairs, KEY_BAR_GAP, cells);
        let narrow = fit(&pairs, " ", cells);
        if narrow.1 > wide.1 { narrow.0 } else { wide.0 }
    }

    /// The pairs of the key bar: "F2 Rename".
    pub fn pairs(&self, keymap: &Keymap) -> Vec<String> {
        self.buttons
            .iter()
            .filter_map(|button| {
                let key = keymap.key_text(&button.commands.first()?.id);
                (!key.is_empty()).then(|| format!("{key} {}", button.label))
            })
            .collect()
    }

    /// The key bar as Say Key Bar says it, every pair, keys spoken in
    /// words: "Key bar: F2 Rename, F3 View, Control plus Q Quit."
    pub fn key_bar_spoken(&self, keymap: &Keymap) -> String {
        let pairs: Vec<String> = self
            .buttons
            .iter()
            .filter_map(|button| {
                let key = keymap.spoken_key(&button.commands.first()?.id);
                (!key.is_empty()).then(|| format!("{key} {}", button.label))
            })
            .collect();
        if pairs.is_empty() {
            return String::new();
        }
        format!("Key bar: {}.", pairs.join(", "))
    }
}

/// What stands between two pairs of the key bar.
pub const KEY_BAR_GAP: &str = "  ";

/// The pairs that fit `cells` with `gap` between them, and how many.
fn fit(pairs: &[String], gap: &str, cells: usize) -> (String, usize) {
    let mut line = String::new();
    let mut fitted = 0;
    for pair in pairs {
        let between = if line.is_empty() { "" } else { gap };
        if line.chars().count() + between.len() + pair.chars().count() > cells {
            break;
        }
        line.push_str(between);
        line.push_str(pair);
        fitted += 1;
    }
    (line, fitted)
}

/// The default bank: the most common operations first, as the key bar
/// shows them: "F2 Rename  F3 View  F4 Edit  F5 Copy  F6 Move  F7 Folder
/// F8 Trash  F9 Menu  Ctrl+Q Quit" (answer 49).
pub fn default_bank() -> Bank {
    let buttons = [
        ("Rename", ids::RENAME),
        ("View", ids::VIEW),
        ("Edit", ids::EDIT),
        ("Copy", ids::COPY),
        ("Move", ids::MOVE),
        ("Folder", ids::MAKE_FOLDER),
        ("Trash", ids::TRASH),
        ("Menu", ids::MENU),
        ("Quit", ids::QUIT),
    ];
    Bank {
        name: "Function keys".to_owned(),
        buttons: buttons
            .into_iter()
            .map(|(label, id)| Button {
                label: label.to_owned(),
                commands: vec![Command::new(id)],
            })
            .collect(),
    }
}

/// The viewer's bank, which the key bar shows while the viewer is open:
/// "Escape Close  / Search  n Next  g Jump  h Hex  w Wrap  Ctrl+D Status".
pub fn viewer_bank() -> Bank {
    let buttons = [
        ("Close", ids::CLOSE_VIEWER),
        ("Search", ids::VIEWER_SEARCH),
        ("Next", ids::VIEWER_NEXT_MATCH),
        ("Jump", ids::VIEWER_JUMP),
        ("Hex", ids::VIEWER_HEX),
        ("Wrap", ids::VIEWER_WRAP),
        ("Status", ids::SAY_STATUS),
    ];
    Bank {
        name: "Viewer".to_owned(),
        buttons: buttons
            .into_iter()
            .map(|(label, id)| Button {
                label: label.to_owned(),
                commands: vec![Command::new(id)],
            })
            .collect(),
    }
}

/// The key bar in the quick view's preview region (plan commit 64): its
/// own keys, Escape first.
pub fn preview_bank() -> Bank {
    let buttons = [
        ("Back", ids::PREVIEW_LEAVE),
        ("Heading", ids::PREVIEW_NEXT_HEADING),
        ("Table", ids::PREVIEW_NEXT_TABLE),
        ("Links", ids::PREVIEW_LINKS),
        ("Find", ids::PREVIEW_FIND),
        ("Next", ids::PREVIEW_NEXT_MATCH),
        ("File", ids::PREVIEW_NEXT_FILE),
        ("Status", ids::SAY_STATUS),
    ];
    Bank {
        name: "Preview".to_owned(),
        buttons: buttons
            .into_iter()
            .map(|(label, id)| Button {
                label: label.to_owned(),
                commands: vec![Command::new(id)],
            })
            .collect(),
    }
}

/// The banks from `banks.toml`, each button's commands read in the command
/// text form, with the problems in words; a command that cannot be read is
/// left out of its button. With no banks configured, the default bank.
pub fn banks(config: &BanksConfig) -> (Vec<Bank>, Vec<ConfigProblem>) {
    if config.banks.is_empty() {
        return (vec![default_bank()], Vec::new());
    }
    let mut problems = Vec::new();
    let banks = config
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
                        .filter_map(|text| match parse_command_text(text) {
                            Ok(command) if commands::info(&command.id).is_some() => Some(command),
                            Ok(command) => {
                                problems.push(ConfigProblem::new(
                                    Paths::BANKS,
                                    format!("Unknown command on {}: {}.", button.label, command.id),
                                ));
                                None
                            }
                            Err(why) => {
                                problems.push(ConfigProblem::new(
                                    Paths::BANKS,
                                    format!("Bad command on {}: {why}.", button.label),
                                ));
                                None
                            }
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    (banks, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Platform;
    use diroxide_config::{KeyOverride, KeymapConfig};
    use diroxide_core::format::within_cells;
    use diroxide_core::{Key, KeyColumn, Layer};

    fn local() -> Capabilities {
        let mut caps = Capabilities::read_only();
        caps.writable = true;
        caps.trash = true;
        caps
    }

    #[test]
    fn the_builtin_menus_read_cleanly_and_show_bound_keys() {
        let config: MenusConfig = toml::from_str(BUILTIN_MENUS_TOML).unwrap();
        let (menus, problems) = Menus::from_config(&config);
        assert_eq!(problems, Vec::new());
        assert_eq!(menus.bar().count(), 8);
        let keymap = Keymap::defaults();
        let edit = menus.list("edit", &keymap, &local()).unwrap();
        assert_eq!(edit.title(), "Edit menu");
        assert_eq!(edit.announce_focus(), "Undo, Ctrl+Z, 1 of 11");
        let bar = menus.bar_list(&keymap);
        assert_eq!(bar.announce_focus(), "File, Alt+F, 1 of 8");
        let view = menus.list("view", &keymap, &local()).unwrap();
        assert!(view.rows().iter().any(|r| r.spoken() == "Sort, submenu"));
    }

    /// Plan commit 67: every command alpha.2 adds is in a menu, in the
    /// palette, and named on the keyboard page by its id.
    #[test]
    fn every_alpha_2_command_is_in_a_menu() {
        let menus = Menus::builtin();
        let alpha2 = [
            ids::SWITCH_BANK,
            ids::OPEN_AS_ARCHIVE,
            ids::PACK,
            ids::UNPACK,
            ids::TEST_ARCHIVE,
            ids::SPLIT,
            ids::JOIN,
            ids::MAKE_LINK,
            ids::RELINK,
            ids::PERMISSIONS,
            ids::COPY_PATH,
            ids::COPY_NAME,
            ids::QUICK_VIEW,
            ids::OPEN_IN_TEXTWEAVER,
        ];
        let viewer = commands::CATALOG
            .iter()
            .filter(|i| matches!(i.layer, Layer::Viewer | Layer::Preview))
            .map(|i| i.id.clone());
        let palette = crate::palette::palette("", &Keymap::defaults());
        for id in alpha2.into_iter().chain(viewer) {
            assert!(menus.reaches(&id), "{id} is in no menu");
            assert!(
                palette.rows().iter().any(|r| r.id == id.as_str()),
                "{id} is not in the palette"
            );
        }
    }

    #[test]
    fn the_access_keys_match_the_menu_commands_keys() {
        let menus = Menus::builtin();
        let keymap = Keymap::defaults();
        for menu in commands::Menu::ALL {
            let def = menus.menu(menu.id()).unwrap();
            let key = Key::parse(&format!("alt+{}", def.access.unwrap())).unwrap();
            assert_eq!(keymap.keys_for(&menu.command()), vec![&key], "{menu}");
        }
        assert_eq!(
            menus
                .item_by_access("edit", 'P', &local())
                .map(|i| i.row_id()),
            Some("clipboard.paste".to_owned())
        );
    }

    #[test]
    fn a_place_without_a_trash_never_offers_the_trash() {
        let menus = Menus::builtin();
        let keymap = Keymap::defaults();
        let mut removable = local();
        removable.trash = false;
        let file = menus.list("file", &keymap, &removable).unwrap();
        assert!(!file.rows().iter().any(|r| r.id == "file.trash"));
        assert!(
            file.rows()
                .iter()
                .any(|r| r.id == "file.delete-permanently")
        );
        let read_only = Capabilities::read_only();
        let file = menus.list("file", &keymap, &read_only).unwrap();
        assert!(!file.rows().iter().any(|r| r.id == "file.rename"));
        assert!(file.rows().iter().any(|r| r.id == "file.view"));
        assert!(menus.item_by_access("file", 't', &removable).is_none());
    }

    #[test]
    fn every_label_fits_and_every_row_says_its_meaning_first() {
        let menus = Menus::builtin();
        for column in KeyColumn::ALL {
            let keymap = Keymap::defaults_for(column).for_platform(Platform::Windows);
            for menu in menus.menus() {
                let list = menus.list(&menu.id, &keymap, &local()).unwrap();
                for row in list.rows() {
                    assert!(row.label.chars().count() <= 40, "{}", row.label);
                    assert!(within_cells(&row.spoken(), &row.label), "{}", row.spoken());
                }
            }
        }
    }

    #[test]
    fn mistakes_in_menus_toml_are_said_and_the_rest_is_used() {
        let config: MenusConfig = toml::from_str(
            "[[menu]]
id = \"file\"
title = \"File\"
access = \"f\"
[[menu.item]]
command = \"file.view\"
access = \"v\"
[[menu.item]]
command = \"file.edit\"
access = \"v\"
[[menu.item]]
command = \"file.copi\"
[[menu.item]]
menu = \"nowhere\"
[[menu.item]]
command = \"file.copy\"
when = \"sometimes\"
[[menu.item]]
command = \"file.move\"
access = \"mv\"
[[menu.item]]
command = \"file.rename\"
label = \"Rename this entry to a new name of your own choosing\"
[[menu]]
id = \"find\"
title = \"Find\"
access = \"F\"
",
        )
        .unwrap();
        let (menus, problems) = Menus::from_config(&config);
        let said: Vec<&str> = problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "Unknown command in menu File: file.copi.",
                "Unknown submenu in menu File: nowhere.",
                "Unknown condition in menu File: sometimes. Use writable, trash, no_trash, restore, links or attributes.",
                "Access key must be one letter: \"mv\" in menu File.",
                "Label too long in menu File: 52 cells, at most 40.",
                "Access key V twice in menu File: View and Edit.",
                "Access key F twice on the menu bar: File and Find."
            ]
        );
        for problem in &problems {
            let meaning = problem.text.split(':').next().unwrap();
            assert!(within_cells(&problem.text, meaning), "{}", problem.text);
        }
        assert_eq!(menus.menu("file").unwrap().items.len(), 4);
        assert!(!menus.reaches(&ids::COPY));
        assert!(menus.reaches(&ids::RENAME));
    }

    #[test]
    fn removing_an_item_leaves_its_command_in_the_palette() {
        let mut config: MenusConfig = toml::from_str(BUILTIN_MENUS_TOML).unwrap();
        for menu in &mut config.menus {
            menu.items
                .retain(|i| i.command.as_deref() != Some("file.duplicate"));
        }
        let (menus, problems) = Menus::from_config(&config);
        assert!(problems.is_empty());
        assert!(!menus.reaches(&ids::DUPLICATE));
        let palette = crate::palette::palette("duplicate", &Keymap::defaults());
        assert_eq!(palette.focused().unwrap().id, "file.duplicate");
    }

    #[test]
    fn the_default_bank_is_the_function_keys_and_the_key_bar_follows_rebinding() {
        let keymap = Keymap::defaults();
        let (banks, problems) = banks(&BanksConfig::default());
        assert!(problems.is_empty());
        let bank = &banks[0];
        assert_eq!(bank.list(&keymap).announce_focus(), "Rename, F2, 1 of 9");
        assert_eq!(
            bank.key_bar(&keymap),
            "F2 Rename  F3 View  F4 Edit  F5 Copy  F6 Move  F7 Folder  F8 Trash  F9 Menu  Ctrl+Q Quit"
        );
        assert_eq!(
            bank.key_bar_within(&keymap, 40),
            "F2 Rename  F3 View  F4 Edit  F5 Copy"
        );
        assert_eq!(bank.key_bar_within(&keymap, 8), "");
        assert_eq!(
            bank.key_bar_within(&keymap, 80),
            "F2 Rename F3 View F4 Edit F5 Copy F6 Move F7 Folder F8 Trash F9 Menu Ctrl+Q Quit",
            "the gap narrows before a pair is left out"
        );
        assert_eq!(
            bank.key_bar_within(&keymap, 100),
            bank.key_bar(&keymap),
            "two spaces where everything fits"
        );
        assert_eq!(
            bank.key_bar_spoken(&keymap),
            "Key bar: F2 Rename, F3 View, F4 Edit, F5 Copy, F6 Move, F7 Folder, F8 Trash, F9 Menu, Control plus Q Quit."
        );
        let rebound = Keymap::defaults().with_overrides(&KeymapConfig {
            overrides: vec![KeyOverride {
                column: KeyColumn::Terminal,
                layer: Layer::Panel,
                command: ids::COPY,
                keys: vec![Key::parse("f12").unwrap()],
                line: 0,
            }],
            problems: Vec::new(),
        });
        assert!(bank.key_bar(&rebound).contains("F12 Copy"));
        let menus = Menus::builtin();
        let edit = menus.list("edit", &rebound, &local()).unwrap();
        assert!(
            edit.rows()
                .iter()
                .any(|r| r.spoken() == "Copy to other panel, F12")
        );
        let palette = crate::palette::palette("copy to other", &rebound);
        assert_eq!(
            palette.focused().unwrap().spoken(),
            "Copy to other panel, F12"
        );
    }

    #[test]
    fn bank_buttons_use_the_command_text_form() {
        let place = if cfg!(windows) {
            r"D:\\backup"
        } else {
            "/backup"
        };
        let config = BanksConfig::from_toml(&format!(
            "[[bank]]\nname = \"Mine\"\n[[bank.button]]\nlabel = \"Backup\"\ncommands = [\"go.path place={place}\", \"file.copi\", \"File.Copy\"]\n"
        ))
        .unwrap();
        let (banks, problems) = banks(&config);
        assert_eq!(banks[0].buttons[0].commands.len(), 1);
        assert_eq!(banks[0].buttons[0].commands[0].id, ids::GO_PATH);
        let said: Vec<&str> = problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "Unknown command on Backup: file.copi.",
                "Bad command on Backup: Not a command id: File.Copy."
            ]
        );
    }
}
