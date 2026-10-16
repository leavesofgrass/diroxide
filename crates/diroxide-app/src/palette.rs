//! The command palette: every command by name, the guarantee that every
//! command is reachable whatever a terminal can deliver (plan section
//! 5.7). Names that start with what was typed come first, then names with
//! a word that starts with it, then names that contain it, then ids that
//! contain it; within each, the commands used most recently come first,
//! then the catalog's order. Each row is the name, then its key from the
//! live keymap.
//!
//! A dialog's own commands (Yes, No, Accept) are not listed: they act only
//! inside the dialog, where its keys and choices reach them.

use diroxide_core::{CommandId, Layer, ListModel, ListRow};

use crate::commands::{CATALOG, CommandInfo};
use crate::keymap::Keymap;

/// How many recent commands the palette remembers.
pub const RECENT_LIMIT: usize = 20;

/// The commands the palette lists, in catalog order.
pub fn reachable() -> impl Iterator<Item = &'static CommandInfo> {
    CATALOG.iter().filter(|info| info.layer != Layer::Dialog)
}

/// The palette, with the commands used most recently, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Palette {
    recent: Vec<CommandId>,
}

fn rank(info: &CommandInfo, wanted: &str) -> Option<u8> {
    if wanted.is_empty() {
        return Some(0);
    }
    let name = info.name.to_lowercase();
    if name.starts_with(wanted) {
        Some(0)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(wanted))
    {
        Some(1)
    } else if name.contains(wanted) {
        Some(2)
    } else if info.id.as_str().contains(wanted) {
        Some(3)
    } else {
        None
    }
}

impl Palette {
    /// A palette with no history.
    pub fn new() -> Self {
        Self::default()
    }

    /// A palette with a remembered history, newest first (the state folder
    /// keeps it between runs).
    pub fn with_recent(recent: Vec<CommandId>) -> Self {
        let mut palette = Self::new();
        for id in recent.into_iter().rev() {
            palette.record(&id);
        }
        palette
    }

    /// Notes that a command ran from the palette.
    pub fn record(&mut self, id: &CommandId) {
        self.recent.retain(|r| r != id);
        self.recent.insert(0, id.clone());
        self.recent.truncate(RECENT_LIMIT);
    }

    /// The recent commands, newest first.
    pub fn recent(&self) -> &[CommandId] {
        &self.recent
    }

    /// The palette's list for a query.
    pub fn list(&self, query: &str, keymap: &Keymap) -> ListModel {
        let wanted = query.trim().to_lowercase();
        let mut ranked: Vec<(u8, usize, usize, &CommandInfo)> = reachable()
            .enumerate()
            .filter_map(|(order, info)| {
                let class = rank(info, &wanted)?;
                let recency = self
                    .recent
                    .iter()
                    .position(|r| r == &info.id)
                    .unwrap_or(RECENT_LIMIT);
                Some((class, recency, order, info))
            })
            .collect();
        ranked.sort_by_key(|(class, recency, order, _)| (*class, *recency, *order));
        let rows = ranked
            .into_iter()
            .map(|(_, _, _, info)| {
                let key = keymap.key_text(&info.id);
                let row = ListRow::new(info.id.as_str(), info.name);
                if key.is_empty() {
                    row
                } else {
                    row.with_details([key])
                }
            })
            .collect();
        let mut list = ListModel::new("Command palette").with_rows(rows);
        list.set_empty_text("No command matches");
        list.set_help("Type to narrow, Up and Down choose, Enter runs, Escape closes.");
        list
    }
}

/// The palette's list for a query, with no history: names that start with
/// it first, then a word that starts with it, then names that contain it.
pub fn palette(query: &str, keymap: &Keymap) -> ListModel {
    Palette::new().list(query, keymap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ids;

    #[test]
    fn names_that_start_with_the_query_come_first() {
        let keymap = Keymap::defaults();
        let list = palette("se", &keymap);
        assert_eq!(list.focused().unwrap().label, "Settings");
        let list = palette("panel", &keymap);
        assert_eq!(list.focused().unwrap().label, "Say other panel");
        assert!(list.rows().iter().any(|r| r.label == "Switch panels"));
        let list = palette("sort by", &keymap);
        assert_eq!(list.focused().unwrap().spoken(), "Sort by name, Ctrl+F3");
        assert_eq!(
            palette("zzz", &keymap).announce_focus(),
            "No command matches"
        );
        assert_eq!(
            palette("key-test", &keymap).focused().unwrap().label,
            "Test keys"
        );
    }

    #[test]
    fn recently_used_commands_rise_within_their_rank() {
        let keymap = Keymap::defaults();
        let mut palette = Palette::new();
        assert_eq!(palette.list("", &keymap).focused().unwrap().label, "Quit");
        palette.record(&ids::SORT_SIZE);
        palette.record(&ids::COPY);
        let list = palette.list("", &keymap);
        assert_eq!(list.rows()[0].label, "Copy to other panel");
        assert_eq!(list.rows()[1].label, "Sort by size");
        let list = palette.list("sort", &keymap);
        assert_eq!(list.focused().unwrap().label, "Sort by size");
        palette.record(&ids::SORT_SIZE);
        assert_eq!(palette.recent()[0], ids::SORT_SIZE);
        assert_eq!(palette.recent().len(), 2);
        let again = Palette::with_recent(palette.recent().to_vec());
        assert_eq!(again, palette);
    }

    #[test]
    fn the_palette_reaches_every_command_but_a_dialogs_own() {
        let keymap = Keymap::defaults();
        let list = palette("", &keymap);
        for info in CATALOG.iter() {
            let listed = list.rows().iter().any(|r| r.id == info.id.as_str());
            assert_eq!(listed, info.layer != Layer::Dialog, "{}", info.id);
        }
    }
}
