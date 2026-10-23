//! Listers, item rows and column declarations (R22, section 2.3, N1 and
//! N2).
//!
//! - **An item row** is an entry of kind `item`: a row that is not a file
//!   (a commit, a changed file's state, a search result's line), with its
//!   name, an opaque identity in its `id` extra value, and string values
//!   for the lister's columns in the rest of `extra`. No file command
//!   takes one: each answers "This row is not a file." ([`not_files`]).
//! - **`lister/open`** (plugin to host) shows a `[[lister]]` the manifest
//!   declares: its title (the plugin's name last), its first rows, in the
//!   active panel or the other. Rows are cleaned like announcements and
//!   capped at [`MAX_LISTER_ROWS`]; more come through `lister/rows`.
//! - **Enter** runs the lister's `activate` command, and its
//!   `[[lister.action]]` keys their commands, with the row or the marked
//!   rows as the selection ([`ListerView::command_params`]).
//! - **`lister/closed`** tells the plugin the owner left a lister.
//! - **`column/format`** (host to plugin, a shape in this alpha; the lazy
//!   columns on file panels are alpha.7's) asks a `[[column]]` for a batch
//!   of at most [`MAX_COLUMN_BATCH`] places of one folder; each value is at
//!   most 40 cells, meaning first, and a value not answered yet reads
//!   "pending", never an empty cell.

use std::collections::BTreeMap;

use diroxide_core::{Entry, EntryKind, Message};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::lines;
use crate::manifest::{ListerSpec, Manifest};
use crate::panels::PanelChoice;
use crate::protocol::{ErrorKind, Params, RpcError};
use crate::services::{MAX_ANNOUNCE_CHARS, clean_text};
use crate::shapes::{WireContext, WireEntry};

/// The most rows a lister shows at once.
pub const MAX_LISTER_ROWS: usize = 5000;

/// The most places one `column/format` batch holds.
pub const MAX_COLUMN_BATCH: usize = 500;

/// The extra value that holds an item row's identity.
pub const ITEM_ID: &str = "id";

/// A lister a plugin opened, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListerView {
    /// The plugin's id.
    pub plugin_id: String,
    /// The plugin's name.
    pub plugin: String,
    /// The lister as the manifest declares it: its columns, the command
    /// Enter runs, and the actions by key.
    pub spec: ListerSpec,
    /// The title, cleaned, the plugin's name last: "Changed files in
    /// diroxide (Git)".
    pub title: String,
    /// The rows, cleaned.
    pub rows: Vec<Entry>,
    /// Where it opens.
    pub panel: PanelChoice,
    /// How many rows there are in all, when the plugin knows.
    pub total: Option<u64>,
    /// Whether the rows were cut at [`MAX_LISTER_ROWS`].
    pub truncated: bool,
}

impl ListerView {
    /// The plugin-local command Enter runs, when the lister has one.
    pub fn activate(&self) -> Option<&str> {
        self.spec.activate.as_deref()
    }

    /// The plugin-local command a key runs, when an action binds it.
    pub fn action_for(&self, key: &str) -> Option<&str> {
        self.spec
            .actions
            .iter()
            .find(|a| a.key == key)
            .map(|a| a.command.as_str())
    }

    /// The column headers, in order, for the panel's header row.
    pub fn headers(&self) -> Vec<&str> {
        self.spec
            .columns
            .iter()
            .map(|c| c.header.as_str())
            .collect()
    }

    /// `command/run`'s parameters for a lister command: the context the
    /// app made, with the rows as its selection.
    pub fn command_params(&self, command: &str, context: WireContext, rows: &[Entry]) -> Value {
        let wire: Vec<WireEntry> = rows
            .iter()
            .map(|row| WireEntry::from_entry(row, None))
            .collect();
        let mut context = context.with_selection(None, wire);
        context.command = format!("{}.{command}", self.plugin_id);
        json!({"command": command, "lister": self.spec.id, "context": context})
    }

    /// The line said when the rows were cut: "First 5,000 shown, narrow
    /// the search."
    pub fn capped_line(&self) -> Option<String> {
        self.truncated.then(lines::lister_capped)
    }
}

/// The refusal a file command gives a selection holding item rows:
/// "This row is not a file."
pub fn not_files(entries: &[Entry]) -> Option<Message> {
    entries
        .iter()
        .any(|e| e.kind == EntryKind::Item)
        .then(|| Message::warning(lines::not_a_file()))
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

/// Rows from a plugin: entries, cleaned, the empty ones left out, at most
/// `limit`; whether any were cut.
pub fn read_rows(value: Option<&Value>, limit: usize) -> Result<(Vec<Entry>, bool), RpcError> {
    let items = match value {
        None | Some(Value::Null) => return Ok((Vec::new(), false)),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(invalid("rows must be a list of entries")),
    };
    let mut rows = Vec::new();
    for item in items.iter().take(limit) {
        let wire: WireEntry =
            serde_json::from_value(item.clone()).map_err(|_| invalid("a row is not an entry"))?;
        let name = clean_text(&wire.name, MAX_ANNOUNCE_CHARS);
        if name.is_empty() {
            continue;
        }
        let mut entry = wire.to_entry();
        entry.name = name;
        entry.extra = wire
            .extra
            .iter()
            .map(|(k, v)| (k.clone(), clean_text(v, MAX_ANNOUNCE_CHARS)))
            .collect();
        rows.push(entry);
    }
    Ok((rows, items.len() > limit))
}

/// Reads `lister/open`'s parameters against the manifest's listers.
pub(crate) fn read_open(
    params: &Params,
    manifest: &Manifest,
    plugin: &str,
) -> Result<ListerView, RpcError> {
    let id = params
        .get("lister")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("lister is missing"))?;
    let spec = manifest
        .listers
        .iter()
        .find(|l| l.id == id)
        .cloned()
        .ok_or_else(|| {
            RpcError::new(
                ErrorKind::DeniedByHost,
                format!("the manifest declares no lister {id}"),
            )
        })?;
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .map(|t| clean_text(t, MAX_ANNOUNCE_CHARS))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| spec.name.clone());
    let (rows, truncated) = read_rows(params.get("rows"), MAX_LISTER_ROWS)?;
    Ok(ListerView {
        plugin_id: manifest.id.clone(),
        plugin: plugin.to_owned(),
        title: lines::with_source(&title, plugin),
        spec,
        rows,
        panel: PanelChoice::read(params)?,
        total: params.get("total").and_then(Value::as_u64),
        truncated,
    })
}

/// One value of a plugin's column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnValue {
    /// What the cell says, at most 40 cells, meaning first.
    pub text: String,
    /// What the column sorts by, when it differs from the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
}

impl ColumnValue {
    /// The value of a cell the plugin has not answered yet: "pending".
    pub fn pending() -> Self {
        Self {
            text: lines::pending(),
            sort: None,
        }
    }
}

/// `column/format`'s parameters: a column, a folder and its places.
pub fn column_params(column: &str, folder: &str, places: &[String]) -> Result<Value, RpcError> {
    if places.len() > MAX_COLUMN_BATCH {
        return Err(RpcError::new(
            ErrorKind::LimitExceeded,
            format!("at most {MAX_COLUMN_BATCH} places in one batch"),
        ));
    }
    Ok(json!({"column": column, "folder": folder, "places": places}))
}

/// Reads `column/format`'s answer: each asked place's value, cleaned and
/// cut to 40 cells, "pending" where the plugin gave none.
pub fn read_column_values(answer: &Value, places: &[String]) -> BTreeMap<String, ColumnValue> {
    let values = answer.get("values").and_then(Value::as_object);
    places
        .iter()
        .map(|place| {
            let value = values
                .and_then(|v| v.get(place))
                .and_then(|v| match v {
                    Value::String(text) => Some(ColumnValue {
                        text: text.clone(),
                        sort: None,
                    }),
                    other => serde_json::from_value::<ColumnValue>(other.clone()).ok(),
                })
                .map(|v| ColumnValue {
                    text: clean_text(&v.text, lines::MAX_LINE_CELLS),
                    sort: v.sort.map(|s| clean_text(&s, MAX_ANNOUNCE_CHARS)),
                })
                .filter(|v| !v.text.is_empty())
                .unwrap_or_else(ColumnValue::pending);
            (place.clone(), value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::CommandId;

    use crate::shapes::PanelRef;

    const MANIFEST: &str = "[plugin]\nid = \"git\"\nname = \"Git\"\nversion = \"0.1.0\"\nprotocol = 1\n[run]\ncommand = [\"x\"]\n[[command]]\nid = \"show_diff\"\nname = \"Show the diff\"\n[[command]]\nid = \"stage\"\nname = \"Stage\"\n[[lister]]\nid = \"changed\"\nname = \"Changed files\"\nactivate = \"show_diff\"\n[[lister.columns]]\nid = \"state\"\nheader = \"State\"\nkind = \"text\"\n[[lister.columns]]\nid = \"path\"\nheader = \"Path\"\nkind = \"text\"\nsorts = true\n[[lister.action]]\nkey = \"s\"\ncommand = \"stage\"\nselection = \"many\"\n[[column]]\nid = \"git\"\nheader = \"Git\"\nkind = \"text\"\n";

    fn manifest() -> Manifest {
        let report = Manifest::check(MANIFEST);
        let text = report.text();
        report.manifest.unwrap_or_else(|| panic!("{text}"))
    }

    fn params(value: Value) -> Params {
        value.as_object().cloned().unwrap()
    }

    /// The changed-files fixture: item rows with their state first.
    fn changed_files() -> Value {
        serde_json::from_str(include_str!(
            "../../../fixtures/plugins/shapes/lister-open-changed.json"
        ))
        .unwrap()
    }

    #[test]
    fn the_manifest_declares_listers_and_columns() {
        let manifest = manifest();
        let lister = &manifest.listers[0];
        assert_eq!(lister.activate.as_deref(), Some("show_diff"));
        assert_eq!(lister.columns.len(), 2);
        assert_eq!(manifest.columns[0].id, "git");
    }

    #[test]
    fn a_lister_opens_with_item_rows_and_its_source_last() {
        let fixture = changed_files();
        let view = read_open(fixture["params"].as_object().unwrap(), &manifest(), "Git").unwrap();
        assert_eq!(view.title, "Changed files in diroxide (Git)");
        assert_eq!(view.rows.len(), 3);
        assert!(view.rows.iter().all(|r| r.kind == EntryKind::Item));
        assert_eq!(view.rows[0].extra["state"], "Modified");
        assert_eq!(view.rows[0].extra[ITEM_ID], "src/main.rs");
        assert_eq!(view.activate(), Some("show_diff"));
        assert_eq!(view.action_for("s"), Some("stage"));
        assert_eq!(view.headers(), ["State", "Path"]);
        assert_eq!(view.panel, PanelChoice::Active);
        assert_eq!(view.capped_line(), None);
    }

    #[test]
    fn an_undeclared_lister_or_bad_rows_are_refused() {
        let undeclared = read_open(&params(json!({"lister": "other"})), &manifest(), "Git");
        assert_eq!(undeclared.unwrap_err().code, -32030);
        let bad = read_open(
            &params(json!({"lister": "changed", "rows": [{"kind": "item"}]})),
            &manifest(),
            "Git",
        );
        assert_eq!(bad.unwrap_err().code, -32602);
    }

    #[test]
    fn rows_are_cleaned_and_capped() {
        let many: Vec<Value> = (0..(MAX_LISTER_ROWS + 10))
            .map(|i| json!({"name": format!("row {i}\u{202E}"), "kind": "item"}))
            .collect();
        let (rows, cut) = read_rows(Some(&Value::Array(many)), MAX_LISTER_ROWS).unwrap();
        assert_eq!(rows.len(), MAX_LISTER_ROWS);
        assert!(cut);
        assert_eq!(rows[0].name, "row 0");
        assert_eq!(
            lines::lister_capped(),
            "First 5,000 shown, narrow the search."
        );
    }

    #[test]
    fn enter_runs_the_activate_command_with_the_row_as_the_selection() {
        let fixture = changed_files();
        let view = read_open(fixture["params"].as_object().unwrap(), &manifest(), "Git").unwrap();
        let place =
            diroxide_core::Place::parse(if cfg!(windows) { "D:\\repo" } else { "/repo" }).unwrap();
        let context = WireContext::new(
            &CommandId::from_static("git.show_diff"),
            PanelRef::for_place(&place),
            PanelRef::for_place(&place),
        );
        let params = view.command_params(view.activate().unwrap(), context, &view.rows[..1]);
        assert_eq!(params["command"], "show_diff");
        assert_eq!(params["lister"], "changed");
        assert_eq!(params["context"]["command"], "git.show_diff");
        assert_eq!(params["context"]["selection"][0]["kind"], "item");
        assert_eq!(
            params["context"]["selection"][0]["extra"]["id"],
            "src/main.rs"
        );
        assert_eq!(params["context"]["selection_count"], 1);
    }

    #[test]
    fn a_file_command_on_an_item_row_is_refused_in_words() {
        let rows = vec![
            Entry::new("a.txt", EntryKind::File),
            Entry::new("abc123", EntryKind::Item),
        ];
        assert_eq!(
            not_files(&rows),
            Some(Message::warning("This row is not a file."))
        );
        assert_eq!(not_files(&rows[..1]), None);
    }

    #[test]
    fn column_values_round_trip_cleaned_with_pending() {
        let places = vec![
            "/r/a.txt".to_owned(),
            "/r/b.txt".to_owned(),
            "/r/c.txt".to_owned(),
        ];
        let asked = column_params("git", "/r", &places).unwrap();
        assert_eq!(asked["places"].as_array().unwrap().len(), 3);
        let answer = json!({"values": {
            "/r/a.txt": {"text": "Modified", "sort": "2"},
            "/r/b.txt": format!("Untracked {}", "x".repeat(80)),
        }});
        let values = read_column_values(&answer, &places);
        assert_eq!(values["/r/a.txt"].text, "Modified");
        assert_eq!(values["/r/a.txt"].sort.as_deref(), Some("2"));
        assert_eq!(values["/r/b.txt"].text.chars().count(), 40);
        assert_eq!(values["/r/c.txt"], ColumnValue::pending());
        let too_many: Vec<String> = (0..=MAX_COLUMN_BATCH).map(|i| format!("/{i}")).collect();
        assert_eq!(
            column_params("git", "/", &too_many).unwrap_err().code,
            -32031
        );
    }
}
