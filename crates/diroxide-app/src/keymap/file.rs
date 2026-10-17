//! Writing `keymap.toml` from the Keys list: one line at a time, through
//! `toml_edit`, so the owner's comments, blank lines, order and the lines
//! this version does not know all survive (research/alpha3-plan.md,
//! "Change key, step by step": "Write").
//!
//! - A change writes the command's line in its column and layer: one key
//!   as a string, several as a list, none as `"none"`. An existing line
//!   keeps its place and its trailing comment; a new one goes at the end
//!   of its table, and a missing table is made.
//! - Reset this key takes the command's lines away; Reset all keys takes
//!   away every line of a built-in command and keeps the tables, the
//!   comments, and the lines of commands this version does not know. The
//!   comment lines above a line taken away move above the next line.
//! - The write is atomic: the text goes to `keymap.toml.new`, flushed, and
//!   is renamed over the old file. When the rename fails (an editor holds
//!   the file, on Windows), the old file is untouched and the sentence
//!   says where the change waits.
//! - A file that is not TOML is never written over; the sentence says so.
//!   Its backup was made when it was read at start.
//! - Unknown commands and keys already in the file stay as they are; the
//!   startup check keeps reporting them.

use std::fs;
use std::io;
use std::path::Path;

use diroxide_config::{KeyOverride, KeymapConfig, Paths, write_atomic};
use diroxide_core::{CommandId, Key, KeyColumn, Layer};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use super::names::for_file;

/// The comment a new `keymap.toml` starts with.
pub const HEADER: &str = "# Your keys for diroxide. Each line gives a command its keys in one\n# column and layer; \"none\" takes its keys away. The built-in keys are\n# in keymap-defaults.toml beside this file. The Keys list writes here.\n";

/// One line to write: a command's keys in one column and layer, or, with
/// `keys` `None`, the line taken away (back to the preset or the default).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyLine {
    /// The column.
    pub column: KeyColumn,
    /// The layer.
    pub layer: Layer,
    /// The command.
    pub command: CommandId,
    /// Its keys (empty for "none"), or `None` to take the line away.
    pub keys: Option<Vec<Key>>,
}

/// What the Keys list writes to `keymap.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeymapEdit {
    /// These lines, in order.
    Lines(Vec<KeyLine>),
    /// Every command line taken away.
    ResetAll,
}

/// Whether a line's name is a built-in command's id.
fn is_known(name: &str) -> bool {
    CommandId::new(name).is_ok_and(|id| crate::commands::info(&id).is_some())
}

fn not_written(cause: &str) -> String {
    format!("Could not save keymap.toml: {cause}. Nothing was written.")
}

fn key_value(keys: &[Key]) -> Value {
    match keys {
        [] => Value::from(KeymapConfig::UNBIND),
        [one] => Value::from(for_file(one)),
        many => {
            let mut array = Array::new();
            for key in many {
                array.push(for_file(key));
            }
            Value::Array(array)
        }
    }
}

/// The table for a column and layer, made when it is missing and `make`
/// is set. An item in the way that is not a table is an error in words.
fn layer_table(
    document: &mut DocumentMut,
    column: KeyColumn,
    layer: Layer,
    make: bool,
) -> Result<Option<&mut Table>, String> {
    let root = document.as_table_mut();
    if !root.contains_key(column.word()) {
        if !make {
            return Ok(None);
        }
        let mut parent = Table::new();
        parent.set_implicit(true);
        root.insert(column.word(), Item::Table(parent));
    }
    let Some(parent) = root.get_mut(column.word()).and_then(Item::as_table_mut) else {
        return Err(not_written(&format!("[{column}] is not a table")));
    };
    if !parent.contains_key(layer.word()) {
        if !make {
            return Ok(None);
        }
        parent.insert(layer.word(), Item::Table(Table::new()));
    }
    match parent.get_mut(layer.word()).and_then(Item::as_table_mut) {
        Some(table) => Ok(Some(table)),
        None => Err(not_written(&format!("[{column}.{layer}] is not a table"))),
    }
}

/// The comment lines before a key, when there are any.
fn leading_comment(table: &Table, name: &str) -> Option<String> {
    table
        .key(name)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .filter(|p| p.contains('#'))
        .map(str::to_owned)
}

/// Takes a line away and keeps the comment lines above it: they move
/// above the next line, or below the line before, or below the table's
/// header when the table is left empty. Returns whether the line was
/// there.
fn take_line_away(table: &mut Table, name: &str) -> bool {
    let comment = leading_comment(table, name);
    let names: Vec<String> = table.iter().map(|(n, _)| n.to_owned()).collect();
    let at = names.iter().position(|n| n == name);
    if table.remove(name).is_none() {
        return false;
    }
    let (Some(comment), Some(at)) = (comment, at) else {
        return true;
    };
    let below = format!("\n{}", comment.trim_end_matches('\n'));
    if let Some(next) = names.get(at + 1)
        && let Some(mut key) = table.key_mut(next)
    {
        let old = key
            .leaf_decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_owned();
        key.leaf_decor_mut().set_prefix(format!("{comment}{old}"));
        return true;
    }
    if let Some(before) = at.checked_sub(1).and_then(|i| names.get(i))
        && let Some(value) = table.get_mut(before).and_then(Item::as_value_mut)
    {
        let old = value
            .decor()
            .suffix()
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_owned();
        value.decor_mut().set_suffix(format!("{old}{below}"));
        return true;
    }
    let old = table
        .decor()
        .suffix()
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_owned();
    table.decor_mut().set_suffix(format!("{old}{below}"));
    true
}

/// Whether a piece of a table's decoration holds a comment.
fn has_comment(decor: &toml_edit::Decor) -> bool {
    [decor.prefix(), decor.suffix()]
        .into_iter()
        .flatten()
        .filter_map(|raw| raw.as_str())
        .any(|text| text.contains('#'))
}

/// Takes away a layer's table left with no lines and no comment, then its
/// column's when that is left empty too, so taking lines away leaves no
/// bare headers behind. A table with a comment stays.
fn drop_if_bare(document: &mut DocumentMut, column: KeyColumn, layer: Layer) {
    let root = document.as_table_mut();
    let Some(parent) = root.get_mut(column.word()).and_then(Item::as_table_mut) else {
        return;
    };
    let bare = parent
        .get(layer.word())
        .and_then(Item::as_table)
        .is_some_and(|t| t.is_empty() && !has_comment(t.decor()));
    if bare {
        parent.remove(layer.word());
    }
    let empty = parent.is_empty() && !has_comment(parent.decor());
    if empty {
        root.remove(column.word());
    }
}

/// Applies an edit to `keymap.toml`'s text and returns the new text and
/// how many lines it set or took away. Comments, order and unknown lines
/// survive; text that is not TOML is refused in words.
pub fn edit_text(text: &str, edit: &KeymapEdit) -> Result<(String, usize), String> {
    let fresh = text.trim().is_empty();
    let source = if fresh { "" } else { text };
    let mut document: DocumentMut = source
        .parse()
        .map_err(|_| not_written("it is not valid TOML; fix it, or move it away to start again"))?;
    let mut count = 0;
    match edit {
        KeymapEdit::Lines(lines) => {
            for line in lines {
                let command = line.command.as_str();
                match &line.keys {
                    Some(keys) => {
                        let Some(table) =
                            layer_table(&mut document, line.column, line.layer, true)?
                        else {
                            continue;
                        };
                        let mut value = key_value(keys);
                        if let Some(old) = table.get(command).and_then(Item::as_value) {
                            *value.decor_mut() = old.decor().clone();
                        }
                        match table.get_mut(command) {
                            Some(item) => *item = Item::Value(value),
                            None => {
                                table.insert(command, Item::Value(value));
                            }
                        }
                        count += 1;
                    }
                    None => {
                        if let Some(table) =
                            layer_table(&mut document, line.column, line.layer, false)?
                            && take_line_away(table, command)
                        {
                            count += 1;
                            drop_if_bare(&mut document, line.column, line.layer);
                        }
                    }
                }
            }
        }
        KeymapEdit::ResetAll => {
            for column in KeyColumn::ALL {
                for layer in Layer::ALL {
                    if let Some(table) = layer_table(&mut document, column, layer, false)? {
                        // A line for a command this version does not know
                        // stays: it may be a newer version's, or a
                        // plugin's.
                        let names: Vec<String> = table
                            .iter()
                            .filter(|(name, item)| item.is_value() && is_known(name))
                            .map(|(name, _)| name.to_owned())
                            .collect();
                        for name in names {
                            if take_line_away(table, &name) {
                                count += 1;
                            }
                        }
                        drop_if_bare(&mut document, column, layer);
                    }
                }
            }
        }
    }
    if fresh && count > 0 {
        return Ok((format!("{HEADER}\n{document}"), count));
    }
    Ok((document.to_string(), count))
}

/// Applies an edit to `keymap.toml` in the config folder, atomically, and
/// returns how many lines it set or took away. A missing file is made
/// with [`HEADER`]. Every failure is one sentence that says what was kept.
pub fn save(paths: &Paths, edit: &KeymapEdit) -> Result<usize, String> {
    let path = paths.config_file(Paths::KEYMAP);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return Err(not_written("it is not UTF-8 text"));
        }
        Err(e) => return Err(not_written(&e.to_string())),
    };
    let (new, count) = edit_text(&text, edit)?;
    if new == text {
        return Ok(count);
    }
    write_atomic(&path, &new).map_err(|e| failed_write(&path, &e))?;
    Ok(count)
}

/// The sentence for a write whose rename failed: the old file is as it
/// was, and the change waits in `keymap.toml.new` when it was written.
fn failed_write(path: &Path, error: &io::Error) -> String {
    let mut pending = path.as_os_str().to_owned();
    pending.push(".new");
    if Path::new(&pending).exists() {
        format!(
            "Could not save keymap.toml: {error}. The change is in keymap.toml.new; keymap.toml is as it was."
        )
    } else {
        not_written(&error.to_string())
    }
}

/// Applies an edit to the lines the app holds in memory, as reading the
/// written file back would: a line replaces its command's line in its
/// column and layer, `None` takes it away, Reset all takes every line
/// away.
pub fn apply_to(config: &mut KeymapConfig, edit: &KeymapEdit) {
    match edit {
        KeymapEdit::ResetAll => config
            .overrides
            .retain(|o| crate::commands::info(&o.command).is_none()),
        KeymapEdit::Lines(lines) => {
            for line in lines {
                config.overrides.retain(|o| {
                    !(o.column == line.column && o.layer == line.layer && o.command == line.command)
                });
                if let Some(keys) = &line.keys {
                    config.overrides.push(KeyOverride {
                        column: line.column,
                        layer: line.layer,
                        command: line.command.clone(),
                        keys: keys.clone(),
                        line: 0,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ids;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    fn set(column: KeyColumn, layer: Layer, command: CommandId, keys: &[&str]) -> KeyLine {
        KeyLine {
            column,
            layer,
            command,
            keys: Some(keys.iter().map(|k| key(k)).collect()),
        }
    }

    /// The owner's file, with comments in every place a comment can be.
    const GOLDEN: &str = "# My keys, kept in my dotfiles.\n\n[terminal.panel]\n# Sorting first.\n\"view.sort-size\" = \"ctrl+f9\" # since the laptop has no F6\n\"plugin.thing\" = \"ctrl+k\" # a command this version does not know\n\n# The window, later.\n[window.global]\n\"app.quit\" = [\"ctrl+q\", \"alt+f4\"]\n";

    #[test]
    fn a_change_keeps_every_comment_the_order_and_unknown_lines() {
        let edit = KeymapEdit::Lines(vec![
            set(
                KeyColumn::Terminal,
                Layer::Panel,
                ids::SORT_SIZE,
                &["ctrl+f8"],
            ),
            set(KeyColumn::Terminal, Layer::Panel, ids::SORT_NAME, &[]),
            set(
                KeyColumn::Terminal,
                Layer::Global,
                ids::QUIT,
                &["ctrl+q", "f10"],
            ),
        ]);
        let (text, count) = edit_text(GOLDEN, &edit).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            text,
            "# My keys, kept in my dotfiles.\n\n[terminal.panel]\n# Sorting first.\n\"view.sort-size\" = \"ctrl+f8\" # since the laptop has no F6\n\"plugin.thing\" = \"ctrl+k\" # a command this version does not know\n\"view.sort-name\" = \"none\"\n\n[terminal.global]\n\"app.quit\" = [\"ctrl+q\", \"f10\"]\n\n# The window, later.\n[window.global]\n\"app.quit\" = [\"ctrl+q\", \"alt+f4\"]\n"
        );
        let read = KeymapConfig::from_toml(&text).unwrap();
        assert!(read.problems.is_empty(), "{:?}", read.problems);
        assert_eq!(read.overrides.len(), 5);
    }

    #[test]
    fn reset_takes_lines_away_and_keeps_the_comments() {
        let one = KeymapEdit::Lines(vec![KeyLine {
            column: KeyColumn::Terminal,
            layer: Layer::Panel,
            command: ids::SORT_SIZE,
            keys: None,
        }]);
        let (text, count) = edit_text(GOLDEN, &one).unwrap();
        assert_eq!(count, 1);
        assert!(!text.contains("view.sort-size"));
        assert!(
            text.contains("# Sorting first.\n\"plugin.thing\""),
            "the comment moves above the next line: {text}"
        );
        // Reset all keeps the line of a command this version does not know.
        let (text, count) = edit_text(GOLDEN, &KeymapEdit::ResetAll).unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            text,
            "# My keys, kept in my dotfiles.\n\n[terminal.panel]\n# Sorting first.\n\"plugin.thing\" = \"ctrl+k\" # a command this version does not know\n\n# The window, later.\n[window.global]\n"
        );
        // The last line of a table: its comment stays below the line
        // before, or below the table's header.
        let last = "[terminal.panel]\n\"view.sort-name\" = \"f12\"\n# Size, my way.\n\"view.sort-size\" = \"ctrl+f9\"\n";
        let size = KeymapEdit::Lines(vec![KeyLine {
            column: KeyColumn::Terminal,
            layer: Layer::Panel,
            command: ids::SORT_SIZE,
            keys: None,
        }]);
        let (text, _) = edit_text(last, &size).unwrap();
        assert_eq!(
            text,
            "[terminal.panel]\n\"view.sort-name\" = \"f12\"\n# Size, my way.\n"
        );
        let alone = "[terminal.panel]\n# Size, my way.\n\"view.sort-size\" = \"ctrl+f9\"\n";
        let (text, _) = edit_text(alone, &size).unwrap();
        assert_eq!(text, "[terminal.panel]\n# Size, my way.\n");
        let missing = KeymapEdit::Lines(vec![KeyLine {
            column: KeyColumn::Window,
            layer: Layer::Viewer,
            command: ids::SORT_SIZE,
            keys: None,
        }]);
        assert_eq!(edit_text(GOLDEN, &missing).unwrap(), (GOLDEN.to_owned(), 0));
    }

    #[test]
    fn a_new_file_starts_with_its_header_and_bad_text_is_never_written() {
        let edit = KeymapEdit::Lines(vec![set(
            KeyColumn::Terminal,
            Layer::Panel,
            ids::SORT_SIZE,
            &["ctrl+f9"],
        )]);
        let (text, _) = edit_text("", &edit).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        assert!(
            text.ends_with("\n[terminal.panel]\n\"view.sort-size\" = \"ctrl+f9\"\n"),
            "{text}"
        );
        assert_eq!(
            edit_text("[terminal.panel\n", &edit).unwrap_err(),
            "Could not save keymap.toml: it is not valid TOML; fix it, or move it away to start again. Nothing was written."
        );
        assert_eq!(
            edit_text("terminal = 3\n", &edit).unwrap_err(),
            "Could not save keymap.toml: [terminal] is not a table. Nothing was written."
        );
    }

    #[test]
    fn the_file_is_written_atomically_and_read_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let file = paths.config_file(Paths::KEYMAP);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, GOLDEN).unwrap();
        let edit = KeymapEdit::Lines(vec![set(
            KeyColumn::Terminal,
            Layer::Panel,
            ids::SORT_SIZE,
            &["ctrl+f8"],
        )]);
        assert_eq!(save(&paths, &edit), Ok(1));
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"view.sort-size\" = \"ctrl+f8\" # since the laptop has no F6"));
        assert!(!file.with_file_name("keymap.toml.new").exists());
        let mut held = KeymapConfig::from_toml(GOLDEN).unwrap();
        apply_to(&mut held, &edit);
        let read = KeymapConfig::from_toml(&text).unwrap();
        let sorted = |c: &KeymapConfig| {
            let mut lines: Vec<String> = c
                .overrides
                .iter()
                .map(|o| format!("{} {} {} {:?}", o.column, o.layer, o.command, o.keys))
                .collect();
            lines.sort();
            lines
        };
        assert_eq!(sorted(&held), sorted(&read));
        apply_to(&mut held, &KeymapEdit::ResetAll);
        assert_eq!(held.overrides.len(), 1, "the unknown command's line stays");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_another_program_holds_is_left_as_it_was() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let file = paths.config_file(Paths::KEYMAP);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, GOLDEN).unwrap();
        // An editor that lets others read the file but not replace it.
        const FILE_SHARE_READ: u32 = 1;
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&file)
            .unwrap();
        let edit = KeymapEdit::Lines(vec![set(
            KeyColumn::Terminal,
            Layer::Panel,
            ids::SORT_SIZE,
            &["ctrl+f8"],
        )]);
        let said = save(&paths, &edit).unwrap_err();
        assert!(said.starts_with("Could not save keymap.toml: "), "{said}");
        assert!(
            said.ends_with("The change is in keymap.toml.new; keymap.toml is as it was."),
            "{said}"
        );
        drop(held);
        assert_eq!(fs::read_to_string(&file).unwrap(), GOLDEN);
        let pending = fs::read_to_string(file.with_file_name("keymap.toml.new")).unwrap();
        assert!(pending.contains("ctrl+f8"));
    }
}
