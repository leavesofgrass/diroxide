//! One row of a flat view: the entry, where it is below the base folder,
//! and the words a panel says for it, the name first.

use diroxide_core::format::human_size;
use diroxide_core::names::display_name;
use diroxide_core::{Entry, EntryKind, ListRow, Place};

use crate::SearchHit;

/// The words for an entry directly in the base folder.
pub const AT_THE_TOP: &str = "at the top";

/// One entry of a flat view.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlatEntry {
    /// Where it is, in full.
    pub place: Place,
    /// What it is, with its size and times.
    pub entry: Entry,
    /// Its folder below the base folder, the names joined with `/`
    /// ("2026/q3"); empty for an entry directly in the base folder.
    pub folder: String,
    /// Folders below the base folder: direct children are 1.
    pub depth: u32,
}

impl FlatEntry {
    /// An entry at a place below `root`, its folder said from there.
    pub fn new(place: Place, entry: Entry, root: &Place, depth: u32) -> Self {
        let folder = relative_folder(&place, root);
        Self {
            place,
            entry,
            folder,
            depth,
        }
    }

    /// A search hit below `root`, as a flat view's entry.
    pub fn from_hit(hit: SearchHit, root: &Place) -> Self {
        Self::new(hit.place, hit.entry, root, hit.depth)
    }

    /// A stable identity for the row: the place.
    pub fn id(&self) -> String {
        self.place.to_string()
    }

    /// Whether this row is a folder.
    pub fn is_folder(&self) -> bool {
        self.entry.kind == EntryKind::Folder
    }

    /// The path below the base folder, folder and name: "2026/q3/report.pdf".
    /// The lister's quick filter matches this text, so a folder's name
    /// finds the files in it.
    pub fn relative_path(&self) -> String {
        if self.folder.is_empty() {
            self.entry.name.clone()
        } else {
            format!("{}/{}", self.folder, self.entry.name)
        }
    }

    /// Where it is, in words: "in 2026/q3", or "at the top" for an entry
    /// directly in the base folder.
    pub fn folder_words(&self) -> String {
        if self.folder.is_empty() {
            AT_THE_TOP.to_owned()
        } else {
            format!("in {}", self.folder)
        }
    }

    /// The row's details after the name, in the order said: what it is
    /// when it is not a file, where it is, then its size.
    pub fn details(&self) -> Vec<String> {
        let mut details = Vec::new();
        let kind = self.entry.kind;
        if kind != EntryKind::File {
            details.push(kind.word().to_owned());
        }
        details.push(self.folder_words());
        if matches!(kind, EntryKind::File | EntryKind::Archive)
            && let Some(size) = self.entry.size
        {
            details.push(human_size(size));
        }
        details
    }

    /// The row a flat lister shows: "report.pdf, in 2026/q3, 2.1 MB".
    pub fn row(&self) -> ListRow {
        ListRow::new(self.id(), self.entry.display_name()).with_details(self.details())
    }
}

/// The folder of `place` below `root`, the names joined with `/`, each
/// said as the panel says names (hidden characters shown); empty when the
/// place is directly in `root`, and the parent's full place when it is
/// not below `root` at all.
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
