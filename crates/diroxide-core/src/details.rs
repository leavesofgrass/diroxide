//! Detail rows: one fact about an entry, a label and a value in words,
//! grouped in a fixed order of facts (research/metadata.md). Alpha.2's
//! item information list, alpha.7's details view and a plugin's preview
//! details all use these rows, so every details list reads the same way.

use serde::{Deserialize, Serialize};

/// Where a fact belongs. The order of the variants is the order of the
/// groups in every details list.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetailGroup {
    /// What it is: name, kind, file type.
    Identity,
    /// What is in it: dimensions, duration, pages, title.
    Content,
    /// How big it is.
    Size,
    /// Its times.
    Dates,
    /// Its attributes, permissions and flags.
    Flags,
    /// The user's own: labels, rating, comments.
    Yours,
    /// Where it is: folder, volume, link target.
    Place,
    /// Technical facts: codec, bit depth, encoding.
    Technical,
    /// What the system keeps: inode, hard links, extended attributes.
    System,
    /// Where it came from: camera, program, download address.
    Source,
}

impl DetailGroup {
    /// The group's heading, as said: "Identity", "Dates".
    pub const fn heading(self) -> &'static str {
        match self {
            DetailGroup::Identity => "Identity",
            DetailGroup::Content => "Content",
            DetailGroup::Size => "Size",
            DetailGroup::Dates => "Dates",
            DetailGroup::Flags => "Flags",
            DetailGroup::Yours => "Yours",
            DetailGroup::Place => "Place",
            DetailGroup::Technical => "Technical",
            DetailGroup::System => "System",
            DetailGroup::Source => "Source",
        }
    }
}

/// One fact: "Dimensions: 4032 by 3024".
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DetailRow {
    /// A stable key, such as `image.width`, for the column and the filter
    /// languages.
    pub key: String,
    /// The label, said first: "Dimensions".
    pub label: String,
    /// The value in words: "4032 by 3024".
    pub value: String,
    /// The group it is listed under.
    pub group: DetailGroup,
    /// Whether the value is personal (a location, an owner's name, a
    /// camera's serial number): shown, never sent anywhere, and offered
    /// for stripping.
    #[serde(default)]
    pub private: bool,
}

impl DetailRow {
    /// A fact that is not private.
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

    /// The same fact, marked private.
    pub fn private(mut self) -> Self {
        self.private = true;
        self
    }

    /// The row as said: "Dimensions: 4032 by 3024".
    pub fn spoken(&self) -> String {
        format!("{}: {}", self.label, self.value)
    }
}

/// Sorts rows into the groups' order, keeping each group's own order.
pub fn order_rows(rows: &mut [DetailRow]) {
    rows.sort_by_key(|row| row.group);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_said_label_first_in_group_order() {
        let mut rows = vec![
            DetailRow::new(
                "time.modified",
                "Modified",
                "today, 2:05 PM",
                DetailGroup::Dates,
            ),
            DetailRow::new(
                "image.size",
                "Dimensions",
                "4032 by 3024",
                DetailGroup::Content,
            ),
            DetailRow::new("gps", "Location", "45.5, -122.7", DetailGroup::Source).private(),
        ];
        order_rows(&mut rows);
        assert_eq!(rows[0].spoken(), "Dimensions: 4032 by 3024");
        assert_eq!(rows[1].group.heading(), "Dates");
        assert!(rows[2].private);
    }
}
