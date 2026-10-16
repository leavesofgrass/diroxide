//! A panel (a lister): a place, its entries, sort, filter, columns,
//! selection, focus and history, shown through the list model so each row
//! reads "report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40".
//!
//! Wave 1 (W1d) owns this module.

use std::cmp::Ordering;

use diroxide_config::Settings;
use diroxide_core::format::human_size;
use diroxide_core::{
    Clock, Entry, EntryKind, ListModel, ListRow, Place, RequestId, Side, SortSpec, clock,
};
use diroxide_filetype::TypeRegistry;

/// A detail said after an entry's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    /// The file type: "PDF", "folder".
    Type,
    /// The size in words: "2.1 MB".
    Size,
    /// "modified Tuesday".
    Modified,
    /// "accessed today, 2:05 PM".
    Accessed,
    /// "changed October 3".
    Changed,
    /// Permissions or attributes.
    Permissions,
    /// The owner.
    Owner,
    /// Where a link points.
    LinkTarget,
    /// The extension.
    Extension,
}

impl Column {
    /// A column from its name in `config.toml`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "type" => Column::Type,
            "size" => Column::Size,
            "modified" => Column::Modified,
            "accessed" => Column::Accessed,
            "changed" => Column::Changed,
            "permissions" => Column::Permissions,
            "owner" => Column::Owner,
            "link" => Column::LinkTarget,
            "extension" => Column::Extension,
            _ => return None,
        })
    }
}

/// The row for an entry: its display name, then "name needs care" when it
/// does, then the columns, each in words; empty details are left out.
pub fn row_for(entry: &Entry, columns: &[Column], clock: &dyn Clock) -> ListRow {
    let mut details = Vec::new();
    if entry.needs_care || entry.name_not_unicode {
        details.push("name needs care".to_owned());
    }
    for column in columns {
        let detail = match column {
            Column::Type => entry
                .type_name
                .clone()
                .unwrap_or_else(|| entry.kind.word().to_owned()),
            Column::Size => match (entry.kind, entry.size) {
                (EntryKind::File | EntryKind::Archive, Some(size)) => human_size(size),
                _ => String::new(),
            },
            Column::Modified => time_detail("modified", entry.modified, clock),
            Column::Accessed => time_detail("accessed", entry.accessed, clock),
            Column::Changed => time_detail("changed", entry.changed, clock),
            Column::Permissions => entry.attributes.map(|a| a.describe()).unwrap_or_default(),
            Column::Owner => entry.owner.clone().unwrap_or_default(),
            Column::LinkTarget => entry
                .link
                .as_ref()
                .map(|l| format!("to {}", l.target))
                .unwrap_or_default(),
            Column::Extension => entry.extension().unwrap_or_default().to_owned(),
        };
        if !detail.is_empty() {
            details.push(detail);
        }
    }
    ListRow::new(entry.name.clone(), entry.display_name()).with_details(details)
}

fn time_detail(word: &str, time: Option<diroxide_core::Timestamp>, clock: &dyn Clock) -> String {
    time.map(|t| format!("{word} {}", clock::describe_time(t, clock)))
        .unwrap_or_default()
}

/// Compares names the way people count: "file2" before "file10", ignoring
/// case.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (dx, dy) = (take(&mut a), take(&mut b));
                let (tx, ty) = (dx.trim_start_matches('0'), dy.trim_start_matches('0'));
                let order = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// One panel.
#[derive(Clone, Debug)]
pub struct Panel {
    side: Side,
    place: Place,
    all_entries: Vec<Entry>,
    entries: Vec<Entry>,
    list: ListModel,
    sort: SortSpec,
    show_hidden: bool,
    columns: Vec<Column>,
    history: Vec<Place>,
    loading: Option<(RequestId, Place, Option<String>)>,
}

impl Panel {
    /// A panel showing `place`, not yet listed.
    pub fn new(side: Side, place: Place, settings: &Settings) -> Self {
        let mut list = ListModel::new(place.title());
        list.set_empty_text("Empty folder");
        list.set_help(
            "Up and Down move, Enter opens, Backspace goes up, Space selects, Tab switches panels, F9 opens the menus.",
        );
        Self {
            side,
            place,
            all_entries: Vec::new(),
            entries: Vec::new(),
            list,
            sort: settings.sort_spec(),
            show_hidden: settings.show_hidden,
            columns: settings
                .columns
                .iter()
                .filter_map(|c| Column::from_name(c))
                .collect(),
            history: Vec::new(),
            loading: None,
        }
    }

    /// Which side it is on.
    pub fn side(&self) -> Side {
        self.side
    }

    /// The place shown.
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// The list of rows.
    pub fn list(&self) -> &ListModel {
        &self.list
    }

    /// The list of rows, to move the focus or select.
    pub fn list_mut(&mut self) -> &mut ListModel {
        &mut self.list
    }

    /// The entries shown, in row order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The places visited before this one, oldest first.
    pub fn history(&self) -> &[Place] {
        &self.history
    }

    /// The focused entry.
    pub fn focused_entry(&self) -> Option<&Entry> {
        let id = &self.list.focused()?.id;
        self.entries.iter().find(|e| &e.name == id)
    }

    /// The focused entry's place.
    pub fn focused_place(&self) -> Option<Place> {
        self.place.join(&self.focused_entry()?.name).ok()
    }

    /// The selected entries' places, or the focused entry's when nothing is
    /// selected: what a command acts on.
    pub fn targets(&self) -> Vec<Place> {
        let selected: Vec<Place> = self
            .list
            .selected_ids()
            .into_iter()
            .filter_map(|id| self.place.join(id).ok())
            .collect();
        if selected.is_empty() {
            self.focused_place().into_iter().collect()
        } else {
            selected
        }
    }

    /// Whether a listing is on its way.
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Notes that `place` is being listed under `request`; the focus will
    /// go to the entry named `focus` when it arrives.
    pub fn begin_load(&mut self, request: RequestId, place: Place, focus: Option<String>) {
        self.loading = Some((request, place, focus));
    }

    /// Whether `request` is the listing this panel waits for.
    pub fn waits_for(&self, request: RequestId) -> bool {
        matches!(&self.loading, Some((r, _, _)) if *r == request)
    }

    /// Takes a finished listing: the panel moves to the place it asked for
    /// (remembering the old one in history) and shows the entries.
    pub fn finish_load(&mut self, entries: Vec<Entry>, types: &TypeRegistry, clock: &dyn Clock) {
        let Some((_, place, focus)) = self.loading.take() else {
            return;
        };
        if place != self.place {
            self.history.push(std::mem::replace(&mut self.place, place));
            self.list.select_none();
            self.list.move_focus(diroxide_core::Move::Home);
        }
        self.list.set_title(self.place.title());
        self.set_entries(entries, types, clock);
        if let Some(name) = focus {
            self.list.focus_id(&name);
        }
    }

    /// Drops a failed listing; the panel stays where it was.
    pub fn cancel_load(&mut self) {
        self.loading = None;
    }

    /// Replaces the entries: fills in types, hides what is hidden, sorts,
    /// and rebuilds the rows, keeping the focus and the selection by name.
    pub fn set_entries(
        &mut self,
        mut entries: Vec<Entry>,
        types: &TypeRegistry,
        clock: &dyn Clock,
    ) {
        for entry in &mut entries {
            if entry.type_name.is_none() {
                entry.type_name = types.short_name(entry);
            }
        }
        self.all_entries = entries;
        self.refresh(clock);
    }

    /// Shows or hides hidden entries.
    pub fn set_show_hidden(&mut self, show: bool, clock: &dyn Clock) {
        self.show_hidden = show;
        self.refresh(clock);
    }

    /// Whether hidden entries are shown.
    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    fn refresh(&mut self, clock: &dyn Clock) {
        let selected: Vec<String> = self
            .list
            .selected_ids()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut shown: Vec<Entry> = self
            .all_entries
            .iter()
            .filter(|e| self.show_hidden || !e.hidden)
            .cloned()
            .collect();
        diroxide_core::sort::sort_entries(&mut shown, &self.sort);
        let rows = shown
            .iter()
            .map(|e| {
                let mut row = row_for(e, &self.columns, clock);
                row.selected = selected.contains(&e.name);
                row
            })
            .collect();
        self.entries = shown;
        self.list.set_rows(rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;
    use diroxide_core::{FixedClock, Timestamp, UtcOffset};

    fn clock() -> FixedClock {
        FixedClock::new(
            Timestamp::from_civil(2026, 10, 10, 15, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        )
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut names = vec!["file10", "File2", "file1", "a", "file02x"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["a", "file1", "File2", "file02x", "file10"]);
    }

    #[test]
    fn rows_read_meaning_first() {
        let clock = clock();
        let tuesday = Timestamp::from_civil(2026, 10, 6, 9, 0, 0, UtcOffset::UTC).unwrap();
        let mut entry = Entry::new("report.pdf", EntryKind::File)
            .with_size(2_100_000)
            .with_modified(tuesday);
        entry.type_name = Some("PDF".into());
        let row = row_for(
            &entry,
            &[Column::Type, Column::Size, Column::Modified],
            &clock,
        );
        assert_eq!(row.spoken(), "report.pdf, PDF, 2.1 MB, modified Tuesday");
        let odd = Entry::new("notes.", EntryKind::File);
        let mut odd = odd;
        odd.needs_care = true;
        assert_eq!(
            row_for(&odd, &[], &clock).spoken(),
            "notes., name needs care"
        );
    }

    #[test]
    fn a_panel_sorts_folders_first_and_hides_hidden() {
        let clock = clock();
        let place = Place::parse_with("/home/ada", PathStyle::Unix).unwrap();
        let mut panel = Panel::new(Side::Left, place, &Settings::default());
        let mut hidden = Entry::new(".profile", EntryKind::File);
        hidden.hidden = true;
        let entries = vec![
            Entry::new("b.txt", EntryKind::File).with_size(10),
            Entry::new("Music", EntryKind::Folder),
            hidden,
            Entry::new("a.pdf", EntryKind::File),
        ];
        panel.set_entries(entries, &TypeRegistry::builtin(), &clock);
        let labels: Vec<&str> = panel
            .list()
            .rows()
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Music", "a.pdf", "b.txt"]);
        assert_eq!(panel.list().announce_focus(), "Music, folder, 1 of 3");
        panel.list_mut().move_focus(diroxide_core::Move::Down);
        panel.list_mut().toggle_selected();
        panel.set_show_hidden(true, &clock);
        assert_eq!(panel.list().len(), 4);
        assert_eq!(panel.list().focused().unwrap().id, "a.pdf");
        assert_eq!(panel.list().selected_ids(), vec!["a.pdf"]);
        assert_eq!(panel.targets().len(), 1);
    }
}
