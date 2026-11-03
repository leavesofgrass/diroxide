//! The window's menus, from the app's one menu model (`menus.toml`,
//! ADR-0005; textweaver's ADR-0046 is the model for the window's side).
//!
//! The app owns the menus: the bar menus (File, Edit, View, Go, Search,
//! Select, Tools, Plugins, Help) and their submenus, each item a command
//! with its access key, its `when` condition, and its key read from the
//! live keymap. This module keeps no second list of commands; [`tree`]
//! turns the model into what a system's menu bar shows, and the platform
//! code builds the native menu from that tree:
//!
//! - **Windows** (the `menus` setting's native, the default): a Win32 menu
//!   bar. UI Automation exposes it as MenuBar, Menu and MenuItem, with the
//!   access key from the `&` letter and the key as `AcceleratorKey` from
//!   the text after a tab. With the `auto_hide_menu` setting (on by
//!   default) the bar is hidden until Alt alone, F10 or Alt with a menu's
//!   letter shows it, and it hides again, silently, when its menu closes.
//!   The keys shown are only shown: the keymap stays the one place keys
//!   are handled, so no accelerator table duplicates it.
//! - **Linux, macOS for now, and with `menus = list`:** the app's list
//!   menus in the window's modal host (F9 for the menu bar, Alt with a
//!   menu's letter), as in the terminal. macOS's own menu bar needs an
//!   AppKit binding the workspace does not have (a contract request).
//!
//! An item whose condition the place does not meet is left out, as the
//! list menus leave it out; the tree is built again when the active
//! place, the keymap or the menus change ([`fingerprint`]).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use diroxide_app::App;
use diroxide_app::menu::ItemAction;

/// How deep submenus go: the model nests one level (View, Sort); a deeper
/// one is drawn no further.
const MAX_DEPTH: usize = 3;

/// One entry of a menu as a system shows it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    /// An item that runs a command.
    Item {
        /// The command's id, given back when the item is chosen.
        command: String,
        /// The label with its access key marked by `&` (a literal `&`
        /// doubled).
        label: String,
        /// The key from the live keymap, written ("Ctrl+F5"), if bound.
        key: Option<String>,
    },
    /// A submenu.
    Submenu {
        /// Its label, access key marked.
        label: String,
        /// Its entries.
        entries: Vec<Entry>,
    },
}

impl Entry {
    /// The text a Windows menu item takes: the label, then a tab and the
    /// key, which UI Automation gives screen readers as the item's
    /// `AcceleratorKey`.
    pub fn windows_text(&self) -> String {
        match self {
            Entry::Item {
                label,
                key: Some(key),
                ..
            } => format!("{label}\t{key}"),
            Entry::Item { label, .. } | Entry::Submenu { label, .. } => label.clone(),
        }
    }
}

/// A menu on the bar.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TopMenu {
    /// The menu's id in `menus.toml`.
    pub id: String,
    /// Its title with the access key marked.
    pub label: String,
    /// Its access key, lowercase: Alt with it opens the menu.
    pub access: Option<char>,
    /// Its entries.
    pub entries: Vec<Entry>,
}

/// A label with its access key marked for a native menu: the first
/// occurrence of the letter, in any case, gets an `&` before it; a
/// literal `&` is doubled; a letter the label lacks is added after it in
/// brackets ("Plugins (&X)"), as Windows programs do.
pub fn marked(label: &str, access: Option<char>) -> String {
    let escaped = label.replace('&', "&&");
    let Some(access) = access else {
        return escaped;
    };
    let mut out = String::with_capacity(escaped.len() + 1);
    let mut done = false;
    for c in escaped.chars() {
        if !done && c.to_lowercase().eq(access.to_lowercase()) {
            out.push('&');
            done = true;
        }
        out.push(c);
    }
    if !done {
        out.push_str(&format!(" (&{})", access.to_uppercase()));
    }
    out
}

/// A menu's entries from the app's own list of it ([`App::menu_list`]):
/// the items whose condition the place meets, the enabled plugins'
/// commands among them (the Plugins menu lists every plugin's), each with
/// its access key from the model where the model has the item.
fn entries(app: &App, menu: &str, depth: usize) -> Vec<Entry> {
    let Some(list) = app.menu_list(menu) else {
        return Vec::new();
    };
    let def = app.menus().menu(menu);
    let mut out = Vec::new();
    for row in list.rows() {
        // A row the app lists as unavailable (a plugin that is off) is
        // left out of the native menu, as an item whose condition fails.
        if !row.enabled {
            continue;
        }
        let item = def.and_then(|d| d.items.iter().find(|i| i.row_id() == row.id));
        let label = match item {
            Some(item) => marked(&item.label, item.access),
            None => marked(&row.label, None),
        };
        if let Some(sub) = row.id.strip_prefix("menu:") {
            if depth < MAX_DEPTH {
                let inner = entries(app, sub, depth + 1);
                if !inner.is_empty() {
                    out.push(Entry::Submenu {
                        label,
                        entries: inner,
                    });
                }
            }
            continue;
        }
        let command = match item.map(|i| &i.action) {
            Some(ItemAction::Command(command)) => command.clone(),
            _ => match diroxide_core::CommandId::new(row.id.clone()) {
                Ok(command) => command,
                Err(_) => continue,
            },
        };
        let key = app.keymap().key_text(&command);
        out.push(Entry::Item {
            command: command.to_string(),
            label,
            key: (!key.is_empty()).then_some(key),
        });
    }
    out
}

/// The menu bar as the app's model gives it for the active place.
pub fn tree(app: &App) -> Vec<TopMenu> {
    app.menus()
        .bar()
        .map(|menu| TopMenu {
            id: menu.id.clone(),
            label: marked(&menu.title, menu.access),
            access: menu.access,
            entries: entries(app, &menu.id, 1),
        })
        .collect()
}

/// A number that changes when the tree does, cheap to compare after every
/// step.
pub fn fingerprint(menus: &[TopMenu]) -> u64 {
    let mut hasher = DefaultHasher::new();
    menus.hash(&mut hasher);
    hasher.finish()
}

/// Whether the window shows the system's menu bar: on Windows, unless the
/// `menus` setting is `list`. Elsewhere the list menus serve.
pub fn native(app: &App) -> bool {
    cfg!(windows) && app.settings().menus != diroxide_app::config::Menus::List
}

/// The bar menu whose access key is `letter`, for Alt with a letter.
pub fn by_access(menus: &[TopMenu], letter: char) -> Option<&TopMenu> {
    let letter = letter.to_lowercase().next()?;
    menus.iter().find(|m| m.access == Some(letter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_app::{App, AppOptions};
    use diroxide_core::Place;

    fn app() -> App {
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        App::new(AppOptions::new(here.clone(), here))
    }

    #[test]
    fn an_access_key_is_marked_once_and_ampersands_are_doubled() {
        assert_eq!(marked("File", Some('f')), "&File");
        assert_eq!(
            marked("Copy to other panel", Some('c')),
            "&Copy to other panel"
        );
        assert_eq!(marked("Sort by size", Some('z')), "Sort by si&ze");
        assert_eq!(marked("Search & replace", Some('r')), "Sea&rch && replace");
        assert_eq!(marked("Plugins", Some('x')), "Plugins (&X)");
        assert_eq!(marked("Help", None), "Help");
    }

    #[test]
    fn the_tree_is_the_models_bar_with_live_keys() {
        let app = app();
        let menus = tree(&app);
        let titles: Vec<&str> = menus.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(titles[0], "&File", "{titles:?}");
        assert_eq!(menus.len(), app.menus().bar().count());
        // Access keys on the bar are unique.
        let mut keys: Vec<char> = menus.iter().filter_map(|m| m.access).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "{menus:?}");
        // Every key shown is the key the keymap binds to that command.
        fn check(app: &App, entries: &[Entry], seen: &mut usize) {
            for e in entries {
                match e {
                    Entry::Item { command, key, .. } => {
                        let id = diroxide_core::CommandId::new(command.clone()).unwrap();
                        let bound = app.keymap().key_text(&id);
                        assert_eq!(key.clone().unwrap_or_default(), bound, "{command}");
                        *seen += 1;
                    }
                    Entry::Submenu { entries, .. } => check(app, entries, seen),
                }
            }
        }
        let mut seen = 0;
        for menu in &menus {
            check(&app, &menu.entries, &mut seen);
        }
        assert!(seen > 30, "{seen} items");
        // A Windows item says its key after a tab.
        let file = &menus[0].entries;
        let with_key = file
            .iter()
            .find(|e| matches!(e, Entry::Item { key: Some(_), .. }))
            .unwrap();
        assert!(with_key.windows_text().contains('\t'), "{with_key:?}");
        assert_eq!(fingerprint(&menus), fingerprint(&tree(&app)));
        assert!(by_access(&menus, 'F').is_some());
    }
}
