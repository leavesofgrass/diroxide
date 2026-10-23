//! What enabled plugins add to the app (R22 commit 36): their commands in
//! the Plugins menu, the palette and the banks, and their proposed keys.
//!
//! - Only an enabled, unchanged item contributes; one that is not enabled
//!   appears only in Manage plugins, and `--no-plugins` contributes
//!   nothing.
//! - A command whose conditions fail (its system, a missing tool, the
//!   places, the selection's count) is shown disabled with its reason:
//!   "Search with Everything, disabled", then "Needs es, not found.".
//! - A proposed key is bound only when it is free in every layer a panel
//!   looks in and has no concern (a terminal that cannot send it, a
//!   screen reader's key, a reserved key, a single letter); it is shown
//!   as proposed, never taken from a built-in command.
//! - A plugin command's id is its plugin's id, a dot and its own id; an id
//!   in one of diroxide's own namespaces is diroxide's, so `menus.toml`
//!   and `banks.toml` may name a plugin's command and it shows only while
//!   the plugin is enabled, with nothing reported.

use std::collections::BTreeSet;

use diroxide_core::{CommandId, Key, Layer, ListRow};
use diroxide_plugins::script::context::RunContext;
use diroxide_plugins::script::words;

use super::catalog::{Catalog, CommandEntry};
use crate::commands::CATALOG;
use crate::keymap::{KeyChoice, Keymap};

/// The menu a plugin command goes to when its manifest names none.
pub const PLUGINS_MENU: &str = "Plugins";

/// The namespaces of diroxide's own command ids (`file`, `view`, ...),
/// with `script`, the scripts' own.
pub fn builtin_namespaces() -> BTreeSet<&'static str> {
    CATALOG
        .iter()
        .filter_map(|c| c.id.as_str().split('.').next())
        .collect()
}

/// Whether an id names a plugin's or a script's command rather than one of
/// diroxide's: its first word is not one of diroxide's namespaces, or it
/// is a script's (`script.NAME`).
pub fn is_plugin_command(id: &CommandId) -> bool {
    let Some((first, rest)) = id.as_str().split_once('.') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    first == "script" || !builtin_namespaces().contains(first)
}

/// One plugin command as a menu row: its name and its key, or "NAME,
/// disabled" with the reason after it.
pub fn menu_row(command: &CommandEntry, keymap: &Keymap, context: &RunContext) -> ListRow {
    match command.refusal(context) {
        Some(reason) => {
            ListRow::new(command.id.as_str(), words::disabled(&command.name)).with_details([reason])
        }
        None => {
            let key = keymap.key_text(&command.id);
            let row = ListRow::new(command.id.as_str(), command.name.clone());
            if key.is_empty() {
                row
            } else {
                row.with_details([key])
            }
        }
    }
}

/// The rows the Plugins menu gains: every contributing command whose menu
/// is the Plugins menu, in item order.
pub fn plugins_menu_rows(catalog: &Catalog, keymap: &Keymap, context: &RunContext) -> Vec<ListRow> {
    catalog
        .commands()
        .filter(|c| in_plugins_menu(catalog, c))
        .map(|c| menu_row(c, keymap, context))
        .collect()
}

fn in_plugins_menu(catalog: &Catalog, command: &CommandEntry) -> bool {
    let menu = catalog
        .item(&command.owner)
        .and_then(|item| item.manifest.as_ref())
        .and_then(|m| m.commands.iter().find(|c| c.id == command.local))
        .map(|c| c.menu.trim().to_owned())
        .unwrap_or_default();
    menu.is_empty() || menu.eq_ignore_ascii_case(PLUGINS_MENU)
}

/// The commands the palette gains: each contributing command's id and
/// name.
pub fn palette_entries(catalog: &Catalog) -> Vec<(CommandId, String)> {
    catalog
        .commands()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect()
}

/// What happened to one proposed key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// The command.
    pub command: CommandId,
    /// The key as the plugin wrote it.
    pub key: String,
    /// Whether it was bound.
    pub bound: bool,
    /// Why not, in words, when it was not.
    pub why: Option<String>,
}

/// The layers a panel looks a key up in; a plugin's key goes in the last.
const LOOKED_IN: [Layer; 3] = [Layer::Panel, Layer::List, Layer::Global];

/// Binds each contributing command's proposed key in the global layer
/// when it is free and has no concern; returns what happened to each.
pub fn propose_keys(keymap: &mut Keymap, catalog: &Catalog) -> Vec<Proposal> {
    let mut out = Vec::new();
    for command in catalog.commands() {
        let Some(text) = command.key.clone() else {
            continue;
        };
        let proposal = match Key::parse(&text) {
            Err(_) => Proposal {
                command: command.id.clone(),
                key: text,
                bound: false,
                why: Some("the key cannot be read".to_owned()),
            },
            Ok(key) => {
                let taken = LOOKED_IN
                    .iter()
                    .find_map(|layer| keymap.binding(*layer, &key).map(|b| b.command.clone()))
                    .filter(|holder| holder != &command.id);
                let concern = keymap
                    .concerns(Layer::Global, &key, &command.id)
                    .first()
                    .map(|c| c.sentence(&key, keymap.platform()));
                match (taken, concern) {
                    (Some(holder), _) => Proposal {
                        command: command.id.clone(),
                        key: text,
                        bound: false,
                        why: Some(format!(
                            "it already runs {}",
                            crate::commands::name_of(&holder)
                        )),
                    },
                    (None, Some(why)) => Proposal {
                        command: command.id.clone(),
                        key: text,
                        bound: false,
                        why: Some(why.trim_end_matches('.').to_owned()),
                    },
                    (None, None) => {
                        keymap.bind(Layer::Global, key, &command.id, true, None, KeyChoice::Take);
                        Proposal {
                            command: command.id.clone(),
                            key: text,
                            bound: true,
                            why: None,
                        }
                    }
                }
            }
        };
        out.push(proposal);
    }
    out
}

/// Takes every plugin command's key out of a keymap, before the keys of a
/// new catalog are proposed.
pub fn forget_keys(keymap: &mut Keymap) {
    let plugin_bindings: Vec<(Layer, Key, CommandId)> = keymap
        .bindings()
        .iter()
        .filter(|b| is_plugin_command(&b.command))
        .map(|b| (b.layer, b.key, b.command.clone()))
        .collect();
    for (layer, key, command) in plugin_bindings {
        keymap.unbind(layer, &key, &command);
    }
}

/// A proposal in words, for Manage plugins: "Ctrl+Alt+L, proposed" or
/// "Ctrl+Alt+L not bound: it already runs Copy".
pub fn proposal_words(proposal: &Proposal) -> String {
    match &proposal.why {
        None => format!("{}, proposed", proposal.key),
        Some(why) => format!("{} not bound: {why}", proposal.key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_are_told_from_diroxides_own() {
        let id = |text: &str| CommandId::new(text).unwrap();
        assert!(is_plugin_command(&id("line-counter.count")));
        assert!(is_plugin_command(&id("script.total-size")));
        assert!(!is_plugin_command(&id("file.copy")));
        assert!(!is_plugin_command(&id("file.not-a-command")));
        assert!(!is_plugin_command(&id("view")));
        assert!(builtin_namespaces().contains("plugins"));
    }
}
