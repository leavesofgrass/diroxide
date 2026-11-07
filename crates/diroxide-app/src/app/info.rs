//! What the app says and lists: Say Status, Say Other Panel, the key bar,
//! help, the context menu, the menus and the palette, settings
//! with set and reset, the layout and theme, config problems, the job
//! history, the key test, the run-command prompt, shell here, the editor,
//! and quit.

use diroxide_config::{Layout, SettingKind};
use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, Effect, Key, KeyColumn, Layer, ListModel, ListRow, Message, Place,
    PlaceKind, Redraw, Side,
};
use diroxide_vfs::Capabilities;

use super::{App, Purpose};
use crate::commands::{self, CATALOG, ids};
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::keymap;
use crate::panel::place_leaf;
use crate::request::{Request, SpawnKind};
use crate::settings;
use crate::state;

/// The keyboard list's first row: the help for where the focus is.
const HELP_ROW: &str = "help";

/// The start of a heading row's id in the keyboard list.
pub(super) const HEADING_ROW: &str = "heading:";

fn read_list(title: impl Into<String>, rows: Vec<ListRow>) -> ListModel {
    let mut list = ListModel::new(title).with_rows(rows);
    list.set_help("Up and Down read, Escape closes.");
    list
}

fn text_rows(lines: Vec<String>) -> Vec<ListRow> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| ListRow::new(format!("line-{i}"), line))
        .collect()
}

/// The last folder on this computer at or above a place: the folder an
/// archive sits in; `None` for a remote place.
fn local_folder(place: &Place) -> Option<Place> {
    match place.kind() {
        PlaceKind::Local => Some(place.clone()),
        PlaceKind::Archive => place
            .archive()
            .and_then(|(container, _)| local_folder(container))
            .and_then(|archive| archive.parent()),
        PlaceKind::Remote => None,
    }
}

impl App {
    /// What the active place can do; until a listing says, a local place
    /// is taken as writable with a trash, so the menus offer the usual.
    pub(super) fn active_capabilities(&self) -> Capabilities {
        self.active_panel().capabilities().unwrap_or_else(|| {
            let mut caps = Capabilities::read_only();
            if self.active_panel().place().kind() == PlaceKind::Local {
                caps.writable = true;
                caps.trash = true;
            }
            caps
        })
    }

    /// Say Status: in a dialog, the dialog's focus; in a panel the
    /// selection first when there is one ("3 selected, 4.2 MB.
    /// Documents, 40 items"), then the focus and its position, any
    /// filter, the jobs' progress and the file clipboard.
    pub(super) fn say_status(&mut self) -> Vec<Effect> {
        // The filter prompt says its text, how it reads and the count.
        if let Some(line) = self.filter_prompt_status() {
            return vec![self.say(Message::info(line))];
        }
        // The results lister says how the search stands first.
        if let Some(line) = self.results_status() {
            return vec![self.say(Message::info(line))];
        }
        if let Some((_, dialog)) = self.dialogs.last() {
            let line = dialog.focus_line();
            return vec![self.say(Message::info(line))];
        }
        let side = self.active;
        let panel = self.panel(side);
        let list = panel.list();
        let selected = list.selected_count() as u64;
        let mut text = if selected > 0 {
            format!(
                "{selected} selected, {}. {}, {}",
                panel.selection_size_words(),
                place_leaf(panel.place()),
                count(list.len() as u64, "item", "items")
            )
        } else {
            format!(
                "{}, {}, no selection",
                place_leaf(panel.place()),
                count(list.len() as u64, "item", "items")
            )
        };
        if let Some(row) = list.focused() {
            text.push_str(&format!(", on {}, {}", row.label, list.position_text()));
        }
        if let Some(filter) = self.filter_title(side) {
            text.push_str(&format!(", {filter}"));
        }
        // The place's free space, as it answered off the input thread; a
        // place still asking says nothing (R25, 3.4).
        if let Some(room) = self.room_status(side) {
            text.push_str(&format!(", {room}"));
        }
        for job in self.jobs.ids() {
            if let Some(detail) = self.jobs.detail(job) {
                text.push_str(&format!(". {detail}"));
            }
        }
        // The jobs that wait their turn, each in its row's words (R25,
        // 2.2: Say Status gives all of them).
        for row in self.waiting_status() {
            text.push_str(&format!(". {row}"));
        }
        // Watch this folder (W3t).
        if let Some(watch) = self.watch_status() {
            text.push_str(&format!(". {watch}"));
        }
        if let Some(clipboard) = &self.clipboard {
            let what = if clipboard.cut { "to move" } else { "to copy" };
            text.push_str(&format!(
                ". {} on the clipboard, {what}",
                count(clipboard.places.len() as u64, "item", "items")
            ));
        }
        vec![self.say(Message::info(text))]
    }

    /// Say Other Panel: "Other panel: Backup, 12 items, on report.pdf, 3
    /// of 12".
    pub(super) fn say_other(&mut self) -> Vec<Effect> {
        if let Some(effects) = self.say_other_preview() {
            return effects;
        }
        let panel = self.panel(self.active.other());
        let list = panel.list();
        let mut text = format!(
            "Other panel: {}, {}",
            place_leaf(panel.place()),
            count(list.len() as u64, "item", "items")
        );
        if let Some(row) = list.focused() {
            text.push_str(&format!(", on {}, {}", row.spoken(), list.position_text()));
        }
        vec![self.say(Message::info(text))]
    }

    /// Say Key Bar: every pair of the bank the key bar shows
    /// ([`App::shown_bank`]), keys in words: "Key bar: F2 Rename, F3
    /// View, ... Control plus Q Quit."
    pub(super) fn say_key_bar(&mut self) -> Vec<Effect> {
        let text = self
            .shown_bank()
            .map(|bank| bank.key_bar_spoken(&self.keymap))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "The key bar is empty.".to_owned());
        vec![self.say(Message::info(text))]
    }

    /// Switch bank: the next bank on the key bar, its name and its first
    /// pairs said ("Bank: Archives. F2 Pack, F3 Unpack").
    pub(super) fn switch_bank(&mut self) -> Vec<Effect> {
        if self.banks.len() < 2 {
            let name = self
                .active_bank()
                .map_or_else(|| "none".to_owned(), |bank| bank.name.clone());
            return vec![self.say(Message::info(format!("Only one bank: {name}.")))];
        }
        self.bank = (self.bank + 1) % self.banks.len();
        let text = match self.active_bank() {
            Some(bank) => {
                let pairs = bank.pairs(&self.keymap);
                let first: Vec<&str> = pairs.iter().take(3).map(String::as_str).collect();
                format!("Bank: {}. {}", bank.name, first.join(", "))
            }
            None => "The key bar is empty.".to_owned(),
        };
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
    }

    /// Help where the focus is: the list's or the field's help first, then
    /// the commands of the panel with their keys (Enter runs one).
    pub(super) fn help(&mut self) -> Vec<Effect> {
        let (title, first) = match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => (list.title().to_owned(), list.help().to_owned()),
            Some((_, Dialog::Question { choices, .. })) => {
                (choices.title().to_owned(), choices.help().to_owned())
            }
            Some((_, Dialog::Edit { field, .. })) => (
                field.label().to_owned(),
                "Type, then Enter. Tab completes; Up and Down recall what you typed before; Escape closes."
                    .to_owned(),
            ),
            Some((_, Dialog::Confirm { question, .. })) => (
                question.text.clone(),
                "y answers yes, n or Escape answers no; No is the default.".to_owned(),
            ),
            Some((_, Dialog::Secret { request, .. })) => (
                request.title(),
                "Type the password, then Enter. Escape gives none.".to_owned(),
            ),
            Some((_, dialog)) => (dialog.title(), String::new()),
            None => (
                "Panel".to_owned(),
                self.active_panel().list().help().to_owned(),
            ),
        };
        if let Some(Purpose::Settings) = self.opens.last().map(|o| &o.purpose)
            && let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last()
            && let Some(row) = list.focused()
            && let Some(help) = settings::setting_help(&row.id)
        {
            return vec![self.say(Message::info(help))];
        }
        if !self.dialogs.is_empty() {
            let line = if first.is_empty() { title } else { first };
            return vec![self.say(Message::info(line))];
        }
        let _ = title;
        let list = self.keyboard_list(first);
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::HELP),
            },
            Purpose::Read,
        )
    }

    /// The keyboard page as a list (answer 49), from the live keymap, so a
    /// key is found from inside the program: the help for where the focus
    /// is first, then a heading for each layer as `docs/keyboard.md` has
    /// them ("Everywhere", "In a panel", "In the viewer"), each command a
    /// row with its name first, then its keys and what it does ("Copy to
    /// other panel, F5, Copies the selection to the other panel's
    /// folder."), and last the commands with no key, with the menu that
    /// holds them. Letters jump by name; Enter runs a command.
    pub(super) fn keyboard_list(&self, first: String) -> ListModel {
        let mut rows = vec![ListRow::new(HELP_ROW, first)];
        let layers = [
            (Layer::Global, "Everywhere"),
            (Layer::Panel, "In a panel"),
            (Layer::List, "In any list"),
            (Layer::Viewer, "In the viewer"),
            (Layer::Preview, "In the preview"),
            (Layer::Dialog, "In a question"),
        ];
        let mut keyless: Vec<&commands::CommandInfo> = Vec::new();
        for (layer, heading) in layers {
            let mut lines = Vec::new();
            for info in CATALOG.iter().filter(|i| i.layer == layer) {
                let keys: Vec<String> = self
                    .keymap
                    .bindings()
                    .iter()
                    .filter(|b| b.layer == layer && b.command == info.id)
                    .map(|b| keymap::written(&b.key, self.keymap.platform()))
                    .collect();
                if keys.is_empty() {
                    keyless.push(info);
                    continue;
                }
                lines.push(
                    ListRow::new(info.id.as_str(), info.name)
                        .with_details([keys.join(" or "), info.help.to_owned()]),
                );
            }
            if !lines.is_empty() {
                rows.push(ListRow::new(format!("{HEADING_ROW}{heading}"), heading));
                rows.extend(lines);
            }
        }
        if !keyless.is_empty() {
            let heading = "By the palette and the menus";
            rows.push(ListRow::new(format!("{HEADING_ROW}{heading}"), heading));
            for info in keyless {
                let way = self
                    .menus
                    .holder_of(&info.id)
                    .map_or_else(|| "the palette".to_owned(), |m| format!("{m} menu"));
                rows.push(
                    ListRow::new(info.id.as_str(), info.name)
                        .with_details([way, info.help.to_owned()]),
                );
            }
        }
        let commands = count(CATALOG.len() as u64, "command", "commands");
        let mut list = ListModel::new(format!("Keyboard, {commands}")).with_rows(rows);
        list.set_help(
            "Up and Down read, a letter jumps to a command by name, Enter runs it, Escape closes.",
        );
        list
    }

    /// The context menu as a list: what the File menu offers here, and the
    /// clipboard, each with its key.
    pub(super) fn context_menu(&mut self) -> Vec<Effect> {
        let side = self.active;
        let targets = self.panel(side).targets();
        let caps = self.active_capabilities();
        let Some(file) = self.menus.list("file", &self.keymap, &caps) else {
            return Vec::new();
        };
        let mut rows: Vec<ListRow> = file
            .rows()
            .iter()
            .filter(|r| {
                ![ids::QUIT, ids::QUIT_TO_FOLDER, ids::QUIT_KEEP_FOLDER]
                    .iter()
                    .any(|quit| r.id == quit.as_str())
            })
            .cloned()
            .collect();
        for id in [
            ids::CLIPBOARD_COPY,
            ids::CLIPBOARD_CUT,
            ids::COPY,
            ids::MOVE,
        ] {
            if rows.iter().any(|r| r.id == id.as_str()) {
                continue;
            }
            let key = self.keymap.key_text(&id);
            let row = ListRow::new(id.as_str(), commands::name_of(&id));
            rows.push(if key.is_empty() {
                row
            } else {
                row.with_details([key])
            });
        }
        let title = match targets.as_slice() {
            [one] => format!(
                "Actions for {}",
                one.name()
                    .map(diroxide_core::names::display_name)
                    .unwrap_or_default()
            ),
            many => format!("Actions for {}", count(many.len() as u64, "item", "items")),
        };
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_help("Up and Down choose, Enter runs, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::CONTEXT_MENU),
            },
            Purpose::ContextMenu,
        )
    }

    pub(super) fn open_palette(&mut self) -> Vec<Effect> {
        let field = EntryField::new("Command palette", CompletionSource::Commands);
        let list = self.palette_list("");
        let dialog = Dialog::Edit {
            field,
            candidates: Some(list),
            in_list: false,
        };
        self.open_dialog(dialog, Purpose::Palette)
    }

    pub(super) fn open_menu_bar(&mut self) -> Vec<Effect> {
        let list = self.menus.bar_list(&self.keymap);
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::MENU),
            },
            Purpose::MenuBar,
        )
    }

    pub(super) fn open_menu_by_command(&mut self, id: &CommandId) -> Vec<Effect> {
        let menu = commands::Menu::ALL
            .iter()
            .find(|m| m.command() == *id)
            .map(|m| m.id().to_owned());
        match menu {
            Some(menu) => self.open_menu(&menu),
            None => Vec::new(),
        }
    }

    pub(super) fn open_menu(&mut self, id: &str) -> Vec<Effect> {
        let caps = self.active_capabilities();
        // The enabled plugins' commands join the menus (W4d).
        let Some(list) = self.menu_list_for(id, &caps) else {
            return vec![self.say(Message::info(format!("No menu is named {id}.")))];
        };
        let dialog = Dialog::Choose {
            list,
            on_enter: Command::new(ids::MENU),
        };
        match self.menu_saying(id) {
            Some(message) => self.open_dialog_saying(dialog, Purpose::Menu(id.to_owned()), message),
            None => self.open_dialog(dialog, Purpose::Menu(id.to_owned())),
        }
    }

    /// Left or Right in a menu: the menu before or after on the bar.
    pub(super) fn next_menu(&mut self, id: &str, forward: bool) -> Vec<Effect> {
        let bar: Vec<String> = self.menus.bar().map(|m| m.id.clone()).collect();
        let Some(at) = bar.iter().position(|m| m == id) else {
            return Vec::new();
        };
        let next = if forward {
            (at + 1) % bar.len()
        } else {
            (at + bar.len() - 1) % bar.len()
        };
        let next = bar[next].clone();
        let mut effects = self.close_all();
        effects.extend(self.open_menu(&next));
        effects
    }

    pub(super) fn open_settings(&mut self) -> Vec<Effect> {
        let list = settings::settings_list_in(&self.settings, self.themes());
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SETTINGS),
            },
            Purpose::Settings,
        )
    }

    /// Enter on a setting: a switch flips; a choice opens its choices with
    /// "Back to default" last; anything else opens a field.
    pub(super) fn setting_chosen(&mut self, key: &str) -> Vec<Effect> {
        if key == settings::KEYS_ROW {
            return self.open_keys_list();
        }
        if key == settings::PLUGINS_ROW {
            return self.open_plugin_settings();
        }
        if key == settings::TOOLS_ROW {
            return self.open_tools_page();
        }
        if key == "preset" {
            return self.preset_chooser();
        }
        let Some(info) = settings::setting(key) else {
            return Vec::new();
        };
        let current = self.settings.value_words(key);
        match info.kind {
            SettingKind::Flag => {
                let now = self
                    .settings
                    .values()
                    .ok()
                    .and_then(|v| v.get(key).and_then(toml::Value::as_bool))
                    .unwrap_or(false);
                self.apply_setting(key, Some(toml::Value::Boolean(!now)))
            }
            SettingKind::Choice(choices) => {
                let mut rows: Vec<ListRow> = choices
                    .iter()
                    .map(|c| {
                        let row = ListRow::new(c.value, c.words);
                        if c.words == current {
                            row.with_details(["current"])
                        } else {
                            row
                        }
                    })
                    .collect();
                rows.push(ListRow::new("\u{1}default", "Back to default"));
                let mut list = ListModel::new(info.name).with_rows(rows);
                if let Some(index) = choices.iter().position(|c| c.words == current) {
                    list.move_focus(diroxide_core::Move::To(index));
                }
                list.set_help("Up and Down choose, Enter sets it, Escape closes.");
                self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::SETTINGS),
                    },
                    Purpose::SettingValue(key.to_owned()),
                )
            }
            SettingKind::SplitPresets => self.split_presets_list(),
            SettingKind::Theme => {
                // The built-in themes, then the owner's theme files.
                let mut rows: Vec<ListRow> = self
                    .theme_names()
                    .into_iter()
                    .map(|name| ListRow::new(name.clone(), name))
                    .collect();
                rows.push(ListRow::new("\u{1}default", "Back to default"));
                let mut list = ListModel::new(info.name).with_rows(rows);
                list.focus_id(&self.settings.theme);
                self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::SETTINGS),
                    },
                    Purpose::SettingValue(key.to_owned()),
                )
            }
            _ => {
                let value = self
                    .settings
                    .values()
                    .ok()
                    .and_then(|v| v.get(key).cloned())
                    .map(|v| match v {
                        toml::Value::String(s) => s,
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                let field = EntryField::new(
                    format!("{}, {}", info.name, info.kind.words()),
                    CompletionSource::None,
                )
                .with_text(value);
                self.open_dialog(Dialog::edit(field), Purpose::SettingText(key.to_owned()))
            }
        }
    }

    pub(super) fn setting_value_chosen(&mut self, key: &str, value: &str) -> Vec<Effect> {
        let mut effects = Vec::new();
        if value == "\u{1}default" {
            effects.extend(self.apply_setting(key, None));
        } else {
            effects.extend(self.apply_setting(key, Some(toml::Value::String(value.to_owned()))));
        }
        effects
    }

    pub(super) fn setting_typed(&mut self, key: &str, text: &str) -> Vec<Effect> {
        let Some(info) = settings::setting(key) else {
            return Vec::new();
        };
        let value = match info.kind {
            SettingKind::Number { .. } => match text.trim().parse::<i64>() {
                Ok(n) => toml::Value::Integer(n),
                Err(_) => {
                    return vec![self.say(Message::error(format!(
                        "{} needs a whole number. Nothing changed.",
                        info.name
                    )))];
                }
            },
            SettingKind::Columns | SettingKind::DriveKinds | SettingKind::Names => {
                toml::Value::Array(
                    text.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| toml::Value::String(s.to_owned()))
                        .collect(),
                )
            }
            _ => toml::Value::String(text.to_owned()),
        };
        self.apply_setting(key, Some(value))
    }

    /// Sets a setting in memory, checked by the schema, and asks the
    /// runtime to write it; `None` puts it back to its default. Says
    /// "Layout: stacked" or "Layout back to side by side".
    pub(super) fn apply_setting(&mut self, key: &str, value: Option<toml::Value>) -> Vec<Effect> {
        let Some(info) = settings::setting(key) else {
            return Vec::new();
        };
        let new_value = match &value {
            Some(value) => value.clone(),
            None => match info.default_value() {
                Some(default) => default,
                None => return Vec::new(),
            },
        };
        let mut table = match toml::Value::try_from(&self.settings) {
            Ok(toml::Value::Table(table)) => table,
            _ => return Vec::new(),
        };
        set_dotted(&mut table, key, new_value);
        let text = toml::to_string(&table).unwrap_or_default();
        let loaded = match diroxide_config::Settings::read(&text) {
            Ok(loaded) => loaded,
            Err(error) => return vec![self.say(Message::error(error.to_string()))],
        };
        if let Some(problem) = loaded.problems.iter().find(|p| p.text.starts_with(key)) {
            let line = format!("{} Nothing changed.", problem.text);
            return vec![self.say(Message::error(line))];
        }
        self.settings = loaded.value;
        self.after_setting(key);
        self.request(Request::SaveSetting {
            key: key.to_owned(),
            value: value.clone(),
        });
        // The look's settings say what they do (lane A6): the theme with
        // its contrast verdict, the same line as Next theme's, and focus
        // mode with what it does in the window.
        let message = match self.look_setting_line(key) {
            Some(line) => Message::info(line),
            None => {
                let words = self.settings.value_words(key);
                match value {
                    Some(_) => crate::messages::setting_changed(info.name, &words),
                    None => crate::messages::setting_reset(info.name, &words),
                }
            }
        };
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(message)];
        if let Some(Purpose::Settings) = self.opens.last().map(|o| &o.purpose) {
            let list = settings::settings_list_in(&self.settings, self.themes());
            if let Some(Dialog::Choose { list: shown, .. }) =
                self.dialogs.last_mut().map(|(_, d)| d)
            {
                let focus = shown.focused().map(|r| r.id.clone());
                *shown = list;
                if let Some(focus) = focus {
                    shown.focus_id(&focus);
                }
            }
            effects.push(Effect::Redraw(Redraw::All));
        }
        effects
    }

    /// What a changed setting changes at once.
    fn after_setting(&mut self, key: &str) {
        match key {
            // Built again from the layers, so the owner's lines stay.
            "single_letters" => self.rebuild_keymap(),
            "screen_reader_mode" => {
                self.screen_reader = !matches!(
                    self.settings.screen_reader_mode,
                    diroxide_config::ScreenReaderMode::Off
                );
            }
            // The file types (lane TA): the retro pack joins or leaves,
            // or a folder is typed by name alone from now on; the rows
            // are typed again and checked again.
            "filetypes.retro" => {
                let retro = self.settings.filetypes.retro;
                self.types = std::sync::Arc::new(self.types.as_ref().clone().with_retro(retro));
                self.retype_panels();
            }
            "check.skip" => self.retype_panels(),
            _ => {}
        }
    }

    /// Change layout: side by side, stacked, single panel, and round.
    pub(super) fn cycle_layout(&mut self) -> Vec<Effect> {
        let next = match self.settings.layout {
            Layout::SideBySide => "stacked",
            Layout::Stacked => "single",
            Layout::Single => "side_by_side",
        };
        self.apply_setting("layout", Some(toml::Value::String(next.to_owned())))
    }

    /// Next theme: the next of the themes, the built-in ones and then the
    /// owner's theme files, its contrast said. The setting's own line is
    /// the theme's with its verdict (see `look_setting_line`), so Next
    /// theme and the Settings list say the same words.
    pub(super) fn next_theme(&mut self) -> Vec<Effect> {
        let names = self.theme_names();
        let at = names.iter().position(|n| *n == self.settings.theme);
        let next = names
            .get(at.map_or(0, |i| (i + 1) % names.len().max(1)))
            .cloned()
            .unwrap_or_default();
        self.apply_setting("theme", Some(toml::Value::String(next)))
    }

    pub(super) fn list_problems(&mut self) -> Vec<Effect> {
        if self.problems.is_empty() {
            return vec![self.say(Message::info("No config problems."))];
        }
        let lines: Vec<String> = self
            .problems
            .iter()
            .map(|p| format!("{}: {}", p.file, p.text))
            .collect();
        let title = format!(
            "Config problems, {}",
            count(lines.len() as u64, "problem", "problems")
        );
        let list = read_list(title, text_rows(lines));
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::PROBLEMS),
            },
            Purpose::Read,
        )
    }

    /// One finished job's failures, skips and warnings with their reasons,
    /// by its place in the history: Show what failed on its row in the
    /// Jobs list, whose Finished group is the job history (lane A2).
    pub(super) fn job_items(&mut self, index: usize) -> Vec<Effect> {
        let Some(job) = self.jobs.history().get(index).cloned() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        lines.extend(job.failed.iter().map(|l| format!("Failed: {l}")));
        lines.extend(job.skipped.iter().map(|l| format!("Skipped: {l}")));
        lines.extend(job.warnings.iter().map(|l| format!("Warning: {l}")));
        if lines.is_empty() {
            lines.push("Nothing failed or was skipped.".to_owned());
        }
        let list = read_list(job.summary, text_rows(lines));
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::JOB_HISTORY),
            },
            Purpose::JobItems,
        )
    }

    /// The key test: every key is said with what it sends and what it
    /// would run, until Cancel's key.
    pub(super) fn start_key_test(&mut self) -> Vec<Effect> {
        self.key_test = true;
        let cancel = self.key(&ids::CANCEL);
        let text = if cancel.is_empty() {
            "Key test: press keys to hear what each sends and runs.".to_owned()
        } else {
            format!("Key test: press keys to hear what each sends and runs. {cancel} ends it.")
        };
        vec![self.say(Message::info(text))]
    }

    pub(super) fn key_test_press(&mut self, key: Key) -> Vec<Effect> {
        let layers = if self.viewer_open() {
            [Layer::Viewer, Layer::List, Layer::Global]
        } else {
            [Layer::Panel, Layer::List, Layer::Global]
        };
        let mut text = keymap::key_test_everywhere(&self.keymap, &key, self.viewer_open());
        // AltGr on Windows arrives as Control plus Alt (crossterm issue
        // 820): such a key types its character in a text field.
        if key.mods.ctrl
            && key.mods.alt
            && let Some(c) = super::typed(&key)
        {
            text.push_str(&format!(" In a text field it types {c}."));
        }
        let ends = matches!(
            self.keymap.lookup(&layers, &key),
            keymap::Lookup::Command(id) if id == ids::CANCEL || id == ids::CLOSE_VIEWER
        );
        if ends {
            self.key_test = false;
            text.push_str(" Key test ended.");
        }
        vec![self.say(Message::info(text))]
    }

    /// Ends the key test when no key came for a while: the frontend calls
    /// this after ten seconds without a key (research/alpha3-plan.md, the
    /// key test), so a key the terminal never sends cannot trap the user.
    pub fn end_key_test(&mut self, idle_seconds: u64) -> Vec<Effect> {
        if self.capturing_key() {
            return self.capture_timed_out();
        }
        if !self.key_test {
            return Vec::new();
        }
        self.key_test = false;
        let text = format!(
            "No key for {idle_seconds} seconds. Key test ended. The terminal may keep the key you tried."
        );
        vec![self.say(Message::info(text))]
    }

    /// The run-command prompt: a command with `text` runs it; otherwise the
    /// prompt opens with its history.
    pub(super) fn run_command_prompt(&mut self, command: &Command) -> Vec<Effect> {
        let side = self.active;
        if let Some(line) = command.text("text") {
            let line = line.to_owned();
            return self.run_line(side, &line);
        }
        let label = format!("Run in {}", place_leaf(self.panel(side).place()));
        let field = EntryField::new(label, CompletionSource::History)
            .with_history(state::keys::RUN_COMMAND);
        self.open_dialog(Dialog::edit(field), Purpose::RunCommand(side))
    }

    /// Shell here, in the active panel's folder. The terminal hands its
    /// screen over, so its line says how to come back first; the GUI's
    /// shell runs in a terminal window of its own while diroxide stays,
    /// so the GUI hears only "Started a shell in work" when it has (lane
    /// A5): the app knows the keymap column it runs with.
    pub(super) fn shell_here(&mut self) -> Vec<Effect> {
        let side = self.active;
        let spawn = self.spawn_for(side, SpawnKind::Shell);
        self.request(Request::Spawn(Box::new(spawn)));
        if self.keymap.column() == KeyColumn::Gui {
            return Vec::new();
        }
        vec![self.say(Message::info("Shell here: exit the shell to come back."))]
    }

    /// F4: the focused file in the editor, the screen handed over.
    pub(super) fn edit_file(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if entry.is_folder_like() {
            let text = format!("Not a file: {}", entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        let spawn = self.spawn_for(side, SpawnKind::Editor(place));
        self.request(Request::Spawn(Box::new(spawn)));
        Vec::new()
    }

    /// A program that had the terminal ended: "Back in diroxide,
    /// Documents", and both panels read again.
    pub(super) fn program_ended(
        &mut self,
        what: &str,
        folder: &Place,
        result: Result<Option<i32>, String>,
    ) -> Vec<Effect> {
        let leaf = place_leaf(folder);
        let message = match result {
            Ok(Some(0) | None) => Message::info(format!("Back in diroxide, {leaf}")),
            Ok(Some(code)) => Message::info(format!(
                "Back in diroxide, {leaf}. {what} ended with code {code}."
            )),
            Err(_) => Message::could_not(
                "start",
                what,
                "the program was not found or could not run",
                "Check the command, or the shell and editor settings",
            ),
        };
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(message)];
        for side in Side::BOTH {
            let place = self.panel(side).place().clone();
            let focus = self.panel(side).list().focused().map(|r| r.id.clone());
            effects.extend(self.open_as(side, place, focus, crate::panel::Load::Refresh));
        }
        effects
    }

    /// A program started without waiting for it (lane A5), the GUI's
    /// handover: "Started a shell in work", "Started the command in
    /// work", "Started the editor on notes.txt", "Started textweaver";
    /// or why not, with the next step: "Could not start a shell in work:
    /// no terminal program was found. Set the TERMINAL variable to a
    /// terminal program, such as xterm." The panels are not read again:
    /// the program still runs, and the watchers see what it changes.
    pub(super) fn program_started(
        &mut self,
        kind: &SpawnKind,
        folder: &Place,
        result: Result<(), String>,
    ) -> Vec<Effect> {
        let what = match kind {
            SpawnKind::Shell => format!("a shell in {}", place_leaf(folder)),
            SpawnKind::CommandLine(_) => format!("the command in {}", place_leaf(folder)),
            SpawnKind::Editor(file) | SpawnKind::EditorAt { file, .. } => {
                format!("the editor on {}", place_leaf(file))
            }
            SpawnKind::Program { name, .. } => name.clone(),
        };
        let message = match result {
            Ok(()) => Message::info(format!("Started {what}")),
            Err(cause) => {
                let cause = cause.trim();
                let cause = if cause.is_empty() {
                    "the program was not found or could not run"
                } else {
                    cause
                };
                let next = if cause.contains("no terminal program") {
                    "Set the TERMINAL variable to a terminal program, such as xterm"
                } else {
                    "Check the command, or the shell and editor settings"
                };
                Message::could_not("start", &what, cause, next)
            }
        };
        vec![Effect::Redraw(Redraw::All), self.say(message)]
    }

    /// Quit: asks only when a job runs or waits, or a prompt holds text
    /// (the owner's answer 74; R25 N7); the question starts on No. With
    /// `keep_shell`, the shell
    /// that started diroxide stays in its own folder even with
    /// `--cwd-file` (R14's "Quit and keep the shell where it was", named
    /// "Quit, shell stays where it was" to fit a Braille line).
    pub(super) fn ask_quit(&mut self, keep_shell: bool) -> Vec<Effect> {
        self.keep_shell = keep_shell;
        // Running and waiting jobs alike: the queue is saved across a restart
        // since alpha 3 (Q2), and a quit interrupts a running job the way a
        // lost connection does (A4), so a copy cut by it resumes at its
        // offset on Retry where the place keeps part files; the jobs still
        // stop, so the question and its words stay.
        let jobs = if self.settings.confirm.quit_with_jobs {
            self.jobs.len() as u64
        } else {
            0
        };
        let typing = self.dialogs.iter().any(|(_, dialog)| match dialog {
            Dialog::Edit { field, .. } => !field.text().is_empty(),
            Dialog::Secret { typed, .. } => !typed.is_empty(),
            _ => false,
        });
        if jobs == 0 && !typing {
            return self.quit_now();
        }
        if matches!(self.opens.last().map(|o| &o.purpose), Some(Purpose::Quit)) {
            return Vec::new();
        }
        if jobs > 0 {
            // "2 jobs are not finished." first, then the question, which
            // starts on No (R25, 2.2; 24 and 26 cells).
            let fact = format!("{} not finished.", count(jobs, "job is", "jobs are"));
            let question = Message::new(
                "Quit and stop them? No is the default. y or n",
                diroxide_core::Importance::High,
                diroxide_core::MessageKind::Question,
            );
            let mut effects = vec![self.say(Message::info(fact.clone()))];
            effects.extend(self.open_dialog(
                Dialog::Confirm {
                    question,
                    listing: vec![fact],
                    on_yes: Command::new(ids::QUIT),
                },
                Purpose::Quit,
            ));
            return effects;
        }
        let question = Message::new(
            "Quit? A prompt holds text. No is the default. y or n",
            diroxide_core::Importance::High,
            diroxide_core::MessageKind::Question,
        );
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing: Vec::new(),
                on_yes: Command::new(ids::QUIT),
            },
            Purpose::Quit,
        )
    }

    /// Quits: running jobs are stopped (the runtime interrupts them,
    /// keeping a copy's part file for a resume on Retry, lane A4), the
    /// session and the path jump store are saved, and the shell's folder
    /// is noted for `--cwd-file` unless the shell keeps its own.
    pub(super) fn quit_now(&mut self) -> Vec<Effect> {
        let mut effects: Vec<Effect> = self.jobs.ids().into_iter().map(Effect::CancelJob).collect();
        self.quit = true;
        self.quit_folder = if self.keep_shell {
            None
        } else {
            local_folder(self.active_panel().place())
        };
        let session = self.session();
        self.request(Request::SaveSession(Box::new(session)));
        let visits = self.visits.to_lines();
        self.request(Request::SaveHistory {
            key: state::VISITS_KEY.to_owned(),
            entries: visits,
        });
        effects.push(Effect::Quit);
        effects
    }
}

/// Sets a value at a dotted key in a table, making the tables on the way.
fn set_dotted(table: &mut toml::Table, key: &str, value: toml::Value) {
    match key.split_once('.') {
        None => {
            table.insert(key.to_owned(), value);
        }
        Some((first, rest)) => {
            let inner = table
                .entry(first.to_owned())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let toml::Value::Table(inner) = inner {
                set_dotted(inner, rest, value);
            }
        }
    }
}
