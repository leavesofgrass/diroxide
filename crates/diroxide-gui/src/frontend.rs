//! The GUI's loop, one step at a time, over any [`Host`] of widgets: the
//! real window's driver calls these steps as winit's events arrive, and
//! the tests call them on Masonry's in-process harness with no display.
//! The terminal's `Session` is the model (`crates/diroxide-tui/src/session.rs`).
//!
//! - Keys go to [`App::handle`], effects to [`Runtime::carry_out`], and
//!   the workers' results come back through [`Runtime::poll`] when the
//!   runtime's waker rings ([`Frontend::wake`]).
//! - The app's timers (the filter's pause, the quick view's wait, the
//!   viewer's slices) and the key test's ten idle seconds run from
//!   [`Frontend::tick`]; [`Frontend::next_deadline`] says when.
//! - After every step, [`Frontend::refresh`] puts the app's state on the
//!   widgets: the three lines, both panels' list boxes with their windows
//!   of rows, the layout, the live region, the open dialog, the key bar,
//!   and the keyboard focus (the active panel, or the dialog).
//! - Announcements go to the live region by importance. Those the app's
//!   start makes before the window exists are held, and said once the
//!   window and a screen reader are there.
//! - The window's colors follow the theme setting, the owner's `[colors]`,
//!   the system's appearance while following it, and the system's high
//!   contrast colors while that mode is on ([`crate::theme`]); a theme
//!   chosen is said with its contrast verdict in words.
//! - Focus mode (the `focus_mode` setting) and full screen: after
//!   [`FADE_AFTER`] with no key and no mouse move, the bars around the
//!   body fade under a veil; any key or a moving mouse brings them back.
//!   Painting only: the tree stays exactly as it was, and the fade says
//!   nothing (textweaver's `bars.rs`, the owner's own project). Full
//!   screen is the app's command, `view.full-screen` (F11 in the GUI
//!   column, the View menu, the palette; lane A6), which says "Full
//!   screen on." or "Full screen off."; the window carries out the app's
//!   [`App::full_screen`] after each event.

use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::keymap::Platform;
use diroxide_app::{App, AppEvent, Dialog, Runtime};
use diroxide_core::{Command, Effect, Key, KeyCode, Message, Modifiers, Side};
use diroxide_theme::PanelRole;
use diroxide_theme::os::OsScheme;
use masonry::accesskit::Role;
use masonry::app::RenderRoot;
use masonry::core::keyboard::KeyboardEvent;
use masonry::core::{
    DefaultProperties, ErasedAction, FromDynWidget, NewWidget, Widget, WidgetId, WidgetMut,
    WidgetTag,
};
use masonry::properties::ContentColor;
use masonry::widgets::{Flex, Label};

use crate::model::{self, Lines, PanelView};
use crate::picture::{Opened, PictureView, Zoom};
use crate::theme::{Choice, Palette, SystemColors};
use crate::widgets::announcer::Announcer;
use crate::widgets::buttons::{ButtonPressed, ButtonRow};
use crate::widgets::choice::ChoiceAction;
use crate::widgets::paint;
use crate::widgets::panel::{PanelAction, PanelBox};
use crate::widgets::panels::Panels;
use crate::widgets::picture::{PictureAction, PictureBox, PictureLines};
use crate::widgets::progress::JobProgressBar;
use crate::widgets::push::PushAction;
use crate::widgets::root::{Region, Root, RootAction};
use crate::widgets::screen::{Body, Screen};
use crate::widgets::viewer::{ViewerAction, ViewerBox};

/// The title line's text: the active place first.
pub const TITLE_LINE: WidgetTag<Label> = WidgetTag::named("diroxide-title-line");
/// The status line's text: the last message, or the folder and its
/// counts.
pub const STATUS_LINE: WidgetTag<Label> = WidgetTag::named("diroxide-status-line");
/// The jobs line's text: "No jobs", or what runs.
pub const JOBS_LINE: WidgetTag<Label> = WidgetTag::named("diroxide-jobs-line");
/// The two panels.
pub const PANELS: WidgetTag<Panels> = WidgetTag::named("diroxide-panels");
/// The left panel's list box.
pub const LEFT_PANEL: WidgetTag<PanelBox> = WidgetTag::named("diroxide-left-panel");
/// The right panel's list box.
pub const RIGHT_PANEL: WidgetTag<PanelBox> = WidgetTag::named("diroxide-right-panel");
/// The live region.
pub const ANNOUNCER: WidgetTag<Announcer> = WidgetTag::named("diroxide-announcer");
/// The root.
pub const ROOT: WidgetTag<Root> = WidgetTag::named("diroxide-root");
/// The panels, or the viewer in their place.
pub const SCREEN: WidgetTag<Screen> = WidgetTag::named("diroxide-screen");
/// The viewer's list box.
pub const VIEWER: WidgetTag<ViewerBox> = WidgetTag::named("diroxide-viewer");
/// The progress of the job in front.
pub const JOB_PROGRESS: WidgetTag<JobProgressBar> = WidgetTag::named("diroxide-job-progress");
/// The picture view's graphic (alpha 4, R30 commit 112).
pub const PICTURE: WidgetTag<PictureBox> = WidgetTag::named("diroxide-picture");
/// The lines under the picture: its zoom, its animation, its keys.
pub const PICTURE_LINES: WidgetTag<PictureLines> = WidgetTag::named("diroxide-picture-lines");

/// How long a picture may take to read before the window says it is
/// opening it, so a quick one is not announced twice.
pub const OPENING_AFTER: Duration = Duration::from_millis(500);

/// Where the picture view was opened from, and goes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureFrom {
    /// The viewer.
    Viewer,
    /// The quick view's preview region.
    Preview,
    /// The panel's focused row: F3 or Show picture with nothing open
    /// (lane VP).
    Panel,
}

/// A PDF page shown as a picture (lane VP): the page, and the pages in
/// all when known.
pub type Page = diroxide_app::app::picture_view::PageAsk;

/// The open picture view.
#[derive(Clone, Debug)]
struct OpenPicture {
    place: diroxide_core::Place,
    from: PictureFrom,
    view: PictureView,
    animation: Option<String>,
    /// For a PDF, the page shown.
    page: Option<Page>,
}

/// A picture being read on a thread of its own.
#[derive(Debug)]
struct Reading {
    place: diroxide_core::Place,
    name: String,
    from: PictureFrom,
    /// For a PDF, the page drawn.
    page: Option<Page>,
    answer: std::sync::mpsc::Receiver<Result<Opened, Message>>,
    since: Instant,
    said: bool,
}

/// A panel's list box tag.
pub const fn panel_tag(side: Side) -> WidgetTag<PanelBox> {
    match side {
        Side::Left => LEFT_PANEL,
        Side::Right => RIGHT_PANEL,
    }
}

/// The key bar.
pub const KEY_BAR: WidgetTag<ButtonRow> = WidgetTag::named("diroxide-key-bar");
/// The left panel's sort buttons.
pub const LEFT_SORT: WidgetTag<ButtonRow> = WidgetTag::named("diroxide-left-sort");
/// The right panel's sort buttons.
pub const RIGHT_SORT: WidgetTag<ButtonRow> = WidgetTag::named("diroxide-right-sort");

/// A panel's sort buttons' tag.
pub const fn sort_tag(side: Side) -> WidgetTag<ButtonRow> {
    match side {
        Side::Left => LEFT_SORT,
        Side::Right => RIGHT_SORT,
    }
}

/// How long the key test waits for a key before it ends, as the
/// terminal's does.
pub const KEY_TEST_IDLE: Duration = Duration::from_secs(10);

/// How often the driver looks again while the live region holds messages
/// for a screen reader that has not asked for the tree yet.
pub const HOLD_RETRY: Duration = Duration::from_millis(200);

/// How long the window rests (no key, no mouse move) before focus mode
/// fades the bars.
pub const FADE_AFTER: Duration = Duration::from_secs(3);

/// Where the driver edits widgets: the real window's render root, or the
/// tests' harness.
pub trait Host {
    /// Edits the widget with `tag`.
    fn edit<W: Widget + FromDynWidget + ?Sized, R>(
        &mut self,
        tag: WidgetTag<W>,
        f: impl FnOnce(WidgetMut<'_, W>) -> R,
    ) -> R;
    /// The id of the widget with `tag`, when it is in the tree.
    fn id_of<W: Widget + FromDynWidget + ?Sized>(&self, tag: WidgetTag<W>) -> Option<WidgetId>;
    /// The widget with the keyboard focus.
    fn focused(&self) -> Option<WidgetId>;
    /// Moves the keyboard focus; false when the widget cannot take it.
    fn focus(&mut self, id: Option<WidgetId>) -> bool;
    /// Names the window's accessibility node (AT-SPI and macOS read the
    /// window's title from it).
    fn set_window_label(&mut self, label: &str);
    /// Replaces the window's default properties: another theme's colors
    /// for every widget at once.
    fn set_default_properties(&mut self, props: DefaultProperties);
    /// Puts text on the system clipboard, or says why not in words. By
    /// default through Masonry's clipboard signal (the window runner's
    /// clipboard; the tests' harness keeps the text).
    fn set_clipboard(&mut self, text: String) -> Result<(), String> {
        self.edit(ROOT, |mut root| root.ctx.set_clipboard(text));
        Ok(())
    }
}

impl Host for RenderRoot {
    fn edit<W: Widget + FromDynWidget + ?Sized, R>(
        &mut self,
        tag: WidgetTag<W>,
        f: impl FnOnce(WidgetMut<'_, W>) -> R,
    ) -> R {
        self.edit_widget_with_tag(tag, f)
    }

    fn id_of<W: Widget + FromDynWidget + ?Sized>(&self, tag: WidgetTag<W>) -> Option<WidgetId> {
        self.get_widget_with_tag(tag).map(|w| w.id())
    }

    fn focused(&self) -> Option<WidgetId> {
        self.focused_widget()
    }

    fn focus(&mut self, id: Option<WidgetId>) -> bool {
        self.focus_on(id)
    }

    fn set_window_label(&mut self, label: &str) {
        RenderRoot::set_window_label(self, label);
    }

    fn set_default_properties(&mut self, props: DefaultProperties) {
        RenderRoot::set_default_properties(self, Arc::new(props));
    }

    #[cfg(windows)]
    fn set_clipboard(&mut self, text: String) -> Result<(), String> {
        // Written directly: the window runner's clipboard would end the
        // window when another program holds the clipboard.
        crate::platform::clipboard::set_text(&text)
    }
}

/// What the widgets show now, so a refresh edits only what changed.
#[derive(Debug, Default)]
struct Shown {
    lines: Lines,
    panels: [Option<PanelView>; 2],
    arranged: Option<(diroxide_app::config::Layout, Side)>,
    /// Whether the viewer is shown in the panels' place.
    viewing: bool,
    /// What the body shows: the panels, the viewer, or the picture view.
    body: Body,
    /// The picture view as shown, and its lines.
    picture: Option<PictureView>,
    picture_lines: Vec<String>,
    /// The viewer's rows as shown.
    viewer: Option<model::ViewerView>,
    /// What the window's colors were made from, as last put on the
    /// widgets.
    colors: Option<Choice>,
    dialog: Option<model::DialogView>,
    key_bar: Vec<model::ButtonView>,
    sorting: Vec<model::ButtonView>,
    /// The window's name as its accessibility node has it.
    label: String,
    /// The job in front's percent as the progress indicator shows it.
    progress: Option<u64>,
}

/// The GUI's driver: the app, its runtime, and what the window shows.
pub struct Frontend {
    app: App,
    runtime: Runtime,
    platform: Platform,
    program: String,
    page: [usize; 2],
    top: [usize; 2],
    shown: Shown,
    /// Messages for the live region, said on the next refresh.
    to_say: Vec<Said>,
    /// Every message the app announced, in order (the tests read it).
    said: Vec<Message>,
    /// Every announcement the live region made, in order: what a screen
    /// reader was told (the tests read it).
    spoken: Vec<Message>,
    /// The window's own focus in the open dialog: a confirmation's
    /// buttons and listing, a text's lines, a secret's toggle.
    local: model::LocalFocus,
    /// Every text the app put on the clipboard, in order (the tests read
    /// it).
    clipboard: Vec<String>,
    /// Text for the system clipboard, put there on the next refresh.
    to_copy: Vec<String>,
    quit: bool,
    key_test_since: Option<Instant>,
    app_tick_at: Option<Instant>,
    quick_tick_at: Option<Instant>,
    viewer_tick_at: Option<Instant>,
    hold_retry_at: Option<Instant>,
    /// The viewer's columns and rows, as its list box last told them.
    viewer_size: (usize, usize),
    /// Every value the progress indicator was given, in order (the tests
    /// read it).
    progress_shown: Vec<Option<u64>>,
    /// The percent of every progress report the workers sent, in order
    /// (the tests read it).
    progress_reports: Vec<u64>,
    /// The window has a native menu bar (Windows, the `menus` setting's
    /// native): the menu keys enter it.
    menu_bar: bool,
    /// What the menu keys asked of the native bar, for the window.
    menu_requests: Vec<MenuEnter>,
    /// The colors the window draws with.
    palette: Arc<Palette>,
    /// The system's high contrast colors while that mode is on.
    system: Option<SystemColors>,
    /// The system's light, dark or high contrast appearance, read at the
    /// start.
    appearance: OsScheme,
    /// The theme setting at the start: following the system's appearance
    /// stops once the owner chooses a theme in the session.
    start_theme: String,
    /// When the window last had a key or a mouse move: focus mode's rest
    /// starts there.
    rest_since: Instant,
    /// Focus mode's veil is down over the bars.
    veiled: bool,
    /// The window is full screen: the app's `view.full-screen`, as last
    /// carried out.
    full_screen: bool,
    /// Full screen turned on or off, for the window to carry out.
    full_screen_asked: Option<bool>,
    /// The open picture view (R30 commit 112).
    picture: Option<OpenPicture>,
    /// A picture being read for the view.
    reading: Option<Reading>,
    /// The guard picture decodes run under: the preview crate's, one at
    /// a time, at the pixel guard's deadline.
    picture_guard: diroxide_preview::doc::Guard,
    /// The picture view's size, as its graphic last told it.
    picture_size: (f64, f64),
    /// Counts the pictures shown, so each is a new image.
    picture_count: u64,
    /// What wakes the window's loop when a picture is read.
    waker: Option<diroxide_app::runtime::Waker>,
    /// Where copied text goes instead of the host's clipboard: the
    /// window's Wayland clipboard on Linux under Wayland (R30 commit 113).
    clipboard_writer: Option<ClipboardWriter>,
}

/// Puts copied text on a system clipboard, or says why not in words.
pub type ClipboardWriter = Box<dyn FnMut(&str) -> Result<(), String>>;

/// A message waiting for the live region.
#[derive(Clone, Debug)]
struct Said {
    message: Message,
    /// A job's start line, dropped when a later line comes in the same
    /// step.
    start: bool,
    /// The line the app said as it opened a dialog: left unsaid when the
    /// dialog says the same by itself as the focus moves into it.
    opening: bool,
}

/// How a key enters the native menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEnter {
    /// The bar, its first menu selected (Alt alone, F10, Menus).
    Bar,
    /// The menu with this access key, open (Alt with the letter).
    Menu(char),
}

impl Frontend {
    /// A driver for the app and runtime `diro` built; `program` ends the
    /// window's title ("Documents - diroxide").
    pub fn new(mut app: App, runtime: Runtime, program: impl Into<String>) -> Self {
        // The quick view reads through the runtime's providers, archives
        // among them, as in the terminal.
        app.set_quick_view_registry(runtime.registry().clone());
        let start_theme = app.settings().theme.clone();
        Self {
            start_theme,
            rest_since: Instant::now(),
            veiled: false,
            full_screen: false,
            full_screen_asked: None,
            palette: Arc::new(Palette::default()),
            system: None,
            appearance: OsScheme::Unknown,
            app,
            runtime,
            platform: Platform::current(),
            program: program.into(),
            page: [20, 20],
            top: [0, 0],
            shown: Shown::default(),
            to_say: Vec::new(),
            said: Vec::new(),
            spoken: Vec::new(),
            local: model::LocalFocus::default(),
            clipboard: Vec::new(),
            to_copy: Vec::new(),
            quit: false,
            key_test_since: None,
            app_tick_at: None,
            quick_tick_at: None,
            viewer_tick_at: None,
            hold_retry_at: None,
            viewer_size: (80, 20),
            progress_shown: Vec::new(),
            progress_reports: Vec::new(),
            menu_bar: false,
            menu_requests: Vec::new(),
            picture: None,
            reading: None,
            picture_guard: diroxide_preview::doc::Guard::new(
                diroxide_preview::images::PixelGuard::DEFAULT.deadline,
                1,
            ),
            picture_size: (640.0, 480.0),
            picture_count: 0,
            waker: None,
            clipboard_writer: None,
        }
    }

    /// Copied text goes through `writer` from now on, not the host's
    /// clipboard: the window's own Wayland clipboard on Linux under
    /// Wayland, where the window runner's does nothing (R30 commit 113).
    pub fn set_clipboard_writer(&mut self, writer: ClipboardWriter) {
        self.clipboard_writer = Some(writer);
    }

    /// What wakes the window's loop when a picture the view reads is in:
    /// the runtime's own waker, which the window gives both.
    pub fn set_waker(&mut self, waker: diroxide_app::runtime::Waker) {
        self.waker = Some(waker);
    }

    /// The same driver pressing keys as on `platform` (the tests check
    /// every system's keys on one).
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }

    /// The app.
    pub fn app(&self) -> &App {
        &self.app
    }

    /// The app, for a test's hooks.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// The runtime, for the real window's waker.
    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    /// Gives back the app, for what `diro` does after the window (the
    /// `--cwd-file`).
    pub fn into_app(self) -> App {
        self.app
    }

    /// Everything the app announced so far, in order.
    pub fn said(&self) -> &[Message] {
        &self.said
    }

    /// Everything the live region announced so far, in order: the app's
    /// lines of each step joined, Low lines and a dialog's opening line
    /// left out where the dialog says it by itself.
    pub fn spoken(&self) -> &[Message] {
        &self.spoken
    }

    /// The window's own focus in the open dialog.
    pub fn local_focus(&self) -> model::LocalFocus {
        self.local
    }

    /// The texts the app put on the clipboard.
    pub fn clipboard_texts(&self) -> &[String] {
        &self.clipboard
    }

    /// Whether the app quit.
    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    /// The colors the window draws with now.
    pub fn palette(&self) -> &Arc<Palette> {
        &self.palette
    }

    /// Whether focus mode's veil is down over the bars.
    pub fn veiled(&self) -> bool {
        self.veiled
    }

    /// Whether the window is full screen.
    pub fn full_screen(&self) -> bool {
        self.full_screen
    }

    /// Full screen turned on (`Some(true)`) or off since the last call,
    /// for the window to carry out.
    pub fn take_full_screen(&mut self) -> Option<bool> {
        self.full_screen_asked.take()
    }

    /// The window stays as it is: a window that runs in the background
    /// for a check never goes full screen. The app is told, so the next
    /// Full screen starts from off.
    pub fn refuse_full_screen(&mut self) {
        self.full_screen = false;
        self.app.set_full_screen(false);
    }

    /// Whether the bars fade when the window rests: focus mode, or full
    /// screen.
    fn fades(&self) -> bool {
        self.app.settings().focus_mode || self.full_screen
    }

    /// A key or a moving mouse: the rest starts over, and the veil lifts.
    fn wake_bars(&mut self, host: &mut impl Host, now: Instant) {
        self.rest_since = now;
        if self.veiled {
            self.veiled = false;
            host.edit(ROOT, |mut root| Root::set_veiled(&mut root, false));
        }
    }

    /// The veil comes down once the window has rested [`FADE_AFTER`] in
    /// focus mode or full screen, with no dialog up; it lifts when
    /// neither is on any more.
    fn fade(&mut self, host: &mut impl Host, now: Instant) {
        let rested = now.saturating_duration_since(self.rest_since) >= FADE_AFTER;
        let veil = self.fades() && self.app.dialog().is_none() && (self.veiled || rested);
        if veil != self.veiled {
            self.veiled = veil;
            host.edit(ROOT, |mut root| Root::set_veiled(&mut root, veil));
        }
    }

    /// The system's high contrast colors while that mode is on (Windows'
    /// contrast themes), or `None`: the window reads them on every step,
    /// so turning the mode on or off applies at once.
    pub fn set_system_colors(&mut self, colors: Option<SystemColors>) {
        self.system = colors;
    }

    /// The system's light, dark or high contrast appearance, read once
    /// at the start (the theme crate's `os::probe`).
    pub fn set_appearance(&mut self, scheme: OsScheme) {
        self.appearance = scheme;
    }

    /// What the window's colors are made from now: the theme setting, or
    /// while following the system's appearance (and no theme was chosen
    /// in this session) the theme the system's setting gives; the owner's
    /// `[colors]`; and the system's high contrast colors, which win.
    /// Elsewhere than Windows, the system's high contrast setting chooses
    /// the built-in high-contrast theme. The themes are the app's
    /// ([`App::themes`]), the owner's theme files among them.
    pub fn colors_choice(&self) -> Choice {
        let settings = self.app.settings();
        let chosen = settings.theme != self.start_theme;
        let theme = if self.appearance == OsScheme::HighContrast && !cfg!(windows) && !chosen {
            "high-contrast".to_owned()
        } else {
            crate::theme::drawn_theme(
                &settings.theme,
                settings.follow_system_appearance && !chosen,
                self.appearance,
                self.app.themes(),
            )
        };
        Choice {
            theme,
            colors: settings.colors.clone(),
            system: self.system,
        }
    }

    /// The window's default properties with the colors as they stand, for
    /// the window's start.
    pub fn window_properties(&mut self) -> DefaultProperties {
        self.palette = Arc::new(Palette::of(&self.colors_choice(), self.app.themes()));
        paint::default_properties(&self.palette)
    }

    /// Puts the colors on the widgets when what they are made from
    /// changed: the default properties of every widget, and the three
    /// lines' text colors (the title line on the page, the status and
    /// jobs lines on the status bar's band). A theme chosen after the
    /// first is said with its verdict; at the first, a theme name the app
    /// does not know is said ([`Self::unknown_theme`]).
    fn sync_colors(&mut self, host: &mut impl Host) {
        let choice = self.colors_choice();
        if self.shown.colors.as_ref() == Some(&choice) {
            return;
        }
        let first = self.shown.colors.is_none();
        let palette = Arc::new(Palette::of(&choice, self.app.themes()));
        host.set_default_properties(paint::default_properties(&palette));
        let text = paint::color(palette.text);
        let status = paint::color(palette.look(PanelRole::StatusLine).fg);
        let jobs = paint::color(palette.look(PanelRole::JobsLine).fg);
        host.edit(TITLE_LINE, |mut l| l.insert_prop(ContentColor::new(text)));
        host.edit(STATUS_LINE, |mut l| {
            l.insert_prop(ContentColor::new(status))
        });
        host.edit(JOBS_LINE, |mut l| l.insert_prop(ContentColor::new(jobs)));
        let changed =
            palette.key != self.palette.key || palette.fell_back() != self.palette.fell_back();
        self.palette = palette;
        self.shown.colors = Some(choice);
        if first {
            if let Some(line) = self.unknown_theme() {
                let effects = self.app.handle(AppEvent::Problem(Message::warning(line)));
                self.carry(effects);
            }
        } else if changed && let Some(line) = self.colors_line() {
            let effects = self.app.handle(AppEvent::Problem(Message::info(line)));
            self.carry(effects);
        }
    }

    /// What the window says at its start when `--theme` or the theme
    /// setting names a theme that is neither built in nor one of the
    /// owner's theme files ([`App::themes`]): "Unknown theme no-such.
    /// Galaxy is in use.", with the theme drawn in its place. Nothing
    /// while the system's high contrast colors are drawn, since no theme
    /// is.
    fn unknown_theme(&self) -> Option<String> {
        let name = &self.app.settings().theme;
        if self.app.themes().get(name).is_some() || self.palette.key == "system" {
            return None;
        }
        Some(crate::theme::unknown_theme_line(name, &self.palette.name))
    }

    /// What is said when the window's colors change: the theme and its
    /// contrast verdict in words ("Theme Nord, meets AA."), with the
    /// window's own roles in the text color when there are any. It takes
    /// the place of the setting's plain line ("Theme: nord"); after Next
    /// theme, whose line already says the verdict, only the window's
    /// part is said.
    fn colors_line(&mut self) -> Option<String> {
        if self.palette.key == "system" {
            return Some("System high contrast colors in use.".to_owned());
        }
        let mut settings = self.app.settings().clone();
        if let Some(choice) = &self.shown.colors {
            settings.theme = choice.theme.clone();
        }
        // The verdict of the theme drawn, found among the app's themes,
        // so an owner's theme file is said by its own name.
        let verdict = diroxide_app::settings::theme_line_in(&settings, self.app.themes());
        let said = self
            .to_say
            .iter()
            .any(|s| s.message.text.starts_with(verdict.as_str()));
        // The setting's own line is replaced by the verdict.
        self.to_say.retain(|s| {
            !(s.message.text.starts_with("Theme: ") || s.message.text.starts_with("Theme back to "))
        });
        let mut parts = Vec::new();
        if !said {
            parts.push(format!("{verdict}."));
        }
        if let Some(words) = self.palette.fallback_words() {
            parts.push(words);
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// The window's title as the app stands: "Documents - diroxide".
    pub fn window_title(&self) -> String {
        crate::window_title(&self.app, &self.program)
    }

    /// Starts the app: its startup lines and both listings. The screen's
    /// effects wait for the first [`Self::refresh`]; its announcements are
    /// held for the live region.
    pub fn start(&mut self) {
        let effects = self.app.start();
        self.carry(effects);
    }

    /// The window's widgets for the app as it stands.
    pub fn root_widget(&mut self) -> NewWidget<Root> {
        let full_passes = std::rc::Rc::new(std::cell::Cell::new(0));
        let lines = model::lines(&self.app);
        let sorting = model::sort_buttons(&self.app);
        self.shown.sorting = sorting.clone();
        let panel = |frontend: &mut Self, side: Side| {
            let view = frontend.view(side);
            frontend.shown.panels[side.index()] = Some(view.clone());
            // The panel's sort buttons above its list box (the header
            // buttons), then the list box.
            let sort = NewWidget::new(ButtonRow::new(model::sort_row_name(side), sorting.clone()))
                .with_tag(sort_tag(side));
            let list = NewWidget::new(PanelBox::new(view)).with_tag(panel_tag(side));
            (sort, list)
        };
        let left = panel(self, Side::Left);
        let right = panel(self, Side::Right);
        let layout = self.app.settings().layout;
        let active = self.app.active_side();
        self.shown.arranged = Some((layout, active));
        let panels = NewWidget::new(Panels::new(left, right, layout, active)).with_tag(PANELS);
        let percent = self.front_percent();
        self.shown.progress = percent;
        let status = Flex::column()
            .with_fixed(NewWidget::new(Label::new(lines.status.clone())).with_tag(STATUS_LINE))
            .with_fixed(NewWidget::new(Label::new(lines.jobs.clone())).with_tag(JOBS_LINE))
            .with_fixed(NewWidget::new(JobProgressBar::new(percent)).with_tag(JOB_PROGRESS));
        let status_bar =
            Region::new(NewWidget::new(status), Role::Status, "").with_band(PanelRole::StatusLine);
        let keys = model::key_bar_buttons(&self.app);
        self.shown.key_bar = keys.clone();
        let key_bar = NewWidget::new(ButtonRow::new(model::KEY_BAR_NAME, keys)).with_tag(KEY_BAR);
        // The viewer takes the panels' place while a file is viewed.
        let viewer = NewWidget::new(ViewerBox::new(model::ViewerView {
            name: "Viewer".to_owned(),
            generation: 0,
            rows: Vec::new(),
            focus: None,
            lines: 0,
        }))
        .with_tag(VIEWER);
        // The body's place in the window, which focus mode's veil leaves
        // clear.
        let body = std::rc::Rc::new(std::cell::Cell::new(masonry::kurbo::Rect::ZERO));
        // The picture view in the same place while a picture is open: the
        // graphic, then the lines that say its zoom and its keys.
        let picture = Flex::column()
            .with(NewWidget::new(PictureBox::new()).with_tag(PICTURE), 1.0)
            .with_fixed(NewWidget::new(PictureLines::new()).with_tag(PICTURE_LINES));
        let screen = NewWidget::new(
            Screen::new(panels, viewer)
                .with_picture(NewWidget::new(picture))
                .with_area(std::rc::Rc::clone(&body)),
        )
        .with_tag(SCREEN);
        let column = Flex::column()
            .with_fixed(NewWidget::new(Label::new(lines.title.clone())).with_tag(TITLE_LINE))
            .with(screen, 1.0)
            .with_fixed(NewWidget::new(status_bar))
            .with_fixed(key_bar);
        self.shown.lines = lines;
        // The start's lines, said before the window existed, wait in the
        // live region for a screen reader to ask for the tree, as one
        // announcement in their order.
        let held = self.take_to_say(None);
        let announcer =
            NewWidget::new(Announcer::new(std::rc::Rc::clone(&full_passes)).with_pending(held))
                .with_tag(ANNOUNCER);
        let main = NewWidget::new(Region::new(
            NewWidget::new(column),
            Role::GenericContainer,
            "",
        ));
        NewWidget::new(Root::new(main, announcer, full_passes).with_body(body)).with_tag(ROOT)
    }

    fn view(&mut self, side: Side) -> PanelView {
        let i = side.index();
        let view = model::panel_view(&self.app, side, self.top[i], self.page[i]);
        self.top[i] = view.top;
        view
    }

    /// Whether a text field has the focus (an Edit or a Secret dialog),
    /// where held keys repeat as typing.
    fn typing(&self) -> bool {
        self.app
            .dialog()
            .is_some_and(|(_, d)| matches!(d, Dialog::Edit { .. } | Dialog::Secret { .. }))
    }

    /// A key event from the window: turned into the app's key, if it is
    /// one, and handed to the app.
    pub fn key_event(&mut self, host: &mut impl Host, event: &KeyboardEvent, now: Instant) {
        if let Some(key) = crate::keys::to_key(event, self.platform, self.typing()) {
            self.key(host, key, now);
        }
    }

    /// A key, as the app reads it.
    pub fn key(&mut self, host: &mut impl Host, key: Key, now: Instant) {
        if self.key_test_since.is_some() {
            self.key_test_since = Some(now);
        }
        // Any key brings the bars back, and says nothing of it. F11 is
        // the app's: the GUI column binds it to view.full-screen, which
        // the window carries out after the event ([`Self::after`]).
        self.wake_bars(host, now);
        if let Some(enter) = self.menu_key(&key) {
            // The native menu bar takes the key: the window shows the bar
            // (when it is hidden) and enters it, as Windows' own menu keys
            // would.
            self.menu_requests.push(enter);
            return;
        }
        if self.picture_key(&key, now) {
            // The picture view's own keys; Enter opening it is the app's
            // Show picture (view.picture, lane AE).
            self.after(now);
            self.refresh(host);
            return;
        }
        if !self.app.key_test_active() && self.local_key(&key) {
            // The window moved its own focus in the dialog, or answered
            // for the focused button.
            self.after(now);
            self.refresh(host);
            return;
        }
        let effects = self.app.handle(AppEvent::Key(key));
        self.carry(effects);
        self.after(now);
        self.refresh(host);
    }

    /// The window's own focus, started again when the top dialog is
    /// another one.
    fn sync_local(&mut self) {
        match self.app.dialog() {
            Some((id, dialog)) if self.local.dialog != Some(id) => {
                self.local = model::LocalFocus::start(id, dialog);
            }
            Some(_) => {}
            None => self.local = model::LocalFocus::default(),
        }
    }

    /// A key the window handles in a dialog by itself, since the app does
    /// not model what it moves: Tab and the arrows between a
    /// confirmation's buttons and listing, the arrows through a text's
    /// lines or the listing, and Enter or Space on a button, which
    /// answers as that button's key does (the app's settle time still
    /// holds for Yes). True when the key was the window's.
    fn local_key(&mut self, key: &Key) -> bool {
        self.sync_local();
        // Change key's prompt waits for any key, exactly as it came.
        if self.app.capturing_key() {
            return false;
        }
        let Some(view) = model::dialog_view_at(&self.app, &self.local) else {
            return false;
        };
        let shift_only = key.mods == Modifiers::SHIFT;
        let plain = key.mods.is_empty();
        if key.code == KeyCode::Tab && (plain || shift_only) {
            let order = view.tab_order();
            if order.len() < 2 {
                return false;
            }
            let at = order.iter().position(|f| *f == view.focus).unwrap_or(0);
            let next = if shift_only {
                (at + order.len() - 1) % order.len()
            } else {
                (at + 1) % order.len()
            };
            self.local.on = order[next];
            return true;
        }
        if !plain {
            return false;
        }
        match view.focus {
            model::DialogFocus::No | model::DialogFocus::Yes => match key.code {
                KeyCode::Left | KeyCode::Up => {
                    self.local.on = model::DialogFocus::No;
                    true
                }
                KeyCode::Right | KeyCode::Down => {
                    self.local.on = model::DialogFocus::Yes;
                    true
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.press(view.focus);
                    true
                }
                _ => false,
            },
            model::DialogFocus::Show => match key.code {
                KeyCode::Char(' ') => {
                    self.press(model::DialogFocus::Show);
                    true
                }
                // Anything typed goes to the field, with the focus.
                _ => {
                    if key.typed_char().is_some()
                        || matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
                    {
                        self.local.on = model::DialogFocus::Control;
                    }
                    false
                }
            },
            model::DialogFocus::Listing
            | model::DialogFocus::Control
            | model::DialogFocus::Message
                if view.local_list || view.focus == model::DialogFocus::Message =>
            {
                let total = match &view.control {
                    _ if view.focus == model::DialogFocus::Message => {
                        view.message.as_ref().map_or(0, |l| l.total)
                    }
                    model::ControlView::List(list) => list.total,
                    model::ControlView::Confirm(confirm) => {
                        confirm.listing.as_ref().map_or(0, |l| l.total)
                    }
                    model::ControlView::Field(_) => 0,
                };
                let page = model::DIALOG_PAGE.saturating_sub(1).max(1);
                let last = total.saturating_sub(1);
                let line = self.local.line.min(last);
                let to = match key.code {
                    KeyCode::Up => line.saturating_sub(1),
                    KeyCode::Down => (line + 1).min(last),
                    KeyCode::PageUp => line.saturating_sub(page),
                    KeyCode::PageDown => (line + page).min(last),
                    KeyCode::Home => 0,
                    KeyCode::End => last,
                    _ => return false,
                };
                self.local.line = to;
                true
            }
            _ => false,
        }
    }

    /// Presses a dialog's button: No and Yes answer as their keys do
    /// (through the app's commands, so Yes waits out the settle time);
    /// Show turns the secret's toggle as its key does.
    fn press(&mut self, button: model::DialogFocus) {
        let effects = match button {
            model::DialogFocus::No => self.app.handle(AppEvent::Command(Command::new(ids::NO))),
            model::DialogFocus::Yes => self.app.handle(AppEvent::Command(Command::new(ids::YES))),
            model::DialogFocus::Show => self
                .app
                .handle(AppEvent::Key(Key::new(KeyCode::Char('s'), Modifiers::ALT))),
            model::DialogFocus::Control
            | model::DialogFocus::Listing
            | model::DialogFocus::Message => Vec::new(),
        };
        self.carry(effects);
    }

    /// The picture view as it shows now.
    pub fn picture_view(&self) -> Option<&PictureView> {
        self.picture.as_ref().map(|open| &open.view)
    }

    /// Whether a picture is being read for the view.
    pub fn picture_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// A line of the window's own for the live region.
    fn say_window(&mut self, message: Message) {
        self.said.push(message.clone());
        self.to_say.push(Said {
            message,
            start: false,
            opening: false,
        });
    }

    /// The picture the app's Show picture asked for (view.picture, lane
    /// AE), read for the picture view; one at a time, so an ask while a
    /// picture is read or shown waits for nothing.
    fn take_picture_ask(&mut self, now: Instant) {
        use diroxide_app::app::picture_view::PictureSource;
        let Some(ask) = self.app.take_picture_ask() else {
            return;
        };
        if self.reading.is_some() || self.picture.is_some() {
            return;
        }
        let from = match ask.from {
            PictureSource::Viewer => PictureFrom::Viewer,
            PictureSource::Preview => PictureFrom::Preview,
            PictureSource::Panel => PictureFrom::Panel,
        };
        self.read_picture(ask.place, ask.name, from, ask.page, now);
    }

    /// Whether the viewer or the preview still shows the picture at
    /// `place`; the picture view closes with its source.
    fn still_shown(&self, from: PictureFrom, place: &diroxide_core::Place) -> bool {
        match from {
            PictureFrom::Viewer => {
                !self.app.in_preview() && self.app.viewer().is_some_and(|v| v.place() == place)
            }
            PictureFrom::Preview => {
                self.app.in_preview()
                    && self
                        .app
                        .quick_view()
                        .and_then(|q| q.shown())
                        .is_some_and(|s| &s.place == place)
            }
            PictureFrom::Panel => {
                !self.app.in_preview()
                    && self.app.viewer().is_none()
                    && self.app.dialog().is_none()
                    && self.app.active_panel().focused_place().as_ref() == Some(place)
            }
        }
    }

    /// The picture view's keys; true when the key was the view's. In the
    /// view: Plus and Minus zoom, 0 fits, 1 shows the actual size, the
    /// arrows move a zoomed picture, Escape closes; every other key is
    /// the app's.
    fn picture_key(&mut self, key: &Key, now: Instant) -> bool {
        if self.app.key_test_active() || self.app.capturing_key() || self.app.dialog().is_some() {
            return false;
        }
        let plain = key.mods.is_empty();
        let bare = !key.mods.ctrl && !key.mods.alt && !key.mods.meta;
        if self.picture.is_some() {
            match key.code {
                KeyCode::Escape if plain => self.picture = None,
                // Enter shows what is shown already; it never reaches the
                // panel's row behind the picture (lane VP).
                KeyCode::Enter if plain => {}
                KeyCode::Char('+' | '=') | KeyCode::KeypadPlus if bare => self.zoom_picture(true),
                KeyCode::Char('-') | KeyCode::KeypadMinus if bare => self.zoom_picture(false),
                KeyCode::Char('0') if bare => self.set_zoom(Zoom::Fit),
                KeyCode::Char('1') if bare => self.set_zoom(Zoom::Scale(1.0)),
                KeyCode::Left if plain => self.pan_picture((-1, 0)),
                KeyCode::Right if plain => self.pan_picture((1, 0)),
                KeyCode::Up if plain => self.pan_picture((0, -1)),
                KeyCode::Down if plain => self.pan_picture((0, 1)),
                // A PDF's next and previous page (lane VP).
                KeyCode::PageDown if plain && self.page_shown() => self.turn_page(true, now),
                KeyCode::PageUp if plain && self.page_shown() => self.turn_page(false, now),
                _ => return false,
            }
            return true;
        }
        false
    }

    /// Reads the picture on a thread of its own: the header through the
    /// preview crate, held to the pixel guard, and the pixels only
    /// through its decoder ([`crate::picture::read_picture`]).
    fn read_picture(
        &mut self,
        place: diroxide_core::Place,
        name: String,
        from: PictureFrom,
        page: Option<Page>,
        now: Instant,
    ) {
        let (tx, answer) = std::sync::mpsc::channel();
        let registry = self.runtime.registry().clone();
        let decoder = self.picture_guard.clone();
        let waker = self.waker.clone();
        // ImageMagick and pdftoppm through the app's tools (lane VP).
        let helpers = self.app.picture_helpers();
        let (at, called) = (place.clone(), name.clone());
        let spawned = std::thread::Builder::new()
            .name("diroxide-picture".to_owned())
            .spawn(move || {
                let guard = diroxide_preview::images::PixelGuard::DEFAULT;
                let local = at.local_path().map(std::path::PathBuf::from);
                let read = match page {
                    // A PDF's page, drawn by pdftoppm.
                    Some(page) => {
                        let line =
                            diroxide_app::app::picture_view::page_words(page.page, page.pages);
                        let png = match &local {
                            Some(path) => helpers.page_png(path, page.page, guard.max_buffer),
                            None => Err(vec![crate::picture::NOT_LOCAL.to_owned()]),
                        };
                        Ok(crate::picture::read_page(png, &line, &guard, &decoder))
                    }
                    None => registry
                        .resolve(&at)
                        .and_then(|provider| provider.open_seekable(&at, &Default::default()))
                        .map(|mut source| {
                            let convert = || match &local {
                                Some(path) => helpers.magick_png(path, guard.max_buffer),
                                None => Err(vec![crate::picture::NOT_LOCAL.to_owned()]),
                            };
                            crate::picture::read_picture_with(
                                &mut source,
                                &called,
                                &guard,
                                &decoder,
                                Some(&convert),
                            )
                        })
                        .map_err(|error| Message::from_error("open", &called, &error)),
                };
                let _ = tx.send(read);
                if let Some(wake) = waker {
                    wake();
                }
            });
        if let Err(error) = spawned {
            self.say_window(Message::could_not(
                "open",
                &name,
                &error.to_string(),
                "Try again in a moment.",
            ));
            return;
        }
        self.reading = Some(Reading {
            place,
            name,
            from,
            page,
            answer,
            since: now,
            said: false,
        });
    }

    /// Whether the picture view shows a PDF's page.
    fn page_shown(&self) -> bool {
        self.picture
            .as_ref()
            .is_some_and(|open| open.page.is_some())
    }

    /// The next or the previous page of the PDF shown (lane VP): drawn on
    /// the picture thread, "Page 2 of 3" said when it is in; at the ends,
    /// said.
    fn turn_page(&mut self, forward: bool, now: Instant) {
        if self.reading.is_some() {
            return;
        }
        let Some(open) = self.picture.as_ref() else {
            return;
        };
        let Some(page) = open.page else {
            return;
        };
        let next = if forward {
            page.page.saturating_add(1)
        } else {
            page.page.saturating_sub(1)
        };
        let past_end = page.pages.is_some_and(|pages| next > pages);
        if next == 0 || past_end {
            let which = if forward { "Last page" } else { "First page" };
            let line = format!(
                "{which}, {}",
                diroxide_app::app::picture_view::page_words(page.page, page.pages)
            );
            self.say_window(Message::info(line));
            return;
        }
        let (place, name, from) = (open.place.clone(), open.view.name.clone(), open.from);
        let page = Page {
            page: next,
            pages: page.pages,
        };
        self.read_picture(place, name, from, Some(page), now);
    }

    /// Takes the picture read, when it is in: the view opens on it while
    /// its source still shows it, or the words say why it is not shown.
    /// A read that takes longer than [`OPENING_AFTER`] is said first.
    fn poll_picture(&mut self, now: Instant) {
        use std::sync::mpsc::TryRecvError;
        let Some(mut reading) = self.reading.take() else {
            return;
        };
        let answer = match reading.answer.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => {
                if !reading.said && now.saturating_duration_since(reading.since) >= OPENING_AFTER {
                    reading.said = true;
                    let line = format!("Opening picture {}", reading.name);
                    self.say_window(Message::info(line));
                }
                self.reading = Some(reading);
                return;
            }
            Err(TryRecvError::Disconnected) => Err(Message::could_not(
                "open",
                &reading.name,
                "the reading stopped",
                "Try again.",
            )),
        };
        if !self.still_shown(reading.from, &reading.place) {
            // The viewer closed or went on to another file meanwhile.
            return;
        }
        match answer {
            Ok(Opened::Shown(picture)) => {
                self.picture_count += 1;
                let dims = (picture.pixels.width, picture.pixels.height);
                let zoom = Zoom::Fit;
                let mut lines = crate::picture::view_lines(
                    crate::picture::zoom_words(zoom, self.picture_size, dims),
                    picture.animation.as_deref(),
                );
                if reading.page.is_some() {
                    lines.push(crate::picture::PAGE_HELP_LINE.to_owned());
                }
                // Straight from the panel or a page turned, the picture's
                // words come first, as the quick view says them (lane VP).
                let words_first = reading.from == PictureFrom::Panel || reading.page.is_some();
                let turned = self
                    .picture
                    .as_ref()
                    .is_some_and(|open| open.page.is_some())
                    && reading.page.is_some();
                let sentence = picture.sentence.clone();
                self.picture = Some(OpenPicture {
                    place: reading.place,
                    from: reading.from,
                    view: PictureView {
                        name: reading.name,
                        description: picture.sentence,
                        lines,
                        pixels: picture.pixels,
                        zoom,
                        center: crate::picture::middle(dims),
                        generation: self.picture_count,
                    },
                    animation: picture.animation,
                    page: reading.page,
                });
                if words_first {
                    self.say_window(Message::info(sentence));
                }
                // The graphic says its name and words as the focus moves
                // to it; its keys are said once, after.
                if !turned {
                    self.say_window(Message::info(crate::picture::HELP_LINE));
                }
            }
            Ok(Opened::Refused(words)) => {
                // A line each: the refusal, then an install line.
                for line in words.lines() {
                    self.say_window(Message::info(line));
                }
            }
            Err(message) => self.say_window(message),
        }
    }

    /// The lines under the open picture, for its zoom now.
    fn picture_lines(&mut self) {
        let size = self.picture_size;
        if let Some(open) = self.picture.as_mut() {
            let dims = (open.view.pixels.width, open.view.pixels.height);
            open.view.lines = crate::picture::view_lines(
                crate::picture::zoom_words(open.view.zoom, size, dims),
                open.animation.as_deref(),
            );
            if open.page.is_some() {
                open.view
                    .lines
                    .push(crate::picture::PAGE_HELP_LINE.to_owned());
            }
        }
    }

    /// Sets the zoom, keeps the view's middle inside the picture, and says
    /// the zoom in words.
    fn set_zoom(&mut self, zoom: Zoom) {
        let size = self.picture_size;
        let Some(open) = self.picture.as_mut() else {
            return;
        };
        let dims = (open.view.pixels.width, open.view.pixels.height);
        open.view.zoom = zoom;
        open.view.center = match zoom {
            Zoom::Fit => crate::picture::middle(dims),
            Zoom::Scale(scale) => crate::picture::pan(open.view.center, (0, 0), scale, size, dims),
        };
        let words = crate::picture::zoom_words(zoom, size, dims);
        self.picture_lines();
        self.say_window(Message::info(words));
    }

    /// One zoom step in or out.
    fn zoom_picture(&mut self, inward: bool) {
        let size = self.picture_size;
        let Some(open) = self.picture.as_ref() else {
            return;
        };
        let dims = (open.view.pixels.width, open.view.pixels.height);
        let zoom = crate::picture::zoom_step(open.view.zoom, size, dims, inward);
        self.set_zoom(zoom);
    }

    /// Moves a zoomed picture by an arrow; a picture wholly in view stays.
    fn pan_picture(&mut self, by: (i8, i8)) {
        let size = self.picture_size;
        let Some(open) = self.picture.as_mut() else {
            return;
        };
        let dims = (open.view.pixels.width, open.view.pixels.height);
        let scale = open.view.zoom.scale(size, dims);
        if open.view.zoom != Zoom::Fit {
            open.view.center = crate::picture::pan(open.view.center, by, scale, size, dims);
        }
    }

    /// Tells the driver whether the window has a native menu bar now, so
    /// the menu keys enter it.
    pub fn set_menu_bar(&mut self, native: bool) {
        self.menu_bar = native;
    }

    /// Takes what the menu keys asked of the native bar since the last
    /// call.
    pub fn take_menu_requests(&mut self) -> Vec<MenuEnter> {
        std::mem::take(&mut self.menu_requests)
    }

    /// Whether `key` enters the native menu bar rather than going to the
    /// app: only with a native bar, no dialog open, and no key test or
    /// capture waiting; the Menus command's keys (F9, F10) enter the bar,
    /// a bar menu's command (Alt+F for File) or Alt with a menu's letter
    /// that the keymap leaves free opens that menu. A key the owner bound
    /// to something else stays the app's.
    fn menu_key(&self, key: &Key) -> Option<MenuEnter> {
        if !self.menu_bar || self.app.dialog().is_some() || self.app.key_test_active() {
            return None;
        }
        let layers = [
            diroxide_core::Layer::Global,
            diroxide_core::Layer::Panel,
            diroxide_core::Layer::List,
        ];
        let tree = crate::menus::tree(&self.app);
        match self.app.keymap().lookup(&layers, key) {
            diroxide_app::Lookup::Command(id) if id == ids::MENU => Some(MenuEnter::Bar),
            diroxide_app::Lookup::Command(id) => {
                let menu = diroxide_app::Menu::ALL.iter().find(|m| m.command() == id)?;
                let top = tree.iter().find(|t| t.id == menu.id())?;
                top.access.map(MenuEnter::Menu)
            }
            diroxide_app::Lookup::NoMatch => {
                let only_alt = key.mods.alt && !key.mods.ctrl && !key.mods.meta;
                match key.code {
                    KeyCode::Char(c) if only_alt => crate::menus::by_access(&tree, c)
                        .and_then(|m| m.access)
                        .map(MenuEnter::Menu),
                    _ => None,
                }
            }
        }
    }

    /// A native menu's item was chosen: an open dialog is closed as
    /// Escape closes it, then the command runs as from the palette.
    pub fn menu_command(&mut self, host: &mut impl Host, command: &str, now: Instant) {
        let Ok(id) = diroxide_core::CommandId::new(command.to_owned()) else {
            return;
        };
        for _ in 0..8 {
            if self.app.dialog().is_none() {
                break;
            }
            let effects = self.app.handle(AppEvent::Key(Key::plain(KeyCode::Escape)));
            self.carry(effects);
        }
        self.command(host, Command::new(id), now);
    }

    /// Text an input method composed, or a paste: typed into the open
    /// field one character at a time up to the first line break; anywhere
    /// else the app refuses it in words, since its letters would run as
    /// keys.
    pub fn text(&mut self, host: &mut impl Host, text: &str, now: Instant) {
        if !self.typing() {
            let effects = self.app.pasted(text);
            self.carry(effects);
            self.after(now);
            self.refresh(host);
            return;
        }
        for c in text.chars() {
            if c == '\r' || c == '\n' {
                break;
            }
            if c.is_control() {
                continue;
            }
            let effects = self
                .app
                .handle(AppEvent::Key(Key::new(KeyCode::Char(c), Modifiers::NONE)));
            self.carry(effects);
        }
        self.after(now);
        self.refresh(host);
    }

    /// Runs a command, as a menu or a button does.
    pub fn command(&mut self, host: &mut impl Host, command: Command, now: Instant) {
        let effects = self.app.handle(AppEvent::Command(command));
        self.carry(effects);
        self.after(now);
        self.refresh(host);
    }

    /// An action a widget submitted.
    pub fn action(&mut self, host: &mut impl Host, action: ErasedAction, now: Instant) {
        let action = match action.downcast::<RootAction>() {
            Ok(root) => {
                match *root {
                    RootAction::Key(event) => self.key_event(host, &event, now),
                    RootAction::Text(text) => self.text(host, &text, now),
                    RootAction::Moved => {
                        self.wake_bars(host, now);
                        self.after(now);
                    }
                    RootAction::AltAlone => {
                        if self.menu_bar && self.app.dialog().is_none() {
                            self.menu_requests.push(MenuEnter::Bar);
                        }
                    }
                }
                return;
            }
            Err(other) => other,
        };
        let action = match action.downcast::<PanelAction>() {
            Ok(panel) => {
                self.panel_action(host, *panel, now);
                return;
            }
            Err(other) => other,
        };
        let action = match action.downcast::<PushAction>() {
            Ok(push) => {
                self.push_action(host, *push, now);
                return;
            }
            Err(other) => other,
        };
        let action = match action.downcast::<ViewerAction>() {
            Ok(viewer) => {
                let ViewerAction::Size(columns, rows) = *viewer;
                if self.viewer_size != (columns, rows) {
                    self.viewer_size = (columns, rows);
                    self.refresh(host);
                }
                return;
            }
            Err(other) => other,
        };
        let action = match action.downcast::<PictureAction>() {
            Ok(picture) => {
                let PictureAction::Size(width, height) = *picture;
                if self.picture_size != (width, height) {
                    self.picture_size = (width, height);
                    self.picture_lines();
                    self.refresh(host);
                }
                return;
            }
            Err(other) => other,
        };
        let action = match action.downcast::<ChoiceAction>() {
            Ok(choice) => {
                self.choice_action(host, *choice, now);
                return;
            }
            Err(other) => other,
        };
        if let Ok(pressed) = action.downcast::<ButtonPressed>() {
            self.button(host, *pressed, now);
        }
    }

    /// A dialog's button was pressed (a click, a screen reader's Click),
    /// or took the focus another way: the window's focus follows it.
    fn push_action(&mut self, host: &mut impl Host, action: PushAction, now: Instant) {
        self.sync_local();
        if self.app.dialog().is_none() {
            return;
        }
        match action {
            PushAction::Focused(id) => {
                if self.local.on != id {
                    self.local.on = id;
                    self.refresh(host);
                }
            }
            PushAction::Pressed(id) => {
                self.local.on = id;
                self.press(id);
                self.after(now);
                self.refresh(host);
            }
        }
    }

    /// A dialog's list took the focus, or a row was clicked: the focus
    /// moves there (the app's list focus, or the window's own line), and
    /// a double click chooses the row as Enter does.
    fn choice_action(&mut self, host: &mut impl Host, action: ChoiceAction, now: Instant) {
        self.sync_local();
        let Some(view) = model::dialog_view_at(&self.app, &self.local) else {
            return;
        };
        let list_focus = match &view.control {
            model::ControlView::Confirm(_) => model::DialogFocus::Listing,
            _ => model::DialogFocus::Control,
        };
        match action {
            ChoiceAction::Focused => {
                if self.local.on != list_focus {
                    self.local.on = list_focus;
                    self.refresh(host);
                }
            }
            ChoiceAction::MessageFocused => {
                if self.local.on != model::DialogFocus::Message {
                    self.local.on = model::DialogFocus::Message;
                    self.refresh(host);
                }
            }
            ChoiceAction::MessageLine(index) => {
                self.local.on = model::DialogFocus::Message;
                self.local.line = index;
                self.refresh(host);
            }
            ChoiceAction::Row { index, open } => {
                self.local.on = list_focus;
                if view.local_list {
                    self.local.line = index;
                } else {
                    // The app's own row at once (lane A5), said by the
                    // focus that follows it, not by the app.
                    let _ = self.app.focus_dialog_row(index);
                }
                if open {
                    let effects = self.app.handle(AppEvent::Key(Key::plain(KeyCode::Enter)));
                    self.carry(effects);
                }
                self.after(now);
                self.refresh(host);
            }
        }
    }

    /// A button pressed: a key bar button runs its commands, in order; a
    /// panel's sort button makes that panel the active one and sorts it.
    fn button(&mut self, host: &mut impl Host, pressed: ButtonPressed, now: Instant) {
        if self.app.dialog().is_some() {
            return;
        }
        if pressed.row == model::KEY_BAR_NAME {
            let commands = pressed
                .id
                .parse::<usize>()
                .ok()
                .and_then(|i| self.app.shown_bank()?.buttons.into_iter().nth(i))
                .map(|button| button.commands)
                .unwrap_or_default();
            for command in commands {
                let effects = self.app.handle(AppEvent::Command(command));
                self.carry(effects);
            }
            self.after(now);
            self.refresh(host);
            return;
        }
        let Some(side) = Side::BOTH
            .into_iter()
            .find(|side| pressed.row == model::sort_row_name(*side))
        else {
            return;
        };
        if side != self.app.active_side() {
            let effects = self.app.dispatch(&Command::new(ids::SWITCH_PANEL));
            self.carry(effects);
        }
        if let Ok(id) = diroxide_core::CommandId::new(pressed.id) {
            self.command(host, Command::new(id), now);
        }
    }

    fn panel_action(&mut self, host: &mut impl Host, action: PanelAction, now: Instant) {
        match action {
            PanelAction::Focused(side) => {
                // A click or a screen reader put the focus on the other
                // panel: it becomes the active one, as Tab makes it (into
                // the preview while the quick view shows there, and out
                // of it again).
                if side != self.focus_side() && self.app.dialog().is_none() {
                    self.command(host, Command::new(ids::SWITCH_PANEL), now);
                }
            }
            PanelAction::Page(side, rows) => {
                // The app is told the first page too, even when it equals
                // the guess the window starts with (20 rows), or Page Up
                // and Page Down would move the app's own default.
                let told = self.app.panel(side).list().page_size() == rows;
                if self.page[side.index()] != rows || !told {
                    self.page[side.index()] = rows;
                    self.app.set_page_rows(side, rows);
                    self.refresh(host);
                }
            }
            PanelAction::Scrolled(side, top) => {
                self.top[side.index()] = top;
                self.refresh(host);
            }
            PanelAction::FocusRow(side, id) => {
                if self.app.dialog().is_some() {
                    return;
                }
                if side != self.focus_side() {
                    let effects = self.app.dispatch(&Command::new(ids::SWITCH_PANEL));
                    self.carry(effects);
                }
                // The folder's row, or a plugin list's while one is up; a
                // click in the preview goes into it, its rows moved by keys
                // (the app has no call to move the region by a row's id).
                if model::shows_preview(&self.app, side) {
                    // Into the preview by the switch above, then its row.
                    self.app.focus_preview_row(&id);
                } else {
                    self.app.focus_shown_row(side, &id);
                }
                self.after(now);
                self.refresh(host);
            }
            PanelAction::Open(side, id) => {
                if self.app.dialog().is_some() {
                    return;
                }
                self.app.focus_shown_row(side, &id);
                self.command(host, Command::new(ids::ENTER), now);
            }
        }
    }

    /// Takes the workers' waiting results to the app.
    pub fn wake(&mut self, host: &mut impl Host, now: Instant) {
        loop {
            let events = self.runtime.poll();
            if events.is_empty() {
                break;
            }
            for event in events {
                self.handle_event(event);
            }
        }
        self.poll_picture(now);
        self.after(now);
        self.refresh(host);
    }

    /// A result from the runtime, as if a worker had sent it: handed to
    /// the app, then the window refreshed (the tests send events the
    /// workers would).
    pub fn event(&mut self, host: &mut impl Host, event: AppEvent, now: Instant) {
        self.handle_event(event);
        self.after(now);
        self.refresh(host);
    }

    /// The job in front's percent, when one runs: none while it only
    /// waits its turn (held or queued), 0 before its first progress.
    fn front_percent(&self) -> Option<u64> {
        let jobs = self.app.jobs();
        let job = jobs.front()?;
        if jobs.waits(job) {
            return None;
        }
        Some(jobs.front_progress().map_or(0, |p| p.percent()))
    }

    /// Every value the progress indicator was given, in order: a percent,
    /// or none when no job ran.
    pub fn progress_shown(&self) -> &[Option<u64>] {
        &self.progress_shown
    }

    /// The percent of every progress report the workers sent, in order,
    /// against which [`Self::progress_shown`] shows how few of them moved
    /// the indicator.
    pub fn progress_reports(&self) -> &[u64] {
        &self.progress_reports
    }

    /// One result to the app; a job's progress is noted first, for the
    /// progress indicator.
    fn handle_event(&mut self, event: AppEvent) {
        // A program the window started says so in the app's own words
        // (the launcher does not wait, so the app hears ProgramStarted).
        if let AppEvent::JobProgress { progress, .. } = &event {
            self.progress_reports.push(progress.percent());
        }
        let effects = self.app.handle(event);
        self.carry(effects);
    }

    /// Waits for the workers to finish, at most `limit`, taking their
    /// results to the app; for tests and the scripted walk.
    pub fn settle(&mut self, host: &mut impl Host, now: Instant, limit: Duration) {
        let deadline = Instant::now() + limit;
        loop {
            self.wake(host, now);
            let busy = self.runtime.is_busy() || self.reading.is_some();
            if !busy || Instant::now() >= deadline {
                break;
            }
            if !self.runtime.is_busy() {
                // Only the picture view's read: its thread wakes no runtime.
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let events = self.runtime.wait(Duration::from_millis(10));
            for event in events {
                self.handle_event(event);
            }
        }
        self.wake(host, now);
    }

    /// The timers: the app's own, the quick view's, the viewer's, the key
    /// test's idle end, and the live region's look-again.
    pub fn tick(&mut self, host: &mut impl Host, now: Instant) {
        if let Some(since) = self.key_test_since
            && now.duration_since(since) >= KEY_TEST_IDLE
        {
            self.key_test_since = None;
            let effects = self.app.end_key_test(KEY_TEST_IDLE.as_secs());
            self.carry(effects);
        }
        match self.app.quick_view_wants_tick() {
            Some(ms) => {
                if self.quick_tick_at.is_none_or(|at| now >= at) {
                    let effects = self.app.quick_view_tick();
                    self.carry(effects);
                    self.quick_tick_at = Some(now + Duration::from_millis(ms));
                }
            }
            None => self.quick_tick_at = None,
        }
        if self.app_tick_at.is_some_and(|at| now >= at) {
            self.app_tick_at = None;
            let effects = self.app.app_tick();
            self.carry(effects);
        }
        match self.app.viewer_wants_tick() {
            Some(ms) => {
                if self.viewer_tick_at.is_none_or(|at| now >= at) {
                    let effects = self.app.viewer_tick();
                    self.carry(effects);
                    let next = self.app.viewer_wants_tick().unwrap_or(ms);
                    self.viewer_tick_at = Some(now + Duration::from_millis(next));
                }
            }
            None => self.viewer_tick_at = None,
        }
        if self.hold_retry_at.is_some_and(|at| now >= at) {
            self.hold_retry_at = None;
        }
        self.poll_picture(now);
        self.fade(host, now);
        self.after(now);
        self.refresh(host);
    }

    /// The next moment a timer needs the loop.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.key_test_since.map(|since| since + KEY_TEST_IDLE),
            self.app_tick_at,
            self.quick_tick_at,
            self.viewer_tick_at,
            self.hold_retry_at,
            (self.fades() && !self.veiled).then_some(self.rest_since + FADE_AFTER),
            self.reading
                .as_ref()
                .filter(|r| !r.said)
                .map(|r| r.since + OPENING_AFTER),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// After a step: the key test's clock, and the app's own timer.
    fn after(&mut self, now: Instant) {
        // Full screen is the app's (view.full-screen, lane A6): a change,
        // from F11, the View menu or the palette, is for the window to
        // carry out.
        let full_screen = self.app.full_screen();
        if full_screen != self.full_screen {
            self.full_screen = full_screen;
            self.full_screen_asked = Some(full_screen);
        }
        // Show picture is the app's too (view.picture, lane AE).
        self.take_picture_ask(now);
        match (self.app.key_test_active(), self.key_test_since) {
            (true, None) => self.key_test_since = Some(now),
            (false, Some(_)) => self.key_test_since = None,
            _ => {}
        }
        match self.app.app_wants_tick() {
            Some(0) => {
                self.app_tick_at = None;
                let effects = self.app.app_tick();
                self.carry(effects);
                if let Some(ms) = self.app.app_wants_tick() {
                    self.app_tick_at = Some(now + Duration::from_millis(ms.max(1)));
                }
            }
            // The app says how long is left on its own clock.
            Some(ms) => self.app_tick_at = Some(now + Duration::from_millis(ms)),
            None => self.app_tick_at = None,
        }
        if self.app.quick_view_wants_tick().is_some() && self.quick_tick_at.is_none() {
            self.quick_tick_at = Some(now);
        }
        if self.app.viewer_wants_tick().is_some() && self.viewer_tick_at.is_none() {
            self.viewer_tick_at = Some(now);
        }
    }

    /// Carries out effects: the runtime's I/O, then the screen's.
    fn carry(&mut self, effects: Vec<Effect>) {
        let screen = self.runtime.carry_out(&mut self.app, effects);
        // The app says a dialog's name as it opens it, in the effect after
        // the opening.
        let mut opened = false;
        for effect in screen {
            match effect {
                Effect::OpenDialog(_) => opened = true,
                Effect::Announce(message) => {
                    self.said.push(message.clone());
                    // A job's start line is the jobs line's too; a later
                    // line in the same step goes before it.
                    let start = message.text == self.app.jobs().line();
                    self.to_say.push(Said {
                        message,
                        start,
                        opening: std::mem::take(&mut opened),
                    });
                }
                Effect::Quit => self.quit = true,
                Effect::SetClipboardText(text) => {
                    self.clipboard.push(text.clone());
                    self.to_copy.push(text);
                }
                _ => {}
            }
        }
        // The status line is the app's; the lines the terminal joins are
        // taken so they do not pile up.
        let _ = self.app.take_status_lines();
        if self.app.wants_quit() {
            self.quit = true;
        }
    }

    /// The waiting lines as the one announcement the live region makes
    /// ([`model::join_messages`]), kept in [`Self::spoken`]. With a dialog
    /// shown for the first time (`fresh`), the line the app said as it
    /// opened it is left out when the dialog says the same by itself (its
    /// name, then its focused control, as the focus moves into it), so
    /// the screen reader hears it once: the settled double.
    fn take_to_say(&mut self, fresh: Option<&model::DialogView>) -> Vec<Message> {
        let lines: Vec<(Message, bool)> = std::mem::take(&mut self.to_say)
            .into_iter()
            .filter(|said| {
                !(said.opening && fresh.is_some_and(|view| view.says(&said.message.text)))
            })
            .map(|said| (said.message, said.start))
            .collect();
        let joined = model::join_messages(&lines);
        self.spoken.extend(joined.iter().cloned());
        joined
    }

    /// Puts the app's state on the widgets where it changed, then the
    /// keyboard focus where the app has it.
    pub fn refresh(&mut self, host: &mut impl Host) {
        self.sync_colors(host);
        let lines = model::lines(&self.app);
        if lines.title != self.shown.lines.title {
            let text = lines.title.clone();
            host.edit(TITLE_LINE, |mut l| Label::set_text(&mut l, text));
        }
        if lines.status != self.shown.lines.status {
            let text = lines.status.clone();
            host.edit(STATUS_LINE, |mut l| Label::set_text(&mut l, text));
        }
        if lines.jobs != self.shown.lines.jobs {
            let text = lines.jobs.clone();
            host.edit(JOBS_LINE, |mut l| Label::set_text(&mut l, text));
            // The progress indicator moves only with the jobs line, which
            // the app paces (a change of state at once, progress at most
            // every progress_interval_seconds in screen-reader mode).
            let percent = self.front_percent();
            if percent != self.shown.progress {
                self.shown.progress = percent;
                self.progress_shown.push(percent);
                host.edit(JOB_PROGRESS, |mut p| {
                    JobProgressBar::set_percent(&mut p, percent)
                });
            }
        }
        self.shown.lines = lines;
        // The window's node is named by its title ("Documents -
        // diroxide"); AT-SPI and macOS read the title from it.
        let label = self.window_title();
        if label != self.shown.label {
            host.set_window_label(&label);
            self.shown.label = label;
        }
        let arranged = (self.app.settings().layout, self.app.active_side());
        if self.shown.arranged != Some(arranged) {
            host.edit(PANELS, |mut p| {
                Panels::arrange(&mut p, arranged.0, arranged.1)
            });
            self.shown.arranged = Some(arranged);
        }
        for side in Side::BOTH {
            let view = self.view(side);
            if self.shown.panels[side.index()].as_ref() != Some(&view) {
                self.shown.panels[side.index()] = Some(view.clone());
                host.edit(panel_tag(side), |mut p| PanelBox::set_view(&mut p, view));
            }
        }
        // The viewer in the panels' place while a file is viewed: the rows
        // on screen for the size its list box told, drawn with the theme.
        let viewing = self.app.viewer().is_some();
        self.shown.viewing = viewing;
        // The picture view in the body's place while a picture is open; it
        // closes with its source (the viewer closed, or moved to another
        // file); the app says what shows a picture (lane AE).
        if let Some(open) = &self.picture
            && !self.still_shown(open.from, &open.place)
        {
            self.picture = None;
        }
        let body = if self.picture.is_some() {
            Body::Picture
        } else if viewing {
            Body::Viewer
        } else {
            Body::Panels
        };
        let picture = self.picture.as_ref().map(|open| open.view.clone());
        if picture != self.shown.picture {
            let lines = picture
                .as_ref()
                .map(|p| p.lines.clone())
                .unwrap_or_default();
            self.shown.picture = picture.clone();
            host.edit(PICTURE, |mut b| PictureBox::set_view(&mut b, picture));
            if lines != self.shown.picture_lines {
                self.shown.picture_lines = lines.clone();
                host.edit(PICTURE_LINES, |mut l| {
                    PictureLines::set_lines(&mut l, lines)
                });
            }
        }
        if body != self.shown.body {
            self.shown.body = body;
            host.edit(SCREEN, |mut s| Screen::show(&mut s, body));
        }
        if viewing {
            let (columns, rows) = self.viewer_size;
            let view = model::viewer_view(&mut self.app, columns, rows);
            if view.is_some() && view != self.shown.viewer {
                self.shown.viewer = view.clone();
                if let Some(view) = view {
                    host.edit(VIEWER, |mut v| ViewerBox::set_view(&mut v, view));
                }
            }
        } else {
            self.shown.viewer = None;
        }
        // The key bar (another bank, a rebound key) and the sort buttons'
        // keys.
        let keys = model::key_bar_buttons(&self.app);
        if keys != self.shown.key_bar {
            self.shown.key_bar = keys.clone();
            host.edit(KEY_BAR, |mut row| ButtonRow::set_buttons(&mut row, keys));
        }
        let sorting = model::sort_buttons(&self.app);
        if sorting != self.shown.sorting {
            self.shown.sorting = sorting.clone();
            for side in Side::BOTH {
                let buttons = sorting.clone();
                host.edit(sort_tag(side), |mut row| {
                    ButtonRow::set_buttons(&mut row, buttons)
                });
            }
        }
        // Copied text to the system clipboard; a clipboard another program
        // holds is said, after the app's own line.
        for text in std::mem::take(&mut self.to_copy) {
            let written = match self.clipboard_writer.as_mut() {
                Some(write) => write(&text),
                None => host.set_clipboard(text),
            };
            if let Err(cause) = written {
                let message = Message::could_not(
                    "copy",
                    "to the clipboard",
                    &cause,
                    "Try again in a moment.",
                );
                self.said.push(message.clone());
                self.to_say.push(Said {
                    message,
                    start: false,
                    opening: false,
                });
            }
        }
        // The modal host: the top dialog, changed in place while it is the
        // same dialog in the same shape, built again otherwise.
        self.sync_local();
        let dialog = model::dialog_view_at(&self.app, &self.local);
        if let Some(view) = &dialog {
            // The window's focus as the view settled it (a listing that
            // is not there falls back to No).
            self.local.on = view.focus;
        }
        // A dialog shown now that was not before says its name and its
        // focused control as the focus moves into it.
        let fresh: Option<model::DialogView> = dialog
            .as_ref()
            .filter(|view| {
                self.shown
                    .dialog
                    .as_ref()
                    .is_none_or(|shown| shown.id != view.id)
            })
            .cloned();
        let say = self.take_to_say(fresh.as_ref());
        if dialog != self.shown.dialog {
            self.shown.dialog = dialog.clone();
            host.edit(ROOT, |mut root| Root::set_dialog(&mut root, dialog));
        }
        // What the app said in this step is one announcement, its lines in
        // their order, as the terminal's status line joins them; a screen
        // reader is not handed a burst of live nodes in an order of the
        // adapter's choosing.
        let holding = host.edit(ANNOUNCER, |mut a| {
            if !say.is_empty() {
                Announcer::say(&mut a, say);
            }
            Announcer::release(&mut a);
            a.widget.holding()
        });
        if holding && self.hold_retry_at.is_none() {
            self.hold_retry_at = Some(Instant::now() + HOLD_RETRY);
        }
        self.place_focus(host);
    }

    /// The widget the keyboard focus belongs on: the open dialog's field
    /// or list, or the active panel's list box, whose active descendant
    /// is the focused row, so closing a dialog returns there.
    fn focus_target(&self, host: &mut impl Host) -> Option<WidgetId> {
        if self.shown.dialog.is_some()
            && let Some(id) = host.edit(ROOT, |mut root| Root::dialog_focus(&mut root))
        {
            return Some(id);
        }
        match self.shown.body {
            Body::Picture => return host.id_of(PICTURE),
            Body::Viewer => return host.id_of(VIEWER),
            Body::Panels => {}
        }
        host.id_of(panel_tag(self.focus_side()))
    }

    /// The side whose list box has the focus: the active panel, or the
    /// other one while the focus is in the quick view's preview there.
    fn focus_side(&self) -> Side {
        if self.app.in_preview() {
            self.app.active_side().other()
        } else {
            self.app.active_side()
        }
    }

    fn place_focus(&mut self, host: &mut impl Host) {
        let Some(target) = self.focus_target(host) else {
            return;
        };
        if host.focused() != Some(target) {
            let _ = host.focus(Some(target));
        }
    }
}
