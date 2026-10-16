//! The JSON shapes plugins and the host share, in `snake_case`: places,
//! entries, the command context, and message importance (R3's review,
//! section 2.5).
//!
//! - A place is a string in the provider's own form (`/home/ada/docs`,
//!   `D:\photos`, `course.zip!week1`); the core parses it.
//! - Times are RFC 3339 in UTC with a `Z`, up to nanoseconds.
//! - Sizes and counts are JSON integers.
//! - A field that holds a name has an optional `_raw` sibling with the
//!   operating system's bytes in base64, for names that are not valid
//!   Unicode; the receiver uses it when present and echoes it back.
//! - The host computes display names; a plugin never replaces characters.
//!
//! Phase 0: the shapes with serde derives and simple conversions to and
//! from the core types. W3a owns this module.

use std::collections::BTreeMap;

use diroxide_core::{
    Attributes, CommandId, Entry, EntryKind, Link, Message, MessageKind, Place, PlaceError,
    Timestamp, UtcOffset,
};
use serde::{Deserialize, Serialize};

use crate::protocol::{PROTOCOL, Params};

/// The most selected entries a context carries; the selection files are
/// never cut.
pub const MAX_CONTEXT_SELECTION: usize = 10_000;

/// A place as JSON: its text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WirePlace(pub String);

impl WirePlace {
    /// Reads the text as a place in the platform's path style.
    pub fn to_place(&self) -> Result<Place, PlaceError> {
        Place::parse(&self.0)
    }
}

impl From<&Place> for WirePlace {
    fn from(place: &Place) -> Self {
        Self(place.to_string())
    }
}

/// Windows attribute words and their flags.
const ATTRIBUTE_WORDS: [(&str, u32); 5] = [
    ("read_only", 0x1),
    ("hidden", 0x2),
    ("system", 0x4),
    ("archive", 0x20),
    ("reparse_point", 0x400),
];

/// An entry as JSON. Only `name` and `kind` are required; absent means
/// unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEntry {
    /// The name, as text (lossy when `name_raw` is set).
    pub name: String,
    /// The name's bytes in base64, when they are not valid Unicode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_raw: Option<String>,
    /// The full place; optional in a provider's answers, always sent by the
    /// host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<WirePlace>,
    /// `folder`, `file`, `link`, `archive`, `item` (a row that is not a
    /// file) or `special`; a kind this version does not know reads as
    /// `special`.
    pub kind: EntryKind,
    /// The size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// When the contents last changed, RFC 3339 in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// Unix permission bits, as a decimal integer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Attribute words: `read_only`, `hidden`, `system`, `archive`,
    /// `reparse_point`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<String>,
    /// The owner's user name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Where a link points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
    /// The media type, when the provider knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// An opaque version (an ETag), passed back on writes to detect
    /// conflicts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// String values for the custom-attribute column.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl WireEntry {
    /// The JSON shape of a core entry at a place.
    pub fn from_entry(entry: &Entry, place: Option<&Place>) -> Self {
        let (mode, attributes) = match entry.attributes {
            Some(Attributes::Unix(mode)) => (Some(mode & 0o7777), Vec::new()),
            Some(Attributes::Windows(flags)) => (
                None,
                ATTRIBUTE_WORDS
                    .iter()
                    .filter(|(_, bit)| flags & bit != 0)
                    .map(|(word, _)| (*word).to_owned())
                    .collect(),
            ),
            None => (None, Vec::new()),
        };
        Self {
            name: entry.name.clone(),
            name_raw: None,
            place: place.map(WirePlace::from),
            kind: entry.kind,
            size: entry.size,
            modified: entry.modified.map(format_time),
            mode,
            attributes,
            owner: entry.owner.clone(),
            link_target: entry.link.as_ref().map(|l| l.target.clone()),
            mime: None,
            revision: None,
            extra: entry.extra.clone(),
        }
    }

    /// The core entry for this shape. A `name_raw` marks the name as not
    /// valid Unicode; unknown attribute words are ignored.
    pub fn to_entry(&self) -> Entry {
        let mut entry = Entry::new(self.name.clone(), self.kind);
        entry.size = self.size;
        entry.modified = self.modified.as_deref().and_then(parse_time);
        entry.owner = self.owner.clone();
        entry.extra = self.extra.clone();
        entry.link = self.link_target.as_ref().map(|target| Link {
            target: target.clone(),
            points_to: None,
        });
        let flags = ATTRIBUTE_WORDS
            .iter()
            .filter(|(word, _)| self.attributes.iter().any(|a| a == word))
            .fold(0, |all, (_, bit)| all | bit);
        entry.attributes = match self.mode {
            Some(mode) => Some(Attributes::Unix(mode)),
            None if flags != 0 => Some(Attributes::Windows(flags)),
            None => None,
        };
        entry.hidden = flags & 0x2 != 0 || entry.name.starts_with('.');
        entry.name_not_unicode = self.name_raw.is_some();
        entry
    }
}

/// A time as RFC 3339 in UTC: "2026-10-06T14:20:11Z", with up to nine
/// digits of fraction when there is one.
pub fn format_time(time: Timestamp) -> String {
    let c = time.to_civil(UtcOffset::UTC);
    let mut text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    );
    if time.subsec_nanos() != 0 {
        let fraction = format!("{:09}", time.subsec_nanos());
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text.push('Z');
    text
}

/// Reads an RFC 3339 time in UTC (`Z` or `+00:00`); `None` for anything
/// else. Phase 0 reads UTC only.
pub fn parse_time(text: &str) -> Option<Timestamp> {
    let text = text
        .strip_suffix('Z')
        .or_else(|| text.strip_suffix("+00:00"))?;
    let (main, fraction) = match text.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (text, None),
    };
    let (date, time) = main.split_once('T')?;
    let mut date = date.split('-');
    let year: i32 = date.next()?.parse().ok()?;
    let month: u8 = date.next()?.parse().ok()?;
    let day: u8 = date.next()?.parse().ok()?;
    let mut time = time.split(':');
    let hour: u8 = time.next()?.parse().ok()?;
    let minute: u8 = time.next()?.parse().ok()?;
    let second: u8 = time.next()?.parse().ok()?;
    let base = Timestamp::from_civil(year, month, day, hour, minute, second, UtcOffset::UTC)?;
    let nanos = match fraction {
        Some(digits)
            if !digits.is_empty()
                && digits.len() <= 9
                && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            format!("{digits:0<9}").parse().ok()?
        }
        Some(_) => return None,
        None => 0,
    };
    Some(Timestamp::from_unix(base.unix_seconds(), nanos))
}

/// A panel as the context names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelRef {
    /// The panel's place.
    pub place: WirePlace,
    /// The provider serving it: `local`, `archive`, `sftp`, or a plugin's
    /// scheme.
    pub provider: String,
}

impl PanelRef {
    /// The reference for a place, naming its provider by the place's kind.
    pub fn for_place(place: &Place) -> Self {
        let provider = match place.remote() {
            Some(remote) => remote.scheme().to_owned(),
            None if place.archive().is_some() => "archive".to_owned(),
            None => "local".to_owned(),
        };
        Self {
            place: place.into(),
            provider,
        }
    }
}

/// What a command receives; script commands get the same object on
/// standard input. The effective selection falls back to the cursor's
/// entry when nothing is marked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireContext {
    /// The protocol's major version.
    pub protocol: u32,
    /// The command's id.
    pub command: String,
    /// The command's arguments.
    #[serde(default)]
    pub args: Params,
    /// The user's locale, such as `en-US`.
    pub locale: String,
    /// The active panel.
    pub active: PanelRef,
    /// The other panel, the destination by the orthodox rule.
    pub other: PanelRef,
    /// The entry under the cursor.
    #[serde(default)]
    pub cursor: Option<WireEntry>,
    /// The selected entries, at most [`MAX_CONTEXT_SELECTION`].
    #[serde(default)]
    pub selection: Vec<WireEntry>,
    /// How many entries are selected in all.
    pub selection_count: u64,
    /// Whether the selection is the cursor's entry because nothing was
    /// marked.
    pub selection_from_cursor: bool,
    /// Whether `selection` was cut at the limit.
    pub selection_truncated: bool,
}

impl WireContext {
    /// A context for a command between two panels, with nothing selected.
    pub fn new(command: &CommandId, active: PanelRef, other: PanelRef) -> Self {
        Self {
            protocol: PROTOCOL,
            command: command.to_string(),
            args: Params::new(),
            locale: "en-US".to_owned(),
            active,
            other,
            cursor: None,
            selection: Vec::new(),
            selection_count: 0,
            selection_from_cursor: false,
            selection_truncated: false,
        }
    }

    /// The same context with a cursor and a selection; an empty selection
    /// falls back to the cursor, and a long one is cut at the limit.
    pub fn with_selection(
        mut self,
        cursor: Option<WireEntry>,
        mut selected: Vec<WireEntry>,
    ) -> Self {
        if selected.is_empty()
            && let Some(entry) = &cursor
        {
            selected.push(entry.clone());
            self.selection_from_cursor = true;
        }
        self.selection_count = selected.len() as u64;
        self.selection_truncated = selected.len() > MAX_CONTEXT_SELECTION;
        selected.truncate(MAX_CONTEXT_SELECTION);
        self.cursor = cursor;
        self.selection = selected;
        self
    }
}

/// How a plugin's message is said: `status`, `info`, `warning` or `error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireImportance {
    /// Shown, said on Say Status (the core's low importance).
    Status,
    /// Said politely (normal importance).
    Info,
    /// Said at once, as a warning (high importance).
    Warning,
    /// Said at once, as an error (high importance).
    Error,
}

impl WireImportance {
    /// The core message for a plugin's text at this importance.
    pub fn message(self, text: impl Into<String>) -> Message {
        match self {
            WireImportance::Status => Message::status(text),
            WireImportance::Info => Message::info(text),
            WireImportance::Warning => Message::warning(text),
            WireImportance::Error => Message::error(text),
        }
    }

    /// The importance a core message travels with.
    pub fn of(message: &Message) -> Self {
        match (message.importance, message.kind) {
            (diroxide_core::Importance::Low, _) => WireImportance::Status,
            (_, MessageKind::Error) => WireImportance::Error,
            (_, MessageKind::Warning) => WireImportance::Warning,
            _ => WireImportance::Info,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;
    use serde_json::json;

    fn p(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    #[test]
    fn times_are_rfc_3339_in_utc() {
        let t = Timestamp::from_civil(2026, 10, 6, 14, 20, 11, UtcOffset::UTC).unwrap();
        assert_eq!(format_time(t), "2026-10-06T14:20:11Z");
        assert_eq!(parse_time("2026-10-06T14:20:11Z"), Some(t));
        let fine = Timestamp::from_unix(t.unix_seconds(), 500_000_000);
        assert_eq!(format_time(fine), "2026-10-06T14:20:11.5Z");
        assert_eq!(parse_time("2026-10-06T14:20:11.5+00:00"), Some(fine));
        assert_eq!(parse_time("2026-10-06T14:20:11-07:00"), None);
        assert_eq!(parse_time("2026-02-30T00:00:00Z"), None);
    }

    #[test]
    fn an_entry_reads_as_the_review_shows_it() {
        let shape: WireEntry = serde_json::from_value(json!({
            "name": "report.pdf",
            "place": "/home/ada/docs/report.pdf",
            "kind": "file",
            "size": 2204160,
            "modified": "2026-10-06T14:20:11Z",
            "mode": 420,
            "attributes": ["read_only"],
            "owner": "ada",
            "link_target": null,
            "mime": "application/pdf",
            "revision": "v12",
            "extra": {"label": "draft"},
            "something_new": 1
        }))
        .unwrap();
        let entry = shape.to_entry();
        assert_eq!(entry.size, Some(2_204_160));
        assert_eq!(entry.attributes, Some(Attributes::Unix(420)));
        assert_eq!(entry.owner.as_deref(), Some("ada"));
        let back = WireEntry::from_entry(&entry, Some(&p("/home/ada/docs/report.pdf")));
        assert_eq!(back.modified.as_deref(), Some("2026-10-06T14:20:11Z"));
        assert_eq!(back.mode, Some(420));
        let minimal: WireEntry =
            serde_json::from_value(json!({"name": "x", "kind": "folder"})).unwrap();
        assert_eq!(
            serde_json::to_value(&minimal).unwrap(),
            json!({"name": "x", "kind": "folder"})
        );
    }

    #[test]
    fn windows_attributes_travel_as_words() {
        let mut entry = Entry::new("a.txt", EntryKind::File);
        entry.attributes = Some(Attributes::Windows(0x3));
        let shape = WireEntry::from_entry(&entry, None);
        assert_eq!(shape.attributes, vec!["read_only", "hidden"]);
        let back = shape.to_entry();
        assert!(back.hidden);
        assert_eq!(back.attributes, Some(Attributes::Windows(0x3)));
    }

    #[test]
    fn the_context_falls_back_to_the_cursor() {
        let cursor = WireEntry::from_entry(
            &Entry::new("a.txt", EntryKind::File),
            Some(&p("/home/ada/docs/a.txt")),
        );
        let context = WireContext::new(
            &CommandId::from_static("line-counter.count_lines"),
            PanelRef::for_place(&p("/home/ada/docs")),
            PanelRef::for_place(&p("/home/ada/out")),
        )
        .with_selection(Some(cursor), Vec::new());
        let json = serde_json::to_value(&context).unwrap();
        assert_eq!(json["protocol"], 1);
        assert_eq!(
            json["active"],
            json!({"place": "/home/ada/docs", "provider": "local"})
        );
        assert_eq!(json["selection_count"], 1);
        assert_eq!(json["selection_from_cursor"], true);
        assert_eq!(json["selection_truncated"], false);
        let back: WireContext = serde_json::from_value(json).unwrap();
        assert_eq!(back, context);
        let remote = PanelRef::for_place(&p("sftp://example.org/x"));
        assert_eq!(remote.provider, "sftp");
    }

    #[test]
    fn importance_maps_both_ways() {
        let m = WireImportance::Warning.message("Disk nearly full");
        assert_eq!(WireImportance::of(&m), WireImportance::Warning);
        assert_eq!(
            WireImportance::of(&Message::status("x")),
            WireImportance::Status
        );
        assert_eq!(
            WireImportance::of(&Message::could_not("copy", "a", "b", "")),
            WireImportance::Error
        );
        assert_eq!(
            serde_json::to_value(WireImportance::Status).unwrap(),
            json!("status")
        );
    }
}
