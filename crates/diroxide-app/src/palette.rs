//! The command palette: every command by name, the guarantee that every
//! command is reachable whatever a terminal can deliver (plan section
//! 5.7). Names that start with what was typed come first, then names with
//! a word that starts with it, then names that contain it, then ids that
//! contain it. Within each, the command used last comes first, then the
//! commands used most, each use counting less as it ages (Worker's
//! command menu: every count is multiplied by 0.8 a day, R1), then the
//! catalog's order; so with nothing typed the last command used is the
//! first row. Each row is the name, then its key from the live keymap, or
//! the menu that holds a command with no key.
//!
//! The uses are kept in the state folder as the palette's history, one
//! line a command, newest first: "file.copy 2.44 1760000000", the id, the
//! aged count at the last use, and that use's time in seconds. A line of
//! an id alone, as alpha.1 wrote them, counts as one old use.
//!
//! A dialog's own commands (Yes, No, Accept) are not listed: they act only
//! inside the dialog, where its keys and choices reach them.

use diroxide_core::{CommandId, Layer, ListModel, ListRow};

use crate::commands::{CATALOG, CommandInfo};
use crate::keymap::Keymap;
use crate::menu::Menus;

/// How many recent commands the palette remembers.
pub const RECENT_LIMIT: usize = 20;

/// What a use is worth a day later: Worker's aging factor.
pub const AGING_PER_DAY: f64 = 0.8;

/// Seconds in a day, for the aging.
const DAY: f64 = 86_400.0;

/// One command's uses: the count, aged to its last use, and that time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Uses {
    count: f64,
    last: i64,
}

impl Uses {
    /// The count aged to a moment.
    fn at(self, now: i64) -> f64 {
        let days = ((now - self.last).max(0)) as f64 / DAY;
        self.count * AGING_PER_DAY.powf(days)
    }
}

/// The commands the palette lists, in catalog order.
pub fn reachable() -> impl Iterator<Item = &'static CommandInfo> {
    CATALOG.iter().filter(|info| info.layer != Layer::Dialog)
}

/// The palette, with the commands used most recently, newest first, and
/// how much each was used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    recent: Vec<CommandId>,
    uses: Vec<Uses>,
}

fn rank(id: &CommandId, name: &str, wanted: &str) -> Option<u8> {
    if wanted.is_empty() {
        return Some(0);
    }
    let name = name.to_lowercase();
    if name.starts_with(wanted) {
        Some(0)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(wanted))
    {
        Some(1)
    } else if name.contains(wanted) {
        Some(2)
    } else if id.as_str().contains(wanted) {
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

    /// A palette with a remembered history, newest first, each command
    /// counted as one old use.
    pub fn with_recent(recent: Vec<CommandId>) -> Self {
        let mut palette = Self::new();
        for id in recent.into_iter().rev() {
            palette.record_at(&id, 0);
        }
        palette
    }

    /// A palette from its history lines, newest first ("file.copy 2.44
    /// 1760000000", or an id alone); lines that cannot be read are
    /// dropped.
    pub fn from_lines(lines: &[String]) -> Self {
        let mut palette = Self::new();
        for line in lines.iter().take(RECENT_LIMIT) {
            let mut parts = line.split_whitespace();
            let Some(id) = parts.next().and_then(|id| CommandId::new(id).ok()) else {
                continue;
            };
            let count = parts
                .next()
                .and_then(|c| c.parse::<f64>().ok())
                .filter(|c| c.is_finite() && *c >= 0.0)
                .unwrap_or(1.0);
            let last = parts
                .next()
                .and_then(|t| t.parse::<i64>().ok())
                .unwrap_or(0);
            if palette.recent.contains(&id) {
                continue;
            }
            palette.recent.push(id);
            palette.uses.push(Uses { count, last });
        }
        palette
    }

    /// The history lines, newest first, as [`Palette::from_lines`] reads
    /// them.
    pub fn to_lines(&self) -> Vec<String> {
        self.recent
            .iter()
            .zip(&self.uses)
            .map(|(id, uses)| format!("{id} {:.4} {}", uses.count, uses.last))
            .collect()
    }

    /// Notes that a command ran from the palette, with no time: one use
    /// now, for callers without a clock.
    pub fn record(&mut self, id: &CommandId) {
        let now = self.uses.iter().map(|u| u.last).max().unwrap_or(0);
        self.record_at(id, now);
    }

    /// Notes that a command ran from the palette at a moment, in seconds
    /// since the epoch: its count ages to now and gains one, and it becomes
    /// the last command used.
    pub fn record_at(&mut self, id: &CommandId, now: i64) {
        let before = self.recent.iter().position(|r| r == id).map(|at| {
            self.recent.remove(at);
            self.uses.remove(at)
        });
        let count = before.map_or(0.0, |uses| uses.at(now)) + 1.0;
        self.recent.insert(0, id.clone());
        self.uses.insert(0, Uses { count, last: now });
        self.recent.truncate(RECENT_LIMIT);
        self.uses.truncate(RECENT_LIMIT);
    }

    /// A command's place in the ranking within its class: the last one
    /// used is 0; the others follow by their aged counts, most first; a
    /// command never used comes after them all. The order of two aged
    /// counts is the same at every moment, so the newest use is the
    /// moment counted to.
    fn standing(&self) -> Vec<(CommandId, usize)> {
        let now = self.uses.iter().map(|u| u.last).max().unwrap_or(0);
        let mut ranked: Vec<(usize, f64)> = self
            .uses
            .iter()
            .enumerate()
            .skip(1)
            .map(|(at, uses)| (at, uses.at(now)))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut standing: Vec<(CommandId, usize)> = Vec::with_capacity(self.recent.len());
        if let Some(first) = self.recent.first() {
            standing.push((first.clone(), 0));
        }
        for (place, (at, _)) in ranked.into_iter().enumerate() {
            standing.push((self.recent[at].clone(), place + 1));
        }
        standing
    }

    /// The recent commands, newest first.
    pub fn recent(&self) -> &[CommandId] {
        &self.recent
    }

    /// The palette's list for a query.
    pub fn list(&self, query: &str, keymap: &Keymap) -> ListModel {
        self.list_with(query, keymap, None)
    }

    /// The palette's list for a query, each row the command's name and its
    /// key beside it ("Copy to other panel, F5"); a command with no key
    /// says the menu that holds it instead ("Switch bank, View menu"), so
    /// every row says how to reach the command again (answer 49).
    pub fn list_with(&self, query: &str, keymap: &Keymap, menus: Option<&Menus>) -> ListModel {
        self.list_with_extra(query, keymap, menus, &[])
    }

    /// The same list with more commands after the built-in ones, ranked
    /// with them: the enabled plugins' commands, by id and name (W4d).
    pub fn list_with_extra(
        &self,
        query: &str,
        keymap: &Keymap,
        menus: Option<&Menus>,
        extra: &[(CommandId, String)],
    ) -> ListModel {
        let wanted = query.trim().to_lowercase();
        let standing = self.standing();
        let commands = reachable()
            .map(|info| (info.id.clone(), info.name.to_owned()))
            .chain(extra.iter().cloned());
        let mut ranked: Vec<(u8, usize, usize, CommandId, String)> = commands
            .enumerate()
            .filter_map(|(order, (id, name))| {
                let class = rank(&id, &name, &wanted)?;
                let used = standing
                    .iter()
                    .find(|(used, _)| used == &id)
                    .map_or(RECENT_LIMIT, |(_, place)| *place);
                Some((class, used, order, id, name))
            })
            .collect();
        ranked.sort_by_key(|(class, recency, order, _, _)| (*class, *recency, *order));
        let rows = ranked
            .into_iter()
            .map(|(_, _, _, id, name)| {
                let key = keymap.key_text(&id);
                let row = ListRow::new(id.as_str(), name);
                if !key.is_empty() {
                    return row.with_details([key]);
                }
                match menus.and_then(|m| m.holder_of(&id)) {
                    Some(menu) => row.with_details([format!("{menu} menu")]),
                    None => row,
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
        assert_eq!(again.recent(), palette.recent());
    }

    #[test]
    fn uses_age_by_a_fifth_a_day_and_the_last_one_leads() {
        let keymap = Keymap::defaults();
        let day = 86_400;
        let mut palette = Palette::new();
        // Sort by size five times on day 0, Swap panels three times on day
        // 9, Copy once on day 10.
        for _ in 0..5 {
            palette.record_at(&ids::SORT_SIZE, 0);
        }
        for _ in 0..3 {
            palette.record_at(&ids::SWAP_PANELS, 9 * day);
        }
        palette.record_at(&ids::COPY, 10 * day);
        let first: Vec<String> = palette.list("", &keymap).rows()[..3]
            .iter()
            .map(|r| r.label.clone())
            .collect();
        // Copy was last; Swap's 3 aged a day (2.4) beat Sort by size's 5
        // aged ten days (0.54).
        assert_eq!(
            first,
            ["Copy to other panel", "Swap panels", "Sort by size"]
        );
        // Within a class, the uses rank too.
        let sort = palette.list("sort", &keymap);
        assert_eq!(sort.focused().unwrap().label, "Sort by size");
        // A use counts on: Sort by size used again on day 10 is the last.
        palette.record_at(&ids::SORT_SIZE, 10 * day);
        assert_eq!(palette.list("", &keymap).rows()[0].label, "Sort by size");
        let lines = palette.to_lines();
        assert!(
            lines[0].starts_with("view.sort-size 1.5369 864000"),
            "{lines:?}"
        );
        assert_eq!(Palette::from_lines(&lines).to_lines(), lines);
        // alpha.1's lines, ids alone, read as one old use each.
        let old = Palette::from_lines(&["file.copy".to_owned(), "Not.An Id".to_owned()]);
        assert_eq!(old.recent(), [ids::COPY]);
        assert_eq!(old.to_lines(), ["file.copy 1.0000 0"]);
    }

    #[test]
    fn every_row_says_its_key_or_its_menu() {
        let keymap = Keymap::defaults();
        let menus = Menus::builtin();
        let list = Palette::new().list_with("", &keymap, Some(&menus));
        for row in list.rows() {
            let info = crate::commands::info(&CommandId::new(row.id.clone()).unwrap()).unwrap();
            if menus.reaches(&info.id) || !keymap.key_text(&info.id).is_empty() {
                assert!(!row.details.is_empty(), "{}", row.id);
            }
        }
        let bank = Palette::new().list_with("switch bank", &keymap, Some(&menus));
        assert_eq!(bank.focused().unwrap().spoken(), "Switch bank, View menu");
        let copy = Palette::new().list_with("copy to other", &keymap, Some(&menus));
        assert_eq!(copy.focused().unwrap().spoken(), "Copy to other panel, F5");
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
