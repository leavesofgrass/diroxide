//! The GUI frontend: Masonry, `masonry_winit`, Vello, Parley and
//! AccessKit, vendored from textweaver at the same revision and patches
//! (`docs/dev/vendoring.md`). Each panel is one focusable AccessKit list box
//! whose options are the rows: an option's name is the row as the list
//! model speaks it (name first, details after), its position and the
//! list's real size are set on it, marked rows are selected, and the
//! focused row is the active descendant. A grid with named columns
//! replaces this only if AccessKit's adapters gain the Grid and Table
//! patterns, or the owner prefers it after a listening session (ADR-0007,
//! amended Saturday, October 10, 2026). Menus are native; announcements go
//! through a live region; dialogs are modal, named, close on Escape and
//! return the focus.
//!
//! The GUI alpha's contract step built the frame: [`run`] takes the app and
//! its runtime as `diro` builds them, opens one window titled with the
//! active place, wakes its event loop when a worker has a result, and
//! quits by the app's quit rule when the window is closed. Every way it can
//! fail to start is a [`GuiError`] said in words with a next step, never a
//! panic, so `diro` can say one sentence and open the terminal instead.
//! The window itself is behind the `gui` feature; without it, [`run`]
//! says the GUI was left out of this build. [`panel_grid`] stays as the
//! data both a list box and a grid are built from.
//!
//! The foundation lane (G1) put the app in the window:
//!
//! - `keys`: the window's key events as the keymap's keys (the GUI
//!   column), with AltGr, the keypad, repeats and macOS's Option.
//! - `frontend`: the loop one step at a time over any host of widgets
//!   (the real window, or the tests' harness): keys to the app, effects
//!   to the runtime, the app's timers, and a refresh that edits only what
//!   changed and keeps the keyboard focus where the app has it.
//! - [`model`]: what the window shows, as plain data: each panel's list
//!   box with its window of rows, the three lines, the open dialog, the
//!   key bar, and one announcement for each step's lines.
//! - `widgets`: the panels' list boxes, the live region, the modal host
//!   with its app-driven text field and lists, the key bar and the sort
//!   buttons, the root that takes every key.
//! - [`menus`]: the app's menu model as a native Windows menu bar, hidden
//!   until a menu key shows it; elsewhere the app's list menus.
//!
//! The dialogs lane (G2) put the app's dialogs and the rest of its screens
//! in the window:
//!
//! - every dialog shape in the modal host: a confirmation as two real
//!   buttons, No focused, with its listing and count; a question named by
//!   its first line with the rest as its description; lists with letters,
//!   help and clicks; a text to read line by line; a secret with its Show
//!   toggle; fields with completion as combo boxes, an input method
//!   session for every field but a password's, and paste. The window
//!   moves its own focus where the app models none
//!   ([`model::LocalFocus`]), and a dialog is said once: by itself as the
//!   focus moves into it, not again by the live region.
//! - `widgets`: buttons and toggles that take the focus (`push`), the job
//!   in front's progress (`progress`), the viewer's list of the rows on
//!   screen (`viewer`) in the panels' place (`screen`), and the quick
//!   view's region in the other panel.
//! - copied text on the system clipboard, and [`spawn`]: the shell, a
//!   command line and the editor started in windows of their own, never
//!   waited for (decision 8).
//! - winit's helper window named, so a screen reader never meets a
//!   nameless pane.
//!
//! The themes lane (G3) gave the window its colors and its other roads:
//!
//! - [`theme`]: the twenty-four themes mapped to what the window draws,
//!   one look for every screen role, the owner's `[colors]` in place; a
//!   role below the floor drawn in the text color, so every theme passes
//!   the theme crate's checks on every pair the window draws; Windows
//!   High Contrast honored with the system's own colors; the system's
//!   light or dark appearance followed through the theme crate's `os`
//!   module. A theme chosen is said with its verdict in words.
//! - A plugin's prompts and the connect flow in the dialog shapes, the
//!   plugin's name last; a question's message of two lines or more is a
//!   list to read line by line ("Message, 11 lines"), so a new server
//!   key's fingerprint is read one part of four groups a line.
//! - Software drawing (the owner's answer 106): the vendored runner asks
//!   for an adapter before it makes the surface, and with none, or only a
//!   software one, draws on the processor (Vello's CPU renderer through
//!   softbuffer, no graphics driver), the hybrid renderer on a software
//!   adapter its fallback, the terminal with one plain sentence last;
//!   `--graphics software` asks for the processor; the road goes to the
//!   `--log` file and, when it is not the graphics card, once to the
//!   status line (`window`).
//! - Focus mode and full screen: with the `focus_mode` setting on, or
//!   full screen (the app's `view.full-screen`, F11), the bars around the
//!   body fade under a veil after three seconds with no key and no mouse
//!   move, and come back on any key; painting only, so the tree is the
//!   same and the fade says nothing. A `--background` window never goes
//!   full screen.
//!
//! Alpha 4's window lane (G5b) added [`picture`]: Enter on a picture in
//! the viewer or the quick view's preview opens it in the body's place as
//! one graphic named by its file and described by the preview crate's
//! words, read through that crate's one pixel guard; Plus, Minus, 0 and
//! 1 zoom and fit, said in words; an animation shows its first frame
//! still; Escape goes back.
//!
//! On Windows `diro` stays a console program, so a shell waits for it and
//! the terminal frontend keeps its console; [`console`] lets go of a
//! console that only the window's start opened (a double-click in
//! Explorer), and gives one back if the window cannot start.

pub mod console;
#[cfg(feature = "gui")]
pub mod fonts;
#[cfg(feature = "gui")]
pub mod frontend;
#[cfg(feature = "gui")]
pub mod keys;
pub mod menus;
pub mod model;
pub mod picture;
mod platform;
pub mod spawn;
pub mod theme;
#[cfg(feature = "gui")]
pub mod widgets;
#[cfg(feature = "gui")]
pub mod window;

use diroxide_app::{App, Panel, Runtime};
use diroxide_core::Explain;
use thiserror::Error;

/// The graphics API the window draws with: `--graphics NAME` or the
/// `graphics` setting, which `diro` reads and maps to this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GraphicsApi {
    /// The graphics driver's own choice (every backend wgpu finds).
    #[default]
    Auto,
    /// Vulkan.
    Vulkan,
    /// Direct3D 12, on Windows.
    Dx12,
    /// Metal, on macOS.
    Metal,
    /// OpenGL.
    Gl,
    /// Drawn on the processor, with no graphics card: Vello's CPU renderer
    /// presented through softbuffer, which needs no graphics driver, the
    /// hybrid renderer on a software adapter its fallback (the owner's
    /// answer 106). `auto` takes this road by itself where there is no
    /// graphics adapter or only one that draws on the processor.
    Software,
}

impl GraphicsApi {
    /// The names `--graphics` and the setting take, in order.
    pub const NAMES: [&'static str; 6] = ["auto", "vulkan", "dx12", "metal", "gl", "software"];

    /// The names as said: "auto, vulkan, dx12, metal, gl or software".
    pub fn names_in_words() -> String {
        let (last, rest) = Self::NAMES.split_last().unwrap_or((&"auto", &[]));
        format!("{} or {last}", rest.join(", "))
    }

    /// The API named `text`, case ignored; `None` for any other word.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "vulkan" => Some(Self::Vulkan),
            "dx12" | "d3d12" => Some(Self::Dx12),
            "metal" => Some(Self::Metal),
            "gl" | "opengl" => Some(Self::Gl),
            "software" => Some(Self::Software),
            _ => None,
        }
    }

    /// The value wgpu reads from `WGPU_BACKEND`, or `None` for the driver's
    /// own choice (and for software drawing, which needs no backend).
    pub const fn wgpu_backend(self) -> Option<&'static str> {
        match self {
            Self::Auto | Self::Software => None,
            Self::Vulkan => Some("vulkan"),
            Self::Dx12 => Some("dx12"),
            Self::Metal => Some("metal"),
            Self::Gl => Some("gl"),
        }
    }
}

/// Options of the GUI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiOptions {
    /// The program's name, last in the window's title: "Documents -
    /// diroxide".
    pub title: String,
    /// `--background`: the window opens off screen, is never activated and
    /// has no taskbar button, so an automated check cannot take the focus
    /// from the person at the machine.
    pub background: bool,
    /// The graphics API to draw with.
    pub graphics: GraphicsApi,
    /// `--log FILE`: the window's diagnostic lines (the graphics adapter
    /// and the drawing road taken) appended to this file.
    pub log: Option<std::path::PathBuf>,
}

impl Default for GuiOptions {
    fn default() -> Self {
        Self {
            title: "diroxide".to_owned(),
            background: false,
            graphics: GraphicsApi::Auto,
            log: None,
        }
    }
}

/// Why the GUI could not run, in words a screen reader reads well; the
/// next step is [`Explain::next_step`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GuiError {
    /// This `diro` was built without its `gui` feature.
    #[error("this diro was built without the GUI")]
    NotBuilt,
    /// No display to open a window on: no Wayland or X11 display, or the
    /// system refused the window.
    #[error("there is no display to open a window on ({0})")]
    NoDisplay(String),
    /// The window could not start its graphics on any road: the graphics
    /// card, the processor, the hybrid renderer on a software adapter.
    #[error("the window could not start its graphics ({0})")]
    Graphics(String),
    /// The window's event loop failed after it started.
    #[error("the window's event loop failed ({0})")]
    EventLoop(String),
    /// The window stopped after an internal error (a panic), caught and
    /// said instead of ending the program.
    #[error("the window stopped after an internal error ({0})")]
    Stopped(String),
}

impl Explain for GuiError {
    fn next_step(&self) -> String {
        match self {
            GuiError::NotBuilt => {
                "Use the terminal with diro --tui, or build diro with its gui feature.".to_owned()
            }
            GuiError::NoDisplay(_) => {
                "Use the terminal with diro --tui, or start diro from the desktop.".to_owned()
            }
            GuiError::Graphics(_) => {
                "Try another graphics API with diro --graphics, such as software, dx12, vulkan or gl. The terminal, diro --tui, needs none."
                    .to_owned()
            }
            GuiError::EventLoop(_) | GuiError::Stopped(_) => {
                "Start diro again; the terminal, diro --tui, does not use the window.".to_owned()
            }
        }
    }
}

/// Whether this build has the window (the `gui` feature).
pub const fn built() -> bool {
    cfg!(feature = "gui")
}

/// Sets what the window's graphics read from the environment before it
/// starts: `WGPU_BACKEND` from [`GuiOptions::graphics`], unless it is
/// already set (a set value wins, as wgpu's own documentation says).
/// Returns the backend set, if any.
///
/// Call it before any other thread starts (before the app and its runtime
/// are built), because it changes the process's environment; `diro` does,
/// right after it chooses the GUI.
pub fn prepare(options: &GuiOptions) -> Option<&'static str> {
    let name = options.graphics.wgpu_backend()?;
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return None;
    }
    platform::set_env_before_threads("WGPU_BACKEND", name);
    Some(name)
}

/// A row of a panel's grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRow {
    /// The cells: the name first, then the details.
    pub cells: Vec<String>,
    /// Whether the row is selected.
    pub selected: bool,
}

/// A panel as the accessibility tree will carry it: a named grid with
/// named columns, rows of cells, a selection and a focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridModel {
    /// The grid's name: the panel's title.
    pub name: String,
    /// The column headers.
    pub columns: Vec<String>,
    /// The rows.
    pub rows: Vec<GridRow>,
    /// The focused row.
    pub focus: Option<usize>,
}

/// The grid for a panel. Phase 0 names the columns "Name" and "Details";
/// W4a names one column per configured detail.
pub fn panel_grid(panel: &Panel) -> GridModel {
    let list = panel.list();
    GridModel {
        name: list.title().to_owned(),
        columns: vec!["Name".to_owned(), "Details".to_owned()],
        rows: list
            .rows()
            .iter()
            .map(|row| GridRow {
                cells: vec![row.label.clone(), row.details.join(", ")],
                selected: row.selected,
            })
            .collect(),
        focus: list.focus_index(),
    }
}

/// The window's title for the app as it stands: the active panel's place
/// first, then the program, as the terminal titles its window ("Documents
/// - diroxide"). A screen reader says it when the window takes the focus.
/// With more than one tab in the active panel the tab follows the place,
/// as in the title line: "Documents, tab 2 of 3 - diroxide".
pub fn window_title(app: &App, program: &str) -> String {
    let panel = app.active_panel();
    // A plugin list over the place names the window, as in the terminal
    // (lane L).
    let shown = match panel.plugin_list() {
        Some(list) => list.list().title().to_owned(),
        None => diroxide_app::panel::place_leaf(panel.loading_place().unwrap_or(panel.place())),
    };
    let shown = diroxide_core::names::display_name(&shown).into_owned();
    let tab = model::tab_after(app, app.active_side());
    format!("{shown}{tab} - {program}")
}

/// Runs the GUI with the app and runtime `diro` built, until the window
/// closes, and gives the app back for what `diro` does after it (the
/// `--cwd-file`). Every failure is a [`GuiError`] in words; the app is
/// dropped then, and `diro` builds a new one for the terminal.
pub fn run(app: App, runtime: Runtime, options: GuiOptions) -> Result<App, GuiError> {
    #[cfg(feature = "gui")]
    {
        window::run(app, runtime, options)
    }
    #[cfg(not(feature = "gui"))]
    {
        let _ = (app, runtime, options);
        Err(GuiError::NotBuilt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_app::AppOptions;
    use diroxide_core::{Place, Side};

    #[test]
    fn a_panel_becomes_a_named_grid() {
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let mut app = App::new(AppOptions::new(here.clone(), here));
        let mut runtime = Runtime::with_local();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        let grid = panel_grid(app.panel(Side::Left));
        assert!(grid.name.starts_with("diroxide-gui, in "));
        assert_eq!(grid.rows[0].cells[0], "src");
        assert_eq!(grid.focus, Some(0));
        assert_eq!(window_title(&app, "diroxide"), "diroxide-gui - diroxide");
    }

    #[test]
    fn graphics_names_parse_and_map_to_wgpu() {
        for name in GraphicsApi::NAMES {
            let api = GraphicsApi::parse(name).unwrap();
            match api {
                GraphicsApi::Auto | GraphicsApi::Software => assert_eq!(api.wgpu_backend(), None),
                _ => assert_eq!(api.wgpu_backend(), Some(name)),
            }
        }
        assert_eq!(GraphicsApi::parse(" Vulkan "), Some(GraphicsApi::Vulkan));
        assert_eq!(GraphicsApi::parse("d3d12"), Some(GraphicsApi::Dx12));
        assert_eq!(GraphicsApi::parse("glide"), None);
        assert_eq!(
            GraphicsApi::names_in_words(),
            "auto, vulkan, dx12, metal, gl or software"
        );
    }

    #[test]
    fn every_error_says_its_cause_first_and_a_next_step() {
        let errors = [
            GuiError::NotBuilt,
            GuiError::NoDisplay("neither WAYLAND_DISPLAY nor DISPLAY is set".into()),
            GuiError::Graphics("no compatible WGPU device found".into()),
            GuiError::EventLoop("the system closed it".into()),
            GuiError::Stopped("index out of bounds".into()),
        ];
        for err in errors {
            let said = diroxide_core::Message::from_error("open", "the GUI", &err).text;
            assert!(said.starts_with("Could not open the GUI: "), "{said}");
            assert!(said.contains("diro --"), "{said}");
            assert!(said.ends_with('.'), "{said}");
        }
    }
}
