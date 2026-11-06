//! Typed actions (alpha 4, lane TA, R30 commit 30; R28 4.3): a file's
//! type says what Enter, View, Preview, Convert and ten numbered user
//! actions run, and names more; an empty list falls back to the parent
//! type's.
//!
//! - **Enter** runs the type's `enter` action, and with none the system's
//!   program, as before. **View** runs the type's `view` action, and with
//!   none the built-in viewer.
//! - **Actions for this file** lists every action by name, the one Enter
//!   runs first ("Actions for report.pdf", 22 cells), each with its slot
//!   in words; a tool action whose tool is missing is shown unavailable
//!   with "Needs ImageMagick, not found." (29), the tools registry asked
//!   once for the list.
//! - **User action 0** to **9** (`type.user0` to `type.user9`) have no
//!   key until the owner binds one.
//! - A tool action's arguments are filled one by one with the host's
//!   placeholders (`{file}`, `{name}`, `{dir}`, `{other}`), never as a
//!   shell string; a command or a plugin's command runs on the focused
//!   file.
//!
//! The lists of this lane (the Actions list, Open with and the encoding
//! chooser) are hosted under the Organize purpose, so the window's test
//! of every purpose keeps its count; a purpose of their own is a contract
//! request for the GUI lane.

use std::collections::HashMap;

use diroxide_core::{
    Command, CommandId, Effect, Entry, EntryKind, ListModel, ListRow, Message, Place,
};
use diroxide_filetype::ActionTarget;
use diroxide_plugins::script::placeholders::{Fill, expand};
use diroxide_tools::{ToolRegistry, ToolStatus};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::request::{Request, SpawnKind};
use crate::typing::{Offered, Slot, offered};

/// The commands of the numbered user actions, in order.
const USER_ACTIONS: [CommandId; 10] = [
    ids::TYPE_USER_0,
    ids::TYPE_USER_1,
    ids::TYPE_USER_2,
    ids::TYPE_USER_3,
    ids::TYPE_USER_4,
    ids::TYPE_USER_5,
    ids::TYPE_USER_6,
    ids::TYPE_USER_7,
    ids::TYPE_USER_8,
    ids::TYPE_USER_9,
];

/// The Actions list's row for the system's own program.
const OPEN_ROW: &str = "open";

/// The Actions list's row for the built-in viewer.
const VIEW_ROW: &str = "view";

/// A typed action's row id: its place in the type's list.
const TYPED_ROW: &str = "typed:";

/// What a list of this lane is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TypeWants {
    /// Actions for this file.
    Actions {
        /// The file.
        place: Place,
    },
    /// The encoding chooser, for the viewer or the quick view.
    Encoding {
        /// Whether it is for the quick view's file.
        quick: bool,
    },
    /// Open with: the file and the programs listed for it.
    OpenWith {
        /// The file.
        place: Place,
        /// The programs.
        list: Box<crate::typing::OpenWithRows>,
    },
}

/// The line for a tool action whose tool is not here: "Needs ImageMagick,
/// not found." (29 cells).
pub(super) fn needs_line(tools: &ToolRegistry, tool: &str) -> String {
    let name = tools.name_of(tool);
    match tools.find(tool) {
        Ok(ToolStatus::Disabled) => format!("Needs {name}, turned off in tools.toml."),
        _ => format!("Needs {name}, not found."),
    }
}

/// The order the Actions list shows the slots in: Enter first.
fn slot_order(slot: Slot) -> u8 {
    match slot {
        Slot::Enter => 0,
        Slot::View => 1,
        Slot::Preview => 2,
        Slot::Convert => 3,
        Slot::User(n) => 4 + n,
        Slot::Named => 14,
    }
}

impl App {
    /// The typed actions' commands; `None` for commands that are not
    /// theirs.
    pub(super) fn type_actions_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let slot = if command.id == ids::FILE_ACTIONS {
            None
        } else {
            let n = USER_ACTIONS.iter().position(|id| *id == command.id)?;
            Some(Slot::User(u8::try_from(n).unwrap_or(0)))
        };
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(match slot {
            None => self.actions_list(),
            Some(slot) => self.run_slot_or_say(slot),
        })
    }

    /// The tools registry this app asks for a tool's program: the
    /// owner's `tools.toml`, then the installed helper components, then
    /// PATH and the known folders (K's request 3).
    pub(super) fn tool_registry(&self) -> ToolRegistry {
        ToolRegistry::builtin()
            .with_overrides(self.tool_overrides.clone())
            .with_components(self.helper_programs.clone())
    }

    /// The focused file and its place, or what is said instead.
    fn focused_file(&mut self) -> Result<(Entry, Place), Vec<Effect>> {
        let panel = self.active_panel();
        match (panel.focused_entry(), panel.focused_place()) {
            (Some(entry), Some(place))
                if matches!(entry.kind, EntryKind::File | EntryKind::Archive) =>
            {
                Ok((entry.clone(), place))
            }
            (Some(entry), Some(_)) => {
                let line = format!("Not a file: {}", entry.display_name());
                Err(vec![self.say(Message::info(line))])
            }
            _ => {
                let empty = panel.list().empty_text().to_owned();
                Err(vec![self.say(Message::status(empty))])
            }
        }
    }

    /// The actions the type of a file offers.
    fn offered_for(&self, entry: &Entry) -> Vec<Offered> {
        offered(&self.types, &self.type_slots, entry.type_index)
    }

    /// Enter or View on a file whose type fills the slot: its action
    /// runs. `None` leaves the built-in one: a folder, a file whose type
    /// has nothing for the slot, or an action that is the slot's own
    /// command (Enter set to Open runs Open, not itself again).
    pub(super) fn typed_slot(&mut self, slot: Slot) -> Option<Vec<Effect>> {
        if self.running_typed {
            return None;
        }
        let panel = self.active_panel();
        let entry = panel.focused_entry()?.clone();
        if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
            return None;
        }
        let place = panel.focused_place()?;
        let action = self
            .offered_for(&entry)
            .into_iter()
            .find(|o| o.slot == slot)?;
        Some(self.run_offered(&action, &entry, place))
    }

    /// A user action: the focused file's, or the line that says it has
    /// none: "photo.jpg has no user action 3."
    fn run_slot_or_say(&mut self, slot: Slot) -> Vec<Effect> {
        let (entry, place) = match self.focused_file() {
            Ok(found) => found,
            Err(said) => return said,
        };
        match self
            .offered_for(&entry)
            .into_iter()
            .find(|o| o.slot == slot)
        {
            Some(action) => self.run_offered(&action, &entry, place),
            None => {
                let line = format!("{} has no {}.", entry.display_name(), slot.words());
                vec![self.say(Message::info(line))]
            }
        }
    }

    /// Runs one action on a file: a command on the focused file, a
    /// plugin's command, or a tool with its arguments filled one by one.
    fn run_offered(&mut self, action: &Offered, entry: &Entry, place: Place) -> Vec<Effect> {
        let command = match &action.target {
            ActionTarget::Command(id) => Some(id.clone()),
            ActionTarget::Plugin { plugin, command } => {
                match CommandId::new(format!("{plugin}.{command}")) {
                    Ok(id) => Some(id),
                    Err(_) => {
                        let line = Message::could_not(
                            "run",
                            &action.name,
                            "its plugin's command is not a command id",
                            "Check filetypes.toml",
                        );
                        return vec![self.say(line)];
                    }
                }
            }
            ActionTarget::Tool { tool, args } => {
                return self.run_tool_action(&action.name, tool, args, entry, &place);
            }
        };
        let Some(id) = command else {
            return Vec::new();
        };
        // An action that is a slot's own command runs the built-in one.
        self.running_typed = true;
        let effects = self.dispatch(&Command::new(id));
        self.running_typed = false;
        effects
    }

    /// A tool action: the program from the tools registry and the
    /// arguments filled, started with the screen handed over; or the line
    /// that says what is missing.
    fn run_tool_action(
        &mut self,
        name: &str,
        tool: &str,
        args: &[String],
        entry: &Entry,
        place: &Place,
    ) -> Vec<Effect> {
        let tools = self.tool_registry();
        let program = match tools.find(tool) {
            Ok(ToolStatus::Found(program)) => program,
            _ => {
                let line = needs_line(&tools, tool);
                return vec![self.say(Message::info(line))];
            }
        };
        let side = self.active;
        let mut fill = Fill::new();
        if let Some(path) = place.local_path() {
            fill.set("file", path);
        }
        fill.set_name(&entry.name);
        let selection: Vec<std::ffi::OsString> = self
            .panel(side)
            .targets()
            .iter()
            .filter_map(Place::local_path)
            .map(std::ffi::OsString::from)
            .collect();
        fill.set_list("files", selection);
        if let Some(dir) = self.panel(side).place().local_path() {
            fill.set("dir", dir);
        }
        if let Some(other) = self.panel(side.other()).place().local_path() {
            fill.set("other", other);
            fill.set("dest", other);
        }
        let args = match expand(args, &fill) {
            Ok(args) => args,
            Err(error) => {
                let line = Message::could_not("run", name, &error.cause(), "Check filetypes.toml");
                return vec![self.say(line)];
            }
        };
        let kind = SpawnKind::Program {
            name: name.to_owned(),
            program,
            args,
        };
        let spawn = self.spawn_for(side, kind);
        self.request(Request::Spawn(Box::new(spawn)));
        let line = format!("Running {name} on {}", entry.display_name());
        vec![self.say(Message::info(line))]
    }

    /// Actions for this file: the type's actions by name, the one Enter
    /// runs first, each with its slot in words.
    fn actions_list(&mut self) -> Vec<Effect> {
        let (entry, place) = match self.focused_file() {
            Ok(found) => found,
            Err(said) => return said,
        };
        let offered = self.offered_for(&entry);
        let tools = self.tool_registry();
        let mut missing: HashMap<String, Option<String>> = HashMap::new();
        let mut ordered: Vec<(usize, &Offered)> = offered.iter().enumerate().collect();
        ordered.sort_by_key(|(_, o)| slot_order(o.slot));
        let mut rows = Vec::new();
        if !offered.iter().any(|o| o.slot == Slot::Enter) {
            rows.push(
                ListRow::new(OPEN_ROW, "Open").with_details(["Enter", "the default program"]),
            );
        }
        let has_view = offered.iter().any(|o| o.slot == Slot::View);
        for (i, action) in ordered {
            if !has_view && slot_order(action.slot) > slot_order(Slot::View) && !rows_have(&rows) {
                rows.push(ListRow::new(VIEW_ROW, "View").with_details(["the built-in viewer"]));
            }
            let mut row = ListRow::new(format!("{TYPED_ROW}{i}"), action.name.clone());
            let words = action.slot.words();
            if !words.is_empty() {
                row.details.push(words);
            }
            if let ActionTarget::Tool { tool, .. } = &action.target {
                let why = missing
                    .entry(tool.clone())
                    .or_insert_with(|| match tools.find(tool) {
                        Ok(ToolStatus::Found(_)) => None,
                        _ => Some(needs_line(&tools, tool)),
                    });
                if let Some(why) = why {
                    row.enabled = false;
                    row.details.push(why.trim_end_matches('.').to_owned());
                }
            }
            rows.push(row);
        }
        if !has_view && !rows_have(&rows) {
            rows.push(ListRow::new(VIEW_ROW, "View").with_details(["the built-in viewer"]));
        }
        let mut list =
            ListModel::new(format!("Actions for {}", entry.display_name())).with_rows(rows);
        list.set_help("Up and Down choose, Enter runs the action, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Types(
                TypeWants::Actions { place },
            ))),
        )
    }

    /// Enter in one of this lane's lists.
    pub(super) fn type_lists_accept(
        &mut self,
        wants: TypeWants,
        row: Option<String>,
    ) -> Vec<Effect> {
        let Some(row) = row else {
            return Vec::new();
        };
        match wants {
            TypeWants::Actions { place } => {
                let mut effects = self.close_all();
                let shown = self.active_panel().focused_place();
                if shown.as_ref() != Some(&place) {
                    effects.push(self.say(Message::info("The focus moved; nothing was run.")));
                    return effects;
                }
                if row == OPEN_ROW {
                    effects.extend(self.open_by_system());
                } else if row == VIEW_ROW {
                    self.running_typed = true;
                    effects.extend(self.dispatch(&Command::new(ids::VIEW)));
                    self.running_typed = false;
                } else if let Some(i) = row
                    .strip_prefix(TYPED_ROW)
                    .and_then(|i| i.parse::<usize>().ok())
                    && let Some(entry) = self.active_panel().focused_entry().cloned()
                    && let Some(action) = self.offered_for(&entry).get(i).cloned()
                {
                    effects.extend(self.run_offered(&action, &entry, place));
                }
                effects
            }
            TypeWants::OpenWith { place, list } => self.open_with_chosen(place, &list, &row),
            TypeWants::Encoding { quick } => self.encoding_chosen(quick, &row),
        }
    }
}

/// Whether the rows hold the built-in viewer's row already.
fn rows_have(rows: &[ListRow]) -> bool {
    rows.iter().any(|r| r.id == VIEW_ROW)
}
