//! Plugins and script commands in the app (W4d, alpha 2, commits 36 to 40
//! of R22's order): what the app and its runtime say to each other about
//! them, and the models the app shows.
//!
//! - [`catalog`]: the plugins and scripts found, their state and their
//!   commands, made by the runtime on a worker ([`catalog::scan`]).
//! - [`registry`]: what an enabled plugin's commands add to the Plugins
//!   menu, the palette and the keys; a command whose conditions fail is
//!   shown disabled with its reason; one that is not enabled contributes
//!   nothing and appears only in Manage plugins.
//! - [`ask`]: a plugin's question (five kinds) and text documents as the
//!   app's own dialogs, the plugin's name last in every title.
//! - [`lister`]: a plugin's lister of item rows, sorted and filtered by its
//!   columns.
//! - [`settings`] and [`log`]: a plugin's settings under its name, kept in
//!   its state folder, and its log, the last error first.
//!
//! The host runs on the runtime's side ([`crate::runtime`]): each enabled
//! process plugin has its own host and supervisor, so every sentence
//! reaches the right job. A plugin's own requests that need the app (a
//! question, the panels, a job) come as [`PluginEvent::Call`] and are
//! answered with [`PluginRequest::Reply`]; the plugin's router thread
//! waits for the answer, never the input thread.
//!
//! Answer 90: a plugin's name comes last in every line it causes, so the
//! meaning stays in the first 40 cells and a plugin cannot pass for the
//! app. Answer 91: consent covers the manifest and the program file; a
//! change asks again before the next run.

pub mod ask;
pub mod catalog;
pub mod lister;
pub mod log;
pub mod registry;
pub mod settings;

use diroxide_core::{CommandId, Entry, JobId, JobRequest, Message};
use diroxide_ops::JobReport;
use diroxide_plugins::ask::{Answer, Question, TextDocument};
use diroxide_plugins::host::HostEvent;
use diroxide_plugins::listers::ListerView;
use diroxide_plugins::panels::{PanelChange, PanelChoice, PanelSnapshot};
use diroxide_plugins::protocol::{Params, RpcError};
use diroxide_plugins::script::context::RunContext;
use diroxide_plugins::script::runner::RunReport;
use serde_json::Value;

pub use catalog::{Catalog, CommandEntry, Item, ItemKind, ItemState, Runs};
pub use diroxide_plugins::ask::TextDocument as PluginDocument;
/// The plugin host's shapes a frontend and its tests name through the app
/// (the terminal depends on core, app and theme only).
pub use diroxide_plugins::ask::{Choice, PathMode, TextKind};
pub use diroxide_plugins::host::HostEvent as PluginHostEvent;
pub use diroxide_plugins::listers::ListerView as PluginLister;
pub use diroxide_plugins::manifest::{ListerColumn, ListerSpec};
pub use diroxide_plugins::panels::PanelChoice as PluginPanel;
pub use diroxide_plugins::{
    Answer as PluginAnswer, Question as PluginQuestion, QuestionKind as PluginQuestionKind,
};

/// One call a plugin made that the app answers, numbered by the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallId(u64);

impl CallId {
    /// Wraps a raw number.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// What a plugin asks of the app, on its router thread, which waits for
/// the [`Reply`].
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// `ui/ask`: one of the five kinds of question.
    Ask(Box<Question>),
    /// `ui/show_text`: a document to read; answered when it is shown, or
    /// when the owner closes it if the plugin waits.
    ShowText(Box<TextDocument>),
    /// `panel/get`.
    Panel(PanelChoice),
    /// `panel/set`, made through the app's own commands.
    SetPanel(Box<PanelChange>),
    /// `fs/*`: a job through the app's confirmed job path.
    RunJob(Box<JobRequest>),
    /// `lister/open`.
    OpenLister(Box<ListerView>),
    /// `command/execute`: a diroxide command by id with its arguments.
    Execute {
        /// The command.
        command: CommandId,
        /// Its arguments.
        args: Params,
    },
}

/// The app's answer to a [`Call`].
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The owner's answer to a question.
    Answer(Answer),
    /// A document was shown (or closed, when the plugin waits).
    Shown,
    /// A panel's state, or none.
    Panel(Option<Box<PanelSnapshot>>),
    /// What a call came to: a value, or the error the plugin receives.
    Done(Result<Value, RpcError>),
    /// A job's report, or why it did not run.
    Job(Result<Box<JobReport>, RpcError>),
}

/// A plugin command to run as a job.
#[derive(Clone, Debug, PartialEq)]
pub struct RunRequest {
    /// The job's id, from the app.
    pub job: JobId,
    /// The command's full id.
    pub command: CommandId,
    /// The panels and the selection, frozen when it was asked.
    pub context: RunContext,
    /// For a lister's command: the lister's id and its chosen rows.
    pub lister: Option<(String, Vec<Entry>)>,
    /// Run once without enabling (after its own question).
    pub once: bool,
}

/// What the app asks the runtime about plugins.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginRequest {
    /// Find the plugins and scripts and read their state; answered with
    /// [`PluginEvent::Found`].
    Scan,
    /// Records the owner's yes for an item, then scans again.
    Enable {
        /// The item's id.
        id: String,
    },
    /// Forgets the owner's yes for an item, stops it, then scans again.
    Disable {
        /// The item's id.
        id: String,
    },
    /// Turns a plugin on again after a policy turned it off.
    TurnOn {
        /// The plugin's id.
        id: String,
    },
    /// Runs a command as a job; answered with [`PluginEvent::Finished`].
    Run(Box<RunRequest>),
    /// Answers a plugin's call.
    Reply {
        /// The call.
        call: CallId,
        /// The answer.
        reply: Box<Reply>,
    },
    /// Changes a plugin's setting (or puts it back with `None`): written
    /// in its state folder, and sent to it as `settings/changed`.
    Setting {
        /// The plugin's id.
        id: String,
        /// The setting's key.
        key: String,
        /// Its new value.
        value: Option<Value>,
    },
    /// Reads an item's log; answered with [`PluginEvent::Logged`].
    Log {
        /// The item's id.
        id: String,
    },
    /// Asks a plugin for candidates for its prompt as the owner types
    /// (`completion/query`); answered with [`PluginEvent::Candidates`]. A
    /// new query cancels the one before it.
    Complete {
        /// The question's call.
        call: CallId,
        /// The question, whose completer asks the plugin.
        question: Box<Question>,
        /// The text typed so far.
        text: String,
        /// The cursor's place in it, in characters.
        cursor: usize,
    },
    /// More rows of a plugin's lister (`lister/rows`), or all of them
    /// again; answered with [`PluginEvent::ListerRows`].
    ListerRows {
        /// The plugin's id.
        plugin: String,
        /// The lister's id.
        lister: String,
        /// How many rows the app holds.
        offset: u64,
        /// Whether to send every row again.
        refresh: bool,
    },
    /// The owner left a plugin's lister.
    ListerClosed {
        /// The plugin's id.
        plugin: String,
        /// The lister's id.
        lister: String,
    },
}

/// How a plugin command ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// A script or manifest-only command ran: the runner's report.
    Script(Box<RunReport>),
    /// A plugin's program answered `command/run`.
    Process(Result<Value, diroxide_plugins::PluginError>),
    /// It did not run at all: the sentence, meaning first.
    Refused(String),
    /// Its consent no longer covers it: the question to ask again.
    Changed(Vec<String>),
    /// It was cancelled before it started.
    Cancelled,
}

/// A plugin command's end.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    /// The job.
    pub job: JobId,
    /// The command's full id.
    pub command: CommandId,
    /// What it came to.
    pub outcome: Outcome,
    /// The last lines of the command's error output, for the failure's
    /// cause line.
    pub last_error: Option<String>,
    /// How long the plugin had to answer, for a time-out's words.
    pub limit: std::time::Duration,
}

/// What the runtime tells the app about plugins.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginEvent {
    /// The plugins and scripts found, with their state.
    Found(Box<Catalog>),
    /// A plugin asks something of the app; answer with
    /// [`PluginRequest::Reply`].
    Call {
        /// The call, for the reply.
        call: CallId,
        /// The plugin's id.
        plugin: String,
        /// What it asks.
        request: Call,
    },
    /// A plugin said something, already cleaned and attributed.
    Said {
        /// The plugin's id.
        plugin: String,
        /// What it said.
        message: Message,
    },
    /// A plugin's progress, as its job's sentence.
    Progress {
        /// The plugin's id.
        plugin: String,
        /// The sentence: "Counting 12 of 300 files (Line counter)".
        message: Message,
    },
    /// Something the host says about a plugin: it did not start, it waits,
    /// it timed out, stopped, was turned off, flooded, stopped when idle,
    /// or its provider lost its device.
    Host(HostEvent),
    /// A line of a running command's output (`output-pane`).
    Output {
        /// The job.
        job: JobId,
        /// The line, cleaned.
        line: String,
    },
    /// A script asked the panels to be listed again.
    Refresh,
    /// A command ended.
    Finished(Box<Finished>),
    /// A plugin's candidates for the text typed in its prompt, each the
    /// text and its kind in a word.
    Candidates {
        /// The question's call.
        call: CallId,
        /// The text they are for.
        text: String,
        /// The candidates.
        candidates: Vec<(String, String)>,
    },
    /// Rows of a plugin's lister.
    ListerRows {
        /// The plugin's id.
        plugin: String,
        /// The lister's id.
        lister: String,
        /// The rows.
        rows: Vec<Entry>,
        /// Whether they replace the rows held.
        refresh: bool,
    },
    /// An item's log, newest line first.
    Logged {
        /// The item's id.
        id: String,
        /// The lines.
        lines: Vec<String>,
    },
    /// A sentence of the runtime's about plugins: "Enabled: Line
    /// counter.", "Could not save the consent: ...".
    Note(Message),
}
