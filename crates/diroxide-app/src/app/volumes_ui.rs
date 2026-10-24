//! The Volumes list (R25 3.1; lane A3, commit 88): one list from the Go
//! menu and the palette over three sources, the local drives and mounts,
//! each plugin's storages and the devices the Volume actions plugin
//! reports, every row its state first ("Mounted: Backup, removable, 412
//! GB free"; "Connected: build, sftp, 1.2 TB free"; "Not mounted: Backup,
//! removable, 931 GB").
//!
//! - Free space is asked off the input thread with a short limit, each
//!   row's answer said in place as it comes; a volume that never answers
//!   says "free space not known", and a drive's media is never read.
//! - **Enter** offers the row's actions: Open, Mount, Unmount, Eject,
//!   Hide. An action the Volume actions plugin would do, when it is not
//!   here, is shown unavailable with its reason ("Eject needs the Volume
//!   actions plugin."), and Enter on it says the reason and offers the
//!   example plugins, asked first.
//! - **Hide** keeps the row's device or place in the "Hidden volumes"
//!   setting; the list leaves it out until it is removed there.
//! - Unmount, Eject and Mount run the plugin's own command with the row's
//!   identity as its selection; a busy volume is never forced, and the
//!   plugin's sentence says so. The list follows when the command ends.

use diroxide_core::{
    Command, CommandId, Effect, EntryKind, Key, Layer, ListModel, ListRow, Message, Place,
    PlaceKind, Redraw,
};
use diroxide_vfs::{VolumeAction, VolumeAnswer, VolumeRow, VolumeTools};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::keymap::Lookup;
use crate::plugins::catalog::{self, ItemState};
use crate::request::Request;

/// The Volume actions plugin's id.
const PLUGIN: &str = "volume-actions";

/// What the app keeps for the Volumes list.
#[derive(Debug, Default)]
pub(super) struct VolumesState {
    /// The rows of the last listing, in order.
    rows: Vec<VolumeRow>,
    /// The listing the rows and their answers belong to: an answer for an
    /// older one is dropped.
    generation: u64,
    /// Whether the list opens when the rows come.
    opening: bool,
    /// Whether the go-to-place list asked for the rows this session.
    asked_for_places: bool,
}

/// What a dialog of the Volumes list is for.
#[derive(Clone, Debug)]
pub(super) enum Wants {
    /// The list itself.
    List,
    /// A row's actions, by its key.
    Actions {
        /// The row's key.
        key: String,
    },
}

impl App {
    /// The Volumes list's command; `None` for commands that are not its.
    pub(super) fn volumes_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::VOLUMES {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(self.ask_volumes(true))
    }

    /// Asks for the rows: the devices first, through the Volume actions
    /// plugin's List devices when it is here, then the runtime's rows,
    /// the hidden ones left out.
    fn ask_volumes(&mut self, opening: bool) -> Vec<Effect> {
        self.volumes.generation += 1;
        self.volumes.opening = opening;
        let mut effects = Vec::new();
        if opening {
            effects.push(self.say(Message::status("Reading the volumes")));
        }
        let devices = self.volume_devices_command();
        match devices.and_then(|id| self.start_devices_run(&id)) {
            Some(started) => effects.extend(started),
            None => self.volume_devices_came(Vec::new()),
        }
        effects
    }

    /// The devices the Volume actions plugin listed, one JSON line each
    /// (none when it is not here): the rows are asked for now.
    pub(super) fn volume_devices_came(&mut self, devices: Vec<String>) {
        self.request(Request::Volumes {
            generation: self.volumes.generation,
            hidden: self.settings.hidden_volumes.clone(),
            devices,
            spaces: true,
        });
    }

    /// A place on this computer to give the Volume actions plugin's
    /// commands as their panels: the active panel's, the other's, or the
    /// home folder.
    pub(super) fn local_context_place(&self) -> Option<Place> {
        [
            self.panel(self.active).place(),
            self.panel(self.active.other()).place(),
        ]
        .into_iter()
        .find(|place| place.kind() == PlaceKind::Local)
        .cloned()
        .or_else(|| self.home.clone())
    }

    /// The Volume actions plugin's command for an action on this
    /// computer, when it is enabled here: `eject-windows` on Windows.
    fn volume_command(&self, action: VolumeAction) -> Option<CommandId> {
        let local = match action {
            VolumeAction::Mount => "mount",
            VolumeAction::Unmount => "unmount",
            VolumeAction::Eject if cfg!(windows) => "eject-windows",
            VolumeAction::Eject => "eject",
            VolumeAction::Open | VolumeAction::Hide => return None,
        };
        let id = CommandId::new(format!("{PLUGIN}.{local}")).ok()?;
        let (item, entry) = self.plugins.catalog.any_command(&id)?;
        (matches!(item.state, ItemState::Enabled) && !entry.other_system).then_some(id)
    }

    /// The rows of the last listing, for the go-to-place list.
    pub(super) fn volume_rows(&self) -> &[VolumeRow] {
        &self.volumes.rows
    }

    /// The go-to-place list reads the volumes once a session, quietly,
    /// without List devices: its rows come as the list is open.
    pub(super) fn ask_volumes_for_places(&mut self) {
        if !self.volumes.rows.is_empty() || self.volumes.asked_for_places {
            return;
        }
        self.volumes.asked_for_places = true;
        self.volumes.generation += 1;
        self.volumes.opening = false;
        // No drive is asked for its label or free space here: the rows
        // are what the places list already knows, and the plugins'.
        self.request(Request::Volumes {
            generation: self.volumes.generation,
            hidden: self.settings.hidden_volumes.clone(),
            devices: Vec::new(),
            spaces: false,
        });
    }

    /// The plugin's List devices command, when it is enabled here.
    fn volume_devices_command(&self) -> Option<CommandId> {
        let id = CommandId::new(format!("{PLUGIN}.devices")).ok()?;
        let (item, entry) = self.plugins.catalog.any_command(&id)?;
        (matches!(item.state, ItemState::Enabled) && !entry.other_system).then_some(id)
    }

    /// Which actions this computer has.
    fn volume_tools(&self) -> VolumeTools {
        VolumeTools {
            mount: self.volume_command(VolumeAction::Mount).is_some(),
            unmount: self.volume_command(VolumeAction::Unmount).is_some(),
            eject: self.volume_command(VolumeAction::Eject).is_some(),
        }
    }

    /// The list's rows: each its state first, its details after.
    fn volumes_list(&self) -> ListModel {
        let rows = self
            .volumes
            .rows
            .iter()
            .map(|row| ListRow::new(row.key(), row.words()).with_details([row.detail()]))
            .collect();
        let title = format!(
            "Volumes, {}",
            diroxide_core::format::count(self.volumes.rows.len() as u64, "volume", "volumes")
        );
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_empty_text("No volumes to list");
        let reload = self.keymap.key_text(&ids::RELOAD);
        list.set_help(if reload.is_empty() {
            "Up and Down choose, Enter offers the actions, Escape closes.".to_owned()
        } else {
            format!(
                "Up and Down choose, Enter offers the actions, {reload} reads them again, Escape closes."
            )
        });
        list
    }

    /// Where the open list is in the dialogs.
    fn volumes_index(&self) -> Option<usize> {
        self.opens
            .iter()
            .rposition(|o| matches!(&o.purpose, Purpose::Volumes(w) if matches!(**w, Wants::List)))
    }

    /// Shows the rows again in an open list, the focus kept by key.
    fn volumes_shown_again(&mut self) -> Vec<Effect> {
        let Some(index) = self.volumes_index() else {
            return Vec::new();
        };
        let fresh = self.volumes_list();
        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index) {
            let focus = list.focused().map(|r| r.id.clone());
            let mut fresh = fresh;
            if let Some(focus) = focus {
                fresh.focus_id(&focus);
            }
            *list = fresh;
        }
        vec![Effect::Redraw(Redraw::All)]
    }

    /// The rows came: the list opens, or an open one follows.
    pub(super) fn volumes_listed(&mut self, generation: u64, rows: Vec<VolumeRow>) -> Vec<Effect> {
        if generation != self.volumes.generation {
            return Vec::new();
        }
        self.volumes.rows = rows;
        // An open go-to-place list shows them.
        if matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Places(_) | Purpose::PickDestination { .. })
        ) {
            return self.refresh_candidates();
        }
        if self.volumes_index().is_some() {
            return self.volumes_shown_again();
        }
        if !std::mem::take(&mut self.volumes.opening) {
            return Vec::new();
        }
        let list = self.volumes_list();
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Volumes(Box::new(Wants::List)),
        )
    }

    /// A volume answered its free space and label: its row changes in
    /// place, and the focused row is said again when it is the one.
    pub(super) fn volume_answered(
        &mut self,
        generation: u64,
        index: usize,
        answer: VolumeAnswer,
    ) -> Vec<Effect> {
        if generation != self.volumes.generation {
            return Vec::new();
        }
        let Some(row) = self.volumes.rows.get_mut(index) else {
            return Vec::new();
        };
        row.apply(answer);
        let (key, words, detail) = (row.key(), row.words(), row.detail());
        let Some(at) = self.volumes_index() else {
            return Vec::new();
        };
        let mut focused = false;
        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(at) {
            let mut rows = list.rows().to_vec();
            if let Some(shown) = rows.iter_mut().find(|r| r.id == key) {
                shown.label = words;
                shown.details = vec![detail];
            }
            focused = list.focused().is_some_and(|r| r.id == key);
            let focus = list.focused().map(|r| r.id.clone());
            list.set_rows(rows);
            if let Some(focus) = focus {
                list.focus_id(&focus);
            }
        }
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        // The focused row's new words are said quietly, once, when it is
        // the list on top (in the terminal the cursor's line changes too).
        if focused && self.volumes_index() == Some(self.opens.len() - 1) {
            let line = match self.dialogs.last() {
                Some((_, dialog)) => dialog.focus_line(),
                None => String::new(),
            };
            effects.push(Effect::Announce(Message::status(line)));
        }
        effects
    }

    fn volume_row(&self, key: &str) -> Option<VolumeRow> {
        self.volumes.rows.iter().find(|r| r.key() == key).cloned()
    }

    /// Enter in one of the list's dialogs.
    pub(super) fn volumes_accept(&mut self, wants: Wants, row: Option<String>) -> Vec<Effect> {
        match wants {
            Wants::List => match row {
                Some(key) => self.volume_actions(&key),
                None => Vec::new(),
            },
            Wants::Actions { key } => {
                let Some(action) = row else {
                    return Vec::new();
                };
                self.volume_action(&key, &action)
            }
        }
    }

    /// A row's actions, each offered or unavailable with its reason.
    fn volume_actions(&mut self, key: &str) -> Vec<Effect> {
        let Some(row) = self.volume_row(key) else {
            return vec![self.say(Message::info("That volume is gone."))];
        };
        let rows = row
            .actions(self.volume_tools())
            .into_iter()
            .map(|offer| {
                let name = offer.action.name();
                let mut shown = ListRow::new(name.to_lowercase(), name);
                // "Eject, needs the Volume actions plugin, unavailable":
                // the reason read right after the action's name.
                if let Some(why) = offer.why_not {
                    shown.enabled = false;
                    let reason = why
                        .strip_prefix(name)
                        .unwrap_or(&why)
                        .trim()
                        .trim_end_matches('.');
                    shown.details.push(reason.to_owned());
                }
                shown
            })
            .collect();
        let mut list = ListModel::new(row.name.clone()).with_rows(rows);
        list.set_help("Up and Down choose, Enter does it, Escape goes back to the volumes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Volumes(Box::new(Wants::Actions {
                key: key.to_owned(),
            })),
        )
    }

    /// One of a row's actions.
    fn volume_action(&mut self, key: &str, action: &str) -> Vec<Effect> {
        let Some(row) = self.volume_row(key) else {
            return vec![self.say(Message::info("That volume is gone."))];
        };
        let Some(offer) = row
            .actions(self.volume_tools())
            .into_iter()
            .find(|offer| offer.action.name().eq_ignore_ascii_case(action))
        else {
            return Vec::new();
        };
        // An action the plugin would do, when it is not here: the reason,
        // then the example plugins, asked first.
        if let Some(why) = offer.why_not {
            let mut effects = self.close_all();
            effects.push(self.say(Message::info(why)));
            effects.extend(self.dispatch(&Command::new(ids::PLUGINS_EXAMPLES)));
            return effects;
        }
        match offer.action {
            VolumeAction::Open => {
                let Some(place) = row.place.clone() else {
                    return Vec::new();
                };
                let mut effects = self.close_all();
                let side = self.active;
                effects.extend(self.open(side, place, None));
                effects
            }
            VolumeAction::Hide => {
                let mut hidden = self.settings.hidden_volumes.clone();
                if !hidden.contains(&row.key()) {
                    hidden.push(row.key());
                }
                let value =
                    toml::Value::Array(hidden.into_iter().map(toml::Value::String).collect());
                let mut effects = self.close_top();
                effects.extend(self.apply_setting("hidden_volumes", Some(value)));
                // The setting's own line is replaced by the list's.
                effects.retain(|e| !matches!(e, Effect::Announce(_)));
                self.status_log.pop();
                self.volumes.rows.retain(|r| r.key() != row.key());
                effects.extend(self.volumes_shown_again());
                effects.push(self.say(Message::info(format!(
                    "Hidden: {}. Settings, Hidden volumes has it.",
                    row.name
                ))));
                effects
            }
            VolumeAction::Mount | VolumeAction::Unmount | VolumeAction::Eject => {
                let Some(id) = self.volume_command(offer.action) else {
                    return Vec::new();
                };
                // The row's identity, as the plugin takes it: the device,
                // else the volume's place.
                let identity = row
                    .device
                    .as_deref()
                    .and_then(|device| Place::parse(device).ok())
                    .or_else(|| row.place.clone());
                let Some(identity) = identity else {
                    return vec![self.say(Message::info("This volume has no identity to act on."))];
                };
                let mut effects = self.close_top();
                // The plugin works on this computer's volumes: its panels
                // are the volume itself.
                let context = catalog::context_of(
                    &identity,
                    &identity,
                    Some((identity.clone(), EntryKind::Folder)),
                    None,
                    vec![(identity.clone(), EntryKind::Folder)],
                );
                effects.extend(self.run_volume_command(&id, context));
                effects
            }
        }
    }

    /// A volume action's command ended: the open list reads the volumes
    /// again.
    pub(super) fn volume_command_ended(&mut self, command: &CommandId) -> Vec<Effect> {
        let ours = command.as_str().starts_with(&format!("{PLUGIN}."));
        if ours && self.volumes_index().is_some() {
            return self.ask_volumes(false);
        }
        Vec::new()
    }

    /// Ctrl+R in the list reads the volumes again.
    pub(super) fn volumes_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let Some(Purpose::Volumes(wants)) = self.opens.last().map(|o| &o.purpose) else {
            return None;
        };
        if !matches!(**wants, Wants::List) {
            return None;
        }
        let layers = [Layer::Panel, Layer::List, Layer::Global];
        let reloads = matches!(
            self.keymap.lookup(&layers, key),
            Lookup::Command(id) if id == ids::RELOAD
        );
        if !reloads {
            return None;
        }
        let mut effects = self.ask_volumes(false);
        effects.push(self.say(Message::status("Reading the volumes again")));
        Some(effects)
    }
}
