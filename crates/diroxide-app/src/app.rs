//! The app: two panels, one active; keys become commands through the
//! keymap; commands change state and return effects. The app never does
//! I/O: what needs the disk goes out as an [`Effect`] or a [`Request`] for
//! the runtime, and the answers come back as [`AppEvent`]s.
//!
//! Every alpha.1 command is wired here (plan section 8): moving, the
//! panels, the history, selection, sorting and the filter, the file
//! operations through the job engine with their confirmations, the file
//! clipboard, undo, the menus, the palette, settings, help, Say Status and
//! Say Other Panel, the run-command prompt, shell here and the editor,
//! the key test, and quit. The behavior lives in the submodules:
//! navigation (`nav`), selection, sort, filter and type-ahead (`select`),
//! jobs and their questions (`files`), dialogs and their keys
//! (`dialogs`), and the lists and words (`info`).
//!
//! Wave 1 (W1d) owns this module.

mod access_dialogs;
mod archive;
mod archive_forms;
mod archive_jobs;
mod bookmarking;
// Compare files (W3a).
mod comparing;
// Optional components (W4d).
mod component_ui;
mod dialogs;
mod files;
// The panel filter on the filter language (W3a).
mod filtering;
mod folder_views;
mod info;
mod keys;
mod labeling;
mod nav;
mod organize;
// Plugins and script commands (W4d).
mod plugin_ui;
mod preview;
// Saved searches, the Space list, sizes and newest files (W3a).
mod search_tools;
// Search, its dialog and its results (W3a).
mod searching;
mod select;
// Selection on the filter language (W3a).
mod selecting;
mod split_join;
mod viewing;
// Watch this folder (W3t).
mod watching;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use diroxide_config::{ConfigProblem, Settings};
use diroxide_core::{
    Clock, Command, CommandId, DialogId, Effect, Entry, IdSource, JobId, Key, Message, Move, Place,
    Redraw, RequestId, Side, SystemClock, Timestamp,
};
use diroxide_filetype::TypeRegistry;
use diroxide_ops::{Interrupted, JobProgress, JobReport, OpsError, Plan, UndoLog};
use diroxide_vfs::{Capabilities, LocalPlace, VfsError, WatchEvent};

use crate::commands::ids;
use crate::dialog::{Answer, Dialog};
use crate::jobs::JobTable;
use crate::keymap::{Keymap, Lookup};
use crate::menu::{Bank, Menus};
use crate::palette::Palette;
use crate::panel::Panel;
use crate::request::{AskId, Question, Request};
use crate::startup::Startup;
use crate::state::{self, Visits};

pub use files::Clipboard;
pub use filtering::FilterContents;

/// What the app starts with.
#[derive(Clone, Debug)]
pub struct AppOptions {
    /// The left panel's place.
    pub left: Place,
    /// The right panel's place.
    pub right: Place,
    /// The settings.
    pub settings: Settings,
    /// The keymap.
    pub keymap: Keymap,
    /// The file types.
    pub types: TypeRegistry,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// What was read at start: problems, menus, banks, the session, the
    /// journal, the histories (see [`crate::startup::load`]).
    pub startup: Startup,
}

impl AppOptions {
    /// Two places with default settings, keys and types, and the system
    /// clock at the local offset; nothing read from disk.
    pub fn new(left: Place, right: Place) -> Self {
        Self {
            left,
            right,
            settings: Settings::default(),
            keymap: Keymap::defaults(),
            types: TypeRegistry::builtin(),
            clock: Arc::new(crate::startup::local_clock()),
            startup: Startup::default(),
        }
    }

    /// The same options in UTC, as Phase 0 had them (for tests that fix
    /// their own clock anyway).
    pub fn in_utc(mut self) -> Self {
        self.clock = Arc::new(SystemClock::utc());
        self
    }
}

/// Something that happened, for the app to handle.
#[derive(Debug)]
pub enum AppEvent {
    /// A key was pressed.
    Key(Key),
    /// A command was asked for: by a menu, the palette, a button, a plugin
    /// or `diro`.
    Command(Command),
    /// A listing finished.
    Listed {
        /// The request it answers.
        request: RequestId,
        /// The place listed.
        place: Place,
        /// The entries, or why not.
        result: Result<Vec<Entry>, VfsError>,
    },
    /// A listing is on its way: how many entries so far.
    ListProgress {
        /// The request.
        request: RequestId,
        /// Entries read so far.
        so_far: u64,
    },
    /// What a listed place can do (sent before its listing).
    PlaceFacts {
        /// The place.
        place: Place,
        /// What it can do.
        capabilities: Capabilities,
    },
    /// A job was planned, or could not be.
    Planned {
        /// The job.
        job: JobId,
        /// The plan, or why not.
        result: Result<Box<Plan>, OpsError>,
    },
    /// A job made progress.
    JobProgress {
        /// The job.
        job: JobId,
        /// How far it got.
        progress: JobProgress,
    },
    /// A job finished.
    JobFinished(Box<JobReport>),
    /// A worker waits for an answer.
    Asks {
        /// The question's number, for the answer.
        ask: AskId,
        /// The question.
        question: Question,
    },
    /// A watched folder changed.
    Watched {
        /// The panel.
        side: Side,
        /// The folder watched.
        place: Place,
        /// What changed.
        event: WatchEvent,
    },
    /// A folder cannot be watched (a network drive, a watch that failed to
    /// start): it is re-read when its panel gets the focus.
    NotWatched {
        /// The panel.
        side: Side,
        /// The folder.
        place: Place,
    },
    /// The drives, the root, the home folder and the mounts.
    Places(Vec<LocalPlace>),
    /// A program that had the terminal ended.
    ProgramEnded {
        /// What ran, as said: "Shell", "Command", the edited file's name.
        what: String,
        /// The folder it ran in.
        folder: Place,
        /// Its exit code, or why it could not start.
        result: Result<Option<i32>, String>,
    },
    /// An archive job was planned, or could not be (the sentence says
    /// why).
    ArchivePlanned {
        /// The job.
        job: JobId,
        /// The plan, or why not.
        result: Result<Box<crate::request::ArchivePlan>, Message>,
    },
    /// An archive's entries were counted for the details list.
    ArchiveEntries {
        /// The archive.
        place: Place,
        /// "Entries: 10 items", or why not.
        line: Result<String, Message>,
    },
    /// A file's bytes for the viewer were read, or could not be (the
    /// sentence says why).
    ViewerSource {
        /// The request it answers.
        request: RequestId,
        /// The file.
        place: Place,
        /// The bytes, or why not.
        result: Result<crate::viewer::ViewerSource, Message>,
    },
    /// An archive job ended: its report, and the runner's lines, the
    /// summary first ("Unpack finished: 4 done.", "Checked 300 of 300
    /// entries, all good.").
    ArchiveFinished {
        /// The report.
        report: Box<JobReport>,
        /// The runner's lines.
        lines: Vec<String>,
    },
    /// What a listed place inside an archive is: its title's words and
    /// the opening line (sent before its listing).
    ArchiveOpened {
        /// The listing request it belongs to.
        request: RequestId,
        /// The facts.
        facts: Box<crate::archives::ArchiveFacts>,
    },
    /// Hits of a search arrived, in the order found (W3a).
    SearchFound {
        /// The search, as [`Request::Search`] numbered it.
        search: u64,
        /// The hits.
        hits: Vec<diroxide_search::SearchHit>,
    },
    /// How far a search got (W3a).
    SearchProgress {
        /// The search.
        search: u64,
        /// How far.
        progress: diroxide_search::SearchProgress,
    },
    /// A search ended (W3a): its summary, or why it could not run.
    SearchDone {
        /// The search.
        search: u64,
        /// The summary, or why not.
        result: Result<diroxide_search::SearchSummary, Message>,
    },
    /// A compare ended (W3a): what it found, or why not.
    Compared {
        /// The request.
        request: RequestId,
        /// The comparison, or why not.
        result: Result<Box<diroxide_diff::FileComparison>, Message>,
    },
    /// How far a compare got (W3a), in bytes read of both files.
    CompareProgress {
        /// The request.
        request: RequestId,
        /// Bytes read so far.
        done: u64,
        /// Bytes in all, when known.
        total: Option<u64>,
    },
    /// The saved searches were read (W3a), each file with its search or
    /// why it could not be read.
    SavedListed {
        /// The request.
        request: RequestId,
        /// The files found.
        found: Vec<(std::path::PathBuf, Result<diroxide_search::Saved, String>)>,
    },
    /// A Space list was counted (W3a).
    SpaceListed {
        /// The request.
        request: RequestId,
        /// The list, or why not.
        result: Result<diroxide_search::SpaceList, Message>,
    },
    /// The largest items below a folder were found (W3a).
    LargestFound {
        /// The results' search number.
        search: u64,
        /// What was found, or why not.
        result: Result<diroxide_search::Largest, Message>,
    },
    /// Folders were sized (W3a), each with its size or why not.
    DirSized {
        /// The request.
        request: RequestId,
        /// The sizes.
        sizes: Vec<(Place, Result<diroxide_search::DirSize, Message>)>,
    },
    /// Newest files were found (W3a).
    NewestFound {
        /// The results' search number.
        search: u64,
        /// What was found, or why not.
        result: Result<diroxide_search::Newest, Message>,
    },
    /// A label's items were looked up for the results lister (W3a).
    LabeledFound {
        /// The search.
        search: u64,
        /// The items there.
        found: Vec<diroxide_search::SearchHit>,
        /// The items gone.
        missing: Vec<Place>,
        /// How many could not be looked at.
        unreadable: usize,
    },
    /// A result's file was checked when it took the focus (W3a).
    StaleChecked {
        /// The search.
        search: u64,
        /// The result's row.
        row: String,
        /// Whether its file is as the list says.
        freshness: diroxide_search::Freshness,
    },
    /// The files a panel's content filter asked about were read (W3a):
    /// those that matched, or why not.
    FilterContents {
        /// The request it answers.
        request: RequestId,
        /// What was read, or why not.
        result: Result<FilterContents, Message>,
    },
    /// What the arrival watch of Watch this folder saw (W3t), tagged with
    /// its watch, so a note from a watch already stopped is dropped.
    Arrivals {
        /// The watch, as [`Request::WatchArrivals`] numbered it.
        watch: u64,
        /// What it saw.
        note: diroxide_ops::tracker::WatchNote,
    },
    /// The places a [`Request::CheckPlaces`] asked about were looked for
    /// (W3l): the ones gone, and how many could not be read.
    PlacesChecked {
        /// The request it answers.
        request: RequestId,
        /// The places that are gone.
        missing: Vec<Place>,
        /// How many could not be looked at (a drive that is not there, no
        /// permission).
        unreadable: usize,
    },
    /// A dialog was answered.
    Answer {
        /// The dialog.
        dialog: DialogId,
        /// The answer.
        answer: Answer,
    },
    /// Something outside the app failed, already in words.
    Problem(Message),
    /// Something about plugins: the catalog, a plugin's call, its words,
    /// its progress, a command's end (W4d).
    Plugin(Box<crate::plugins::PluginEvent>),
    /// Something about optional components: the list, a download's
    /// progress and end, a check, a removal (W4d).
    Component(Box<crate::components::ComponentEvent>),
}

/// What an open dialog is for: what its answer does. The [`Dialog`] is
/// what the frontend draws; this stays in the app.
#[derive(Clone, Debug)]
enum Purpose {
    /// Run a planned job on Yes.
    RunPlan(Box<Plan>),
    /// The typed confirmation of a big plan.
    TypedPlan(Box<Plan>, String),
    /// Quit on Yes.
    Quit,
    /// Run a program on Yes.
    RunProgram(Place),
    /// A job's conflict.
    Conflict {
        ask: AskId,
        conflict: Box<diroxide_ops::Conflict>,
        choices: Vec<diroxide_ops::ConflictChoice>,
    },
    /// The name for "Rename this copy".
    ConflictName { ask: AskId },
    /// An item's error.
    ItemError {
        ask: AskId,
        choices: Vec<diroxide_core::ErrorAction>,
    },
    /// A password.
    Secret { ask: AskId },
    /// "Wrong password for course.zip. Try again? y or n", before the
    /// next try's prompt.
    WrongPassword {
        ask: AskId,
        request: diroxide_core::SecretRequest,
    },
    /// Rename, one entry at a time: the one asked now, the ones after,
    /// and the answers so far.
    Rename {
        side: Side,
        current: Place,
        rest: Vec<Place>,
        done: Vec<(Place, String)>,
    },
    /// Duplicate, one entry at a time.
    Duplicate {
        side: Side,
        current: Place,
        rest: Vec<Place>,
    },
    /// Make a folder.
    MakeFolder(Side),
    /// Go to a path.
    GoPath(Side),
    /// Go to a place, and path jump.
    Places(Side),
    /// The folder history.
    History(Side),
    /// The filter prompt, live; the filter before it opened.
    Filter(Side, Option<diroxide_core::filter::Filter>),
    /// The question before a panel's filter reads files (W3a).
    FilterContents {
        side: Side,
        filter: Box<diroxide_core::filter::Filter>,
        previous: Option<Box<diroxide_core::filter::Filter>>,
    },
    /// Select or unselect by pattern.
    Pattern { side: Side, select: bool },
    /// The count before a pattern's selection changes (W3a).
    SelectCount {
        side: Side,
        select: bool,
        ids: Vec<String>,
    },
    /// Select by kind's list (W3a).
    SelectKind(Side),
    /// Select the next rows' count (W3a).
    NextRows(Side),
    /// The search dialog: its settings (W3a).
    SearchForm(Box<searching::SearchForm>),
    /// One of the search dialog's text settings being typed (W3a).
    SearchText(Box<searching::SearchForm>, searching::SearchField),
    /// The results lister (W3a).
    Results,
    /// The results' refining filter (W3a).
    Refine,
    /// The actions on the results (W3a).
    ResultsMenu,
    /// The name to save the last search under (W3a).
    SaveSearchName,
    /// The saved searches (W3a).
    SavedList(Vec<search_tools::SavedRow>),
    /// The question before a saved search is deleted (W3a).
    DeleteSaved(std::path::PathBuf, String),
    /// A Space list (W3a).
    Space(Box<diroxide_search::SpaceList>),
    /// The size sort offered after Size every folder (W3a).
    OfferSizeSort,
    /// A compare's change list (W3a).
    Compare(Box<comparing::Compared>),
    /// A compare's unified view (W3a).
    Unified(Box<comparing::Compared>),
    /// The choice for a file too large to compare as text (W3a).
    CompareLarge(Box<comparing::Pair>),
    /// The actions on a compare (W3a).
    CompareMenu,
    /// Copy or move to a place picked from a list.
    PickDestination {
        side: Side,
        kind: diroxide_core::JobKind,
    },
    /// The run-command prompt.
    RunCommand(Side),
    /// The palette.
    Palette,
    /// The menu bar.
    MenuBar,
    /// A menu, by id.
    Menu(String),
    /// The context menu.
    ContextMenu,
    /// The Sort list.
    SortList,
    /// An archive limit's question.
    Limit { ask: AskId },
    /// One of an archive plan's limit questions, before its confirmation.
    ArchiveQuestion {
        plan: Box<crate::request::ArchivePlan>,
        question: diroxide_archive::Question,
        rest: Vec<diroxide_archive::Question>,
    },
    /// Run an archive job on Yes.
    RunArchive(Box<crate::request::ArchivePlan>),
    /// One of a plan's questions, before its confirmation.
    PlanQuestion {
        plan: Box<Plan>,
        question: Box<diroxide_ops::Preflight>,
        rest: Vec<diroxide_ops::Preflight>,
        forced: bool,
    },
    /// The pack dialog: its settings.
    PackForm(Box<archive_forms::PackForm>),
    /// The pack dialog's name field.
    PackName(Box<archive_forms::PackForm>),
    /// A pack setting's choices.
    PackChoice(Box<archive_forms::PackForm>, archive_forms::PackChoice),
    /// A new archive password, typed twice.
    NewPassword {
        form: Box<archive_forms::PackForm>,
        first: Option<diroxide_core::Secret>,
    },
    /// The unpack dialog: its settings.
    UnpackForm(Box<archive_forms::UnpackForm>),
    /// Split file's sizes.
    SplitSize {
        side: Side,
        files: Vec<Place>,
        destination: Place,
        note: Option<String>,
    },
    /// Split file's number of parts or part size.
    SplitTyped {
        side: Side,
        files: Vec<Place>,
        destination: Place,
        note: Option<String>,
        equal: bool,
    },
    /// The offer to trash a verified join's parts.
    TrashParts(Vec<Place>),
    /// The link dialog.
    LinkForm(Box<access_dialogs::LinkSettings>),
    /// The junction offered after the privilege's refusal.
    JunctionInstead { targets: Vec<Place>, folder: Place },
    /// Change link target's prompt.
    Relink { side: Side, link: Place },
    /// The permissions dialog.
    AccessForm(Box<access_dialogs::AccessForm>),
    /// The details of an entry.
    Details(Place),
    /// The viewer's jump prompt.
    ViewerJump,
    /// The viewer's search prompt.
    ViewerSearch { backward: bool },
    /// The viewer's outline.
    ViewerOutline,
    /// The preview's find prompt.
    PreviewFind,
    /// The preview's links.
    PreviewLinks,
    /// The question before opening a link from the preview.
    PreviewLink,
    /// The settings list.
    Settings,
    /// A setting's choices.
    SettingValue(String),
    /// A setting typed in a field.
    SettingText(String),
    /// A list to read: details, help, problems.
    Read,
    /// The labels, bookmarks, folder views and History list (W3l): what
    /// their dialogs are for.
    Organize(Box<organize::Wants>),
    /// The job history.
    JobHistory,
    /// One job's items.
    JobItems,
    /// A dialog of Change key, the Keys list or the presets (W3k).
    Keys(Box<keys::KeysPurpose>),
    /// A plugin's dialog, or one of Manage plugins (W4d).
    Plugin(Box<plugin_ui::Wants>),
    /// A dialog of the optional components (W4d).
    Component(Box<component_ui::Wants>),
}

/// An open dialog's purpose and moment of opening, beside the dialog
/// itself in the app's dialog stack.
#[derive(Clone, Debug)]
struct Open {
    purpose: Purpose,
    opened: Timestamp,
}

/// What a planned or running job is for, beyond its plan.
#[derive(Clone, Debug)]
struct Flow {
    /// The panel the action started from.
    side: Side,
    /// The selection frozen when it was asked, for the plan's first line.
    note: Option<String>,
    /// What to do when it ends.
    then: Then,
    /// Whether the plan is shown and confirmed before it runs.
    confirm: bool,
    /// What kind of job, for its words.
    kind: diroxide_core::JobKind,
}

/// What a finished job leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Then {
    /// Nothing more than its report.
    Report,
    /// Focus the new folder: "Created folder Reports".
    MadeFolder(String),
    /// Focus the renamed entry: "Renamed a to b".
    Renamed(Vec<(String, String)>),
    /// Clear the file clipboard (a cut was pasted).
    ClearClipboard,
    /// Links made: a folder link the privilege refused is offered a
    /// junction.
    Linked {
        targets: Vec<Place>,
        folder: Place,
        form: diroxide_core::access::LinkForm,
    },
    /// Say what an undo brought back.
    Undid(Option<diroxide_core::JobKind>),
    /// The permanent delete the trash could not take: its plan is shown
    /// under the "no trash" question ("This drive has no trash. Delete 3
    /// items for good?").
    NoTrash(Message),
    /// A plugin's file job: the plugin hears its report (W4d).
    PluginJob(crate::plugins::CallId),
}

/// Type-ahead: the text typed so far and when it was last typed.
#[derive(Clone, Debug)]
struct Quick {
    text: String,
    at: Timestamp,
}

/// Path completion waiting for a folder's listing.
#[derive(Clone, Debug)]
struct Completing {
    request: RequestId,
    folder: Place,
    entries: Option<Vec<Entry>>,
}

/// The app core.
pub struct App {
    panels: [Panel; 2],
    active: Side,
    keymap: Keymap,
    settings: Settings,
    types: TypeRegistry,
    clock: Arc<dyn Clock>,
    ids: IdSource,
    dialogs: Vec<(DialogId, Dialog)>,
    opens: Vec<Open>,
    jobs: JobTable,
    undo: UndoLog,
    status: String,
    /// The status lines said since the frontend last took them.
    status_log: Vec<String>,
    quit: bool,
    requests: Vec<Request>,
    next_job: u64,
    flows: HashMap<JobId, Flow>,
    menus: Menus,
    banks: Vec<Bank>,
    /// The bank the key bar shows.
    bank: usize,
    palette: Palette,
    histories: BTreeMap<String, Vec<String>>,
    visits: Visits,
    clipboard: Option<Clipboard>,
    problems: Vec<ConfigProblem>,
    interrupted: Vec<Interrupted>,
    home: Option<Place>,
    screen_reader: bool,
    quick: Option<Quick>,
    key_test: bool,
    quit_folder: Option<Place>,
    keep_shell: bool,
    completing: Option<Completing>,
    /// The field's text as the last Tab left it with the list open, so a
    /// second Tab moves into the list (plan section 5.19).
    tabbed: Option<String>,
    places: Vec<LocalPlace>,
    first_run: bool,
    session: Option<diroxide_config::SessionState>,
    restore: [Option<String>; 2],
    kept: Vec<Kept>,
    /// What the runtime said about the archive each panel shows.
    archive_facts: [Option<crate::archives::ArchiveFacts>; 2],
    /// Archive facts waiting for their listing, by request.
    pending_archive: Vec<(RequestId, crate::archives::ArchiveFacts)>,
    /// Whether each panel's last listing entered its archive from outside.
    archive_entered: [bool; 2],
    /// The runner's lines of the archive job ending now, which its result
    /// says instead of the report's summary.
    finish_lines: Option<Vec<String>>,
    /// The open viewer.
    viewing: Option<viewing::Viewing>,
    /// The viewer source being read, and what for.
    viewer_wait: Option<(RequestId, viewing::SourceFor)>,
    /// The highlighter, one for the run, handed to each viewer.
    highlighter: Option<crate::viewer::highlight::Session>,
    /// The quick view and the focus in its region.
    preview: preview::Preview,
    /// Change key's capture, the Keys list and the presets (W3k).
    keys: keys::KeysState,
    /// The labels, bookmarks and folder views, and what their lists wait
    /// for (W3l).
    organize: organize::Organize,
    /// The folder watched for arrivals (W3t).
    watching: watching::WatchState,
    /// The filter's pause, stashes and content reads (W3a).
    filtering: filtering::FilterState,
    /// Where each panel's range started (W3a).
    selecting: selecting::SelectState,
    /// The search dialog, the results and their parking (W3a).
    searching: searching::SearchState,
    /// What the search tools wait for (W3a).
    tools: search_tools::ToolsState,
    /// The compare on its way (W3a).
    comparing: comparing::CompareState,
    /// Plugins and script commands (W4d).
    plugins: plugin_ui::PluginState,
    /// Optional components (W4d).
    components: component_ui::ComponentState,
}

/// A selection a wholesale change replaced, kept for Restore selection:
/// the folder it was in and its marks by id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Kept {
    place: Place,
    ids: Vec<String>,
}

/// How many folders' replaced selections are kept.
const KEPT_LIMIT: usize = 16;

/// How many status lines wait for a frontend that does not take them.
const STATUS_LOG_LIMIT: usize = 16;

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("active", &self.active)
            .field("left", self.panels[0].place())
            .field("right", self.panels[1].place())
            .field("dialogs", &self.dialogs.len())
            .field("jobs", &self.jobs.len())
            .finish_non_exhaustive()
    }
}

/// What a key or command means for the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Focus(Move),
    SwitchPanel,
    SwapPanels,
    ShowInOther,
    ThisInOther,
    Enter,
    Into,
    OpenAsArchive,
    Split,
    Join,
    MakeLink,
    Relink,
    Permissions,
    CopyPath,
    CopyName,
    Pack,
    Unpack,
    TestArchive,
    Up,
    Reload,
    Back,
    Forward,
    Home,
    GoPath,
    Places,
    History,
    ToggleSelect,
    SelectAll,
    SelectNone,
    SelectInvert,
    SelectPattern,
    UnselectPattern,
    SelectNext,
    SelectPrevious,
    SelectRestore,
    View,
    Edit,
    Copy,
    Move,
    CopyTo,
    MoveTo,
    MakeFolder,
    Trash,
    DeletePermanently,
    Rename,
    Duplicate,
    Details,
    ContextMenu,
    RunCommand,
    ShellHere,
    ClipCopy,
    ClipCut,
    Paste,
    ToggleHidden,
    Filter,
    FilterClear,
    Sort,
    SortList,
    Plugins,
    Quit,
    QuitToFolder,
    QuitKeepFolder,
    Help,
    SayStatus,
    SayOther,
    SayKeyBar,
    SwitchBank,
    Palette,
    MenuBar,
    Menu,
    Settings,
    KeyTest,
    Cancel,
    Layout,
    NextTheme,
    Problems,
    Undo,
    JobHistory,
    Yes,
    No,
    Accept,
    QuickView,
    OpenInTextweaver,
}

const ROUTES: [(CommandId, Action); 105] = [
    (ids::QUIT, Action::Quit),
    (ids::QUIT_TO_FOLDER, Action::QuitToFolder),
    (ids::QUIT_KEEP_FOLDER, Action::QuitKeepFolder),
    (ids::HELP, Action::Help),
    (ids::SAY_STATUS, Action::SayStatus),
    (ids::SAY_OTHER, Action::SayOther),
    (ids::SAY_KEY_BAR, Action::SayKeyBar),
    (ids::SWITCH_BANK, Action::SwitchBank),
    (ids::PALETTE, Action::Palette),
    (ids::MENU, Action::MenuBar),
    (ids::SETTINGS, Action::Settings),
    (ids::KEY_TEST, Action::KeyTest),
    (ids::CANCEL, Action::Cancel),
    (ids::LAYOUT, Action::Layout),
    (ids::NEXT_THEME, Action::NextTheme),
    (ids::PROBLEMS, Action::Problems),
    (ids::UNDO, Action::Undo),
    (ids::JOB_HISTORY, Action::JobHistory),
    (ids::MENU_FILE, Action::Menu),
    (ids::MENU_EDIT, Action::Menu),
    (ids::MENU_VIEW, Action::Menu),
    (ids::MENU_GO, Action::Menu),
    (ids::MENU_SELECT, Action::Menu),
    (ids::MENU_TOOLS, Action::Menu),
    (ids::MENU_PLUGINS, Action::Menu),
    (ids::MENU_HELP, Action::Menu),
    (ids::SWITCH_PANEL, Action::SwitchPanel),
    (ids::SWAP_PANELS, Action::SwapPanels),
    (ids::SHOW_IN_OTHER, Action::ShowInOther),
    (ids::THIS_IN_OTHER, Action::ThisInOther),
    (ids::ENTER, Action::Enter),
    (ids::INTO, Action::Into),
    (ids::OPEN_AS_ARCHIVE, Action::OpenAsArchive),
    (ids::MAKE_LINK, Action::MakeLink),
    (ids::RELINK, Action::Relink),
    (ids::PERMISSIONS, Action::Permissions),
    (ids::COPY_PATH, Action::CopyPath),
    (ids::COPY_NAME, Action::CopyName),
    (ids::SPLIT, Action::Split),
    (ids::JOIN, Action::Join),
    (ids::PACK, Action::Pack),
    (ids::UNPACK, Action::Unpack),
    (ids::TEST_ARCHIVE, Action::TestArchive),
    (ids::UP, Action::Up),
    (ids::RELOAD, Action::Reload),
    (ids::BACK, Action::Back),
    (ids::FORWARD, Action::Forward),
    (ids::HOME, Action::Home),
    (ids::GO_PATH, Action::GoPath),
    (ids::PLACES, Action::Places),
    (ids::HISTORY, Action::History),
    (ids::LIST_UP, Action::Focus(Move::Up)),
    (ids::LIST_DOWN, Action::Focus(Move::Down)),
    (ids::LIST_PAGE_UP, Action::Focus(Move::PageUp)),
    (ids::LIST_PAGE_DOWN, Action::Focus(Move::PageDown)),
    (ids::LIST_HOME, Action::Focus(Move::Home)),
    (ids::LIST_END, Action::Focus(Move::End)),
    (ids::SELECT_TOGGLE, Action::ToggleSelect),
    (ids::SELECT_ALL, Action::SelectAll),
    (ids::SELECT_NONE, Action::SelectNone),
    (ids::SELECT_INVERT, Action::SelectInvert),
    (ids::SELECT_PATTERN, Action::SelectPattern),
    (ids::UNSELECT_PATTERN, Action::UnselectPattern),
    (ids::SELECT_NEXT, Action::SelectNext),
    (ids::SELECT_PREVIOUS, Action::SelectPrevious),
    (ids::SELECT_RESTORE, Action::SelectRestore),
    (ids::VIEW, Action::View),
    (ids::EDIT, Action::Edit),
    (ids::COPY, Action::Copy),
    (ids::MOVE, Action::Move),
    (ids::COPY_TO, Action::CopyTo),
    (ids::MOVE_TO, Action::MoveTo),
    (ids::MAKE_FOLDER, Action::MakeFolder),
    (ids::TRASH, Action::Trash),
    (ids::DELETE_PERMANENTLY, Action::DeletePermanently),
    (ids::RENAME, Action::Rename),
    (ids::DUPLICATE, Action::Duplicate),
    (ids::DETAILS, Action::Details),
    (ids::CONTEXT_MENU, Action::ContextMenu),
    (ids::RUN_COMMAND, Action::RunCommand),
    (ids::SHELL_HERE, Action::ShellHere),
    (ids::CLIPBOARD_COPY, Action::ClipCopy),
    (ids::CLIPBOARD_CUT, Action::ClipCut),
    (ids::CLIPBOARD_PASTE, Action::Paste),
    (ids::TOGGLE_HIDDEN, Action::ToggleHidden),
    (ids::FILTER, Action::Filter),
    (ids::FILTER_CLEAR, Action::FilterClear),
    (ids::SORT_NAME, Action::Sort),
    (ids::SORT_EXTENSION, Action::Sort),
    (ids::SORT_MODIFIED, Action::Sort),
    (ids::SORT_SIZE, Action::Sort),
    (ids::SORT_UNSORTED, Action::Sort),
    (ids::SORT_CREATED, Action::Sort),
    (ids::SORT_ACCESSED, Action::Sort),
    (ids::SORT_KIND, Action::Sort),
    (ids::SORT_ATTRIBUTES, Action::Sort),
    (ids::SORT_OWNER, Action::Sort),
    (ids::SORT_LABEL, Action::Sort),
    (ids::SORT_LIST, Action::SortList),
    (ids::PLUGINS, Action::Plugins),
    (ids::YES, Action::Yes),
    (ids::NO, Action::No),
    (ids::ACCEPT, Action::Accept),
    (ids::QUICK_VIEW, Action::QuickView),
    (ids::OPEN_IN_TEXTWEAVER, Action::OpenInTextweaver),
];

fn route(id: &CommandId) -> Option<Action> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// An app with two panels, the left one active. Call [`App::start`]
    /// for the first listings and the startup lines.
    pub fn new(options: AppOptions) -> Self {
        let mut settings = options.settings;
        let startup = options.startup;
        let owner_keys = startup.keymap.clone();
        if let Some(layout) = startup.layout {
            settings.layout = layout;
        }
        let keymap = options.keymap.with_single_letters(settings.single_letters);
        let mut undo = UndoLog::default();
        for record in startup.undo.iter().cloned() {
            undo.record(record);
        }
        let palette = startup.palette();
        let visits = Visits::from_lines(
            startup
                .histories
                .get(state::VISITS_KEY)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        );
        let screen_reader = !matches!(
            settings.screen_reader_mode,
            diroxide_config::ScreenReaderMode::Off
        );
        let mut app = Self {
            panels: [
                Panel::new(Side::Left, options.left, &settings),
                Panel::new(Side::Right, options.right, &settings),
            ],
            active: Side::Left,
            keymap,
            settings,
            types: options.types,
            clock: options.clock,
            ids: IdSource::new(),
            dialogs: Vec::new(),
            opens: Vec::new(),
            jobs: JobTable::new(),
            undo,
            status: String::new(),
            status_log: Vec::new(),
            quit: false,
            requests: Vec::new(),
            next_job: startup.first_job.max(1),
            flows: HashMap::new(),
            menus: startup.menus,
            banks: startup.banks,
            bank: 0,
            palette,
            histories: startup.histories,
            visits,
            clipboard: None,
            problems: startup.problems,
            interrupted: startup.interrupted,
            home: startup.home,
            screen_reader,
            quick: None,
            key_test: false,
            quit_folder: None,
            keep_shell: false,
            completing: None,
            tabbed: None,
            places: Vec::new(),
            first_run: startup.first_run,
            session: startup.session,
            restore: [None, None],
            kept: Vec::new(),
            archive_facts: [None, None],
            pending_archive: Vec::new(),
            archive_entered: [false, false],
            finish_lines: None,
            viewing: None,
            viewer_wait: None,
            highlighter: None,
            preview: preview::Preview::default(),
            keys: keys::KeysState::new(owner_keys),
            organize: organize::Organize::new(startup.labels, startup.bookmarks, startup.views),
            watching: watching::WatchState::default(),
            filtering: filtering::FilterState::default(),
            selecting: selecting::SelectState::default(),
            searching: searching::SearchState::default(),
            tools: search_tools::ToolsState::default(),
            comparing: comparing::CompareState::default(),
            plugins: plugin_ui::PluginState::default(),
            components: component_ui::ComponentState::default(),
        };
        app.apply_session();
        for side in Side::BOTH {
            app.set_panel_help(side);
        }
        app
    }

    /// The first effects: the startup lines (the interrupted jobs of the
    /// last session, the config problems, or the first-run welcome), then
    /// the listing of both panels.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut effects = self.startup_lines();
        self.sweep_at_start();
        self.plugins_start();
        for side in Side::BOTH {
            let place = self.panel(side).place().clone();
            let focus = self.restored_focus(side);
            effects.extend(self.open(side, place, focus));
        }
        effects
    }

    /// Which panel is active.
    pub fn active_side(&self) -> Side {
        self.active
    }

    /// A panel.
    pub fn panel(&self, side: Side) -> &Panel {
        &self.panels[side.index()]
    }

    fn panel_mut(&mut self, side: Side) -> &mut Panel {
        &mut self.panels[side.index()]
    }

    /// The active panel.
    pub fn active_panel(&self) -> &Panel {
        self.panel(self.active)
    }

    /// The keymap.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// The settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The running jobs.
    pub fn jobs(&self) -> &JobTable {
        &self.jobs
    }

    /// The undo log.
    pub fn undo_log(&self) -> &UndoLog {
        &self.undo
    }

    /// The dialog on top, if one is open.
    pub fn dialog(&self) -> Option<(DialogId, &Dialog)> {
        self.dialogs.last().map(|(id, dialog)| (*id, dialog))
    }

    /// Every open dialog, the top last.
    pub fn dialogs(&self) -> &[(DialogId, Dialog)] {
        &self.dialogs
    }

    /// Whether the app asked to quit.
    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    /// The folder the shell moves to when the app quits with
    /// `--cwd-file`: the active panel's last local folder. App-side hook
    /// for W1d-2's entry point, which writes it.
    pub fn quit_folder(&self) -> Option<&Place> {
        self.quit_folder.as_ref()
    }

    /// Whether the quit was "Quit, shell stays where it was": the
    /// `--cwd-file` is then written empty. App-side hook for W1d-2's entry
    /// point.
    pub fn keeps_shell_folder(&self) -> bool {
        self.quit && self.keep_shell
    }

    /// The file clipboard.
    pub fn clipboard(&self) -> Option<&Clipboard> {
        self.clipboard.as_ref()
    }

    /// The menus.
    pub fn menus(&self) -> &Menus {
        &self.menus
    }

    /// The key bar: the active bank's pairs on one line, key first, the
    /// most common operations first ("F2 Rename  F3 View  F4 Edit"). The
    /// terminal draws it on its last line in every mode (answer 49); see
    /// [`App::key_bar_within`] for a line of fixed width.
    pub fn key_bar(&self) -> String {
        if self.viewing.is_some() {
            return crate::menu::viewer_bank().key_bar(&self.keymap);
        }
        if self.in_preview() {
            return crate::menu::preview_bank().key_bar(&self.keymap);
        }
        if self.results_shown() {
            return crate::menu::results_bank().key_bar(&self.keymap);
        }
        if self.compare_shown() {
            return crate::menu::compare_bank().key_bar(&self.keymap);
        }
        self.active_bank()
            .map(|bank| bank.key_bar(&self.keymap))
            .unwrap_or_default()
    }

    /// The key bar cut to `cells`, whole pairs only, the first first; the
    /// rest are said by Say Key Bar. Empty when the `key_bar` setting is
    /// off.
    pub fn key_bar_within(&self, cells: usize) -> String {
        if !self.settings.key_bar {
            return String::new();
        }
        if self.viewing.is_some() {
            return crate::menu::viewer_bank().key_bar_within(&self.keymap, cells);
        }
        if self.in_preview() {
            return crate::menu::preview_bank().key_bar_within(&self.keymap, cells);
        }
        if self.results_shown() {
            return crate::menu::results_bank().key_bar_within(&self.keymap, cells);
        }
        if self.compare_shown() {
            return crate::menu::compare_bank().key_bar_within(&self.keymap, cells);
        }
        self.active_bank()
            .map(|bank| bank.key_bar_within(&self.keymap, cells))
            .unwrap_or_default()
    }

    /// The bank the key bar shows: the first, until Switch bank.
    pub fn active_bank(&self) -> Option<&Bank> {
        self.banks.get(self.bank).or_else(|| self.banks.first())
    }

    /// Whether the key test is on: the frontend sends every key, as it
    /// came, and the app says what it is.
    pub fn key_test_active(&self) -> bool {
        self.key_test || self.capturing_key()
    }

    /// Sets how many rows Page Up and Page Down move in each panel: the
    /// frontend gives the rows it shows.
    pub fn set_page_rows(&mut self, side: Side, rows: usize) {
        self.panel_mut(side).list_mut().set_page_size(rows);
    }

    /// Whether the app behaves for a screen reader: progress at its
    /// interval, the inactive panel never redrawn but on demand.
    pub fn screen_reader(&self) -> bool {
        self.screen_reader
    }

    /// Sets screen-reader behavior. App-side hook for W1d-2: the frontend
    /// decides it from the setting and its own detection.
    pub fn set_screen_reader(&mut self, on: bool) {
        self.screen_reader = on;
    }

    /// The config problems found at start and since.
    pub fn problems(&self) -> &[ConfigProblem] {
        &self.problems
    }

    /// Takes the requests the app queued for the runtime.
    /// [`Runtime::carry_out`](crate::Runtime::carry_out) calls it.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Lists a place into a panel, as Go to a path does. App-side hook for
    /// the server's `panel/set` and the GUI's address bar.
    pub fn open_place(&mut self, side: Side, place: Place) -> Vec<Effect> {
        self.open(side, place, None)
    }

    /// Puts a panel's focus on the row with this name (its real name);
    /// whether it was found. App-side hook for the server and the GUI.
    /// The quick view follows, as it does a key's move.
    pub fn focus_name(&mut self, side: Side, name: &str) -> bool {
        let found = self.panel_mut(side).list_mut().focus_id(name);
        self.follow_cursor();
        found
    }

    /// Marks or unmarks the rows with these names; returns how many
    /// changed. App-side hook for the server and the GUI.
    pub fn mark_names(&mut self, side: Side, names: &[String], mark: bool) -> usize {
        let list = self.panel_mut(side).list_mut();
        let before = list.selected_count();
        if mark {
            list.select_matching(|row| names.contains(&row.id));
            list.selected_count() - before
        } else {
            let keep: Vec<String> = list
                .selected_ids()
                .into_iter()
                .filter(|id| !names.iter().any(|n| n == id))
                .map(str::to_owned)
                .collect();
            list.select_none();
            list.select_matching(|row| keep.contains(&row.id));
            before - list.selected_count()
        }
    }

    /// The title line, the active place first and the panel before the
    /// parent (research/alpha1-plan.md part three): "Documents, left
    /// panel, in D:\work". A filter and a sort other than the configured
    /// one follow the panel: "Documents, left panel, filter *.pdf, in
    /// D:\work, sorted by size". Hidden characters in names are drawn as
    /// question marks (the display-name rule).
    pub fn title_line(&self) -> String {
        self.title_line_within(usize::MAX)
    }

    /// The title line cut to `cells` characters: a long parent is cut from
    /// its left with three dots ("in ...\finance\2026"), so the leaf and
    /// the panel always fit; the whole path is Go to a path's default
    /// text. The terminal draws this; the GUI uses the whole line.
    pub fn title_line_within(&self, cells: usize) -> String {
        if let Some(title) = self.viewer_title(cells) {
            return title;
        }
        if let Some(title) = self.preview_title(cells) {
            return title;
        }
        let panel = self.active_panel();
        let place = panel.loading_place().unwrap_or(panel.place());
        if place.kind() == diroxide_core::PlaceKind::Archive {
            let mut tail = String::new();
            if panel.is_loading() {
                tail.push_str(", loading");
            }
            if let Some(filter) = self.filter_title(self.active) {
                tail.push_str(&format!(", {filter}"));
            }
            let sort = panel.sort();
            if *sort != self.settings.sort_spec() {
                if sort.key == diroxide_core::SortKey::Unsorted {
                    tail.push_str(", not sorted");
                } else {
                    tail.push_str(&format!(", sorted by {}", sort.key.word()));
                }
            }
            let side_words = format!("{} panel", self.active.word());
            return self.archive_title_line(place, &side_words, &tail, cells);
        }
        let mut head = format!(
            "{}, {} panel",
            crate::panel::place_leaf(place),
            self.active.word()
        );
        if panel.is_loading() {
            head.push_str(", loading");
        }
        if let Some(filter) = self.filter_title(self.active) {
            head.push_str(&format!(", {filter}"));
        }
        let sort = panel.sort();
        let tail = if *sort == self.settings.sort_spec() {
            String::new()
        } else if sort.key == diroxide_core::SortKey::Unsorted {
            ", not sorted".to_owned()
        } else {
            format!(", sorted by {}", sort.key.word())
        };
        let parent = place
            .parent()
            .map(|p| diroxide_core::place::strip_extended_prefix(&p.to_string()).into_owned())
            .map(|p| diroxide_core::names::display_name(&p).into_owned());
        let Some(parent) = parent else {
            return format!("{head}{tail}");
        };
        let whole = format!("{head}, in {parent}{tail}");
        let used = whole.chars().count();
        if used <= cells {
            return whole;
        }
        // Cut the parent from its left; keep at least its last part.
        let room =
            cells.saturating_sub(head.chars().count() + ", in ...".len() + tail.chars().count());
        let kept: String = {
            let chars: Vec<char> = parent.chars().collect();
            let start = chars.len().saturating_sub(room.max(1));
            chars[start..].iter().collect()
        };
        format!("{head}, in ...{kept}{tail}")
    }

    /// The status line, the key fact first: the last message, or the
    /// active panel's folder and counts ("Documents, 40 items, 2
    /// selected"). It never follows the focus: the cursor's row says
    /// where the focus is, so a move changes nothing here and a screen
    /// reader that speaks changed text says the row once (R4 section
    /// 5.1). A focus move keeps the last message.
    pub fn status_line(&self) -> String {
        if self.status.is_empty()
            && let Some(viewer) = self.viewer()
        {
            return viewer.status();
        }
        if self.status.is_empty() {
            let panel = self.active_panel();
            let list = panel.list();
            let mut line = format!(
                "{}, {}",
                crate::panel::place_leaf(panel.place()),
                diroxide_core::format::count(list.len() as u64, "item", "items")
            );
            let selected = list.selected_count();
            if selected > 0 {
                line.push_str(&format!(", {selected} selected"));
            }
            line
        } else {
            self.status.clone()
        }
    }

    /// The session as it stands, for the state folder.
    pub fn session(&self) -> diroxide_config::SessionState {
        let mut session = diroxide_config::SessionState {
            layout: self.settings.layout,
            active: self.active,
            ..diroxide_config::SessionState::default()
        };
        for side in Side::BOTH {
            let panel = self.panel(side);
            let state = session.panel_mut(side);
            state.place = Some(panel.place().clone());
            state.focused = panel.list().focused().map(|r| r.id.clone());
            // While a folder's own view is applied, the panel's usual
            // view is what it has elsewhere, so that is what is kept (W3l).
            match &self.organize.usual[side.index()] {
                Some(usual) => {
                    state.sort = usual.sort.clone();
                    state.show_hidden = usual.show_hidden;
                }
                None => {
                    state.sort = panel.sort().clone();
                    state.show_hidden = panel.shows_hidden();
                }
            }
            state.history = panel.history().to_vec();
        }
        session
    }

    /// Handles an event and returns what to do about it. When the app
    /// queued requests for its runtime, the effects end with
    /// [`Effect::AppRequests`], where [`Runtime::carry_out`](crate::Runtime::carry_out)
    /// takes them.
    pub fn handle(&mut self, event: AppEvent) -> Vec<Effect> {
        let mut effects = self.handle_event(event);
        effects.extend(self.plugin_queue_next());
        self.follow_cursor();
        self.mark_requests(effects)
    }

    /// Adds [`Effect::AppRequests`] at the end when requests wait and the
    /// effects do not say so yet.
    fn mark_requests(&self, mut effects: Vec<Effect>) -> Vec<Effect> {
        if !self.requests.is_empty() && !effects.contains(&Effect::AppRequests) {
            effects.push(Effect::AppRequests);
        }
        effects
    }

    fn handle_event(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::Key(key) => self.press(key),
            AppEvent::Command(command) => self.dispatch(&command),
            AppEvent::Listed {
                request,
                place,
                result,
            } => self.listed(request, &place, result),
            AppEvent::ListProgress { request, so_far } => self.list_progress(request, so_far),
            AppEvent::PlaceFacts {
                place,
                capabilities,
            } => {
                for panel in &mut self.panels {
                    if panel.loading_place() == Some(&place) || panel.place() == &place {
                        panel.set_capabilities(capabilities);
                    }
                }
                Vec::new()
            }
            AppEvent::Planned { job, result } => self.planned(job, result),
            AppEvent::JobProgress { job, progress } => self.job_progress(job, &progress),
            AppEvent::JobFinished(report) => self.job_finished(*report),
            AppEvent::Asks { ask, question } => self.asks(ask, question),
            AppEvent::Watched { side, place, event } => {
                if self.panel(side).place() == &place {
                    self.quick_view_watch(side, &event);
                }
                self.watched(side, &place, &event)
            }
            AppEvent::NotWatched { side, place } => self.not_watched(side, &place),
            AppEvent::Places(places) => self.places_arrived(places),
            AppEvent::ProgramEnded {
                what,
                folder,
                result,
            } => self.program_ended(&what, &folder, result),
            AppEvent::ArchiveOpened { request, facts } => self.archive_opened(request, *facts),
            AppEvent::ArchivePlanned { job, result } => self.archive_planned(job, result),
            AppEvent::ArchiveEntries { place, line } => self.archive_entries_counted(&place, line),
            AppEvent::ViewerSource {
                request,
                place,
                result,
            } => self.viewer_source(request, place, result),
            AppEvent::ArchiveFinished { report, lines } => self.archive_finished(*report, lines),
            AppEvent::PlacesChecked {
                request,
                missing,
                unreadable,
            } => self.places_checked(request, missing, unreadable),
            AppEvent::Arrivals { watch, note } => self.arrivals(watch, note),
            AppEvent::FilterContents { request, result } => {
                self.filter_contents_arrived(request, result)
            }
            AppEvent::SearchFound { search, hits } => self.search_found(search, hits),
            AppEvent::SearchProgress { search, progress } => {
                self.search_progress(search, &progress)
            }
            AppEvent::SearchDone { search, result } => self.search_done(search, result),
            AppEvent::SavedListed { request, found } => self.saved_listed(request, found),
            AppEvent::Compared { request, result } => self.compared(request, result),
            AppEvent::CompareProgress {
                request,
                done,
                total,
            } => self.compare_progress(request, done, total),
            AppEvent::SpaceListed { request, result } => self.space_listed(request, result),
            AppEvent::LargestFound { search, result } => self.largest_found(search, result),
            AppEvent::DirSized { request, sizes } => self.folders_sized(request, sizes),
            AppEvent::NewestFound { search, result } => self.newest_found(search, result),
            AppEvent::LabeledFound {
                search,
                found,
                missing,
                ..
            } if self.plugin_owns_search(search) => {
                self.plugin_places_found(search, found, missing)
            }
            AppEvent::LabeledFound {
                search,
                found,
                missing,
                unreadable,
            } => self.labeled_found(search, found, missing, unreadable),
            AppEvent::StaleChecked {
                search,
                row,
                freshness,
            } => self.stale_checked(search, row, freshness),
            AppEvent::Answer { dialog, answer } => self.answer(dialog, answer),
            AppEvent::Problem(message) => vec![self.say(message)],
            AppEvent::Plugin(event) => self.plugin_event(*event),
            AppEvent::Component(event) => self.component_event(*event),
        }
    }

    /// Runs a command and returns what to do about it, with
    /// [`Effect::AppRequests`] at the end when it queued requests.
    pub fn dispatch(&mut self, command: &Command) -> Vec<Effect> {
        let mut effects = self.dispatch_command(command);
        effects.extend(self.plugin_queue_next());
        self.follow_cursor();
        self.mark_requests(effects)
    }

    fn dispatch_command(&mut self, command: &Command) -> Vec<Effect> {
        if command.id != ids::SAY_STATUS
            && command.id != ids::LIST_UP
            && command.id != ids::LIST_DOWN
        {
            self.quick = None;
        }
        let action = route(&command.id);
        if self.dialogs.is_empty()
            && let Some(effects) = self.viewer_dispatch(action, &command.id)
        {
            return effects;
        }
        if self.dialogs.is_empty()
            && let Some(effects) = self.preview_dispatch(action, &command.id)
        {
            return effects;
        }
        // The labels, bookmarks, folder views and History list (W3l).
        if let Some(effects) = self.organize_dispatch(command) {
            return effects;
        }
        // Watch this folder, and Cancel while it watches (W3t).
        if let Some(effects) = self.watch_dispatch(command, action) {
            return effects;
        }
        // Say filter, stash and restore, the kind toggles (W3a).
        if let Some(effects) = self.filtering_dispatch(command) {
            return effects;
        }
        // Select by kind, size, date, extension, range and rows (W3a).
        if let Some(effects) = self.selecting_dispatch(command) {
            return effects;
        }
        // Find files and the results' commands (W3a).
        if let Some(effects) = self.search_dispatch(command) {
            return effects;
        }
        // Saved searches, the Space list, sizes and newest files (W3a).
        if let Some(effects) = self.tools_dispatch(command) {
            return effects;
        }
        // Compare files, its properties and the differences (W3a).
        if let Some(effects) = self.compare_dispatch(command) {
            return effects;
        }
        // Manage plugins and the plugins' own commands (W4d).
        if let Some(effects) = self.plugins_dispatch(command) {
            return effects;
        }
        // The optional components and Get example plugins (W4d).
        if let Some(effects) = self.components_dispatch(command) {
            return effects;
        }
        if preview::preview_route(&command.id).is_some() {
            return vec![self.say(Message::status("Close this dialog first."))];
        }
        if viewing::viewer_route(&command.id).is_some() {
            if !self.dialogs.is_empty() {
                return vec![self.say(Message::status("Close this dialog first."))];
            }
            return self.viewer_closed_command();
        }
        if keys::is_keys_command(&command.id) {
            return self.keys_dispatch(command);
        }
        let Some(action) = action else {
            let message =
                Message::could_not("run", command.id.as_str(), "no command has that id", "");
            return vec![self.say(message)];
        };
        if !self.dialogs.is_empty() && !matches!(action, Action::SayStatus | Action::Help) {
            return self.dialog_command(action, command);
        }
        let side = self.active;
        // A change inside an archive is refused in words (W2a, P2b): an
        // archive is read only here, and Pack makes a new one.
        let changes_here = matches!(
            action,
            Action::MakeFolder
                | Action::Trash
                | Action::DeletePermanently
                | Action::Rename
                | Action::Duplicate
                | Action::Paste
                | Action::Move
                | Action::MoveTo
                | Action::ClipCut
        );
        if changes_here && let Some(refused) = self.read_only_refusal(side) {
            return refused;
        }
        match action {
            Action::Focus(movement) => self.move_focus(movement),
            // With the quick view on, Tab goes into its preview region.
            Action::SwitchPanel if self.quick_view_on() => self.enter_preview(),
            Action::SwitchPanel => self.switch_panel(),
            Action::SwapPanels => self.swap_panels(),
            Action::ShowInOther => self.show_in_other(),
            Action::ThisInOther => self.this_in_other(),
            Action::Enter => self.enter(),
            Action::Into => self.into(),
            Action::OpenAsArchive => self.open_as_archive(),
            Action::MakeLink => self.link_dialog(),
            Action::Relink => self.relink_prompt(),
            Action::Permissions => self.permissions_dialog(),
            Action::CopyPath => self.copy_path_or_name(false),
            Action::CopyName => self.copy_path_or_name(true),
            Action::Split => self.split_dialog(),
            Action::Join => self.join_files(),
            Action::Pack => self.pack_dialog(),
            Action::Unpack => self.unpack_dialog(),
            Action::TestArchive => self.test_archive(),
            Action::Up => self.up(),
            Action::Reload => self.reload(side),
            Action::Back => self.back(),
            Action::Forward => self.forward(),
            Action::Home => self.home(),
            Action::GoPath => self.go_path(command),
            Action::Places => self.go_places(),
            Action::History => self.folder_history(),
            Action::ToggleSelect => self.toggle_select(),
            Action::SelectAll => self.select_all(),
            Action::SelectNone => self.select_none(),
            Action::SelectInvert => self.select_invert(),
            Action::SelectPattern => self.pattern_prompt(true, command),
            Action::UnselectPattern => self.pattern_prompt(false, command),
            Action::SelectNext => self.next_selected(true),
            Action::SelectPrevious => self.next_selected(false),
            Action::SelectRestore => self.restore_selection(),
            Action::View => self.view_file(),
            Action::Edit => self.edit_file(),
            Action::Copy => self.copy_or_move(diroxide_core::JobKind::Copy),
            Action::Move => self.copy_or_move(diroxide_core::JobKind::Move),
            Action::CopyTo => self.pick_destination(diroxide_core::JobKind::Copy),
            Action::MoveTo => self.pick_destination(diroxide_core::JobKind::Move),
            Action::MakeFolder => self.make_folder(command),
            Action::Trash => self.trash(),
            Action::DeletePermanently => self.delete_permanently(),
            Action::Rename => self.rename(command),
            Action::Duplicate => self.duplicate(),
            Action::Details => self.details(),
            Action::ContextMenu => self.context_menu(),
            Action::RunCommand => self.run_command_prompt(command),
            Action::ShellHere => self.shell_here(),
            Action::ClipCopy => self.clip(false),
            Action::ClipCut => self.clip(true),
            Action::Paste => self.paste(),
            Action::ToggleHidden => self.toggle_hidden(),
            Action::Filter => self.filter_prompt(command),
            Action::FilterClear => self.filter_clear(),
            Action::Sort => self.sort_by(&command.id),
            Action::SortList => self.sort_list(),
            Action::Plugins => self.open_manage_plugins(),
            Action::Quit | Action::QuitToFolder => self.ask_quit(false),
            Action::QuitKeepFolder => self.ask_quit(true),
            Action::Help => self.help(),
            Action::SayStatus => self.say_status(),
            Action::SayOther => self.say_other(),
            Action::SayKeyBar => self.say_key_bar(),
            Action::SwitchBank => self.switch_bank(),
            Action::Palette => self.open_palette(),
            Action::MenuBar => self.open_menu_bar(),
            Action::Menu => self.open_menu_by_command(&command.id),
            Action::Settings => self.open_settings(),
            Action::KeyTest => self.start_key_test(),
            Action::Cancel => self.cancel_front_job(),
            Action::Layout => self.cycle_layout(),
            Action::NextTheme => self.next_theme(),
            Action::Problems => self.list_problems(),
            Action::Undo => self.undo(),
            Action::JobHistory => self.job_history(),
            Action::Yes | Action::No | Action::Accept => Vec::new(),
            Action::QuickView => self.toggle_quick_view(),
            Action::OpenInTextweaver => self.open_in_textweaver(),
        }
    }

    /// Says a message: it becomes the status line and is announced, and
    /// the status lines said since the frontend last took them keep it.
    fn say(&mut self, message: Message) -> Effect {
        self.status = message.text.clone();
        if self.status_log.len() >= STATUS_LOG_LIMIT {
            self.status_log.remove(0);
        }
        self.status_log.push(message.text.clone());
        Effect::Announce(message)
    }

    /// Takes the status lines said since the last call, oldest first. A
    /// terminal draws one frame for several messages (a refusal and the
    /// prompt that opens again, a job's result and the listing after it),
    /// so it shows them together, the first first, instead of only the
    /// last. App-side hook for the terminal's status line.
    pub fn take_status_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.status_log)
    }

    /// The written key of a command from the live keymap ("Ctrl+R"), or
    /// an empty string.
    fn key(&self, id: &CommandId) -> String {
        self.keymap.key_text(id)
    }

    fn now(&self) -> Timestamp {
        self.clock.now()
    }

    fn request(&mut self, request: Request) {
        self.requests.push(request);
    }

    fn next_job_id(&mut self) -> JobId {
        let job = JobId::new(self.next_job);
        self.next_job += 1;
        job
    }

    fn remember(&mut self, key: &str, entry: &str) {
        let history = self.histories.entry(key.to_owned()).or_default();
        state::remember(history, entry);
        let entries = history.clone();
        self.request(Request::SaveHistory {
            key: key.to_owned(),
            entries,
        });
    }

    fn history_of(&self, key: &str) -> Vec<String> {
        self.histories.get(key).cloned().unwrap_or_default()
    }

    fn press(&mut self, key: Key) -> Vec<Effect> {
        if self.capturing_key() {
            return self.capture_press(key);
        }
        if self.key_test {
            return self.key_test_press(key);
        }
        if !self.dialogs.is_empty() {
            return self.dialog_key(key);
        }
        if self.in_preview() {
            return self.preview_press(key);
        }
        if self.viewing.is_some() {
            let layers = [
                diroxide_core::Layer::Viewer,
                diroxide_core::Layer::List,
                diroxide_core::Layer::Global,
            ];
            return match self.keymap.lookup(&layers, &key) {
                Lookup::Command(id) => {
                    self.status.clear();
                    self.dispatch(&Command::new(id))
                }
                // A panel's key says the viewer is in the way.
                Lookup::NoMatch => match self.keymap.lookup(&[diroxide_core::Layer::Panel], &key) {
                    Lookup::Command(id) => self.dispatch(&Command::new(id)),
                    Lookup::NoMatch => Vec::new(),
                },
            };
        }
        let layers = [
            diroxide_core::Layer::Panel,
            diroxide_core::Layer::List,
            diroxide_core::Layer::Global,
        ];
        match self.keymap.lookup(&layers, &key) {
            Lookup::Command(id) => {
                // A focus move keeps the last message, so the status line
                // does not change under a screen reader on every move.
                let moves = [
                    ids::LIST_UP,
                    ids::LIST_DOWN,
                    ids::LIST_PAGE_UP,
                    ids::LIST_PAGE_DOWN,
                    ids::LIST_HOME,
                    ids::LIST_END,
                ];
                if !moves.contains(&id) {
                    self.status.clear();
                }
                self.dispatch(&Command::new(id))
            }
            Lookup::NoMatch => match typed(&key) {
                Some(letter) if self.settings.single_letters.types_ahead() => {
                    self.type_ahead(letter)
                }
                _ => Vec::new(),
            },
        }
    }

    fn redraw_panel(&self, side: Side) -> Effect {
        Effect::Redraw(Redraw::Panel(side))
    }
}

/// The character a key types: a printable character with at most Shift,
/// and on Windows also Control and Alt together (AltGr arrives so, R19's
/// note on crossterm issue 820).
fn typed(key: &Key) -> Option<char> {
    if let Some(c) = key.typed_char() {
        return Some(c);
    }
    let altgr = cfg!(windows) && key.mods.ctrl && key.mods.alt && !key.mods.meta;
    match key.code {
        diroxide_core::KeyCode::Char(c) if altgr && !c.is_control() => Some(c),
        _ => None,
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests;
