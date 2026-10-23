//! Host services, part three: the panels (`panel/get`, `panel/set`) and
//! diroxide's commands (`command/execute`), each behind the manifest's
//! permissions (R3, section 3.6).
//!
//! - `panel/get` needs `permissions.panel = "read"` or `"write"`; it gives
//!   a panel's place, cursor, selection, sort and filter, with entries in
//!   the shared shape.
//! - `panel/set` needs `"write"`; it changes a panel through ordinary app
//!   commands, so the app says its own position message and the plugin
//!   need not announce navigation. Every place is parsed, never trusted.
//! - `command/execute` runs a diroxide command by id when an id pattern
//!   of `permissions.commands` matches it.
//!
//! A call the manifest does not allow is answered -32030; the first one
//! in a session is also said, in two lines ("Line counter may not do
//! that." and "Its manifest does not allow it."), and each is counted.

use std::sync::atomic::{AtomicUsize, Ordering};

use diroxide_core::{CommandId, Entry, Place, Side};
use serde_json::{Map, Value, json};

use crate::places::{plugin_place, plugin_places};
use crate::protocol::{ErrorKind, Params, RpcError};
use crate::shapes::{MAX_CONTEXT_SELECTION, PanelRef, WireEntry};

/// Which panel a call is about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PanelChoice {
    /// The panel with the focus.
    #[default]
    Active,
    /// The other one, the destination by the orthodox rule.
    Other,
    /// The left panel.
    Left,
    /// The right panel.
    Right,
}

impl PanelChoice {
    /// The word on the wire.
    pub const fn word(self) -> &'static str {
        match self {
            PanelChoice::Active => "active",
            PanelChoice::Other => "other",
            PanelChoice::Left => "left",
            PanelChoice::Right => "right",
        }
    }

    /// Reads the word; absent means the active panel.
    pub fn read(params: &Params) -> Result<Self, RpcError> {
        Ok(match params.get("panel").and_then(Value::as_str) {
            None | Some("active") => PanelChoice::Active,
            Some("other") => PanelChoice::Other,
            Some("left") => PanelChoice::Left,
            Some("right") => PanelChoice::Right,
            Some(other) => {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    format!("panel {other} is not active, other, left or right"),
                ));
            }
        })
    }
}

/// A panel as a plugin sees it: entries are named in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelSnapshot {
    /// Which panel.
    pub side: Side,
    /// Whether it has the focus.
    pub active: bool,
    /// Its place.
    pub place: Place,
    /// The entry under the cursor.
    pub cursor: Option<Entry>,
    /// The marked entries.
    pub selection: Vec<Entry>,
    /// Its sort, in words: "name, folders first".
    pub sort: Option<String>,
    /// Its filter, as typed.
    pub filter: Option<String>,
}

impl PanelSnapshot {
    /// A panel at a place with nothing under the cursor or marked.
    pub fn at(side: Side, place: Place) -> Self {
        Self {
            side,
            active: true,
            place,
            cursor: None,
            selection: Vec::new(),
            sort: None,
            filter: None,
        }
    }

    fn wire_entry(&self, entry: &Entry) -> WireEntry {
        let place = self.place.join(&entry.name).ok();
        WireEntry::from_entry(entry, place.as_ref())
    }

    /// The `panel/get` answer, with only `fields` when the plugin named
    /// some (the panel's name always).
    pub fn to_value(&self, fields: Option<&[String]>) -> Value {
        let mut out = Map::new();
        out.insert(
            "panel".into(),
            json!(match self.side {
                Side::Left => "left",
                Side::Right => "right",
            }),
        );
        let wanted = |key: &str| fields.is_none_or(|f| f.iter().any(|k| k == key));
        out.insert("active".into(), json!(self.active));
        if wanted("place") {
            let reference = PanelRef::for_place(&self.place);
            out.insert("place".into(), json!(reference.place));
            out.insert("provider".into(), json!(reference.provider));
        }
        if wanted("cursor") {
            out.insert(
                "cursor".into(),
                self.cursor
                    .as_ref()
                    .map_or(Value::Null, |e| json!(self.wire_entry(e))),
            );
        }
        if wanted("selection") {
            let selection: Vec<WireEntry> = self
                .selection
                .iter()
                .take(MAX_CONTEXT_SELECTION)
                .map(|e| self.wire_entry(e))
                .collect();
            out.insert("selection".into(), json!(selection));
            out.insert("selection_count".into(), json!(self.selection.len()));
            out.insert(
                "selection_truncated".into(),
                json!(self.selection.len() > MAX_CONTEXT_SELECTION),
            );
        }
        if wanted("sort") {
            out.insert("sort".into(), json!(self.sort));
        }
        if wanted("filter") {
            out.insert("filter".into(), json!(self.filter));
        }
        Value::Object(out)
    }
}

/// A change to a panel a plugin asks for, performed as ordinary commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelChange {
    /// Which panel.
    pub panel: PanelChoice,
    /// A place to go to.
    pub place: Option<Place>,
    /// An entry to put the cursor on.
    pub cursor: Option<Place>,
    /// Entries to mark.
    pub select: Vec<Place>,
    /// Entries to unmark.
    pub deselect: Vec<Place>,
    /// Whether to unmark everything first.
    pub clear_selection: bool,
    /// Whether to list the panel again.
    pub refresh: bool,
}

impl PanelChange {
    /// Reads `panel/set`'s parameters; a call that changes nothing, or
    /// marks more than the context limit, is refused.
    pub fn read(params: &Params) -> Result<Self, RpcError> {
        let place = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .map(plugin_place)
                .transpose()
        };
        let list = |key: &str| match params.get(key) {
            Some(Value::Array(items)) => plugin_places(items),
            Some(_) => Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!("{key} must be a list of places"),
            )),
            None => Ok(Vec::new()),
        };
        let flag = |key: &str| params.get(key).and_then(Value::as_bool).unwrap_or(false);
        let change = Self {
            panel: PanelChoice::read(params)?,
            place: place("place")?,
            cursor: place("cursor")?,
            select: list("select")?,
            deselect: list("deselect")?,
            clear_selection: flag("clear_selection"),
            refresh: flag("refresh"),
        };
        if change.select.len() + change.deselect.len() > MAX_CONTEXT_SELECTION {
            return Err(RpcError::new(
                ErrorKind::LimitExceeded,
                format!("at most {MAX_CONTEXT_SELECTION} places at once"),
            ));
        }
        if change.is_empty() {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "panel/set needs something to change",
            ));
        }
        Ok(change)
    }

    fn is_empty(&self) -> bool {
        self.place.is_none()
            && self.cursor.is_none()
            && self.select.is_empty()
            && self.deselect.is_empty()
            && !self.clear_selection
            && !self.refresh
    }
}

/// Reads `command/execute`'s parameters: the command's id and its
/// arguments.
pub fn read_execute(params: &Params) -> Result<(CommandId, Params), RpcError> {
    let id = params
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "command is missing"))?;
    let command = CommandId::new(id).map_err(|_| {
        RpcError::new(
            ErrorKind::InvalidParams,
            format!("{id} is not a command id"),
        )
    })?;
    let args = match params.get("args") {
        Some(Value::Object(args)) => args.clone(),
        None | Some(Value::Null) => Params::new(),
        Some(_) => {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "args must be an object",
            ));
        }
    };
    Ok((command, args))
}

/// Counts what the manifest refused, and says the first refusal once.
#[derive(Debug, Default)]
pub struct Denials {
    count: AtomicUsize,
}

impl Denials {
    /// Records a refusal; true for the first one, which is said.
    pub fn record(&self) -> bool {
        self.count.fetch_add(1, Ordering::SeqCst) == 0
    }

    /// How many calls the manifest refused.
    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::EntryKind;

    fn params(value: Value) -> Params {
        value.as_object().cloned().unwrap()
    }

    fn root() -> Place {
        Place::parse(if cfg!(windows) { "D:\\docs" } else { "/docs" }).unwrap()
    }

    #[test]
    fn a_snapshot_reads_as_the_review_shows_it() {
        let mut panel = PanelSnapshot::at(Side::Left, root());
        panel.cursor = Some(Entry::new("a.txt", EntryKind::File));
        panel.selection = vec![
            Entry::new("a.txt", EntryKind::File),
            Entry::new("b", EntryKind::Folder),
        ];
        panel.sort = Some("name".into());
        let all = panel.to_value(None);
        assert_eq!(all["panel"], "left");
        assert_eq!(all["provider"], "local");
        assert_eq!(all["cursor"]["name"], "a.txt");
        assert!(all["cursor"]["place"].as_str().unwrap().ends_with("a.txt"));
        assert_eq!(all["selection_count"], 2);
        let some = panel.to_value(Some(&["place".to_owned()]));
        assert!(some.get("cursor").is_none());
        assert!(some.get("place").is_some());
    }

    #[test]
    fn a_change_is_read_and_checked() {
        let place = root().to_string();
        let change = PanelChange::read(&params(json!({
            "panel": "other", "place": place, "select": [place], "refresh": true
        })))
        .unwrap();
        assert_eq!(change.panel, PanelChoice::Other);
        assert_eq!(change.select.len(), 1);
        assert!(change.refresh);
        assert!(PanelChange::read(&params(json!({}))).is_err());
        assert!(PanelChange::read(&params(json!({"panel": "other"}))).is_err());
        assert!(PanelChange::read(&params(json!({"panel": "top", "refresh": true}))).is_err());
        assert!(PanelChange::read(&params(json!({"place": "C:"}))).is_err());
        assert!(PanelChange::read(&params(json!({"select": "x"}))).is_err());
    }

    #[test]
    fn command_ids_are_checked() {
        let (id, args) = read_execute(&params(json!({"command": "panel.refresh"}))).unwrap();
        assert_eq!(id.as_str(), "panel.refresh");
        assert!(args.is_empty());
        assert!(read_execute(&params(json!({"command": "Not An Id"}))).is_err());
        assert!(read_execute(&params(json!({"command": "panel.refresh", "args": 3}))).is_err());
    }

    #[test]
    fn only_the_first_denial_is_said() {
        let denials = Denials::default();
        assert!(denials.record());
        assert!(!denials.record());
        assert!(!denials.record());
        assert_eq!(denials.count(), 3);
    }
}
