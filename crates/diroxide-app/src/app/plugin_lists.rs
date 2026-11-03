//! A plugin's list inside a panel (lane L; R22 commit 39 and its N1): a
//! lister a plugin opens with `lister/open` shows in the panel the plugin
//! names, over the panel's place, and the focus moves to that panel so it
//! is heard. The place beneath is untouched (its rows, marks, history and
//! watch) and shows again when the list is left; the list is never saved
//! in the session or the history.
//!
//! What the owner hears (every line meaning first, the plugin's name
//! last, answer 90):
//!
//! - On open, the landing line: "Changed files, 2 rows, left panel
//!   (Changed files)", with the cap sentence when rows were cut.
//! - Rows as the list model says them ("b.txt, state Modified, lines 12,
//!   2 of 2"); more rows are asked for at the end.
//! - Space says "Marked" or "Unmarked"; Enter runs the lister's
//!   `activate` command as a job on the marked rows or the focused one;
//!   the manifest's action keys run theirs.
//! - Every file command refuses with "This row is not a file." before a
//!   file is looked for, and so do the commands that work on the folder
//!   beneath (its filter, its labels, its views, select by pattern).
//! - The Sort commands offer the lister's columns, and the filter field
//!   matches text in every column, not the filter language.
//! - Left, Backspace and Alt+Up (`panel.up`) leave the list; Escape leaves
//!   it when no job or list filter is there to cancel; Back and Forward
//!   only leave it and act on the history at the next press. Leaving tells
//!   the plugin (`lister/closed`) and says "Plugin list closed (Changed
//!   files)", then the folder's landing line.
//! - A place opened in the panel closes the list: a plugin's `panel/set`
//!   quietly (the Sites list's Connect lands in the folder), the owner's
//!   Go to a path, Home or a place list with the closing line.

use diroxide_core::format::count;
use diroxide_core::{Command, CommandId, Effect, Key, Layer, Message, Move, Redraw, Side};
use diroxide_plugins::lines;
use diroxide_plugins::listers::ListerView;
use diroxide_plugins::manifest::Selection;

use super::plugin_ui::{Refresh, Wants, side_of};
use super::{Action, App, Purpose, typed};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::Lookup;
use crate::plugins::lister::{Lister, PanelList};
use crate::plugins::{PluginRequest, registry};
use crate::request::Request;

/// The commands a plugin list refuses with "This row is not a file.":
/// every one that acts on files, and those that work on the folder
/// beneath the list, which the owner cannot see while the list is up.
fn acts_on_files(action: Action) -> bool {
    matches!(
        action,
        Action::View
            | Action::Edit
            | Action::Copy
            | Action::Move
            | Action::CopyTo
            | Action::MoveTo
            | Action::MakeFolder
            | Action::Trash
            | Action::DeletePermanently
            | Action::Rename
            | Action::Duplicate
            | Action::Details
            | Action::ContextMenu
            | Action::RunCommand
            | Action::ClipCopy
            | Action::ClipCut
            | Action::Paste
            | Action::Pack
            | Action::Unpack
            | Action::TestArchive
            | Action::Split
            | Action::Join
            | Action::MakeLink
            | Action::Relink
            | Action::Permissions
            | Action::OpenAsArchive
            | Action::CopyPath
            | Action::CopyName
            | Action::ShowInOther
            | Action::SelectPattern
            | Action::UnselectPattern
            | Action::SelectRestore
            | Action::ToggleHidden
            | Action::QuickView
            | Action::OpenInTextweaver
    )
}

/// Whether a command outside the panel's own routes runs while a plugin
/// list is up: the menus, the dialogs' answers, the keys, the jobs, the
/// plugins and the components, and the lists that open over the panels
/// (the trash, the volumes, the bookmarks, the labels, the history, find
/// and the saved searches). The rest work on the files or the folder
/// beneath, and are refused in words.
fn passes(id: &CommandId) -> bool {
    let space = id.as_str().split('.').next().unwrap_or_default();
    matches!(
        space,
        "app"
            | "menu"
            | "dialog"
            | "keys"
            | "jobs"
            | "plugins"
            | "components"
            | "trash"
            | "viewer"
            | "preview"
    ) || [
        ids::UNDO,
        ids::TRASH_BROWSER,
        ids::VOLUMES,
        ids::BOOKMARKS,
        ids::HISTORY_LIST,
        ids::LABELS,
        ids::FIND,
        ids::SEARCH_RESULTS,
        ids::SEARCH_SAVED,
        ids::SEARCH_PAUSE,
    ]
    .contains(id)
}

impl App {
    // ---- The panel's commands and keys in a list ----

    /// A command while a plugin list is up in the active panel: its own
    /// moves, marks, Enter, sort, filter, reload, leaving and Say Status;
    /// a refusal in words for the file commands; `None` for the commands
    /// that run as they always do (the menus, Switch panels, a plugin's own
    /// command, which the catalog refuses on rows that are not files).
    /// Say Other Panel speaks of a list in the other panel too.
    pub(super) fn plugin_list_dispatch(
        &mut self,
        command: &Command,
        action: Option<Action>,
    ) -> Option<Vec<Effect>> {
        if !self.dialogs.is_empty() || self.viewing.is_some() || self.in_preview() {
            return None;
        }
        let side = self.active;
        if action == Some(Action::SayOther) && self.panel(side.other()).plugin_list().is_some() {
            return Some(self.say_other_list(side.other()));
        }
        self.panel(side).plugin_list()?;
        if registry::is_plugin_command(&command.id) || command.id == ids::PLUGINS {
            return None;
        }
        let Some(action) = action else {
            return (!passes(&command.id)).then(|| self.not_a_file());
        };
        Some(match action {
            Action::Focus(movement) => self.list_focus(side, movement),
            Action::ToggleSelect => self.list_mark(side),
            Action::SelectAll => self.list_select(side, |list| list.select_all()),
            Action::SelectNone => self.list_select(side, |list| list.select_none()),
            Action::SelectInvert => self.list_select(side, |list| list.invert_selection()),
            Action::SelectNext => self.list_next_marked(side, true),
            Action::SelectPrevious => self.list_next_marked(side, false),
            Action::Enter => self.list_activate(side),
            Action::Into => self.list_into(side),
            Action::Up | Action::Back | Action::Forward => self.leave_plugin_list(side),
            Action::Reload => self.list_reload(side),
            Action::Sort | Action::SortList => self.list_sort_choices(side),
            Action::Filter => self.list_filter_prompt(side),
            Action::FilterClear => self.list_filter_off(side),
            Action::Cancel => self.list_cancel(side),
            Action::SayStatus => self.list_say_status(side),
            Action::Help => self.list_help(side),
            action if acts_on_files(action) => self.not_a_file(),
            _ => return None,
        })
    }

    /// A key while a plugin list is up in the active panel, before the
    /// keymap: the manifest's action keys run their commands, and a letter
    /// no key takes jumps to the next row that starts with it.
    pub(super) fn plugin_list_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let side = self.active;
        let shown = self.panel(side).plugin_list()?;
        let action = shown.lister().view.spec.actions.iter().find_map(|action| {
            Key::parse(&action.key)
                .ok()
                .filter(|k| k == key)
                .map(|_| (action.command.clone(), action.selection))
        });
        if let Some((local, selection)) = action {
            return Some(self.list_command(side, &local, selection != Selection::None));
        }
        let layers = [Layer::Panel, Layer::List, Layer::Global];
        if matches!(self.keymap.lookup(&layers, key), Lookup::NoMatch)
            && self.settings.single_letters.types_ahead()
            && let Some(letter) = typed(key)
        {
            return Some(self.list_row_letter(side, letter));
        }
        None
    }

    fn not_a_file(&mut self) -> Vec<Effect> {
        vec![self.say(Message::warning(lines::not_a_file()))]
    }

    fn list_focus(&mut self, side: Side, movement: Move) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        let list = shown.list_mut();
        if list.is_empty() {
            let empty = list.empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        }
        let moved = list.move_focus(movement);
        let at_end = list.focus_index().map(|i| i + 1) == Some(list.len());
        let line = list.announce_focus();
        let more = shown.lister().has_more() && !shown.lister().asking;
        let mut effects = if moved {
            vec![
                Effect::Redraw(Redraw::Panel(side)),
                Effect::Announce(Message::status(line)),
            ]
        } else {
            let edge = if matches!(movement, Move::Up | Move::PageUp | Move::Home) {
                "Top of the list"
            } else {
                "Bottom of the list"
            };
            vec![self.say(Message::status(edge))]
        };
        // The end of the rows: more from the plugin, when it has more.
        if at_end && more {
            effects.extend(self.ask_list_rows(side, false));
        }
        effects
    }

    fn list_mark(&mut self, side: Side) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        let list = shown.list_mut();
        let Some(marked) = list.toggle_selected() else {
            let empty = list.empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        list.move_focus(Move::Down);
        let word = if marked { "Marked" } else { "Unmarked" };
        vec![self.redraw_panel(side), self.say(Message::info(word))]
    }

    fn list_select(
        &mut self,
        side: Side,
        change: impl FnOnce(&mut diroxide_core::ListModel),
    ) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        change(shown.list_mut());
        let marked = shown.list().selected_count() as u64;
        let text = if marked == 0 {
            "Nothing selected".to_owned()
        } else {
            format!("{} selected", count(marked, "row", "rows"))
        };
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }

    fn list_next_marked(&mut self, side: Side, forward: bool) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        let list = shown.list_mut();
        let focus = list.focus_index().unwrap_or(0);
        let rows = list.rows();
        let found = if forward {
            (focus + 1..rows.len()).find(|&i| rows[i].selected)
        } else {
            (0..focus).rev().find(|&i| rows[i].selected)
        };
        match found {
            Some(index) => {
                list.move_focus(Move::To(index));
                let line = list.announce_focus();
                vec![
                    Effect::Redraw(Redraw::Panel(side)),
                    Effect::Announce(Message::status(line)),
                ]
            }
            None => {
                let text = if forward {
                    "No selected row below"
                } else {
                    "No selected row above"
                };
                vec![self.say(Message::status(text))]
            }
        }
    }

    /// A letter in a plugin list: the next row that starts with it.
    fn list_row_letter(&mut self, side: Side, letter: char) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        let list = shown.list_mut();
        if list.letter_jump(letter) {
            let line = list.announce_focus();
            return vec![
                Effect::Redraw(Redraw::Panel(side)),
                Effect::Announce(Message::status(line)),
            ];
        }
        vec![self.say(Message::status(format!("No row starts with {letter}")))]
    }

    /// Right in a plugin list: a row is not a folder to go into.
    fn list_into(&mut self, side: Side) -> Vec<Effect> {
        let Some(shown) = self.panel(side).plugin_list() else {
            return Vec::new();
        };
        let text = match shown.list().focused() {
            Some(row) => format!("Not a folder: {}, a plugin list row", row.label),
            None => shown.list().empty_text().to_owned(),
        };
        vec![self.say(Message::info(text))]
    }

    /// Escape in a plugin list: the job in front stops first, then the
    /// list's filter comes off, and with nothing else to cancel the list
    /// is left.
    fn list_cancel(&mut self, side: Side) -> Vec<Effect> {
        if self.jobs.front().is_some() {
            return self.cancel_front_job();
        }
        let filtered = self
            .panel(side)
            .plugin_list()
            .is_some_and(|shown| shown.lister().filter.is_some());
        if filtered {
            return self.list_filtered(side, "");
        }
        self.leave_plugin_list(side)
    }

    /// Say Status in a plugin list: the selection first when there is one,
    /// the list and its count, the focus and its position, its filter and
    /// sort, the plugin's name, then the jobs.
    fn list_say_status(&mut self, side: Side) -> Vec<Effect> {
        let Some(shown) = self.panel(side).plugin_list() else {
            return Vec::new();
        };
        let list = shown.list();
        let selected = list.selected_count();
        let mut text = if selected > 0 {
            format!(
                "{selected} selected. {}, {}",
                shown.bare_title(),
                shown.rows_words()
            )
        } else {
            format!(
                "{}, {}, no selection",
                shown.bare_title(),
                shown.rows_words()
            )
        };
        if let Some(row) = list.focused() {
            text.push_str(&format!(", on {}, {}", row.label, list.position_text()));
        }
        if let Some(filter) = &shown.lister().filter {
            text.push_str(&format!(", filter {filter}"));
        }
        if let Some(sort) = shown.lister().sort_words() {
            text.push_str(&format!(", {sort}"));
        }
        text.push_str(&format!(" ({})", shown.plugin()));
        for job in self.jobs.ids() {
            if let Some(detail) = self.jobs.detail(job) {
                text.push_str(&format!(". {detail}"));
            }
        }
        vec![self.say(Message::info(text))]
    }

    /// Say Other Panel when the other panel shows a plugin list.
    fn say_other_list(&mut self, side: Side) -> Vec<Effect> {
        let Some(shown) = self.panel(side).plugin_list() else {
            return Vec::new();
        };
        let list = shown.list();
        let mut text = format!(
            "Other panel: {}, {}",
            shown.bare_title(),
            shown.rows_words()
        );
        if let Some(row) = list.focused() {
            text.push_str(&format!(", on {}, {}", row.spoken(), list.position_text()));
        }
        text.push_str(&format!(" ({})", shown.plugin()));
        vec![self.say(Message::info(text))]
    }

    /// F1 in a plugin list: its own help first, then the keys.
    fn list_help(&mut self, side: Side) -> Vec<Effect> {
        let first = self
            .panel(side)
            .plugin_list()
            .map(|shown| shown.list().help().to_owned())
            .unwrap_or_default();
        let list = self.keyboard_list(first);
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::HELP),
            },
            Purpose::Read,
        )
    }

    /// What F1 says first in a plugin list, from the live keymap and the
    /// lister's own keys: "Enter runs the plugin's command, Space marks,
    /// Left or Backspace leaves the list, ...".
    fn list_help_text(&self, view: &ListerView) -> String {
        let mut parts = Vec::new();
        for (id, words) in [
            (ids::ENTER, "runs the plugin's command"),
            (ids::SELECT_TOGGLE, "marks"),
            (ids::SORT_LIST, "sorts by its columns"),
            (ids::FILTER, "filters its rows"),
            (ids::RELOAD, "asks for the rows again"),
        ] {
            let key = self.keymap.key_text(&id);
            if !key.is_empty() {
                parts.push(format!("{key} {words}"));
            }
        }
        for action in &view.spec.actions {
            let Ok(key) = Key::parse(&action.key) else {
                continue;
            };
            let name = CommandId::new(format!("{}.{}", view.plugin_id, action.command))
                .ok()
                .and_then(|id| self.plugins.catalog.any_command(&id))
                .map_or_else(|| action.command.clone(), |(_, c)| c.name.clone());
            parts.push(format!(
                "{} {name}",
                crate::keymap::written(&key, self.keymap.platform())
            ));
        }
        let up = self.keymap.keys_text(&ids::UP);
        if !up.is_empty() {
            parts.push(format!("{up} leaves the list"));
        }
        format!("{}.", parts.join(", "))
    }

    // ---- The title line, the status line and the landing words ----

    /// The title line while the active panel shows a plugin list: the
    /// list's title, the panel, its filter and sort, the plugin's name
    /// last: "Changed files, left panel, sorted by lines (Changed files)".
    pub(super) fn plugin_list_title(&self, cells: usize) -> Option<String> {
        let shown = self.active_panel().plugin_list()?;
        let mut text = format!("{}, {} panel", shown.bare_title(), self.active.word());
        if let Some(filter) = &shown.lister().filter {
            text.push_str(&format!(", filter {filter}"));
        }
        if let Some(sort) = shown.lister().sort_words() {
            text.push_str(&format!(", {sort}"));
        }
        text.push_str(&format!(" ({})", shown.plugin()));
        Some(text.chars().take(cells).collect())
    }

    /// The status line with no message while the active panel shows a
    /// plugin list: "Changed files, 2 rows, 1 selected".
    pub(super) fn plugin_list_status_line(&self) -> Option<String> {
        let shown = self.active_panel().plugin_list()?;
        let mut line = format!("{}, {}", shown.bare_title(), shown.rows_words());
        let selected = shown.list().selected_count();
        if selected > 0 {
            line.push_str(&format!(", {selected} selected"));
        }
        Some(line)
    }

    /// A panel as Switch panels says it: a plugin list's title, its count
    /// and its plugin, or `None` for a folder.
    pub(super) fn plugin_list_words(&self, side: Side) -> Option<String> {
        let shown = self.panel(side).plugin_list()?;
        Some(format!(
            "{}, {} ({})",
            shown.bare_title(),
            shown.rows_words(),
            shown.plugin()
        ))
    }

    // ---- Opening, finding and leaving ----

    /// The side whose plugin list is this plugin's lister of this id.
    fn list_side(&self, plugin: &str, lister: &str) -> Option<Side> {
        Side::BOTH.into_iter().find(|side| {
            self.panel(*side)
                .plugin_list()
                .is_some_and(|shown| shown.is(plugin, lister))
        })
    }

    /// A plugin opened a lister (`lister/open`): it shows in the panel the
    /// plugin names, over the place, and the focus moves there with the
    /// landing line. When the same lister is shown already, its rows come
    /// up to date in place, the focus and the marks kept by row id.
    /// `opener` is the command that opened it, and whether that run brings
    /// the rows up to date quietly.
    pub(super) fn open_lister(
        &mut self,
        view: ListerView,
        opener: Option<(CommandId, Refresh)>,
    ) -> Vec<Effect> {
        let silent = opener
            .as_ref()
            .is_some_and(|(_, refresh)| *refresh == Refresh::Silent);
        let opened_by = opener.map(|(command, _)| command);
        if let Some(side) = self.list_side(&view.plugin_id, &view.spec.id) {
            let count = view.rows.len() as u64;
            let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
                return Vec::new();
            };
            shown.change(|lister| {
                lister.view = view;
                lister.asking = false;
                if opened_by.is_some() {
                    lister.opened_by = opened_by;
                }
            });
            if silent {
                return vec![Effect::Redraw(Redraw::Panel(side))];
            }
            let landing = shown.landing_line(side);
            let line = lines::with_source(&count_rows(count), shown.plugin());
            if side != self.active {
                self.active = side;
                return vec![
                    Effect::Redraw(Redraw::All),
                    self.say(Message::info(landing)),
                ];
            }
            return vec![
                Effect::Redraw(Redraw::Panel(side)),
                Effect::Redraw(Redraw::TitleLine),
                self.say(Message::info(line)),
            ];
        }
        let side = side_of(self.active, view.panel);
        let help = self.list_help_text(&view);
        let mut lister = Lister::new(view);
        lister.opened_by = opened_by;
        let page = self.panel(side).list().page_size();
        let shown = PanelList::new(lister, help, page);
        let mut line = shown.landing_line(side);
        if let Some(capped) = shown.lister().view.capped_line() {
            line.push_str(". ");
            line.push_str(&capped);
        }
        if let Some(old) = self.panel_mut(side).show_plugin_list(shown) {
            self.tell_closed(&old);
        }
        self.quick = None;
        self.active = side;
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
    }

    fn tell_closed(&mut self, old: &PanelList) {
        self.request(Request::Plugins(Box::new(PluginRequest::ListerClosed {
            plugin: old.plugin_id().to_owned(),
            lister: old.id().to_owned(),
        })));
    }

    /// Takes a panel's plugin list away, telling its plugin
    /// (`lister/closed`); `said` says "Plugin list closed (Changed
    /// files)". Nothing when the panel shows none.
    pub(super) fn close_plugin_list(&mut self, side: Side, said: bool) -> Vec<Effect> {
        let Some(old) = self.panel_mut(side).take_plugin_list() else {
            return Vec::new();
        };
        self.tell_closed(&old);
        self.quick = None;
        let mut effects = vec![Effect::Redraw(Redraw::Panel(side))];
        if side == self.active {
            effects.push(Effect::Redraw(Redraw::TitleLine));
        }
        if said {
            let line = lines::with_source("Plugin list closed", old.plugin());
            effects.push(self.say(Message::info(line)));
        }
        effects
    }

    /// Leaves a plugin list: it closes, and the folder beneath is said as
    /// when it was landed in.
    fn leave_plugin_list(&mut self, side: Side) -> Vec<Effect> {
        let mut effects = self.close_plugin_list(side, true);
        let line = self.folder_landing(side);
        effects.push(Effect::Redraw(Redraw::All));
        effects.push(self.say(Message::info(line)));
        effects
    }

    // ---- Sort, filter and rows ----

    /// Changes a panel's plugin list and says the change, then the focus:
    /// "Sorted by lines. b.txt, state Modified, lines 12, 2 of 2".
    fn list_change(
        &mut self,
        side: Side,
        change: impl FnOnce(&mut Lister) -> String,
    ) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        let sentence = shown.change(change);
        let line = format!("{sentence}. {}", shown.list().announce_focus());
        vec![
            Effect::Redraw(Redraw::Panel(side)),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    fn list_sort_choices(&mut self, side: Side) -> Vec<Effect> {
        let Some(list) = self
            .panel(side)
            .plugin_list()
            .map(|shown| shown.lister().sort_list())
        else {
            return Vec::new();
        };
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::ListerSort { side })),
        )
    }

    /// A column chosen in the Sort list of a plugin list.
    pub(super) fn list_sorted(&mut self, side: Side, column: &str) -> Vec<Effect> {
        self.list_change(side, |lister| lister.sort_by(column))
    }

    fn list_filter_prompt(&mut self, side: Side) -> Vec<Effect> {
        let Some(text) = self
            .panel(side)
            .plugin_list()
            .map(|shown| shown.lister().filter.clone().unwrap_or_default())
        else {
            return Vec::new();
        };
        let field = EntryField::new("Filter the rows by", CompletionSource::None).with_text(text);
        self.open_dialog(
            Dialog::edit(field),
            Purpose::Plugin(Box::new(Wants::ListerFilter { side })),
        )
    }

    /// The filter typed for a plugin list: text found in any column; empty
    /// text takes it off.
    pub(super) fn list_filtered(&mut self, side: Side, text: &str) -> Vec<Effect> {
        let filter = text.trim().to_owned();
        self.list_change(side, move |lister| {
            if filter.is_empty() {
                lister.filter = None;
                "Filter off".to_owned()
            } else {
                let said = format!("Filter {filter}");
                lister.filter = Some(filter);
                said
            }
        })
    }

    fn list_filter_off(&mut self, side: Side) -> Vec<Effect> {
        let filtered = self
            .panel(side)
            .plugin_list()
            .is_some_and(|shown| shown.lister().filter.is_some());
        if filtered {
            return self.list_filtered(side, "");
        }
        vec![self.say(Message::status("No filter"))]
    }

    /// Asks the plugin for more rows, or for all of them again.
    fn ask_list_rows(&mut self, side: Side, refresh: bool) -> Vec<Effect> {
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        let offset = if refresh {
            0
        } else {
            shown.lister().view.rows.len() as u64
        };
        shown.change(|lister| lister.asking = true);
        let request = PluginRequest::ListerRows {
            plugin: shown.plugin_id().to_owned(),
            lister: shown.id().to_owned(),
            offset,
            refresh,
        };
        let words = if refresh {
            "Asking for the rows again"
        } else {
            "Asking for more rows"
        };
        let line = lines::with_source(words, shown.plugin());
        self.request(Request::Plugins(Box::new(request)));
        vec![self.say(Message::status(line))]
    }

    /// Rows a plugin sent for its open list: added after the ones shown,
    /// or in their place; after one of the list's own commands, quietly.
    pub(super) fn lister_rows_came(
        &mut self,
        plugin: &str,
        lister: &str,
        rows: Vec<diroxide_core::Entry>,
        refresh: bool,
    ) -> Vec<Effect> {
        let Some(side) = self.list_side(plugin, lister) else {
            return Vec::new();
        };
        let Some(shown) = self.panel_mut(side).plugin_list_mut() else {
            return Vec::new();
        };
        if refresh && shown.lister().quiet_refresh {
            shown.change(|held| {
                held.quiet_refresh = false;
                held.take_rows(rows, true);
            });
            return vec![Effect::Redraw(Redraw::Panel(side))];
        }
        let added = shown.change(|held| held.take_rows(rows, refresh)) as u64;
        let words = if refresh {
            count_rows(added)
        } else {
            format!("{} more", count_rows(added))
        };
        let line = lines::with_source(&words, shown.plugin());
        vec![
            Effect::Redraw(Redraw::Panel(side)),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    /// The reload key in a plugin list: every row asked for again, the
    /// new count said.
    fn list_reload(&mut self, side: Side) -> Vec<Effect> {
        let Some((plugin, lister)) = self
            .panel(side)
            .plugin_list()
            .map(|shown| (shown.plugin_id().to_owned(), shown.id().to_owned()))
        else {
            return Vec::new();
        };
        self.refresh_lister(&plugin, &lister, true)
    }

    /// Brings an open list's rows up to date, asking its plugin for all of
    /// them again (`lister/rows`); a plugin without that method has the
    /// list's opening command run again quietly instead
    /// ([`App::lister_no_rows`]). `said` says the new count, for the
    /// owner's reload; otherwise nothing is said. Nothing when the list is
    /// not open.
    pub(super) fn refresh_lister(&mut self, plugin: &str, lister: &str, said: bool) -> Vec<Effect> {
        let Some(side) = self.list_side(plugin, lister) else {
            return Vec::new();
        };
        if let Some(shown) = self.panel_mut(side).plugin_list_mut() {
            shown.change(|held| held.quiet_refresh = !said);
        }
        if said {
            return self.ask_list_rows(side, true);
        }
        self.request(Request::Plugins(Box::new(PluginRequest::ListerRows {
            plugin: plugin.to_owned(),
            lister: lister.to_owned(),
            offset: 0,
            refresh: true,
        })));
        Vec::new()
    }

    /// The plugin has no `lister/rows`: its list's opening command runs
    /// again quietly, and its new rows replace the old in place, the focus
    /// and the marks kept (the SFTP plugin's Sites list, after New site or
    /// a lost connection).
    pub(super) fn lister_no_rows(&mut self, plugin: &str, lister: &str) -> Vec<Effect> {
        let Some(side) = self.list_side(plugin, lister) else {
            return Vec::new();
        };
        let Some(held) = self
            .panel(side)
            .plugin_list()
            .map(|shown| shown.lister().clone())
        else {
            return Vec::new();
        };
        // One refresh at a time: a run of the opener still going will
        // bring the rows.
        if let Some(command) = &held.opened_by
            && self.plugin_refresh_running(command)
        {
            return Vec::new();
        }
        let entry = held
            .opened_by
            .as_ref()
            .and_then(|command| self.plugins.catalog.any_command(command))
            .map(|(_, entry)| entry.clone());
        let Some(entry) = entry else {
            let line = lines::with_source("These rows cannot be asked again", &held.view.plugin);
            return vec![self.say(Message::info(line))];
        };
        let context = self.plugin_context();
        let refresh = if held.quiet_refresh {
            Refresh::Silent
        } else {
            Refresh::Said
        };
        self.start_plugin_run_with(&entry, context, None, false, refresh)
    }

    /// Brings every open list of a plugin up to date (a lost connection
    /// turns a site's row to "lost").
    pub(super) fn refresh_listers_of(&mut self, plugin: &str) -> Vec<Effect> {
        let open: Vec<String> = Side::BOTH
            .into_iter()
            .filter_map(|side| self.panel(side).plugin_list())
            .filter(|shown| shown.plugin_id() == plugin)
            .map(|shown| shown.id().to_owned())
            .collect();
        let mut effects = Vec::new();
        for lister in open {
            effects.extend(self.refresh_lister(plugin, &lister, false));
        }
        effects
    }

    // ---- Enter and the action keys ----

    /// Enter in a plugin list: the lister's `activate` command, as a job,
    /// with the marked rows or the focused one.
    fn list_activate(&mut self, side: Side) -> Vec<Effect> {
        let Some(shown) = self.panel(side).plugin_list() else {
            return Vec::new();
        };
        let Some(local) = shown.lister().view.activate().map(str::to_owned) else {
            let line = lines::with_source("Enter does nothing here", shown.plugin());
            return vec![self.say(Message::info(line))];
        };
        self.list_command(side, &local, true)
    }

    /// Runs one of a list's commands with its chosen rows; `needs_row` is
    /// false for an action that takes no selection (New site in an empty
    /// Sites list), which runs with the focused row when there is one,
    /// and without one otherwise.
    fn list_command(&mut self, side: Side, local: &str, needs_row: bool) -> Vec<Effect> {
        let Some(shown) = self.panel(side).plugin_list() else {
            return Vec::new();
        };
        let rows = shown.chosen();
        if rows.is_empty() && needs_row {
            return vec![self.say(Message::info("No row is chosen"))];
        }
        let lister = shown.id().to_owned();
        let Ok(id) = CommandId::new(format!("{}.{local}", shown.plugin_id())) else {
            return Vec::new();
        };
        let entry = self
            .plugins
            .catalog
            .any_command(&id)
            .map(|(_, entry)| entry.clone());
        let Some(entry) = entry else {
            let text = format!("No enabled plugin has the command {id}.");
            return vec![self.say(Message::info(text))];
        };
        let context = self.plugin_context();
        self.start_plugin_run(&entry, context, Some((lister, rows)), false)
    }
}

fn count_rows(rows: u64) -> String {
    count(rows, "row", "rows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_commands_are_refused_and_the_menus_pass() {
        assert!(acts_on_files(Action::Trash));
        assert!(acts_on_files(Action::MakeFolder));
        assert!(!acts_on_files(Action::Focus(Move::Down)));
        assert!(!acts_on_files(Action::SwitchPanel));
        assert!(passes(&ids::MENU_FILE));
        assert!(passes(&ids::JOBS_LIST));
        assert!(passes(&ids::FIND));
        assert!(!passes(&ids::LABEL_SET));
        assert!(!passes(&ids::SELECT_BY_KIND));
        assert!(!passes(&ids::WATCH_FOLDER));
    }
}
