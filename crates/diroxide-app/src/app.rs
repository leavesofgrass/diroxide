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

mod dialogs;
mod files;
mod info;
mod nav;
mod select;

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
    /// A dialog was answered.
    Answer {
        /// The dialog.
        dialog: DialogId,
        /// The answer.
        answer: Answer,
    },
    /// Something outside the app failed, already in words.
    Problem(Message),
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
    /// The name filter, live; the filter before it opened.
    Filter(Side, Option<diroxide_core::Pattern>),
    /// Select or unselect by pattern.
    Pattern { side: Side, select: bool },
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
    /// The settings list.
    Settings,
    /// A setting's choices.
    SettingValue(String),
    /// A setting typed in a field.
    SettingText(String),
    /// A list to read: details, help, problems.
    Read,
    /// The job history.
    JobHistory,
    /// One job's items.
    JobItems,
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
    /// Say what an undo brought back.
    Undid(Option<diroxide_core::JobKind>),
    /// The permanent delete the trash could not take: its plan is shown
    /// under the "no trash" question ("This drive has no trash. Delete 3
    /// items for good?").
    NoTrash(Message),
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
}

const ROUTES: [(CommandId, Action); 91] = [
    (ids::QUIT, Action::Quit),
    (ids::QUIT_TO_FOLDER, Action::QuitToFolder),
    (ids::QUIT_KEEP_FOLDER, Action::QuitKeepFolder),
    (ids::HELP, Action::Help),
    (ids::SAY_STATUS, Action::SayStatus),
    (ids::SAY_OTHER, Action::SayOther),
    (ids::SAY_KEY_BAR, Action::SayKeyBar),
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
        if let Some(layout) = startup.layout {
            settings.layout = layout;
        }
        let keymap = options.keymap.with_single_letters(settings.single_letters);
        let mut undo = UndoLog::default();
        for record in startup.undo.iter().cloned() {
            undo.record(record);
        }
        let palette = Palette::with_recent(startup.palette_recent());
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

    /// The key bar's bank: the first bank.
    pub fn key_bar(&self) -> String {
        self.banks
            .first()
            .map(|bank| bank.key_bar(&self.keymap))
            .unwrap_or_default()
    }

    /// Whether the key test is on: the frontend sends every key, as it
    /// came, and the app says what it is.
    pub fn key_test_active(&self) -> bool {
        self.key_test
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
    /// the server's `panel/set` and the window's address bar.
    pub fn open_place(&mut self, side: Side, place: Place) -> Vec<Effect> {
        self.open(side, place, None)
    }

    /// Puts a panel's focus on the row with this name (its real name);
    /// whether it was found. App-side hook for the server and the window.
    pub fn focus_name(&mut self, side: Side, name: &str) -> bool {
        self.panel_mut(side).list_mut().focus_id(name)
    }

    /// Marks or unmarks the rows with these names; returns how many
    /// changed. App-side hook for the server and the window.
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
    /// text. The terminal draws this; the window uses the whole line.
    pub fn title_line_within(&self, cells: usize) -> String {
        let panel = self.active_panel();
        let place = panel.loading_place().unwrap_or(panel.place());
        let mut head = format!(
            "{}, {} panel",
            crate::panel::place_leaf(place),
            self.active.word()
        );
        if panel.is_loading() {
            head.push_str(", loading");
        }
        if let Some(filter) = panel.filter() {
            head.push_str(&format!(
                ", filter {}",
                diroxide_core::names::display_name(filter.text())
            ));
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
            state.sort = panel.sort().clone();
            state.show_hidden = panel.shows_hidden();
            state.history = panel.history().to_vec();
        }
        session
    }

    /// Handles an event and returns what to do about it. When the app
    /// queued requests for its runtime, the effects end with
    /// [`Effect::AppRequests`], where [`Runtime::carry_out`](crate::Runtime::carry_out)
    /// takes them.
    pub fn handle(&mut self, event: AppEvent) -> Vec<Effect> {
        let effects = self.handle_event(event);
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
            AppEvent::Watched { side, place, event } => self.watched(side, &place, &event),
            AppEvent::NotWatched { side, place } => self.not_watched(side, &place),
            AppEvent::Places(places) => self.places_arrived(places),
            AppEvent::ProgramEnded {
                what,
                folder,
                result,
            } => self.program_ended(&what, &folder, result),
            AppEvent::Answer { dialog, answer } => self.answer(dialog, answer),
            AppEvent::Problem(message) => vec![self.say(message)],
        }
    }

    /// Runs a command and returns what to do about it, with
    /// [`Effect::AppRequests`] at the end when it queued requests.
    pub fn dispatch(&mut self, command: &Command) -> Vec<Effect> {
        let effects = self.dispatch_command(command);
        self.mark_requests(effects)
    }

    fn dispatch_command(&mut self, command: &Command) -> Vec<Effect> {
        if command.id != ids::SAY_STATUS
            && command.id != ids::LIST_UP
            && command.id != ids::LIST_DOWN
        {
            self.quick = None;
        }
        let Some(action) = route(&command.id) else {
            let message =
                Message::could_not("run", command.id.as_str(), "no command has that id", "");
            return vec![self.say(message)];
        };
        if !self.dialogs.is_empty() && !matches!(action, Action::SayStatus | Action::Help) {
            return self.dialog_command(action, command);
        }
        let side = self.active;
        match action {
            Action::Focus(movement) => self.move_focus(movement),
            Action::SwitchPanel => self.switch_panel(),
            Action::SwapPanels => self.swap_panels(),
            Action::ShowInOther => self.show_in_other(),
            Action::ThisInOther => self.this_in_other(),
            Action::Enter => self.enter(),
            Action::Into => self.into(),
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
            Action::View => vec![self.say(Message::info("The viewer comes with alpha.2."))],
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
            Action::Plugins => vec![self.say(Message::info("Plugins come with alpha.4."))],
            Action::Quit | Action::QuitToFolder => self.ask_quit(false),
            Action::QuitKeepFolder => self.ask_quit(true),
            Action::Help => self.help(),
            Action::SayStatus => self.say_status(),
            Action::SayOther => self.say_other(),
            Action::SayKeyBar => self.say_key_bar(),
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
        if self.key_test {
            return self.key_test_press(key);
        }
        if !self.dialogs.is_empty() {
            return self.dialog_key(key);
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
