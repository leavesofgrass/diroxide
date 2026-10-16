//! What the app says and lists: Say Status, Say Other Panel, the key bar,
//! help, details, the context menu, the menus and the palette, settings
//! with set and reset, the layout and theme, config problems, the job
//! history, the key test, the run-command prompt, shell here, the editor,
//! and quit.

use diroxide_config::{Layout, SettingKind};
use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, Effect, Key, Layer, ListModel, ListRow, Message, Place, PlaceKind, Redraw,
    Side, Timestamp,
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

fn time_words(time: Option<Timestamp>, clock: &dyn diroxide_core::Clock) -> String {
    time.map(|t| {
        let civil = t.to_civil(clock.offset());
        format!("{}, {}", civil.long_date(), civil.clock_time())
    })
    .unwrap_or_else(|| "not known".to_owned())
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
        if let Some(filter) = panel.filter() {
            text.push_str(&format!(", filter {}", filter.text()));
        }
        for job in self.jobs.ids() {
            if let Some(detail) = self.jobs.detail(job) {
                text.push_str(&format!(". {detail}"));
            }
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

    pub(super) fn say_key_bar(&mut self) -> Vec<Effect> {
        let text = self
            .banks
            .first()
            .map(|bank| bank.key_bar_spoken(&self.keymap))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "The key bar is empty.".to_owned());
        vec![self.say(Message::info(text))]
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
        let mut rows = vec![ListRow::new("help", first)];
        for info in CATALOG
            .iter()
            .filter(|i| matches!(i.layer, Layer::Panel | Layer::List | Layer::Global))
        {
            let key = self.keymap.key_text(&info.id);
            let row = ListRow::new(info.id.as_str(), info.name);
            rows.push(if key.is_empty() {
                row.with_details([info.help])
            } else {
                row.with_details([key, info.help.to_owned()])
            });
        }
        let mut list = ListModel::new(format!("Help, {title}")).with_rows(rows);
        list.set_help("Up and Down read, Enter runs a command, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::HELP),
            },
            Purpose::Read,
        )
    }

    /// Details of the focused entry, one fact a row, each said first.
    pub(super) fn details(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let clock = self.clock.as_ref();
        let name = entry.display_name().into_owned();
        let mut lines = vec![format!("Name: {name}")];
        if entry.needs_care {
            let issues = diroxide_vfs::check_name(&entry.name, &panel.names());
            for issue in issues {
                lines.push(issue.line());
            }
            lines.push(format!(
                "Spelled out: {}",
                diroxide_core::names::spell_out(&entry.name)
            ));
        }
        lines.push(format!(
            "Kind: {}",
            entry
                .type_name
                .clone()
                .unwrap_or_else(|| entry.kind.word().to_owned())
        ));
        match (entry.is_folder_like(), entry.size) {
            (true, _) => lines.push("Size: folder size not computed".to_owned()),
            (false, Some(size)) => lines.push(format!(
                "Size: {}, {} bytes",
                diroxide_core::format::human_size(size),
                crate::messages::grouped(size)
            )),
            (false, None) => lines.push("Size: not known".to_owned()),
        }
        lines.push(format!("Modified: {}", time_words(entry.modified, clock)));
        lines.push(format!("Created: {}", time_words(entry.created, clock)));
        lines.push(format!("Accessed: {}", time_words(entry.accessed, clock)));
        if let Some(attributes) = entry.attributes {
            lines.push(format!("Attributes: {}", attributes.words(entry.kind)));
        }
        if let Some(owner) = &entry.owner {
            lines.push(format!("Owner: {owner}"));
        }
        if let Some(group) = &entry.group {
            lines.push(format!("Group: {group}"));
        }
        if let Some(link) = &entry.link {
            lines.push(format!(
                "Link to: {}",
                diroxide_core::names::display_name(&link.target)
            ));
        }
        if entry.hidden {
            lines.push("Hidden: yes".to_owned());
        }
        lines.push(format!(
            "Path: {}",
            diroxide_core::names::spell_out(&place.to_string())
        ));
        let list = read_list(format!("Details of {name}"), text_rows(lines));
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::DETAILS),
            },
            Purpose::Read,
        )
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
        let list = self.palette.list("", &self.keymap);
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
        let Some(list) = self.menus.list(id, &self.keymap, &caps) else {
            return vec![self.say(Message::info(format!("No menu is named {id}.")))];
        };
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::MENU),
            },
            Purpose::Menu(id.to_owned()),
        )
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
        let list = settings::settings_list(&self.settings);
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
            SettingKind::Theme => {
                let mut rows: Vec<ListRow> = settings::theme_names()
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
        let words = self.settings.value_words(key);
        let message = match value {
            Some(_) => crate::messages::setting_changed(info.name, &words),
            None => crate::messages::setting_reset(info.name, &words),
        };
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(message)];
        if let Some(Purpose::Settings) = self.opens.last().map(|o| &o.purpose) {
            let list = settings::settings_list(&self.settings);
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
            "single_letters" => {
                let full = keymap::Keymap::defaults_for(self.keymap.column())
                    .for_platform(self.keymap.platform());
                self.keymap = full.with_single_letters(self.settings.single_letters);
            }
            "screen_reader_mode" => {
                self.screen_reader = !matches!(
                    self.settings.screen_reader_mode,
                    diroxide_config::ScreenReaderMode::Off
                );
            }
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

    /// Next theme: the next of the built-in themes, its contrast said.
    pub(super) fn next_theme(&mut self) -> Vec<Effect> {
        let names = settings::theme_names();
        let at = names.iter().position(|n| *n == self.settings.theme);
        let next = names
            .get(at.map_or(0, |i| (i + 1) % names.len().max(1)))
            .cloned()
            .unwrap_or_default();
        let mut effects = self.apply_setting("theme", Some(toml::Value::String(next.clone())));
        // One line, not two: the theme's own line with its contrast
        // replaces "Theme: galaxy-light".
        if effects.iter().any(|e| matches!(e, Effect::Announce(_))) {
            effects.retain(|e| !matches!(e, Effect::Announce(_)));
            self.status_log.pop();
        }
        let line = settings::theme_line(&self.settings, None);
        effects.push(self.say(Message::info(line)));
        effects
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

    /// The job history, newest first; Enter lists a job's failures, skips
    /// and warnings with their reasons.
    pub(super) fn job_history(&mut self) -> Vec<Effect> {
        let history = self.jobs.history();
        if history.is_empty() {
            return vec![self.say(Message::info("No jobs have run yet."))];
        }
        let rows = history
            .iter()
            .enumerate()
            .rev()
            .map(|(i, job)| ListRow::new(i.to_string(), job.summary.clone()))
            .collect();
        let mut list = ListModel::new("Job history").with_rows(rows);
        list.set_help("Up and Down choose, Enter lists what failed and why, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::JOB_HISTORY),
            },
            Purpose::JobHistory,
        )
    }

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
        let layers = [Layer::Panel, Layer::List, Layer::Global];
        let mut text = keymap::key_test(&self.keymap, &layers, &key);
        // AltGr on Windows arrives as Control plus Alt (crossterm issue
        // 820): such a key types its character in a text field.
        if key.mods.ctrl
            && key.mods.alt
            && let Some(c) = super::typed(&key)
        {
            text.push_str(&format!(" In a text field it types {c}."));
        }
        if self.keymap.lookup(&layers, &key) == keymap::Lookup::Command(ids::CANCEL) {
            self.key_test = false;
            text.push_str(" Key test ended.");
        }
        vec![self.say(Message::info(text))]
    }

    /// Ends the key test when no key came for a while: the frontend calls
    /// this after ten seconds without a key (research/alpha3-plan.md, the
    /// key test), so a key the terminal never sends cannot trap the user.
    pub fn end_key_test(&mut self, idle_seconds: u64) -> Vec<Effect> {
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

    pub(super) fn shell_here(&mut self) -> Vec<Effect> {
        let side = self.active;
        let spawn = self.spawn_for(side, SpawnKind::Shell);
        self.request(Request::Spawn(Box::new(spawn)));
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

    /// Quit: asks only when a job runs or a prompt holds text (the owner's
    /// answer 74); the question starts on No. With `keep_shell`, the shell
    /// that started diroxide stays in its own folder even with
    /// `--cwd-file` (R14's "Quit and keep the shell where it was", named
    /// "Quit, shell stays where it was" to fit a Braille line).
    pub(super) fn ask_quit(&mut self, keep_shell: bool) -> Vec<Effect> {
        self.keep_shell = keep_shell;
        let jobs = self.jobs.len() as u64;
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
        let why = if jobs > 0 {
            format!("{} running", count(jobs, "job is", "jobs are"))
        } else {
            "a prompt holds text".to_owned()
        };
        let question = Message::new(
            format!(
                "Quit? {}. No is the default. y or n",
                super::capitalized(&why)
            ),
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

    /// Quits: running jobs are cancelled, the session and the path jump
    /// store are saved, and the shell's folder is noted for `--cwd-file`
    /// unless the shell keeps its own.
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
