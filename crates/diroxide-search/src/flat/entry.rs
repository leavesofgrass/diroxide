//! A flat view's rows are search hits: the results lister shows them
//! with "in Documents/q3" after the name. This module gives each one its
//! key, the path below the base folder, because names repeat in a flat
//! view and a row's key must not.

use diroxide_core::names::display_name;
use diroxide_core::{ListModel, ListRow, Place};

use crate::SearchHit;

/// The folder of `place` below `root`, the names joined with `/`, each
/// said as the panel says names; empty when the place is directly in
/// `root`, and the parent's full place when it is not below `root`.
pub fn relative_folder(place: &Place, root: &Place) -> String {
    let Some(parent) = place.parent() else {
        return String::new();
    };
    if parent == *root {
        return String::new();
    }
    if !parent.is_within(root) {
        return display_name(&parent.to_string()).into_owned();
    }
    let mut names = Vec::new();
    let mut current = Some(parent);
    while let Some(folder) = current {
        if folder == *root {
            break;
        }
        names.push(display_name(folder.name().unwrap_or_default()).into_owned());
        current = folder.parent();
    }
    names.reverse();
    names.join("/")
}

/// A hit's key in a flat view: its path below the base folder, folder
/// and name joined with `/` ("2026/q3/report.pdf"). Two files with one
/// name in two folders have two keys. The quick filter matches this
/// text, so a folder's name finds the files in it.
pub fn flat_key(hit: &SearchHit, root: &Place) -> String {
    let folder = relative_folder(&hit.place, root);
    let name = display_name(&hit.entry.name);
    if folder.is_empty() {
        name.into_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// A hit's row in a flat view: the results lister's row under `root`
/// ("report.pdf, in work/2026/q3, 2.1 KB"), keyed by [`flat_key`].
pub fn flat_row(hit: &SearchHit, root: &Place) -> ListRow {
    ListRow::new(flat_key(hit, root), hit.entry.display_name())
        .with_details(hit.details(Some(root)))
}

/// The hits as a list titled "Flat view of Photos", each row keyed by
/// its path below the base folder.
pub fn flat_list(root: &Place, hits: &[SearchHit]) -> ListModel {
    let mut list = ListModel::new(format!("Flat view of {}", super::root_name(root)))
        .with_rows(hits.iter().map(|hit| flat_row(hit, root)).collect());
    list.set_empty_text("No files below this folder");
    list
}
