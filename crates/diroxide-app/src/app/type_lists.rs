//! The Groups list, Open with and the theme words of file types (alpha
//! 4, lane TA, R30 commit 31; R28 4.3).
//!
//! - **Groups** lists the file groups, the built-in ones and the owner's
//!   own from `filetypes.toml`, each with its sentence and how many files
//!   of it are here ("Images: 41 files here", 21 cells), under the line
//!   "Groups: 8 built in, 2 yours"; Enter selects that group's files, as
//!   Select by kind does, and the last row turns the retro types on or
//!   off ("Retro types: off", 16).
//! - **Open with** lists the programs the system has for the focused file
//!   ("Open report.pdf with:", 21): "Default program" first, each program
//!   by name, "Choose another program" last where the system has its own
//!   dialog. Enter starts the program through the runtime, never a shell
//!   string; the list is read off the input thread.
//! - **Theme words:** a row's type and group are always said in words;
//!   a theme file may also map a group to one of its existing color roles
//!   (`[group-colors] images = "heading2"`), so no new color enters the
//!   contrast check. The app answers a row's role ([`App::row_role`]);
//!   drawing it is the frontends'.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use diroxide_core::format::count;
use diroxide_core::{
    Command, Effect, Entry, EntryKind, ListModel, ListRow, Message, Place, Redraw, Side,
};
use diroxide_filetype::retro_line;
use diroxide_preview::encoding::{Choice, Chosen, ENCODING_LIST, listed};
use diroxide_theme::ColorRole;
use diroxide_vfs::OpenWithList;

use crate::typing::OpenWithRows;

use super::type_actions::TypeWants;
use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::request::Request;

/// The Groups list's row that turns the retro types on or off.
pub(super) const RETRO_ROW: &str = "retro";

/// Open with's row for the system's own choice.
const DEFAULT_ROW: &str = "default";

/// Open with's row for the system's own dialog.
const CHOOSE_ROW: &str = "choose";

/// Open with's row for a listed program: its place in the list.
const APP_ROW: &str = "app:";

/// The encoding chooser's first row: the automatic choice.
const AUTOMATIC_ROW: &str = "automatic";

/// A theme file's table mapping groups to color roles.
const GROUP_COLORS: &str = "group-colors";

/// "41 files here", "1 file here", "no files here".
fn files_here(n: usize) -> String {
    let files = if n > 1 {
        format!("{} files", crate::messages::grouped(n as u64))
    } else {
        count(n as u64, "file", "files")
    };
    format!("{files} here")
}

impl App {
    /// The commands of this module; `None` for commands that are not
    /// theirs.
    pub(super) fn type_lists_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let ours = [ids::GROUPS_LIST, ids::OPEN_WITH, ids::ENCODING];
        if !ours.contains(&command.id) {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(if command.id == ids::GROUPS_LIST {
            self.groups_list(self.active)
        } else if command.id == ids::ENCODING {
            self.encoding_list()
        } else {
            self.ask_open_with()
        })
    }

    /// The Groups list: each group with how many files of it are here.
    fn groups_list(&mut self, side: Side) -> Vec<Effect> {
        let clock = Arc::clone(&self.clock);
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in self.panel(side).entries() {
            if entry.is_folder_like() {
                continue;
            }
            if let Ok(groups) = self.types.groups_of(entry, clock.as_ref()) {
                for group in groups {
                    *counts.entry(group.id.as_str()).or_default() += 1;
                }
            }
        }
        let mut rows: Vec<ListRow> = self
            .types
            .groups()
            .iter()
            .map(|group| {
                let n = counts.get(group.id.as_str()).copied().unwrap_or(0);
                let mut row = ListRow::new(
                    format!("group {}", super::selecting::group_word(&group.id)),
                    format!("{}: {}", group.name, files_here(n)),
                )
                .with_details([group.about.clone()]);
                if group.yours {
                    row.details.push("yours".to_owned());
                }
                row
            })
            .collect();
        let retro = self.types.retro();
        let turn = if retro {
            "Enter turns them off"
        } else {
            "Enter turns them on"
        };
        rows.push(ListRow::new(RETRO_ROW, retro_line(retro)).with_details([turn]));
        let mut list = ListModel::new(self.types.groups_line()).with_rows(rows);
        list.set_help("Up and Down choose, Enter selects the group's files here, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SELECT_BY_KIND),
            },
            Purpose::SelectKind(side),
        )
    }

    /// The retro row of the Groups list: the retro types on or off.
    pub(super) fn toggle_retro(&mut self) -> Vec<Effect> {
        let mut effects = self.close_top();
        let on = !self.settings.filetypes.retro;
        effects.extend(self.apply_setting("filetypes.retro", Some(toml::Value::Boolean(on))));
        // The setting's own line gives way to the type model's words.
        effects.retain(|e| !matches!(e, Effect::Announce(_)));
        self.status_log.pop();
        effects.push(self.say(Message::info(retro_line(on))));
        effects
    }

    /// Open with: the focused file's programs, read off the input thread.
    fn ask_open_with(&mut self) -> Vec<Effect> {
        let panel = self.active_panel();
        let Some(entry) = panel.focused_entry().cloned() else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let Some(place) = panel.focused_place() else {
            return Vec::new();
        };
        if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
            let line = format!("Not a file: {}", entry.display_name());
            return vec![self.say(Message::info(line))];
        }
        if place.local_path().is_none() {
            let line = Message::could_not(
                "open",
                &format!("{} with another program", entry.display_name()),
                "only files on this computer open in other programs",
                "Copy it out first",
            );
            return vec![self.say(line)];
        }
        let mime = self.focused_mime();
        self.request(Request::ListOpenWith { place, mime });
        let line = format!("Finding programs for {}", entry.display_name());
        vec![self.say(Message::status(line))]
    }

    /// The focused file's media type, from its type.
    fn focused_mime(&self) -> Option<String> {
        let entry = self.active_panel().focused_entry()?;
        self.types.by_index(entry.type_index)?.mime.clone()
    }

    /// The programs came: the list opens when its file is still the
    /// focused one and nothing else is open.
    pub(super) fn open_with_listed(&mut self, place: Place, list: OpenWithRows) -> Vec<Effect> {
        if self.active_panel().focused_place().as_ref() != Some(&place) || !self.dialogs.is_empty()
        {
            return Vec::new();
        }
        let name = place
            .name()
            .map(diroxide_core::names::display_name)
            .unwrap_or_default()
            .into_owned();
        let mut rows = Vec::new();
        let mut default = ListRow::new(DEFAULT_ROW, OpenWithList::DEFAULT_PROGRAM);
        if let Some(app) = list.rows.iter().find(|a| a.default) {
            default.details.push(app.name.clone());
        }
        rows.push(default);
        for (i, app) in list.rows.iter().enumerate() {
            let mut row = ListRow::new(format!("{APP_ROW}{i}"), app.name.clone());
            if app.default {
                row.details.push("the default".to_owned());
            }
            rows.push(row);
        }
        if list.choose_another {
            rows.push(ListRow::new(CHOOSE_ROW, OpenWithList::CHOOSE_ANOTHER));
        }
        if let Some(line) = &list.unread {
            let mut row = ListRow::new("unread", line.clone());
            row.enabled = false;
            rows.push(row);
        }
        let mut shown = ListModel::new(OpenWithList::title(&name)).with_rows(rows);
        shown.set_help("Up and Down choose, Enter opens the file with it, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list: shown,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Types(
                TypeWants::OpenWith {
                    place,
                    list: Box::new(list),
                },
            ))),
        )
    }

    /// Enter in Open with.
    pub(super) fn open_with_chosen(
        &mut self,
        place: Place,
        list: &OpenWithRows,
        row: &str,
    ) -> Vec<Effect> {
        let mut effects = self.close_all();
        if self.active_panel().focused_place().as_ref() != Some(&place) {
            effects.push(self.say(Message::info("The focus moved; nothing was opened.")));
            return effects;
        }
        let name = place
            .name()
            .map(diroxide_core::names::display_name)
            .unwrap_or_default()
            .into_owned();
        if row == DEFAULT_ROW {
            effects.extend(self.open_by_system());
        } else if row == CHOOSE_ROW {
            self.request(Request::ChooseAnother(place));
            effects.push(self.say(Message::info(format!(
                "The system's Open with list, for {name}"
            ))));
        } else if let Some(app) = row
            .strip_prefix(APP_ROW)
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| list.rows.get(i))
        {
            effects.push(self.say(Message::info(format!("Opening {name} with {}", app.name))));
            let mime = self.focused_mime();
            self.request(Request::StartWith {
                place,
                program: app.clone(),
                mime,
            });
        }
        effects
    }

    /// The encoding chooser (R30 commit 32): in the viewer, or for the
    /// quick view's file. "Automatic" first with what it found ("Automatic:
    /// UTF-8 (byte-order mark)"), then the encodings by language
    /// ("Western European (Windows-1252)", 31 cells), the focus on the one
    /// in use.
    fn encoding_list(&mut self) -> Vec<Effect> {
        let (name, current, automatic, quick) = if let Some(viewing) = self.viewing.as_mut() {
            let viewer = &mut viewing.viewer;
            (
                viewer.name().to_owned(),
                viewer.encoding_choice(),
                viewer.automatic_choice(),
                false,
            )
        } else if let Some(shown) = self.quick_view().and_then(|v| v.shown()) {
            let Some(automatic) = shown.automatic else {
                let line = format!("No text shown: {} is not read as text", shown.name);
                return vec![self.say(Message::info(line))];
            };
            let current = match shown.encoding {
                Some(encoding) => Choice::by_you(encoding, b""),
                None => automatic,
            };
            (shown.name.clone(), current, automatic, true)
        } else {
            let line = "Encoding works in the viewer and the quick view.";
            return vec![self.say(Message::info(line))];
        };
        let mut rows = vec![ListRow::new(AUTOMATIC_ROW, automatic.automatic_line())];
        for (label, words) in ENCODING_LIST {
            let mut row = ListRow::new(label, words);
            if label == "windows-1252" {
                row.details.push("also ISO-8859-1".to_owned());
            }
            rows.push(row);
        }
        let mut list = ListModel::new(format!("Encoding of {name}")).with_rows(rows);
        list.set_help("Up and Down choose, Enter reads the file in it, Escape closes.");
        if current.chosen == Chosen::ByYou
            && let Some((label, _)) = ENCODING_LIST
                .iter()
                .find(|(label, _)| listed(label) == Some(current.encoding))
        {
            list.focus_id(label);
        }
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Types(
                TypeWants::Encoding { quick },
            ))),
        )
    }

    /// Enter in the encoding chooser: the file read again in the chosen
    /// encoding, or by the automatic choice.
    pub(super) fn encoding_chosen(&mut self, quick: bool, row: &str) -> Vec<Effect> {
        let mut effects = self.close_all();
        let encoding = if row == AUTOMATIC_ROW {
            None
        } else {
            let Some(encoding) = listed(row) else {
                return effects;
            };
            Some(encoding)
        };
        let said = if quick {
            match self.preview.view.as_mut().filter(|v| v.is_on()) {
                Some(view) => view.choose_encoding(encoding, Instant::now()),
                None => vec![Message::info("The quick view is off; nothing changed.")],
            }
        } else {
            match self.viewer_mut() {
                Some(viewer) => viewer.set_encoding(encoding),
                None => vec![Message::info("The viewer closed; nothing changed.")],
            }
        };
        effects.push(Effect::Redraw(Redraw::All));
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// The color role a theme gives a panel row by its file's group
    /// (`[group-colors]` in the theme file), for a frontend that draws
    /// it; `None` when the theme maps none of its groups, which is the
    /// default. The row's words never change: its type and group are said
    /// either way.
    pub fn row_role(&self, side: Side, row: &str) -> Option<ColorRole> {
        let entry = self.panel(side).entry(row)?;
        self.group_role(entry)
    }

    /// The color role the active theme maps one of an entry's groups to.
    fn group_role(&self, entry: &Entry) -> Option<ColorRole> {
        let (theme, _) = self.themes().resolve(&self.settings.theme);
        let table = theme.extra.get(GROUP_COLORS)?.as_table()?;
        let groups = self.types.groups_of(entry, self.clock.as_ref()).ok()?;
        groups.iter().find_map(|group| {
            table
                .get(&group.id)
                .and_then(toml::Value::as_str)
                .and_then(ColorRole::from_key)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counts_read_as_words() {
        assert_eq!(files_here(0), "no files here");
        assert_eq!(files_here(1), "1 file here");
        assert_eq!(files_here(41), "41 files here");
        assert_eq!(files_here(1_200), "1,200 files here");
        assert_eq!(format!("Images: {}", files_here(41)).chars().count(), 21);
    }
}
