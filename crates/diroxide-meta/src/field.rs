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
        }
    }

    /// The same field, private.
    pub fn private(mut self) -> Self {
        self.private = true;
        self
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
}

impl Cell {
    /// The cell's words: the value, "reading", "not available", "not
    /// computed", or "unreadable: REASON".
    pub fn words(&self) -> String {
        match self {
            Cell::Value(value) => value.clone(),
            Cell::Reading => "reading".to_owned(),
            Cell::NotAvailable => "not available".to_owned(),
            Cell::NotComputed => "not computed".to_owned(),
            Cell::Unreadable(why) => format!("unreadable: {why}"),
        }
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
}
