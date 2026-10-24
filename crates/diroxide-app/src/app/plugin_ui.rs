//! Plugins and script commands in the app (W4d, R22 commits 36 to 40):
//! the catalog the runtime finds, the Plugins menu, the palette and the
//! keys it feeds, Manage plugins with each item's consent, settings and
//! log, a plugin command as a job, a plugin's own calls (its questions,
//! documents, listers, the panels, jobs and commands), and the host's
//! events in words.
//!
//! The rules (answers 90 and 91; R22 sections 7 and 9):
//!
//! - A plugin's name comes last in every line it causes; the host and the
//!   runner have attributed what reaches here, and the app's own lines
//!   about a plugin put it last too.
//! - Nothing runs that is not enabled; the first enable shows the consent
//!   question, starting on No; a changed manifest or program asks again
//!   before the next run.
//! - A plugin's dialog never replaces a destructive confirmation, a
//!   password prompt, a job's question or another plugin question: it
//!   waits behind them and opens when they close.
//! - Escape closes a plugin's dialog with "cancelled" for the plugin and
//!   returns the focus; in a running command it cancels the job.
//! - A crash, a time-out or a failed start is said once, however many
//!   ways it is reported.

use std::collections::{HashMap, HashSet, VecDeque};

use diroxide_core::{
    ArgValue, Command, CommandId, Effect, EntryKind, JobId, JobKind, Key, Layer, ListModel,
    ListRow, Message, MessageKind, Place, Redraw, Side, Timestamp,
};
use diroxide_plugins::ask::{Answer as PluginAnswer, Question, QuestionKind, TextDocument};
use diroxide_plugins::host::HostEvent;
use diroxide_plugins::lines;
use diroxide_plugins::listers::ListerView;
use diroxide_plugins::panels::{PanelChange, PanelChoice, PanelSnapshot};
use diroxide_plugins::protocol::{ErrorKind, RpcError};
use diroxide_plugins::script::RunMode;
use diroxide_plugins::script::context::RunContext;
use diroxide_plugins::script::directives::Directive;
use diroxide_plugins::script::runner::RunReport;
use diroxide_plugins::script::words::{self as script_words, Ending};
use diroxide_search::SearchHit;
use diroxide_vfs::Capabilities;
use serde_json::Value;

use super::{Action, App, Flow, Purpose, Then, route};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::{FinishedJob, SIGHTED_INTERVAL_MS, millis_between};
use crate::keymap::Lookup;
use crate::plugins::catalog::{self, Catalog, CommandEntry, ItemState, Runs};
use crate::plugins::lister::Lister;
use crate::plugins::registry::{self, Proposal};
use crate::plugins::{
    Call, CallId, Finished, Outcome, PluginEvent, PluginRequest, Reply, RunRequest, ask, log,
    settings,
};
use crate::request::Request;

/// How long after a plugin's failure was said another report of it is
/// left unsaid, in milliseconds.
const SAID_ONCE_MS: u64 = 3000;

/// The most output lines a command keeps for its output list.
const MAX_OUTPUT_LINES: usize = 10_000;

/// A plugin command running as a job.
#[derive(Clone, Debug)]
pub(super) struct Running {
    command: CommandId,
    name: String,
    source: String,
    plugin: String,
    side: Side,
    mode: Option<RunMode>,
    input: Option<String>,
    output: Vec<String>,
    shown_at: Option<Timestamp>,
    /// Whether the plugin said something of its own while it ran: then
    /// its end is shown, not said again.
    announced: bool,
    /// A lister's rows brought up to date: neither its start nor its end
    /// is said, and it leaves no row in the Jobs list; a failure is still
    /// said (lane A3).
    refresh: Refresh,
    /// The plugin and lister it was run from, so the lister's rows are
    /// brought up to date when it ends (lane A3).
    from_lister: Option<(String, String)>,
}

/// Whether a plugin command runs to bring a lister's rows up to date
/// (lane A3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refresh {
    /// A command the owner ran.
    No,
    /// After one of the lister's own commands, or a loss: nothing said.
    Silent,
    /// The owner's reload: only the new count is said.
    Said,
}

/// What the app keeps for plugins.
#[derive(Clone, Debug, Default)]
pub(super) struct PluginState {
    /// The plugins and scripts found, with their state.
    pub(super) catalog: Catalog,
    /// Whether a catalog came yet.
    scanned: bool,
    /// What happened to each proposed key.
    proposals: Vec<Proposal>,
    /// The plugin commands whose keys the owner's `keymap.toml` sets
    /// (lane A2).
    owner_set: Vec<CommandId>,
    /// Plugin dialogs waiting behind a confirmation or a question.
    queue: VecDeque<(CallId, String, Call)>,
    /// The plugin commands running, by job.
    running: HashMap<JobId, Running>,
    /// When a plugin's failure was last said, by plugin.
    failure_said: HashMap<String, Timestamp>,
    /// Why a plugin is off for this session, by plugin.
    off: HashMap<String, String>,
    /// A command to run once its item is enabled again.
    pending: Option<(CommandId, RunContext)>,
    /// The results listers made from a script's places, by search: the
    /// title and whether the places were cut at the cap.
    searches: HashMap<u64, (String, bool)>,
    /// The log asked for, by item id.
    log_wanted: Option<String>,
}

/// What a plugin dialog is for.
#[derive(Clone, Debug)]
pub(super) enum Wants {
    /// Manage plugins.
    Manage,
    /// One item's actions.
    Actions { id: String },
    /// The consent question; a command to run after the yes.
    Consent {
        id: String,
        then: Option<Box<(CommandId, RunContext)>>,
    },
    /// A plugin's question.
    Ask {
        call: CallId,
        question: Box<Question>,
    },
    /// A plugin's text document, answered on close when the plugin waits.
    Document { call: Option<CallId> },
    /// A plugin's lister.
    Lister { lister: Box<Lister> },
    /// The lister's sort choices.
    ListerSort,
    /// The lister's filter, typed.
    ListerFilter,
    /// A command's output to read.
    Output,
    /// An item's commands and keys.
    Commands,
    /// The plugins that have settings, from the Settings list.
    SettingsPlugins,
    /// An item's settings.
    Settings { id: String },
    /// A setting's choices.
    SettingChoice { id: String, key: String },
    /// A setting typed.
    SettingText { id: String, key: String },
    /// An item's log.
    Log,
    /// A script's own question before it runs.
    RunConfirm {
        command: CommandId,
        context: Box<RunContext>,
    },
    /// A script's prompt, whose answer it runs with.
    RunPrompt {
        command: CommandId,
        context: Box<RunContext>,
    },
}

/// The side a plugin names.
fn side_of(active: Side, which: PanelChoice) -> Side {
    match which {
        PanelChoice::Active => active,
        PanelChoice::Other => active.other(),
        PanelChoice::Left => Side::Left,
        PanelChoice::Right => Side::Right,
    }
}

/// A command argument from a plugin's JSON value: a place for the place
/// arguments, else text, a number, a flag, or several places.
fn argument(name: &str, value: &Value) -> Option<ArgValue> {
    const PLACES: [&str; 6] = ["place", "dest", "to", "from", "source", "target"];
    let place = |text: &str| diroxide_plugins::places::plugin_place(text).ok();
    Some(match value {
        Value::String(text) if PLACES.contains(&name) => ArgValue::Place(place(text)?),
        Value::String(text) => ArgValue::Text(text.clone()),
        Value::Number(n) => ArgValue::Number(n.as_i64()?),
        Value::Bool(b) => ArgValue::Flag(*b),
        Value::Array(items) => ArgValue::Places(
            items
                .iter()
                .map(|i| i.as_str().and_then(place))
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

/// The commands a row of a lister refuses: every one that acts on files.
fn acts_on_files(action: Action) -> bool {
    matches!(
        action,
        Action::View
            | Action::Edit
            | Action::Copy
            | Action::Move
            | Action::CopyTo
            | Action::MoveTo
            | Action::Trash
            | Action::DeletePermanently
            | Action::Rename
            | Action::Duplicate
            | Action::Details
            | Action::ClipCopy
            | Action::ClipCut
            | Action::Paste
            | Action::Pack
            | Action::Unpack
            | Action::TestArchive
            | Action::Split
            | Action::Join
            | Action::MakeLink
            | Action::Permissions
            | Action::OpenAsArchive
            | Action::CopyPath
            | Action::CopyName
            | Action::QuickView
            | Action::OpenInTextweaver
    )
}

impl App {
    /// The first scan, at start (`App::start`).
    pub(super) fn plugins_start(&mut self) {
        self.request(Request::Plugins(Box::new(PluginRequest::Scan)));
    }

    /// The catalog of plugins and scripts, as the app knows it.
    pub fn plugin_catalog(&self) -> &Catalog {
        &self.plugins.catalog
    }

    /// What happened to the enabled plugins' proposed keys.
    pub fn plugin_key_proposals(&self) -> &[Proposal] {
        &self.plugins.proposals
    }

    /// Binds the plugins' keys again: each command's proposed key where it
    /// is free, then the owner's `keymap.toml` lines for plugin commands,
    /// which win (lane A2). Called when a catalog comes and whenever the
    /// keymap is made again from its layers.
    pub(super) fn bind_plugin_keys(&mut self) {
        registry::forget_keys(&mut self.keymap);
        self.plugins.proposals = registry::propose_keys(&mut self.keymap, &self.plugins.catalog);
        let set =
            registry::apply_owner_keys(&mut self.keymap, &self.plugins.catalog, &self.keys.owner);
        for proposal in &mut self.plugins.proposals {
            if set.contains(&proposal.command) {
                proposal.bound = false;
                proposal.why = Some("set in keymap.toml".to_owned());
            }
        }
        self.plugins.owner_set = set;
    }

    /// The Keys list's rows for the plugins' commands (lane A2): the
    /// command's name first, its keys, how it got them, and the plugin
    /// last.
    pub(super) fn plugin_key_rows(&self, query: &str) -> Vec<ListRow> {
        let query = query.trim().to_lowercase();
        self.plugins
            .catalog
            .commands()
            .filter(|c| {
                query.is_empty()
                    || c.name.to_lowercase().contains(&query)
                    || c.source.to_lowercase().contains(&query)
            })
            .map(|c| {
                let keys = self.keymap.keys_in(Layer::Global, &c.id);
                let keys_words = if keys.is_empty() {
                    "no key".to_owned()
                } else {
                    keys.iter()
                        .map(|k| crate::keymap::written(k, self.keymap.platform()))
                        .collect::<Vec<_>>()
                        .join(" or ")
                };
                let state = if self.plugins.owner_set.contains(&c.id) {
                    "set in keymap.toml".to_owned()
                } else {
                    match self.plugins.proposals.iter().find(|p| p.command == c.id) {
                        Some(p) if p.bound => "proposed".to_owned(),
                        Some(p) => {
                            let key = Key::parse(&p.key).map_or_else(
                                |_| p.key.clone(),
                                |k| crate::keymap::written(&k, self.keymap.platform()),
                            );
                            let why = p.why.clone().unwrap_or_default();
                            format!("{key} not bound: {why}")
                        }
                        None => "not proposed".to_owned(),
                    }
                };
                ListRow::new(c.id.as_str(), c.name.clone()).with_details([
                    keys_words,
                    state,
                    format!("by {}", c.source),
                ])
            })
            .collect()
    }

    /// Enter on a plugin command's row of the Keys list: how its key is
    /// changed, in words.
    pub(super) fn plugin_key_row_entered(&mut self, id: &CommandId) -> Option<Vec<Effect>> {
        let command = self.plugins.catalog.command(id)?.clone();
        let text = format!(
            "{}: change its key in keymap.toml under \"{}\". By {}.",
            command.name, command.id, command.source
        );
        Some(vec![self.say(Message::info(text))])
    }

    /// The palette's list for a query, the enabled plugins' commands with
    /// the built-in ones.
    pub(super) fn palette_list(&self, query: &str) -> ListModel {
        let extra = registry::palette_entries(&self.plugins.catalog);
        self.palette
            .list_with_extra(query, &self.keymap, Some(&self.menus), &extra)
    }

    /// A menu's list for the active place, with the enabled plugins'
    /// commands: in the Plugins menu after its own items, and wherever
    /// `menus.toml` names one.
    pub(super) fn menu_list(&self, id: &str, caps: &Capabilities) -> Option<ListModel> {
        let context = self.plugin_context();
        let catalog = &self.plugins.catalog;
        let keymap = &self.keymap;
        let mut list = self.menus.list_with(id, keymap, caps, &|command| {
            catalog
                .command(command)
                .map(|c| registry::menu_row(c, keymap, &context))
        })?;
        if id == crate::commands::Menu::Plugins.id() {
            let mut rows = list.rows().to_vec();
            let named: HashSet<String> = rows.iter().map(|r| r.id.clone()).collect();
            rows.extend(
                registry::plugins_menu_rows(catalog, keymap, &context)
                    .into_iter()
                    .filter(|r| !named.contains(&r.id)),
            );
            list.set_rows(rows);
        }
        Some(list)
    }

    /// The line the Plugins menu says when it opens with plugins off.
    pub(super) fn menu_saying(&self, id: &str) -> Option<Message> {
        (id == crate::commands::Menu::Plugins.id() && self.plugins.catalog.off)
            .then(|| Message::info(lines::plugins_off()))
    }

    /// The context of a run now: both panels, their cursors and the marks.
    fn plugin_context(&self) -> RunContext {
        let side = self.active;
        let entry = |side: Side| {
            let panel = self.panel(side);
            panel
                .focused_entry()
                .and_then(|e| panel.place().join(&e.name).ok().map(|p| (p, e.kind)))
        };
        let panel = self.panel(side);
        let marked = panel
            .selected_entries()
            .into_iter()
            .filter_map(|e| panel.place().join(&e.name).ok().map(|p| (p, e.kind)))
            .collect();
        catalog::context_of(
            panel.place(),
            self.panel(side.other()).place(),
            entry(side),
            entry(side.other()),
            marked,
        )
    }

    /// Enter on a Sites row of the go-to-place list (lane A3): the site's
    /// plugin connects through its own `connect` command, with the site's
    /// place as the active place, so its questions, its refusals and its
    /// start folders are the plugin's own ("Connecting to build.", then
    /// the remote folder in this panel and the site's local folder in the
    /// other). `None` when the place is not a site, or its plugin has no
    /// `connect` command: the place then simply opens.
    pub(super) fn connect_site(&mut self, side: Side, place: &Place) -> Option<Vec<Effect>> {
        let site = self.sites.iter().find(|s| &s.place == place)?.clone();
        let id = CommandId::new(format!("{}.connect", site.plugin_id)).ok()?;
        self.plugins.catalog.any_command(&id)?;
        let context = catalog::context_of(
            place,
            self.panel(side.other()).place(),
            Some((place.clone(), EntryKind::Folder)),
            None,
            Vec::new(),
        );
        Some(self.run_plugin_command(&id, context, None, false))
    }

    /// The plugin commands and the screens of this module; `None` for
    /// commands that are not theirs.
    pub(super) fn plugins_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id == ids::PLUGINS {
            return Some(if self.dialogs.is_empty() {
                self.open_manage_plugins()
            } else {
                vec![self.say(Message::status("Close this dialog first."))]
            });
        }
        if !registry::is_plugin_command(&command.id) {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(self.run_plugin_command(&command.id, self.plugin_context(), None, false))
    }

    /// Runs a plugin's command: its conditions, its consent, its own
    /// question or prompt, then the job.
    fn run_plugin_command(
        &mut self,
        id: &CommandId,
        context: RunContext,
        lister: Option<(String, Vec<diroxide_core::Entry>)>,
        once: bool,
    ) -> Vec<Effect> {
        if self.plugins.catalog.off {
            return vec![self.say(Message::info(lines::plugins_off()))];
        }
        let found = self
            .plugins
            .catalog
            .any_command(id)
            .map(|(item, entry)| (item.clone(), entry.clone()));
        let Some((item, entry)) = found else {
            let text = format!("No enabled plugin has the command {id}.");
            return vec![self.say(Message::info(text))];
        };
        match &item.state {
            ItemState::Enabled => {}
            ItemState::NotEnabled if once => {}
            ItemState::NotEnabled => {
                let text = format!("{} is not enabled. Enable it in Manage plugins.", item.name);
                return vec![self.say(Message::info(text))];
            }
            ItemState::Changed => {
                return self.open_consent(&item.id, Some((id.clone(), context)));
            }
            ItemState::HasErrors(first) => {
                let text = format!("Could not run {}: it has errors. {first}", entry.name);
                return vec![self.say(Message::error(text))];
            }
        }
        if let Some(reason) = self.plugins.off.get(&item.id).cloned() {
            return vec![self.say(Message::info(reason))];
        }
        if lister.is_none()
            && let Some(reason) = entry.refusal(&context)
        {
            return vec![self.say(Message::info(reason))];
        }
        if let Runs::Script(script) = &entry.runs
            && lister.is_none()
        {
            if let Some(question) = &script.recipe.confirm {
                let head = question.trim().trim_end_matches(['?', '.']);
                let line = format!("{head}? No is the default. y or n ({})", entry.source);
                let dialog = Dialog::Confirm {
                    question: Message::new(
                        line,
                        diroxide_core::Importance::High,
                        MessageKind::Question,
                    ),
                    listing: Vec::new(),
                    on_yes: Command::new(ids::YES),
                };
                return self.open_dialog(
                    dialog,
                    Purpose::Plugin(Box::new(Wants::RunConfirm {
                        command: id.clone(),
                        context: Box::new(context),
                    })),
                );
            }
            if let Some(prompt) = &script.recipe.prompt {
                return self.open_run_prompt(id, prompt, &entry.source, context);
            }
        }
        self.start_plugin_run(&entry, context, lister, once)
    }

    fn open_run_prompt(
        &mut self,
        id: &CommandId,
        prompt: &str,
        source: &str,
        context: RunContext,
    ) -> Vec<Effect> {
        let label = lines::with_source(prompt.trim_end_matches(':'), source);
        let key = format!("plugin-prompt:{id}");
        let field = EntryField::new(label, CompletionSource::History).with_history(key);
        self.open_dialog(
            Dialog::edit(field),
            Purpose::Plugin(Box::new(Wants::RunPrompt {
                command: id.clone(),
                context: Box::new(context),
            })),
        )
    }

    /// Starts a plugin command as a job: the jobs line says it started,
    /// the plugin's name last.
    fn start_plugin_run(
        &mut self,
        entry: &CommandEntry,
        context: RunContext,
        lister: Option<(String, Vec<diroxide_core::Entry>)>,
        once: bool,
    ) -> Vec<Effect> {
        self.start_plugin_run_with(entry, context, lister, once, Refresh::No)
    }

    /// Starts a plugin command as a job, said, or quietly to bring a
    /// lister's rows up to date.
    fn start_plugin_run_with(
        &mut self,
        entry: &CommandEntry,
        context: RunContext,
        lister: Option<(String, Vec<diroxide_core::Entry>)>,
        once: bool,
        refresh: Refresh,
    ) -> Vec<Effect> {
        let job = self.next_job_id();
        let line = lines::progress_started(&entry.name, &entry.source);
        let now = self.now();
        self.jobs
            .begin(job, JobKind::PluginCommand, line.clone(), now);
        let mode = match &entry.runs {
            Runs::Script(script) => Some(script.recipe.mode),
            Runs::Process => None,
        };
        let from_lister = lister
            .as_ref()
            .map(|(id, _)| (entry.owner.clone(), id.clone()));
        self.plugins.running.insert(
            job,
            Running {
                command: entry.id.clone(),
                name: entry.name.clone(),
                source: entry.source.clone(),
                plugin: entry.owner.clone(),
                side: self.active,
                mode,
                input: context.input.clone(),
                output: Vec::new(),
                shown_at: Some(now),
                announced: false,
                refresh,
                from_lister,
            },
        );
        self.plugins.failure_said.remove(&entry.owner);
        self.request(Request::Plugins(Box::new(PluginRequest::Run(Box::new(
            RunRequest {
                job,
                command: entry.id.clone(),
                context,
                lister,
                once,
            },
        )))));
        if refresh != Refresh::No {
            return vec![Effect::Redraw(Redraw::JobsLine)];
        }
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::status(line)),
        ]
    }

    /// An event of the runtime's about plugins.
    pub(super) fn plugin_event(&mut self, event: PluginEvent) -> Vec<Effect> {
        match event {
            PluginEvent::Found(catalog) => self.catalog_found(*catalog),
            PluginEvent::Call {
                call,
                plugin,
                request,
            } => self.plugin_call(call, plugin, request),
            PluginEvent::Said { plugin, message } => {
                for running in self.plugins.running.values_mut() {
                    if running.plugin == plugin {
                        running.announced = true;
                    }
                }
                vec![self.say(message)]
            }
            PluginEvent::Candidates {
                call,
                text,
                candidates,
            } => self.plugin_candidates(call, &text, &candidates),
            PluginEvent::Progress { plugin, message } => self.plugin_progress(&plugin, message),
            PluginEvent::Host(event) => self.host_event(event),
            PluginEvent::Output { job, line } => {
                if let Some(running) = self.plugins.running.get_mut(&job)
                    && running.output.len() < MAX_OUTPUT_LINES
                {
                    running.output.push(line);
                }
                Vec::new()
            }
            PluginEvent::Refresh => {
                let mut effects = Vec::new();
                for side in Side::BOTH {
                    effects.extend(self.reload(side));
                }
                effects
            }
            PluginEvent::Finished(finished) => self.plugin_finished(*finished),
            PluginEvent::Logged { id, lines } => self.open_log(&id, &lines),
            PluginEvent::ListerRows {
                plugin,
                lister,
                rows,
                refresh,
            } => self.lister_rows_came(&plugin, &lister, rows, refresh),
            PluginEvent::ListerNoRows { plugin, lister } => self.lister_no_rows(&plugin, &lister),
            PluginEvent::Note(message) => vec![self.say(message)],
        }
    }

    /// A new catalog: the proposed keys bound again, an open Manage
    /// plugins list made again, a command waiting for its enable run.
    fn catalog_found(&mut self, catalog: Catalog) -> Vec<Effect> {
        self.plugins.catalog = catalog;
        self.bind_plugin_keys();
        self.plugins.scanned = true;
        let mut effects = Vec::new();
        let rows = self.manage_rows();
        if let Some((_, Dialog::Choose { list, .. })) = self
            .dialogs
            .iter_mut()
            .zip(&self.opens)
            .rev()
            .find(|(_, open)| matches!(&open.purpose, Purpose::Plugin(w) if matches!(**w, Wants::Manage)))
            .map(|(dialog, _)| dialog)
        {
            list.set_rows(rows);
            effects.push(Effect::Redraw(Redraw::All));
        }
        if let Some((command, context)) = self.plugins.pending.take() {
            let ready = self
                .plugins
                .catalog
                .any_command(&command)
                .is_some_and(|(item, _)| item.state == ItemState::Enabled);
            if ready && self.dialogs.is_empty() {
                effects.extend(self.run_plugin_command(&command, context, None, false));
            }
        }
        effects
    }

    // ---- Manage plugins ----

    fn manage_rows(&self) -> Vec<ListRow> {
        self.plugins
            .catalog
            .items
            .iter()
            .map(|item| {
                let mut row = ListRow::new(item.id.clone(), item.row());
                if item.example {
                    row.details.push("example".to_owned());
                }
                if let Some(reason) = self.plugins.off.get(&item.id) {
                    row.details.push(reason.clone());
                }
                row
            })
            .collect()
    }

    /// Manage plugins: every plugin and script, its state first after its
    /// name; the list is read again as it opens.
    pub(super) fn open_manage_plugins(&mut self) -> Vec<Effect> {
        self.request(Request::Plugins(Box::new(PluginRequest::Scan)));
        let mut list = ListModel::new("Manage plugins").with_rows(self.manage_rows());
        list.set_empty_text(if self.plugins.scanned {
            "No plugins or scripts. Get example plugins is in the Plugins menu"
        } else {
            "Looking for plugins"
        });
        list.set_help("Up and Down choose, Enter offers what to do, Escape closes.");
        let saying = if self.plugins.catalog.off {
            Some(Message::info(lines::plugins_off()))
        } else {
            None
        };
        let dialog = Dialog::Choose {
            list,
            on_enter: Command::new(ids::ACCEPT),
        };
        let purpose = Purpose::Plugin(Box::new(Wants::Manage));
        match saying {
            Some(message) => self.open_dialog_saying(dialog, purpose, message),
            None => self.open_dialog(dialog, purpose),
        }
    }

    fn open_item_actions(&mut self, id: &str) -> Vec<Effect> {
        let Some(item) = self.plugins.catalog.item(id).cloned() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        match &item.state {
            ItemState::NotEnabled => rows.push(ListRow::new("enable", "Enable")),
            ItemState::Enabled => rows.push(ListRow::new("disable", "Disable")),
            ItemState::Changed => {
                rows.push(ListRow::new("enable", "Enable again"));
                rows.push(ListRow::new("disable", "Disable"));
            }
            ItemState::HasErrors(first) => {
                rows.push(
                    ListRow::new("errors", "Why it has errors").with_details([first.clone()]),
                );
            }
        }
        if self.plugins.off.contains_key(&item.id) {
            rows.push(ListRow::new("turn-on", "Turn it on again"));
        }
        if !item.commands.is_empty() {
            rows.push(ListRow::new("commands", "Commands and keys"));
        }
        if !item.settings.is_empty() {
            rows.push(ListRow::new("settings", "Settings"));
        }
        rows.push(ListRow::new("log", "Log, the last error first"));
        let mut list = ListModel::new(item.name.clone()).with_rows(rows);
        list.set_help("Up and Down choose, Enter does it, Escape goes back.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::Actions { id: id.to_owned() })),
        )
    }

    /// The consent question for an item: its first row as the question,
    /// starting on No, and the rows between as the listing.
    fn open_consent(&mut self, id: &str, then: Option<(CommandId, RunContext)>) -> Vec<Effect> {
        let Some(item) = self.plugins.catalog.item(id).cloned() else {
            return Vec::new();
        };
        let rows = if item.state == ItemState::Changed {
            item.changed_rows.clone()
        } else {
            item.enable_rows.clone()
        };
        let Some((first, rest)) = rows.split_first() else {
            return vec![self.say(Message::error(format!(
                "Could not ask about {}: it cannot be read.",
                item.name
            )))];
        };
        let middle = rest.split_last().map_or(&[][..], |(_, middle)| middle);
        let question = if item.state == ItemState::Changed {
            Message::new(
                format!("{first} Enable it again? No is the default. y or n"),
                diroxide_core::Importance::High,
                MessageKind::Question,
            )
        } else {
            with_default_no(Message::question(first))
        };
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing: middle.to_vec(),
                on_yes: Command::new(ids::YES),
            },
            Purpose::Plugin(Box::new(Wants::Consent {
                id: id.to_owned(),
                then: then.map(Box::new),
            })),
        )
    }

    fn open_commands(&mut self, id: &str) -> Vec<Effect> {
        let Some(item) = self.plugins.catalog.item(id).cloned() else {
            return Vec::new();
        };
        let context = self.plugin_context();
        let rows = item
            .commands
            .iter()
            .map(|c| {
                let mut details = Vec::new();
                match self.plugins.proposals.iter().find(|p| p.command == c.id) {
                    Some(p) => details.push(registry::proposal_words(p)),
                    None => details.push("no key".to_owned()),
                }
                if let Some(reason) = c.refusal(&context) {
                    details.push(reason);
                }
                ListRow::new(c.id.as_str(), c.name.clone()).with_details(details)
            })
            .collect();
        let mut list = ListModel::new(format!("Commands of {}", item.name)).with_rows(rows);
        list.set_help("Up and Down read a command, its key and what it needs, Escape goes back.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::Commands)),
        )
    }

    /// The plugins that have settings, from the Settings list's "Plugin
    /// settings": each by its name, with its state and how many it has.
    pub(super) fn open_plugin_settings(&mut self) -> Vec<Effect> {
        let rows = self
            .plugins
            .catalog
            .items
            .iter()
            .filter(|item| !item.settings.is_empty())
            .map(|item| {
                ListRow::new(item.id.clone(), item.name.clone()).with_details([
                    item.state.words().to_owned(),
                    diroxide_core::format::count(item.settings.len() as u64, "setting", "settings"),
                ])
            })
            .collect();
        let mut list = ListModel::new("Plugin settings").with_rows(rows);
        list.set_empty_text("No plugin has settings");
        list.set_help("Up and Down choose, Enter opens a plugin's settings, Escape goes back.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::SettingsPlugins)),
        )
    }

    fn open_settings_of(&mut self, id: &str) -> Vec<Effect> {
        let Some(item) = self.plugins.catalog.item(id).cloned() else {
            return Vec::new();
        };
        let list = settings::settings_list(&item.name, &item.settings, &item.values);
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::Settings { id: id.to_owned() })),
        )
    }

    fn plugin_setting_chosen(&mut self, id: &str, key: &str) -> Vec<Effect> {
        let Some(item) = self.plugins.catalog.item(id).cloned() else {
            return Vec::new();
        };
        let Some(spec) = item.settings.iter().find(|s| s.key == key).cloned() else {
            return Vec::new();
        };
        let value = settings::current(&spec, &item.values);
        match spec.kind.as_str() {
            "bool" => {
                let flipped = Value::Bool(!value.as_bool().unwrap_or(false));
                self.change_setting(id, key, flipped)
            }
            "choice" => {
                let now = settings::value_words(&value);
                let rows = spec
                    .choices
                    .iter()
                    .map(|c| {
                        let row = ListRow::new(c.clone(), c.clone());
                        if *c == now {
                            row.with_details(["current"])
                        } else {
                            row
                        }
                    })
                    .collect();
                let mut list = ListModel::new(settings::setting_name(key)).with_rows(rows);
                list.focus_id(&now);
                self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Purpose::Plugin(Box::new(Wants::SettingChoice {
                        id: id.to_owned(),
                        key: key.to_owned(),
                    })),
                )
            }
            _ => {
                let source = if spec.kind == "path" {
                    CompletionSource::Paths
                } else {
                    CompletionSource::None
                };
                let field = EntryField::new(settings::setting_name(key), source)
                    .with_text(settings::value_words(&value).replace("none", ""));
                self.open_dialog(
                    Dialog::edit(field),
                    Purpose::Plugin(Box::new(Wants::SettingText {
                        id: id.to_owned(),
                        key: key.to_owned(),
                    })),
                )
            }
        }
    }

    /// Writes a setting and tells the plugin; the list shows it at once.
    fn change_setting(&mut self, id: &str, key: &str, value: Value) -> Vec<Effect> {
        let words = settings::value_words(&value);
        let mut name = id.to_owned();
        if let Some(item) = self.plugins.catalog.items.iter_mut().find(|i| i.id == id) {
            item.values.insert(key.to_owned(), value.clone());
            name.clone_from(&item.name);
        }
        self.request(Request::Plugins(Box::new(PluginRequest::Setting {
            id: id.to_owned(),
            key: key.to_owned(),
            value: Some(value),
        })));
        let rows = self
            .plugins
            .catalog
            .item(id)
            .map(|item| settings::settings_list(&item.name, &item.settings, &item.values));
        if let (Some(fresh), Some((_, Dialog::Choose { list, .. }))) =
            (rows, self.dialogs.last_mut())
            && list.title() == fresh.title()
        {
            list.set_rows(fresh.rows().to_vec());
        }
        let text = lines::with_source(&format!("{}: {words}", settings::setting_name(key)), &name);
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
    }

    fn open_log(&mut self, id: &str, newest_first: &[String]) -> Vec<Effect> {
        if self.plugins.log_wanted.as_deref() != Some(id) {
            return Vec::new();
        }
        self.plugins.log_wanted = None;
        let name = self
            .plugins
            .catalog
            .item(id)
            .map_or_else(|| id.to_owned(), |i| i.name.clone());
        let list = log::log_list(&name, newest_first);
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::Log)),
        )
    }

    // ---- The hooks of the dialogs ----

    /// Enter in a plugin dialog.
    pub(super) fn plugin_accept(
        &mut self,
        wants: Wants,
        row: Option<String>,
        text: String,
    ) -> Vec<Effect> {
        match wants {
            Wants::Manage => match row {
                Some(id) => self.open_item_actions(&id),
                None => Vec::new(),
            },
            Wants::Actions { id } => {
                let Some(row) = row else {
                    return Vec::new();
                };
                match row.as_str() {
                    "enable" => self.open_consent(&id, None),
                    "disable" => {
                        let mut effects = self.close_top();
                        self.request(Request::Plugins(Box::new(PluginRequest::Disable {
                            id: id.clone(),
                        })));
                        effects.push(Effect::Redraw(Redraw::All));
                        effects
                    }
                    "turn-on" => {
                        self.plugins.off.remove(&id);
                        self.request(Request::Plugins(Box::new(PluginRequest::TurnOn {
                            id: id.clone(),
                        })));
                        self.close_top()
                    }
                    "commands" => self.open_commands(&id),
                    "settings" => self.open_settings_of(&id),
                    "log" => {
                        self.plugins.log_wanted = Some(id.clone());
                        self.request(Request::Plugins(Box::new(PluginRequest::Log { id })));
                        Vec::new()
                    }
                    "errors" => {
                        let text = match self.plugins.catalog.item(&id).map(|i| &i.state) {
                            Some(ItemState::HasErrors(first)) => first.clone(),
                            _ => "No errors.".to_owned(),
                        };
                        vec![self.say(Message::info(text))]
                    }
                    _ => Vec::new(),
                }
            }
            Wants::Ask { call, question } => {
                // Enter in the candidates takes one into the field first.
                if let (Some((_, Dialog::Edit { in_list: true, .. })), Some(row)) =
                    (self.dialogs.last(), &row)
                {
                    let row = row.clone();
                    return self.take_candidate(&row);
                }
                self.ask_accepted(call, &question, row, text)
            }
            Wants::Document { call } => {
                let effects = self.close_top();
                if let Some(call) = call {
                    self.reply(call, Reply::Shown);
                }
                effects
            }
            Wants::Lister { lister } => self.lister_activate(&lister),
            Wants::ListerSort => {
                let Some(column) = row else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.lister_change(|lister| lister.sort_by(&column)));
                effects
            }
            Wants::ListerFilter => {
                let mut effects = self.close_top();
                let filter = text.trim().to_owned();
                effects.extend(self.lister_change(|lister| {
                    if filter.is_empty() {
                        lister.filter = None;
                        "Filter off".to_owned()
                    } else {
                        lister.filter = Some(filter.clone());
                        format!("Filter {filter}")
                    }
                }));
                effects
            }
            Wants::Output | Wants::Commands | Wants::Log => Vec::new(),
            Wants::Settings { id } => match row {
                Some(key) => self.plugin_setting_chosen(&id, &key),
                None => Vec::new(),
            },
            Wants::SettingsPlugins => match row {
                Some(id) => self.open_settings_of(&id),
                None => Vec::new(),
            },
            Wants::SettingChoice { id, key } => {
                let Some(value) = row else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.change_setting(&id, &key, Value::String(value)));
                effects
            }
            Wants::SettingText { id, key } => {
                let spec = self
                    .plugins
                    .catalog
                    .item(&id)
                    .and_then(|i| i.settings.iter().find(|s| s.key == key).cloned());
                let Some(spec) = spec else {
                    return self.close_top();
                };
                match settings::parse_value(&spec, &text) {
                    Ok(value) => {
                        let mut effects = self.close_top();
                        effects.extend(self.change_setting(&id, &key, value));
                        effects
                    }
                    Err(why) => vec![self.say(Message::info(why))],
                }
            }
            Wants::RunConfirm { .. } | Wants::Consent { .. } => Vec::new(),
            Wants::RunPrompt { command, context } => {
                let mut effects = self.close_top();
                let typed = text.trim().to_owned();
                if typed.is_empty() {
                    let name = self
                        .plugins
                        .catalog
                        .command(&command)
                        .map_or_else(|| command.to_string(), |c| c.name.clone());
                    effects.push(self.say(Message::info(lines::cancelled(&name))));
                    return effects;
                }
                self.remember(&format!("plugin-prompt:{command}"), &typed);
                let mut context = *context;
                context.input = Some(typed);
                let entry = self.plugins.catalog.command(&command).cloned();
                if let Some(entry) = entry {
                    effects.extend(self.start_plugin_run(&entry, context, None, false));
                }
                effects
            }
        }
    }

    /// Yes in a plugin's confirmation.
    pub(super) fn plugin_yes(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::Consent { id, then } => {
                if let Some(then) = then {
                    self.plugins.pending = Some(*then);
                }
                self.request(Request::Plugins(Box::new(PluginRequest::Enable { id })));
                if matches!(
                    self.opens.last().map(|o| &o.purpose),
                    Some(Purpose::Plugin(w)) if matches!(**w, Wants::Actions { .. })
                ) {
                    return self.close_top();
                }
                Vec::new()
            }
            Wants::Ask { call, .. } => {
                self.reply(call, Reply::Answer(ask::confirmed(true)));
                Vec::new()
            }
            Wants::RunConfirm { command, context } => {
                let entry = self.plugins.catalog.command(&command).cloned();
                match entry {
                    Some(entry) => self.start_plugin_run(&entry, *context, None, false),
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// Escape (or No) in a plugin dialog: the plugin hears "cancelled",
    /// a consent says "Not enabled.", a lister tells its plugin it closed.
    pub(super) fn plugin_escape(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::Consent { .. } => {
                self.plugins.pending = None;
                let mut effects = Vec::new();
                if matches!(
                    self.opens.last().map(|o| &o.purpose),
                    Some(Purpose::Plugin(w)) if matches!(**w, Wants::Actions { .. })
                ) {
                    effects.extend(self.close_top());
                }
                effects.push(self.say(Message::info(
                    diroxide_plugins::script::consent::not_enabled_line(),
                )));
                effects
            }
            Wants::Ask { call, question } => {
                let answer = match question.kind {
                    QuestionKind::Confirm { .. } => PluginAnswer::Confirmed(false),
                    _ => PluginAnswer::Cancelled,
                };
                self.reply(call, Reply::Answer(answer));
                Vec::new()
            }
            Wants::Document { call: Some(call) } => {
                self.reply(call, Reply::Shown);
                Vec::new()
            }
            Wants::Lister { lister } => {
                self.request(Request::Plugins(Box::new(PluginRequest::ListerClosed {
                    plugin: lister.view.plugin_id.clone(),
                    lister: lister.view.spec.id.clone(),
                })));
                vec![self.say(Message::info(lines::with_source(
                    "Lister closed",
                    &lister.view.plugin,
                )))]
            }
            Wants::RunConfirm { command, .. } => {
                let name = self
                    .plugins
                    .catalog
                    .command(&command)
                    .map_or_else(|| command.to_string(), |c| c.name.clone());
                vec![self.say(Message::info(lines::cancelled(&name)))]
            }
            _ => Vec::new(),
        }
    }

    /// The answer to a plugin's question, on Enter: a choice, typed text,
    /// a place or a secret.
    fn ask_accepted(
        &mut self,
        call: CallId,
        question: &Question,
        row: Option<String>,
        text: String,
    ) -> Vec<Effect> {
        let answer = match &question.kind {
            QuestionKind::Confirm { .. } => PluginAnswer::Confirmed(row.as_deref() == Some("yes")),
            QuestionKind::Choose { multiple, .. } => match self.dialogs.last() {
                Some((_, Dialog::Question { choices, .. })) => ask::chosen(choices, *multiple),
                _ => PluginAnswer::Cancelled,
            },
            QuestionKind::Text { .. } => {
                if let Err(why) = question.check_text(&text) {
                    return vec![self.say(Message::info(why))];
                }
                if let Some(key) = ask::history_key(question)
                    && !text.is_empty()
                {
                    let history = self.histories.entry(key).or_default();
                    crate::state::remember(history, &text);
                }
                PluginAnswer::Text(text)
            }
            QuestionKind::Path { .. } => {
                if text.trim().is_empty() {
                    return vec![self.say(Message::info("Type a place, or Escape."))];
                }
                let here = self.panel(self.active).place().clone();
                match Place::resolve(text.trim(), &here) {
                    Ok(place) => PluginAnswer::Place(place.to_string()),
                    Err(error) => {
                        return vec![self.say(Message::from_error("use", text.trim(), &error))];
                    }
                }
            }
            QuestionKind::Secret => match self.dialogs.last() {
                Some((_, Dialog::Secret { typed, .. })) => PluginAnswer::Secret(typed.clone()),
                _ => PluginAnswer::Cancelled,
            },
        };
        let effects = self.close_top();
        self.reply(call, Reply::Answer(answer));
        effects
    }

    /// A plugin's candidates for its prompt: the list under the field, when
    /// the question is still open and its text has not changed since.
    fn plugin_candidates(
        &mut self,
        call: CallId,
        text: &str,
        candidates: &[(String, String)],
    ) -> Vec<Effect> {
        let current = matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Plugin(w)) if matches!(&**w, Wants::Ask { call: c, .. } if *c == call)
        );
        let Some((
            _,
            Dialog::Edit {
                field,
                candidates: list,
                in_list,
            },
        )) = self.dialogs.last_mut()
        else {
            return Vec::new();
        };
        if !current || field.text() != text {
            return Vec::new();
        }
        if candidates.is_empty() {
            *list = None;
            *in_list = false;
            return vec![Effect::Redraw(Redraw::All)];
        }
        let rows = candidates
            .iter()
            .map(|(text, kind)| {
                let row = ListRow::new(text.clone(), text.clone());
                if kind.is_empty() {
                    row
                } else {
                    row.with_details([kind.clone()])
                }
            })
            .collect();
        let mut fresh = ListModel::new(field.label().to_owned()).with_rows(rows);
        fresh.set_help("Up and Down choose, Enter takes it, Escape closes the list.");
        *list = Some(fresh);
        let count =
            diroxide_core::format::count(candidates.len() as u64, "candidate", "candidates");
        vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::status(count)),
        ]
    }

    /// The text of a plugin's prompt changed: its plugin completer asks
    /// for candidates (`completion/query`).
    pub(super) fn plugin_field_changed(&mut self, text: &str) -> Vec<Effect> {
        let Some(Purpose::Plugin(wants)) = self.opens.last().map(|o| o.purpose.clone()) else {
            return Vec::new();
        };
        let Wants::Ask { call, question } = *wants else {
            return Vec::new();
        };
        if !matches!(
            question.completion,
            Some(diroxide_plugins::ask::Completion::Plugin(_))
        ) {
            return Vec::new();
        }
        let cursor = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => field.cursor(),
            _ => return Vec::new(),
        };
        self.request(Request::Plugins(Box::new(PluginRequest::Complete {
            call,
            question,
            text: text.to_owned(),
            cursor,
        })));
        Vec::new()
    }

    /// Keys of a plugin's dialogs before the dialog's own: Space marks in
    /// a choose list with several answers and in a lister; a lister's
    /// action keys, sort and filter; a file command on a lister's row is
    /// refused.
    pub(super) fn plugin_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let wants = match self.opens.last().map(|o| &o.purpose) {
            Some(Purpose::Plugin(wants)) => (**wants).clone(),
            _ => return None,
        };
        let layers = [Layer::Panel, Layer::List, Layer::Global];
        let command = match self.keymap.lookup(&layers, key) {
            Lookup::Command(id) => Some(id),
            Lookup::NoMatch => None,
        };
        let action = command.as_ref().and_then(route);
        match wants {
            // F1 on a plugin's setting: its description, its kind, its
            // default, the plugin's name last.
            Wants::Settings { id } if action == Some(Action::Help) => {
                let item = self.plugins.catalog.item(&id)?.clone();
                let key = match self.dialogs.last() {
                    Some((_, Dialog::Choose { list, .. })) => list.focused()?.id.clone(),
                    _ => return None,
                };
                let spec = item.settings.iter().find(|s| s.key == key)?;
                let help = settings::setting_help(&item.name, spec);
                Some(vec![self.say(Message::info(help))])
            }
            Wants::Ask { question, .. } => {
                if let QuestionKind::Choose { multiple: true, .. } = question.kind
                    && action == Some(Action::ToggleSelect)
                {
                    let state = match self.dialogs.last_mut() {
                        Some((_, Dialog::Question { choices, .. })) => {
                            let state = choices.toggle_selected();
                            choices.move_focus(diroxide_core::Move::Down);
                            state
                        }
                        _ => None,
                    };
                    let word = if state == Some(true) {
                        "Marked"
                    } else {
                        "Unmarked"
                    };
                    return Some(vec![
                        Effect::Redraw(Redraw::All),
                        self.say(Message::status(word)),
                    ]);
                }
                None
            }
            Wants::Lister { lister } => {
                if let Some(effects) = self.lister_action_key(&lister, key) {
                    return Some(effects);
                }
                match action? {
                    // The end of the rows: more from the plugin, when it
                    // has more.
                    Action::Focus(movement) => {
                        let at_end = match self.dialogs.last_mut() {
                            Some((_, Dialog::Choose { list, .. })) => {
                                list.move_focus(movement);
                                list.focus_index().map(|i| i + 1) == Some(list.len())
                            }
                            _ => false,
                        };
                        let line = match self.dialogs.last() {
                            Some((_, dialog)) => dialog.focus_line(),
                            None => String::new(),
                        };
                        let mut effects = vec![
                            Effect::Redraw(Redraw::All),
                            Effect::Announce(Message::status(line)),
                        ];
                        if at_end && lister.has_more() && !lister.asking {
                            effects.extend(self.ask_lister_rows(&lister, false));
                        }
                        Some(effects)
                    }
                    Action::Reload => Some(self.refresh_lister(
                        &lister.view.plugin_id,
                        &lister.view.spec.id,
                        true,
                    )),
                    Action::ToggleSelect => {
                        let state = match self.dialogs.last_mut() {
                            Some((_, Dialog::Choose { list, .. })) => {
                                let state = list.toggle_selected();
                                list.move_focus(diroxide_core::Move::Down);
                                state
                            }
                            _ => None,
                        };
                        let word = if state == Some(true) {
                            "Marked"
                        } else {
                            "Unmarked"
                        };
                        Some(vec![
                            Effect::Redraw(Redraw::All),
                            self.say(Message::status(word)),
                        ])
                    }
                    Action::SortList | Action::Sort => {
                        let list = lister.sort_list();
                        Some(self.open_dialog(
                            Dialog::Choose {
                                list,
                                on_enter: Command::new(ids::ACCEPT),
                            },
                            Purpose::Plugin(Box::new(Wants::ListerSort)),
                        ))
                    }
                    Action::Filter => {
                        let field = EntryField::new("Filter the rows by", CompletionSource::None)
                            .with_text(lister.filter.clone().unwrap_or_default());
                        Some(self.open_dialog(
                            Dialog::edit(field),
                            Purpose::Plugin(Box::new(Wants::ListerFilter)),
                        ))
                    }
                    action if acts_on_files(action) => {
                        Some(vec![self.say(Message::warning(lines::not_a_file()))])
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The Secret purpose's check: a plugin's secret question takes its
    /// keys as the masked prompt does.
    pub(super) fn plugin_secret(&self) -> bool {
        matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Plugin(w)) if matches!(&**w, Wants::Ask { question, .. } if question.kind == QuestionKind::Secret)
        )
    }

    // ---- A plugin's calls ----

    /// Whether a plugin dialog must wait: a confirmation, a password, a
    /// job's question, a field the owner types in (so a typed letter never
    /// answers the plugin), a typed confirmation or another plugin question
    /// is open.
    fn plugin_dialog_blocked(&self) -> bool {
        self.dialogs.iter().zip(&self.opens).any(|((_, dialog), open)| {
            matches!(
                dialog,
                Dialog::Confirm { .. }
                    | Dialog::Secret { .. }
                    | Dialog::Question { .. }
                    | Dialog::Edit { .. }
            ) || matches!(open.purpose, Purpose::TypedPlan(..))
                || matches!(&open.purpose, Purpose::Plugin(w) if matches!(**w, Wants::Ask { .. }))
        })
    }

    /// Opens a plugin dialog that waited, when nothing blocks it now.
    pub(super) fn plugin_queue_next(&mut self) -> Vec<Effect> {
        if self.plugins.queue.is_empty() || self.plugin_dialog_blocked() {
            return Vec::new();
        }
        match self.plugins.queue.pop_front() {
            Some((call, plugin, request)) => self.plugin_call(call, plugin, request),
            None => Vec::new(),
        }
    }

    fn reply(&mut self, call: CallId, reply: Reply) {
        self.request(Request::Plugins(Box::new(PluginRequest::Reply {
            call,
            reply: Box::new(reply),
        })));
    }

    fn plugin_call(&mut self, call: CallId, plugin: String, request: Call) -> Vec<Effect> {
        let opens_dialog = matches!(
            request,
            Call::Ask(_) | Call::ShowText(_) | Call::OpenLister(_)
        );
        if opens_dialog && self.plugin_dialog_blocked() {
            self.plugins.queue.push_back((call, plugin, request));
            return Vec::new();
        }
        match request {
            Call::Ask(question) => {
                if let Some(key) = ask::history_key(&question)
                    && !question.history.is_empty()
                {
                    self.histories.insert(key, question.history.clone());
                }
                let (dialog, message) = ask::dialog_for(&question);
                self.open_dialog_saying(
                    dialog,
                    Purpose::Plugin(Box::new(Wants::Ask { call, question })),
                    message,
                )
            }
            Call::ShowText(document) => self.show_document(call, &document),
            Call::Panel(which) => {
                let snapshot = self.snapshot(which);
                self.reply(call, Reply::Panel(Some(Box::new(snapshot))));
                Vec::new()
            }
            Call::SetPanel(change) => {
                // A plugin that opens a place in the active panel from its
                // own lister (Connect's start folder) is done with the
                // lister: it closes, so the owner lands in the folder.
                let mut effects = Vec::new();
                if change.place.is_some()
                    && side_of(self.active, change.panel) == self.active
                    && self.top_lister_of(&plugin)
                {
                    effects.extend(self.close_top());
                }
                effects.extend(self.apply_panel_change(&change));
                self.reply(call, Reply::Done(Ok(Value::Null)));
                effects
            }
            Call::RunJob(job) => self.plugin_job(call, *job),
            Call::OpenLister(view) => {
                let opener = self
                    .running_of(&plugin)
                    .and_then(|job| self.plugins.running.get(&job))
                    .map(|running| (running.command.clone(), running.refresh));
                let effects = self.open_lister(*view, opener);
                self.reply(call, Reply::Done(Ok(Value::Null)));
                effects
            }
            Call::Execute { command, args } => {
                let mut built = Command::new(command.clone());
                for (name, value) in &args {
                    match argument(name, value) {
                        Some(value) => built = built.with_arg(name.clone(), value),
                        None => {
                            self.reply(
                                call,
                                Reply::Done(Err(RpcError::new(
                                    ErrorKind::InvalidParams,
                                    format!("the argument {name} has a value of the wrong kind"),
                                ))),
                            );
                            return Vec::new();
                        }
                    }
                }
                let effects = self.dispatch(&built);
                self.reply(call, Reply::Done(Ok(Value::Null)));
                effects
            }
        }
    }

    fn show_document(&mut self, call: CallId, document: &TextDocument) -> Vec<Effect> {
        let list = ask::document_list(document);
        let wait = document.wait;
        let effects = self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::Document {
                call: wait.then_some(call),
            })),
        );
        if !wait {
            self.reply(call, Reply::Shown);
        }
        effects
    }

    /// A panel as a plugin sees it.
    fn snapshot(&self, which: PanelChoice) -> PanelSnapshot {
        let side = side_of(self.active, which);
        let panel = self.panel(side);
        PanelSnapshot {
            side,
            active: side == self.active,
            place: panel.place().clone(),
            cursor: panel.focused_entry().cloned(),
            selection: panel.selected_entries().into_iter().cloned().collect(),
            sort: Some(panel.sort().describe()),
            filter: panel.filter().map(|f| f.text().to_owned()),
        }
    }

    /// A plugin's `panel/set`, made through the app's own moves, so the
    /// app says its usual lines.
    fn apply_panel_change(&mut self, change: &PanelChange) -> Vec<Effect> {
        let side = side_of(self.active, change.panel);
        if let Some(place) = &change.place {
            let focus = change
                .cursor
                .as_ref()
                .and_then(|c| c.name())
                .map(str::to_owned);
            return self.open(side, place.clone(), focus);
        }
        let name_of = |place: &Place| place.name().map(str::to_owned);
        if change.clear_selection {
            self.panel_mut(side).list_mut().select_none();
        }
        let select: Vec<String> = change.select.iter().filter_map(name_of).collect();
        let deselect: Vec<String> = change.deselect.iter().filter_map(name_of).collect();
        self.mark_names(side, &select, true);
        self.mark_names(side, &deselect, false);
        let mut effects = vec![Effect::Redraw(Redraw::Panel(side))];
        if let Some(name) = change.cursor.as_ref().and_then(name_of) {
            self.focus_name(side, &name);
            let line = self.panel(side).list().announce_focus();
            effects.push(self.say(Message::status(line)));
        }
        if change.refresh {
            effects.extend(self.reload(side));
        }
        effects
    }

    /// A plugin's file job, through the app's own path: the plan, its
    /// listing, the confirmation that starts on No, the trash, the undo
    /// log. The plugin hears the report when it ends.
    fn plugin_job(&mut self, call: CallId, mut job: diroxide_core::JobRequest) -> Vec<Effect> {
        job.id = self.next_job_id();
        let confirm = match job.kind {
            JobKind::Copy => self.settings.confirm.copy,
            JobKind::Move => self.settings.confirm.move_items,
            JobKind::Trash => self.settings.confirm.trash,
            JobKind::DeletePermanently => true,
            _ => false,
        };
        self.plan_job(self.active, job, None, Then::PluginJob(call), confirm)
    }

    /// A plugin's file job that ended or never ran: the plugin hears it.
    pub(super) fn plugin_job_ended(
        &mut self,
        flow: Option<&Flow>,
        result: Result<Box<diroxide_ops::JobReport>, String>,
    ) {
        if let Some(Flow {
            then: Then::PluginJob(call),
            ..
        }) = flow
        {
            let reply = match result {
                Ok(report) => Reply::Job(Ok(report)),
                Err(why) => Reply::Job(Err(RpcError::new(ErrorKind::Failed, why))),
            };
            self.reply(*call, reply);
        }
    }

    // ---- Listers ----

    /// Whether the dialog on top is a lister of this plugin.
    fn top_lister_of(&self, plugin: &str) -> bool {
        matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Plugin(w)) if matches!(&**w, Wants::Lister { lister } if lister.view.plugin_id == plugin)
        )
    }

    /// Where the open lister of a plugin is, by its id, in the dialogs.
    fn lister_index(&self, plugin: &str, lister: &str) -> Option<usize> {
        self.opens.iter().rposition(|o| {
            matches!(&o.purpose, Purpose::Plugin(w) if matches!(&**w, Wants::Lister { lister: l } if l.view.plugin_id == plugin && l.view.spec.id == lister))
        })
    }

    /// A plugin opened a lister: a new dialog, or, when the same lister is
    /// open, its rows brought up to date in place with the focus and the
    /// marks kept by row id. `opener` is the command that opened it, and
    /// whether that run brings the rows up to date.
    fn open_lister(
        &mut self,
        view: ListerView,
        opener: Option<(CommandId, Refresh)>,
    ) -> Vec<Effect> {
        let silent = opener
            .as_ref()
            .is_some_and(|(_, refresh)| *refresh == Refresh::Silent);
        let opened_by = opener.map(|(command, _)| command);
        if let Some(index) = self.lister_index(&view.plugin_id, &view.spec.id) {
            let count = view.rows.len() as u64;
            let Purpose::Plugin(wants) = &mut self.opens[index].purpose else {
                return Vec::new();
            };
            let Wants::Lister { lister } = &mut **wants else {
                return Vec::new();
            };
            lister.view = view;
            lister.asking = false;
            if opened_by.is_some() {
                lister.opened_by = opened_by;
            }
            let lister = lister.clone();
            if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index) {
                let fresh = lister.list(Some(list));
                *list = fresh;
            }
            if silent {
                return vec![Effect::Redraw(Redraw::All)];
            }
            let line = lines::with_source(
                &diroxide_core::format::count(count, "row", "rows"),
                &lister.view.plugin,
            );
            return vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))];
        }
        let mut lister = Lister::new(view);
        lister.opened_by = opened_by;
        let list = lister.list(None);
        let count = lister.view.rows.len();
        let suffix = format!(" ({})", lister.view.plugin);
        let title = lister.title();
        let bare = title.strip_suffix(&suffix).unwrap_or(&title);
        let mut line = lines::with_source(
            &format!(
                "{bare}, {}",
                diroxide_core::format::count(count as u64, "row", "rows")
            ),
            &lister.view.plugin,
        );
        if let Some(capped) = lister.view.capped_line() {
            line.push_str(". ");
            line.push_str(&capped);
        }
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Plugin(Box::new(Wants::Lister {
                lister: Box::new(lister),
            })),
            Message::info(line),
        )
    }

    /// Changes the lister under the top dialog (or on top), and shows it
    /// again with its marks and focus kept.
    fn lister_change(&mut self, change: impl FnOnce(&mut Lister) -> String) -> Vec<Effect> {
        let Some(index) = self.opens.iter().rposition(
            |o| matches!(&o.purpose, Purpose::Plugin(w) if matches!(**w, Wants::Lister { .. })),
        ) else {
            return Vec::new();
        };
        let Purpose::Plugin(wants) = &mut self.opens[index].purpose else {
            return Vec::new();
        };
        let Wants::Lister { lister } = &mut **wants else {
            return Vec::new();
        };
        let sentence = change(lister);
        let lister = lister.clone();
        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index) {
            let fresh = lister.list(Some(list));
            *list = fresh;
        }
        let line = match self.dialogs.get(index) {
            Some((_, Dialog::Choose { list, .. })) => {
                format!("{sentence}. {}", list.announce_focus())
            }
            _ => sentence,
        };
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
    }

    /// Asks the plugin for more rows, or for all of them again.
    fn ask_lister_rows(&mut self, lister: &Lister, refresh: bool) -> Vec<Effect> {
        let offset = if refresh {
            0
        } else {
            lister.view.rows.len() as u64
        };
        self.request(Request::Plugins(Box::new(PluginRequest::ListerRows {
            plugin: lister.view.plugin_id.clone(),
            lister: lister.view.spec.id.clone(),
            offset,
            refresh,
        })));
        self.lister_change(|lister| {
            lister.asking = true;
            lines::with_source("Asking for more rows", &lister.view.plugin)
        })
    }

    /// Rows a plugin sent for its open lister: added after the ones shown,
    /// or in their place.
    fn lister_rows_came(
        &mut self,
        plugin: &str,
        lister: &str,
        rows: Vec<diroxide_core::Entry>,
        refresh: bool,
    ) -> Vec<Effect> {
        let Some(index) = self.lister_index(plugin, lister) else {
            return Vec::new();
        };
        // A refresh after one of the lister's own commands says nothing:
        // the rows change in place, the focus and the marks kept.
        if let Purpose::Plugin(wants) = &mut self.opens[index].purpose
            && let Wants::Lister { lister: held } = &mut **wants
            && refresh
            && held.quiet_refresh
        {
            held.quiet_refresh = false;
            held.take_rows(rows, true);
            let held = (**held).clone();
            if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index) {
                let fresh = held.list(Some(list));
                *list = fresh;
            }
            return vec![Effect::Redraw(Redraw::All)];
        }
        self.lister_change(|held| {
            let added = held.take_rows(rows, refresh);
            let words = if refresh {
                diroxide_core::format::count(added as u64, "row", "rows")
            } else {
                format!(
                    "{} more",
                    diroxide_core::format::count(added as u64, "row", "rows")
                )
            };
            lines::with_source(&words, &held.view.plugin)
        })
    }

    fn lister_rows(&self, lister: &Lister) -> Vec<diroxide_core::Entry> {
        match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => lister.chosen(list),
            _ => Vec::new(),
        }
    }

    /// Enter in a lister: its `activate` command with the chosen rows.
    fn lister_activate(&mut self, lister: &Lister) -> Vec<Effect> {
        let Some(local) = lister.view.activate().map(str::to_owned) else {
            return vec![self.say(Message::info(lines::with_source(
                "Enter does nothing here",
                &lister.view.plugin,
            )))];
        };
        self.lister_command(lister, &local, true)
    }

    /// Runs one of a lister's commands with its chosen rows; `needs_row`
    /// is false for an action that takes no selection (New site in an
    /// empty Sites list), which runs with the focused row when there is
    /// one, and without one otherwise.
    fn lister_command(&mut self, lister: &Lister, local: &str, needs_row: bool) -> Vec<Effect> {
        let rows = self.lister_rows(lister);
        if rows.is_empty() && needs_row {
            return vec![self.say(Message::status("No row is chosen"))];
        }
        let Ok(id) = CommandId::new(format!("{}.{local}", lister.view.plugin_id)) else {
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
        self.start_plugin_run(
            &entry,
            context,
            Some((lister.view.spec.id.clone(), rows)),
            false,
        )
    }

    fn lister_action_key(&mut self, lister: &Lister, key: &Key) -> Option<Vec<Effect>> {
        let (local, selection) = lister.view.spec.actions.iter().find_map(|action| {
            Key::parse(&action.key)
                .ok()
                .filter(|k| k == key)
                .map(|_| (action.command.clone(), action.selection))
        })?;
        let needs_row = selection != diroxide_plugins::manifest::Selection::None;
        Some(self.lister_command(lister, &local, needs_row))
    }

    /// Brings an open lister's rows up to date, asking its plugin for all
    /// of them again (`lister/rows`); a plugin without that method has its
    /// lister's opening command run again quietly instead
    /// (`lister_no_rows`). `said` says the new count, for the
    /// owner's reload; otherwise nothing is said. Nothing when the lister
    /// is not open.
    fn refresh_lister(&mut self, plugin: &str, lister: &str, said: bool) -> Vec<Effect> {
        let Some(index) = self.lister_index(plugin, lister) else {
            return Vec::new();
        };
        let Purpose::Plugin(wants) = &mut self.opens[index].purpose else {
            return Vec::new();
        };
        let Wants::Lister { lister: held } = &mut **wants else {
            return Vec::new();
        };
        held.quiet_refresh = !said;
        let held = (**held).clone();
        if said {
            return self.ask_lister_rows(&held, true);
        }
        self.request(Request::Plugins(Box::new(PluginRequest::ListerRows {
            plugin: plugin.to_owned(),
            lister: lister.to_owned(),
            offset: 0,
            refresh: true,
        })));
        Vec::new()
    }

    /// The plugin has no `lister/rows`: its lister's opening command runs
    /// again quietly, and its new rows replace the old in place, the focus
    /// and the marks kept (the SFTP plugin's Sites list, after New site or
    /// a lost connection).
    fn lister_no_rows(&mut self, plugin: &str, lister: &str) -> Vec<Effect> {
        let Some(index) = self.lister_index(plugin, lister) else {
            return Vec::new();
        };
        let held = match &self.opens[index].purpose {
            Purpose::Plugin(w) => match &**w {
                Wants::Lister { lister } => (**lister).clone(),
                _ => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        // One refresh at a time: a run of the opener still going will
        // bring the rows.
        let opener_running = held.opened_by.as_ref().is_some_and(|command| {
            self.plugins
                .running
                .values()
                .any(|r| r.refresh != Refresh::No && &r.command == command)
        });
        if opener_running {
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

    /// Brings every open lister of a plugin up to date (a lost
    /// connection turns a site's row to "lost").
    fn refresh_listers_of(&mut self, plugin: &str) -> Vec<Effect> {
        let open: Vec<String> = self
            .opens
            .iter()
            .filter_map(|o| match &o.purpose {
                Purpose::Plugin(w) => match &**w {
                    Wants::Lister { lister } if lister.view.plugin_id == plugin => {
                        Some(lister.view.spec.id.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let mut effects = Vec::new();
        for lister in open {
            effects.extend(self.refresh_lister(plugin, &lister, false));
        }
        effects
    }

    // ---- Jobs and their ends ----

    fn running_of(&self, plugin: &str) -> Option<JobId> {
        self.plugins
            .running
            .iter()
            .filter(|(_, r)| r.plugin == plugin)
            .map(|(job, _)| *job)
            .max()
    }

    fn plugin_progress(&mut self, plugin: &str, message: Message) -> Vec<Effect> {
        let Some(job) = self.running_of(plugin) else {
            return vec![self.say(message)];
        };
        let interval = if self.screen_reader {
            self.settings.progress_interval_seconds.saturating_mul(1000)
        } else {
            SIGHTED_INTERVAL_MS
        };
        let now = self.now();
        let due = self
            .plugins
            .running
            .get(&job)
            .and_then(|r| r.shown_at)
            .is_none_or(|at| millis_between(at, now) >= interval);
        self.jobs.update(job, message.text.clone());
        if !due {
            return vec![Effect::Redraw(Redraw::JobsLine)];
        }
        if let Some(running) = self.plugins.running.get_mut(&job) {
            running.shown_at = Some(now);
        }
        vec![
            Effect::Redraw(Redraw::JobsLine),
            Effect::Announce(Message::progress(message.text)),
        ]
    }

    /// Whether a failure of this plugin was said a moment ago.
    fn failure_said_lately(&self, plugin: &str) -> bool {
        let now = self.now();
        self.plugins
            .failure_said
            .get(plugin)
            .is_some_and(|at| millis_between(*at, now) < SAID_ONCE_MS)
    }

    fn note_failure_said(&mut self, plugin: &str) {
        let now = self.now();
        self.plugins.failure_said.insert(plugin.to_owned(), now);
    }

    /// The host's events in words; a failure is said once.
    fn host_event(&mut self, event: HostEvent) -> Vec<Effect> {
        match event {
            HostEvent::StartFailed { plugin, message }
            | HostEvent::TimedOut { plugin, message }
            | HostEvent::Crashed { plugin, message } => {
                if self.failure_said_lately(&plugin) {
                    return Vec::new();
                }
                self.note_failure_said(&plugin);
                vec![self.say(message)]
            }
            HostEvent::TurnedOff { plugin, message } => {
                self.plugins
                    .off
                    .insert(plugin.clone(), message.text.clone());
                self.note_failure_said(&plugin);
                vec![self.say(message)]
            }
            HostEvent::Flooded { plugin, message } => {
                self.note_failure_said(&plugin);
                vec![self.say(message)]
            }
            HostEvent::Waiting { plugin, message } => match self.running_of(&plugin) {
                Some(job) => {
                    self.jobs.update(job, message.text.clone());
                    vec![Effect::Redraw(Redraw::JobsLine), self.say(message)]
                }
                None => Vec::new(),
            },
            HostEvent::Said { message, .. } => vec![self.say(message)],
            HostEvent::IdleStopped { .. } => Vec::new(),
            // A lost device or session is said; a job on it ends by its
            // own errors when the loss is final, never by this (R25). The
            // plugin's open listers and the Sites group follow, so a
            // site's row says "lost" (lane A3).
            HostEvent::ProviderLost {
                plugin, message, ..
            } => {
                let mut effects = vec![self.say(message)];
                effects.extend(self.refresh_listers_of(&plugin));
                effects.extend(self.refresh_sites());
                effects
            }
        }
    }

    /// A plugin command ended: its lines, its deferred moves, its lister
    /// or output, and the jobs line.
    fn plugin_finished(&mut self, finished: Finished) -> Vec<Effect> {
        let Some(running) = self.plugins.running.remove(&finished.job) else {
            return Vec::new();
        };
        let job = finished.job;
        let mut said: Vec<Message> = Vec::new();
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        let mut summary = lines::progress_done(&running.name, &running.source);
        let mut failed = Vec::new();
        match finished.outcome {
            Outcome::Changed(rows) => {
                self.jobs.finish(job);
                if let Some(item) = self
                    .plugins
                    .catalog
                    .items
                    .iter_mut()
                    .find(|i| i.id == running.plugin)
                {
                    item.state = ItemState::Changed;
                    item.changed_rows = rows;
                }
                let context = self.plugin_context();
                effects.extend(
                    self.open_consent(&running.plugin, Some((running.command.clone(), context))),
                );
                return effects;
            }
            Outcome::Cancelled => {
                summary = lines::cancelled(&running.name);
                said.push(Message::info(summary.clone()));
            }
            Outcome::Refused(sentence) => {
                summary = sentence.clone();
                said.push(Message::info(sentence));
            }
            Outcome::Process(Ok(value)) => {
                let count = |key: &str| match value.get(key) {
                    Some(Value::Array(items)) => items.len() as u64,
                    Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
                    _ => 0,
                };
                let (done, bad) = (count("done"), count("failed"));
                if bad > 0 {
                    summary = lines::could_not_run(&running.name, &format!("{bad} failed"));
                    said.push(Message::warning(summary.clone()));
                    said.push(Message::info(lines::next_log(&running.plugin)));
                    failed.push(format!("{bad} failed"));
                } else {
                    if done > 0 {
                        summary = lines::command_done(
                            &running.name,
                            &diroxide_core::format::count(done, "item", "items"),
                            &running.source,
                        );
                    }
                    // Said when the plugin said nothing of its own; shown
                    // either way.
                    said.push(if running.announced {
                        Message::status(summary.clone())
                    } else {
                        Message::done(summary.clone())
                    });
                }
            }
            Outcome::Process(Err(error)) => {
                use diroxide_plugins::PluginError;
                let quiet = self.failure_said_lately(&running.plugin);
                // The plugin's next step, from its error's hint (S-A's
                // request 3), said after the failure, its name last.
                let hint = match &error {
                    PluginError::Remote {
                        hint: Some(hint), ..
                    } if !hint.trim().is_empty() => Some(hint.clone()),
                    _ => None,
                };
                summary = match &error {
                    PluginError::Remote { code, .. }
                        if *code == ErrorKind::RequestCancelled.code() =>
                    {
                        lines::cancelled(&running.name)
                    }
                    PluginError::Timeout(_) => lines::timed_out(&running.name, finished.limit),
                    PluginError::Stopped(_) => format!(
                        "{} {}",
                        lines::stopped_unexpectedly(&running.source),
                        lines::starts_again()
                    ),
                    PluginError::Remote { message, code, .. } => {
                        lines::could_not_run(&running.name, &format!("{message} (code {code})"))
                    }
                    PluginError::Start { message, .. } => message.clone(),
                    other => lines::could_not_run(&running.name, &other.to_string()),
                };
                let cancelled = summary == lines::cancelled(&running.name);
                if cancelled {
                    said.push(Message::info(summary.clone()));
                } else if !quiet {
                    said.push(Message::error(summary.clone()));
                    if let Some(hint) = hint {
                        let hint = diroxide_plugins::services::clean_text(
                            &hint,
                            diroxide_plugins::services::MAX_ANNOUNCE_CHARS,
                        );
                        said.push(Message::info(format!(
                            "{} By {}.",
                            hint.trim_end_matches('.'),
                            running.source
                        )));
                    }
                    if let Some(last) = &finished.last_error {
                        said.push(Message::info(lines::last_error(last)));
                    }
                    said.push(Message::info(lines::next_log(&running.plugin)));
                    self.note_failure_said(&running.plugin);
                    failed.push(summary.clone());
                }
            }
            Outcome::Script(report) => {
                let (lines_said, extra) = self.script_finished(&running, *report);
                if let Some(first) = lines_said.first() {
                    summary.clone_from(&first.text);
                }
                said.extend(lines_said);
                effects.extend(extra);
            }
        }
        // A lister's rows brought up to date say only a failure, and leave
        // no row in the Jobs list when all went well.
        if running.refresh != Refresh::No {
            said.retain(|m| matches!(m.kind, MessageKind::Error | MessageKind::Warning));
            if said.is_empty() {
                self.jobs.finish(job);
                return effects;
            }
        }
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::PluginCommand,
            summary,
            failed,
            ..FinishedJob::default()
        });
        for message in said {
            effects.push(self.say(message));
        }
        // One of a lister's own commands ended (New site, Delete, Forget
        // password): its rows are brought up to date, quietly.
        if let Some((plugin, lister)) = &running.from_lister {
            effects.extend(self.refresh_lister(plugin, lister, false));
        }
        effects
    }

    /// A script's end: its verdict lines, its deferred moves applied, its
    /// places as a results lister, its output to read.
    fn script_finished(
        &mut self,
        running: &Running,
        report: RunReport,
    ) -> (Vec<Message>, Vec<Effect>) {
        // A script that said its own words and ended well is not said to
        // be done again: its verdict is shown only.
        let quiet = running.announced && report.ending.is_done();
        let mut said: Vec<Message> = report
            .said
            .iter()
            .cloned()
            .map(|m| if quiet { Message::status(m.text) } else { m })
            .collect();
        let failed = !report.ending.is_done() && report.ending != Ending::Cancelled;
        if failed {
            said.push(Message::info(lines::next_log(&running.plugin)));
        }
        let mut effects = Vec::new();
        for directive in &report.deferred {
            effects.extend(self.deferred(running.side, directive));
        }
        if running.mode == Some(RunMode::Lister) && report.ending.is_done() {
            effects.extend(self.places_lister(running, &report));
        }
        let mut output = running.output.clone();
        if !report.output.is_empty() {
            output.extend(report.output.iter().cloned());
        }
        let shows_output = matches!(
            running.mode,
            Some(RunMode::OutputPane | RunMode::OutputViewer)
        );
        if shows_output && !output.is_empty() && self.dialogs.is_empty() {
            let rows = output
                .iter()
                .enumerate()
                .map(|(i, line)| ListRow::new(i.to_string(), line.clone()))
                .collect();
            let mut list = ListModel::new(lines::with_source(
                &format!("Output of {}", running.name),
                &running.source,
            ))
            .with_rows(rows);
            list.set_help("Up and Down read a line, Escape closes.");
            if report.output_cut {
                said.push(Message::info(script_words::output_cut()));
            }
            effects.extend(self.open_dialog(
                Dialog::Choose {
                    list,
                    on_enter: Command::new(ids::ACCEPT),
                },
                Purpose::Plugin(Box::new(Wants::Output)),
            ));
        }
        (said, effects)
    }

    /// A deferred directive, applied as the app's own move.
    fn deferred(&mut self, side: Side, directive: &Directive) -> Vec<Effect> {
        let Some(place) = directive
            .place()
            .and_then(|p| diroxide_plugins::places::plugin_place(p).ok())
        else {
            return Vec::new();
        };
        match directive {
            Directive::Cd(_) => self.open(side, place, None),
            Directive::CdOther(_) => self.open(side.other(), place, None),
            Directive::Select(_) | Directive::Unselect(_) => {
                let select = matches!(directive, Directive::Select(_));
                if let Some(name) = place.name() {
                    self.mark_names(side, &[name.to_owned()], select);
                }
                vec![Effect::Redraw(Redraw::Panel(side))]
            }
            Directive::Cursor(_) => {
                if let Some(name) = place.name() {
                    self.focus_name(side, name);
                }
                vec![Effect::Redraw(Redraw::Panel(side))]
            }
            _ => Vec::new(),
        }
    }

    /// A lister script's places as the results lister of real files: the
    /// panel's keys work on them, as on a search's results.
    fn places_lister(&mut self, running: &Running, report: &RunReport) -> Vec<Effect> {
        let query = running.input.clone().unwrap_or_default();
        let places: Vec<Place> = report
            .places
            .iter()
            .filter_map(|p| diroxide_plugins::places::plugin_place(p).ok())
            .collect();
        if places.is_empty() {
            return vec![self.say(Message::info(script_words::no_results(&query)))];
        }
        let title = script_words::results_title(places.len(), &query, &running.source);
        let id = self.begin_results(None, title.clone());
        self.plugins
            .searches
            .insert(id, (title.clone(), report.capped));
        self.request(Request::StatPlaces { search: id, places });
        self.open_results_saying(title)
    }

    /// Whether a search's places came from a lister script.
    pub(super) fn plugin_owns_search(&self, search: u64) -> bool {
        self.plugins.searches.contains_key(&search)
    }

    /// A lister script's places, read: the results filled.
    pub(super) fn plugin_places_found(
        &mut self,
        search: u64,
        found: Vec<SearchHit>,
        missing: Vec<Place>,
    ) -> Vec<Effect> {
        let Some((title, capped)) = self.plugins.searches.remove(&search) else {
            return Vec::new();
        };
        let mut hits = found;
        for place in &missing {
            let name = place.name().unwrap_or_default().to_owned();
            hits.push(SearchHit::new(
                place.clone(),
                diroxide_core::Entry::new(name, EntryKind::File),
            ));
        }
        let mut line = title;
        if !missing.is_empty() {
            line.push_str(&format!(", {} missing", missing.len()));
        }
        if capped {
            line.push_str(". ");
            line.push_str(&lines::lister_capped());
        }
        self.finish_results(search, hits, line, None)
    }

    /// Cancel's words for a plugin job: "Count lines: cancelling".
    pub(super) fn plugin_cancel_words(&self, job: JobId) -> Option<String> {
        self.plugins
            .running
            .get(&job)
            .map(|r| format!("{}: cancelling", r.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugins_place_arguments_are_parsed_never_trusted() {
        assert!(argument("dest", &Value::String("C:".into())).is_none());
        assert!(matches!(
            argument("text", &Value::String("hello".into())),
            Some(ArgValue::Text(_))
        ));
        assert_eq!(side_of(Side::Left, PanelChoice::Other), Side::Right);
        assert!(acts_on_files(Action::Trash));
        assert!(!acts_on_files(Action::Focus(diroxide_core::Move::Down)));
    }
}
