//! Hits: what a search found, and the rows a results list shows for them.
//! Each row says the name first, then what the hit is, then where:
//! "notes.txt, in Documents/q3, 4 KB", "report.txt, line 42: total due,
//! 3 more, in Documents/q3", "q3, folder, in Documents". The list model
//! adds the position last.

use std::ops::Range;

use diroxide_core::format::human_size;
use diroxide_core::names::display_name;
use diroxide_core::{Entry, EntryKind, ListRow, Place};

/// A matching line in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentMatch {
    /// The line number, from one.
    pub line_number: u64,
    /// The line's text, trimmed, with tabs as spaces and hidden
    /// characters as question marks; a long line is cut to a stretch
    /// around the match.
    pub text: String,
    /// Where the first match is in `text`, in bytes.
    pub matched: Range<usize>,
    /// Where the first match starts in the whole line, in characters from
    /// one: what the viewer needs to open at the match.
    pub column: u64,
}

impl ContentMatch {
    /// A line with no match position known.
    pub fn new(line_number: u64, text: impl Into<String>) -> Self {
        Self {
            line_number,
            text: text.into(),
            matched: 0..0,
            column: 1,
        }
    }

    /// The line in words, for a row: "line 42: total due".
    pub fn words(&self) -> String {
        if self.text.is_empty() {
            format!("line {}", self.line_number)
        } else {
            format!("line {}: {}", self.line_number, self.text)
        }
    }
}

/// One result.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchHit {
    /// Where it is.
    pub place: Place,
    /// What it is.
    pub entry: Entry,
    /// The first matching line, for a content search; in one-row-per-line
    /// results, this row's line.
    pub line: Option<ContentMatch>,
    /// How many other lines of the file match (one row per file).
    pub more: u64,
    /// How many folders below the start it is: direct children are 1.
    pub depth: u32,
}

impl SearchHit {
    /// A hit by name: no line, depth unknown (0).
    pub fn new(place: Place, entry: Entry) -> Self {
        Self {
            place,
            entry,
            line: None,
            more: 0,
            depth: 0,
        }
    }

    /// The same hit with a matching line and the count of the others.
    pub fn with_line(mut self, line: ContentMatch, more: u64) -> Self {
        self.line = Some(line);
        self.more = more;
        self
    }

    /// The same hit at a depth.
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// A stable identity for the row: the place, and the line number in
    /// one-row-per-line results, so two rows of one file differ.
    pub fn id(&self) -> String {
        match &self.line {
            Some(line) => format!("{}#{}", self.place, line.line_number),
            None => self.place.to_string(),
        }
    }

    /// Where the hit is, from the search's start: "in Documents/q3" for a
    /// file two levels down in a search of Documents, "in Documents" for
    /// a direct child. With no start known, the whole folder: "in
    /// D:\docs\q3".
    pub fn folder_words(&self, root: Option<&Place>) -> String {
        let Some(parent) = self.place.parent() else {
            return String::new();
        };
        let Some(root) = root.filter(|root| parent.is_within(root)) else {
            return format!("in {}", display_name(&parent.to_string()));
        };
        let mut names = Vec::new();
        let mut current = Some(parent);
        while let Some(place) = current {
            if place == *root {
                names.push(top_name(&place));
                break;
            }
            names.push(place.name().unwrap_or_default().to_owned());
            current = place.parent();
        }
        names.reverse();
        let mut text = String::from("in ");
        for (index, name) in names.iter().enumerate() {
            if index > 0 && !text.ends_with(['/', '\\']) {
                text.push('/');
            }
            text.push_str(&display_name(name));
        }
        text
    }

    /// The row's details after the name, in the order said.
    pub fn details(&self, root: Option<&Place>) -> Vec<String> {
        let mut details = Vec::new();
        match &self.line {
            Some(line) => {
                details.push(line.words());
                if self.more > 0 {
                    details.push(format!("{} more", self.more));
                }
                details.push(self.folder_words(root));
            }
            None => {
                let kind = self.entry.kind;
                if kind != EntryKind::File {
                    details.push(kind.word().to_owned());
                }
                details.push(self.folder_words(root));
                if matches!(kind, EntryKind::File | EntryKind::Archive)
                    && let Some(size) = self.entry.size
                {
                    details.push(human_size(size));
                }
            }
        }
        details.retain(|d| !d.is_empty());
        details
    }

    /// The row a results list shows: the name, then what the hit is, then
    /// where.
    pub fn row(&self, root: Option<&Place>) -> ListRow {
        ListRow::new(self.id(), self.entry.display_name()).with_details(self.details(root))
    }
}

/// A start's name for "in ...": its own name, or the drive or share
/// itself at a top.
fn top_name(place: &Place) -> String {
    match place.name() {
        Some(name) => name.to_owned(),
        None => place.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(text: &str) -> Place {
        Place::parse_with(text, diroxide_core::place::PathStyle::Windows).unwrap()
    }

    #[test]
    fn rows_say_the_name_then_what_then_where() {
        let root = place(r"D:\Users\ada\Documents");
        let file = SearchHit::new(
            place(r"D:\Users\ada\Documents\q3\notes.txt"),
            Entry::new("notes.txt", EntryKind::File).with_size(4_000),
        );
        assert_eq!(
            file.row(Some(&root)).spoken(),
            "notes.txt, in Documents/q3, 4.0 KB"
        );
        let folder = SearchHit::new(
            place(r"D:\Users\ada\Documents\q3"),
            Entry::new("q3", EntryKind::Folder),
        );
        assert_eq!(folder.row(Some(&root)).spoken(), "q3, folder, in Documents");
        let content = SearchHit::new(
            place(r"D:\Users\ada\Documents\q3\report.txt"),
            Entry::new("report.txt", EntryKind::File).with_size(900),
        )
        .with_line(ContentMatch::new(42, "total due"), 3);
        assert_eq!(
            content.row(Some(&root)).spoken(),
            "report.txt, line 42: total due, 3 more, in Documents/q3"
        );
        assert_eq!(content.id(), r"D:\Users\ada\Documents\q3\report.txt#42");
        assert_eq!(file.folder_words(None), r"in D:\Users\ada\Documents\q3");
        let drive = place(r"D:\");
        let top = SearchHit::new(place(r"D:\a\b.txt"), Entry::new("b.txt", EntryKind::File));
        assert_eq!(top.folder_words(Some(&drive)), r"in D:\a");
    }
}
