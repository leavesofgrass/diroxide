//! A plugin's lister of item rows (R22 commit 39): rows that are not
//! files, each its name first and its columns' values after it in the
//! lister's order, sorted and filtered by those columns, marked with
//! Space; Enter runs the lister's `activate` command and its action keys
//! their commands, with the marked rows (or the one under the cursor) as
//! the selection. A file command on a row answers "This row is not a
//! file.".
//!
//! The rows keep the plugin's order until the owner sorts; a sort by a
//! number, size or time column compares numbers, a text column compares
//! words naturally, and the same column again reverses it.

use std::cmp::Ordering;

use diroxide_core::{Entry, ListModel, ListRow};
use diroxide_plugins::listers::{ITEM_ID, ListerView};
use diroxide_plugins::manifest::ListerColumn;

use crate::panel::natural_cmp;

/// The column a lister is sorted by, and which way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListerSort {
    /// The column's id; `name` for the rows' names.
    pub column: String,
    /// Whether the order is reversed.
    pub reversed: bool,
}

/// A plugin's lister as the app keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lister {
    /// What the plugin opened.
    pub view: ListerView,
    /// The sort, once the owner chose one.
    pub sort: Option<ListerSort>,
    /// The filter text, as typed; rows whose name or columns hold it are
    /// shown.
    pub filter: Option<String>,
}

/// A row's identity: its `id` extra value, or its name.
pub fn row_id(entry: &Entry) -> String {
    entry
        .extra
        .get(ITEM_ID)
        .cloned()
        .unwrap_or_else(|| entry.name.clone())
}

impl Lister {
    /// A lister as the plugin opened it, unsorted and unfiltered.
    pub fn new(view: ListerView) -> Self {
        Self {
            view,
            sort: None,
            filter: None,
        }
    }

    /// The lister's columns.
    pub fn columns(&self) -> &[ListerColumn] {
        &self.view.spec.columns
    }

    /// The rows shown now, in order.
    pub fn shown(&self) -> Vec<&Entry> {
        let filter = self.filter.as_ref().map(|f| f.to_lowercase());
        let mut rows: Vec<&Entry> = self
            .view
            .rows
            .iter()
            .filter(|row| {
                filter.as_ref().is_none_or(|wanted| {
                    row.name.to_lowercase().contains(wanted)
                        || row
                            .extra
                            .iter()
                            .filter(|(k, _)| k.as_str() != ITEM_ID)
                            .any(|(_, v)| v.to_lowercase().contains(wanted))
                })
            })
            .collect();
        if let Some(sort) = &self.sort {
            let kind = self
                .columns()
                .iter()
                .find(|c| c.id == sort.column)
                .map_or("text", |c| c.kind.as_str());
            rows.sort_by(|a, b| {
                let order = compare(a, b, &sort.column, kind);
                if sort.reversed {
                    order.reverse()
                } else {
                    order
                }
            });
        }
        rows
    }

    /// One row as the owner reads it: the name, then each column's value
    /// with its header ("Modified, state Changed").
    pub fn row(&self, entry: &Entry) -> ListRow {
        let details: Vec<String> = self
            .columns()
            .iter()
            .filter_map(|c| entry.extra.get(&c.id).map(|v| (c, v)))
            .filter(|(_, v)| !v.is_empty())
            .map(|(c, v)| format!("{} {v}", c.header.to_lowercase()))
            .collect();
        ListRow::new(row_id(entry), entry.name.clone()).with_details(details)
    }

    /// The list for the dialog, its marks and focus kept from `old`.
    pub fn list(&self, old: Option<&ListModel>) -> ListModel {
        let rows = self
            .shown()
            .into_iter()
            .map(|entry| {
                let mut row = self.row(entry);
                row.selected =
                    old.is_some_and(|l| l.rows().iter().any(|r| r.id == row.id && r.selected));
                row
            })
            .collect();
        let mut list = ListModel::new(self.title()).with_rows(rows);
        if let Some(focus) = old.and_then(ListModel::focused) {
            list.focus_id(&focus.id);
        }
        list.set_empty_text(if self.filter.is_some() {
            "No row matches the filter"
        } else {
            "No rows"
        });
        list.set_help(
            "Enter runs the lister's command, Space marks, the sort and filter keys sort and filter by its columns, Escape closes.",
        );
        list
    }

    /// The title: the plugin's own, its name last, with the count and the
    /// sort said when there is one.
    pub fn title(&self) -> String {
        self.view.title.clone()
    }

    /// The rows chosen for a command: the marked rows of the list, or the
    /// one under the cursor.
    pub fn chosen(&self, list: &ListModel) -> Vec<Entry> {
        let marked: Vec<&str> = list.selected_ids();
        let ids: Vec<String> = if marked.is_empty() {
            list.focused()
                .map(|r| vec![r.id.clone()])
                .unwrap_or_default()
        } else {
            marked.into_iter().map(str::to_owned).collect()
        };
        self.view
            .rows
            .iter()
            .filter(|e| ids.contains(&row_id(e)))
            .cloned()
            .collect()
    }

    /// The sort choices: the name, then each column that sorts, the
    /// current one marked.
    pub fn sort_list(&self) -> ListModel {
        let mut rows = vec![ListRow::new("name", "Name")];
        rows.extend(
            self.columns()
                .iter()
                .filter(|c| c.sorts)
                .map(|c| ListRow::new(c.id.clone(), c.header.clone())),
        );
        for row in &mut rows {
            if self.sort.as_ref().is_some_and(|s| s.column == row.id) {
                row.details.push("current".to_owned());
            }
        }
        let mut list = ListModel::new("Sort the rows by").with_rows(rows);
        list.set_help(
            "Up and Down choose, Enter sorts, the same choice again reverses, Escape closes.",
        );
        list
    }

    /// Sorts by a column (`name` or a column's id); the same column again
    /// reverses. Returns the sentence: "Sorted by state", "Sorted by
    /// state, reversed".
    pub fn sort_by(&mut self, column: &str) -> String {
        let reversed = self
            .sort
            .as_ref()
            .is_some_and(|s| s.column == column && !s.reversed);
        self.sort = Some(ListerSort {
            column: column.to_owned(),
            reversed,
        });
        let header = if column == "name" {
            "name".to_owned()
        } else {
            self.columns()
                .iter()
                .find(|c| c.id == column)
                .map_or_else(|| column.to_owned(), |c| c.header.to_lowercase())
        };
        if reversed {
            format!("Sorted by {header}, reversed")
        } else {
            format!("Sorted by {header}")
        }
    }
}

fn number(text: &str) -> Option<f64> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    cleaned.parse().ok()
}

fn value<'a>(entry: &'a Entry, column: &str) -> &'a str {
    if column == "name" {
        &entry.name
    } else {
        entry.extra.get(column).map_or("", String::as_str)
    }
}

/// Two rows by a column of a kind: numbers as numbers (a value that is
/// not one sorts last), the rest in natural order.
fn compare(a: &Entry, b: &Entry, column: &str, kind: &str) -> Ordering {
    let (x, y) = (value(a, column), value(b, column));
    match kind {
        "number" | "size" => match (number(x), number(y)) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => natural_cmp(x, y),
        },
        _ => natural_cmp(x, y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::EntryKind;
    use diroxide_plugins::manifest::ListerSpec;
    use diroxide_plugins::panels::PanelChoice;

    fn item(name: &str, state: &str, lines: &str) -> Entry {
        let mut entry = Entry::new(name, EntryKind::Item);
        entry.extra.insert("id".into(), name.into());
        entry.extra.insert("state".into(), state.into());
        entry.extra.insert("lines".into(), lines.into());
        entry
    }

    pub(crate) fn changed_files() -> ListerView {
        ListerView {
            plugin_id: "changed-files".into(),
            plugin: "Changed files".into(),
            spec: ListerSpec {
                id: "changes".into(),
                name: "Changed files".into(),
                columns: vec![
                    ListerColumn {
                        id: "state".into(),
                        header: "State".into(),
                        kind: "text".into(),
                        sorts: true,
                    },
                    ListerColumn {
                        id: "lines".into(),
                        header: "Lines".into(),
                        kind: "number".into(),
                        sorts: true,
                    },
                ],
                activate: Some("diff".into()),
                actions: Vec::new(),
            },
            title: "Changed files: 3 (Changed files)".into(),
            rows: vec![
                item("b.txt", "Modified", "12"),
                item("a.txt", "Added", "3"),
                item("c.txt", "Deleted", "100"),
            ],
            panel: PanelChoice::Active,
            total: Some(3),
            truncated: false,
        }
    }

    #[test]
    fn rows_sort_and_filter_by_their_columns() {
        let mut lister = Lister::new(changed_files());
        let list = lister.list(None);
        assert_eq!(
            list.announce_focus(),
            "b.txt, state Modified, lines 12, 1 of 3"
        );
        assert_eq!(lister.sort_by("lines"), "Sorted by lines");
        let names: Vec<&str> = lister.shown().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt", "c.txt"]);
        assert_eq!(lister.sort_by("lines"), "Sorted by lines, reversed");
        assert_eq!(lister.shown()[0].name, "c.txt");
        lister.filter = Some("modif".into());
        assert_eq!(lister.shown().len(), 1);
        assert_eq!(lister.sort_list().rows().len(), 3);
    }

    #[test]
    fn marks_choose_the_rows_and_survive_a_sort() {
        let mut lister = Lister::new(changed_files());
        let mut list = lister.list(None);
        list.toggle_selected();
        assert_eq!(lister.chosen(&list).len(), 1);
        lister.sort_by("name");
        let list = lister.list(Some(&list));
        assert_eq!(list.selected_ids(), ["b.txt"]);
        let mut plain = lister.list(None);
        plain.focus_id("c.txt");
        assert_eq!(lister.chosen(&plain)[0].name, "c.txt");
    }
}
