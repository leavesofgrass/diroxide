//! Fields and cells: one fact a field, its value in words, and what a
//! column says when it has no value.

use diroxide_core::details::{DetailGroup, DetailRow};

/// One fact read from a file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Field {
    /// A plain, stable key: `image.width`, `audio.artist`, `doc.pages`,
    /// `file.sha256`, `custom.project`.
    pub key: String,
    /// The label as said: "Dimensions".
    pub label: String,
    /// The value in words: "4032 by 3024".
    pub value: String,
    /// Where it sits in the order of facts.
    pub group: DetailGroup,
    /// Whether it is private (a location, a camera serial): left out of
    /// Copy details unless asked, and named by Check before sharing.
    pub private: bool,
    /// The value as a number for sorting, in the field's own unit
    /// (milliseconds for a length, bytes for a size, a plain count for
    /// pages and lines), so "2 min" sorts after "45 sec" and "1.2 GB"
    /// after "300 MB". `None` sorts by the words.
    pub sort: Option<i64>,
}

impl Field {
    /// A field that is not private.
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        group: DetailGroup,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
            group,
            private: false,
            sort: None,
        }
    }

    /// The same field, private.
    pub fn private(mut self) -> Self {
        self.private = true;
        self
    }

    /// The same field with a number to sort by, in its own unit.
    pub fn sorted_by(mut self, number: i64) -> Self {
        self.sort = Some(number);
        self
    }

    /// The same field sorting by a count or size that may not fit an
    /// `i64`: the largest `i64` then, which still sorts last.
    pub fn sorted_by_count(self, number: u64) -> Self {
        self.sorted_by(i64::try_from(number).unwrap_or(i64::MAX))
    }

    /// The same field sorting by a length of time, in milliseconds.
    pub fn sorted_by_length(self, length: std::time::Duration) -> Self {
        self.sorted_by(i64::try_from(length.as_millis()).unwrap_or(i64::MAX))
    }

    /// A key that sorts as plain text in the field's true order, for a
    /// caller that keeps cells as strings: a number becomes `n` and 20
    /// digits (offset so a negative sorts first), words become `t` and
    /// the value in small letters, so every number sorts before words.
    pub fn sort_key(&self) -> String {
        match self.sort {
            Some(number) => {
                let offset = i128::from(number) - i128::from(i64::MIN);
                format!("n{offset:020}")
            }
            None => format!("t{}", self.value.to_lowercase()),
        }
    }

    /// Compares two fields' values in their true order: by number when
    /// both have one, a number before words, else by the words with
    /// small and capital letters alike.
    pub fn compare(&self, other: &Field) -> std::cmp::Ordering {
        match (self.sort, other.sort) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => self
                .value
                .to_lowercase()
                .cmp(&other.value.to_lowercase())
                .then_with(|| self.value.cmp(&other.value)),
        }
    }

    /// The field as a row of the details view.
    pub fn row(&self) -> DetailRow {
        let row = DetailRow::new(&self.key, &self.label, &self.value, self.group);
        if self.private { row.private() } else { row }
    }

    /// The field as said, the label first: "Dimensions: 4032 by 3024".
    pub fn line(&self) -> String {
        format!("{}: {}", self.label, self.value)
    }
}

/// What a column's cell says: a value, or why there is none. Never blank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    /// The value in words.
    Value(String),
    /// Being read now.
    Reading,
    /// This file has none.
    NotAvailable,
    /// On demand, and not asked for yet.
    NotComputed,
    /// It could not be read, with why.
    Unreadable(String),
    /// What gives it cannot run now, with why: a plugin column whose
    /// plugin stopped says "unavailable: plugin stopped".
    Unavailable(String),
}

impl Cell {
    /// The cell's words: the value, "reading", "not available", "not
    /// computed", "unreadable: REASON" or "unavailable: REASON". A value
    /// that is empty says "not available", so a cell is never blank.
    pub fn words(&self) -> String {
        match self {
            Cell::Value(value) if value.trim().is_empty() => "not available".to_owned(),
            Cell::Value(value) => value.clone(),
            Cell::Reading => "reading".to_owned(),
            Cell::NotAvailable => "not available".to_owned(),
            Cell::NotComputed => "not computed".to_owned(),
            Cell::Unreadable(why) => format!("unreadable: {why}"),
            Cell::Unavailable(why) => format!("unavailable: {why}"),
        }
    }

    /// The cell of a reader that failed: "unreadable: REASON"; a reader
    /// refused because the guard was busy is still "reading", since the
    /// caller asks again.
    pub fn from_error(error: &crate::MetaError) -> Self {
        match error {
            crate::MetaError::Busy => Cell::Reading,
            _ => Cell::Unreadable(error.to_string()),
        }
    }

    /// The cell of a plugin column whose plugin stopped.
    pub fn plugin_stopped() -> Self {
        Cell::Unavailable("plugin stopped".to_owned())
    }
}

impl Field {
    /// The same field with its label and value cleaned like a file name
    /// ([`crate::clean::clean_text`]) and its key made plain when it is
    /// not; an empty value says "none".
    pub fn cleaned(mut self) -> Self {
        if !crate::clean::is_plain_key(&self.key) {
            self.key = self
                .key
                .split('.')
                .map(crate::clean::plain_key)
                .collect::<Vec<_>>()
                .join(".");
        }
        self.label = crate::clean::clean_text(&self.label);
        self.value = crate::clean::clean_text(&self.value);
        if self.value.is_empty() {
            self.value = "none".to_owned();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_reads_label_first_and_a_cell_is_never_blank() {
        let field = Field::new(
            "image.width",
            "Dimensions",
            "4032 by 3024",
            DetailGroup::Content,
        );
        assert_eq!(field.line(), "Dimensions: 4032 by 3024");
        assert!(!field.row().private);
        let place = Field::new(
            "image.gps",
            "Location",
            "45.50, -122.68",
            DetailGroup::Source,
        )
        .private();
        assert!(place.row().private);
        for cell in [
            Cell::Value("Ada Example".to_owned()),
            Cell::Reading,
            Cell::NotAvailable,
            Cell::NotComputed,
            Cell::Unreadable("not a PDF".to_owned()),
        ] {
            assert!(!cell.words().is_empty());
            assert!(cell.words().chars().count() <= 40, "{cell:?}");
        }
        assert_eq!(
            Cell::Unreadable("too large".to_owned()).words(),
            "unreadable: too large"
        );
    }

    #[test]
    fn the_cells_words_fit_a_line_with_the_state_first() {
        let lines = [
            Cell::plugin_stopped().words(),
            Cell::from_error(&crate::MetaError::ReaderFailed).words(),
            Cell::from_error(&crate::MetaError::TooSlow).words(),
            Cell::from_error(&crate::MetaError::TooLarge).words(),
            Cell::from_error(&crate::MetaError::Busy).words(),
            Cell::Value("  ".to_owned()).words(),
        ];
        assert_eq!(
            lines,
            [
                "unavailable: plugin stopped",
                "unreadable: the reader failed",
                "unreadable: it took too long to read",
                "unreadable: it is too large to read here",
                "reading",
                "not available",
            ]
        );
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn lengths_and_sizes_sort_as_numbers_not_words() {
        use std::time::Duration;
        let length = |secs: u64, words: &str| {
            Field::new("audio.length", "Length", words, DetailGroup::Content)
                .sorted_by_length(Duration::from_secs(secs))
        };
        let mut fields = [
            length(120, "2 min"),
            length(45, "45 sec"),
            length(3720, "1 hr 2 min"),
            Field::new("audio.length", "Length", "odd", DetailGroup::Content),
            length(0, "under 1 sec"),
        ];
        fields.sort_by(Field::compare);
        let said: Vec<&str> = fields.iter().map(|f| f.value.as_str()).collect();
        assert_eq!(said, ["under 1 sec", "45 sec", "2 min", "1 hr 2 min", "odd"]);
        let mut keys: Vec<String> = fields.iter().map(Field::sort_key).collect();
        let before = keys.clone();
        keys.sort();
        assert_eq!(keys, before, "the text keys keep the same order");
        let low = Field::new("x.n", "N", "-5", DetailGroup::Content).sorted_by(-5);
        let high = Field::new("x.n", "N", "7", DetailGroup::Content).sorted_by(7);
        assert!(low.sort_key() < high.sort_key());
        let huge = Field::new("x.n", "N", "big", DetailGroup::Content).sorted_by_count(u64::MAX);
        assert_eq!(huge.sort, Some(i64::MAX));
        assert!(high.sort_key() < huge.sort_key());
        assert_eq!(
            Field::new("x.n", "N", "Words", DetailGroup::Content).sort_key(),
            "twords"
        );
    }

    #[test]
    fn a_cleaned_field_is_one_plain_line() {
        let field = Field::new(
            "ExifTool.XMP-dc.Creator",
            "Creator\n",
            "Ada\u{202E} Example\r\n",
            DetailGroup::Source,
        )
        .cleaned();
        assert_eq!(field.key, "exiftool.xmp-dc.creator");
        assert_eq!(field.line(), "Creator: Ada? Example");
        let empty = Field::new("doc.title", "Title", "\0\0", DetailGroup::Content).cleaned();
        assert_eq!(empty.value, "none");
    }
}
