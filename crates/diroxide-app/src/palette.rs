//! The command palette: every command, ranked by how well its name matches
//! what was typed: names that start with it first, then names with a word
//! that starts with it, then names that contain it.
//!
//! Wave 1 (W1c) owns this module.

use diroxide_core::{ListModel, ListRow};

use crate::commands::CATALOG;
use crate::keymap::Keymap;

/// The palette's list for a query; an empty query lists every command in
/// catalog order.
pub fn palette(query: &str, keymap: &Keymap) -> ListModel {
    let wanted = query.trim().to_lowercase();
    let mut ranked: Vec<(u8, usize)> = CATALOG
        .iter()
        .enumerate()
        .filter_map(|(i, info)| {
            let name = info.name.to_lowercase();
            let rank = if wanted.is_empty() || name.starts_with(&wanted) {
                0
            } else if name.split(' ').any(|word| word.starts_with(&wanted)) {
                1
            } else if name.contains(&wanted) {
                2
            } else {
                return None;
            };
            Some((rank, i))
        })
        .collect();
    ranked.sort();
    let rows = ranked
        .into_iter()
        .map(|(_, i)| {
            let info = &CATALOG[i];
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
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_start_with_the_query_come_first() {
        let keymap = Keymap::defaults();
        let list = palette("se", &keymap);
        assert_eq!(list.focused().unwrap().label, "Select or unselect");
        let list = palette("panel", &keymap);
        assert_eq!(list.focused().unwrap().label, "Switch panels");
        assert_eq!(
            palette("zzz", &keymap).announce_focus(),
            "No command matches"
        );
    }
}
