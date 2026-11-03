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

use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::keymap::Platform;
use diroxide_app::{App, AppEvent, Dialog, Runtime};
use diroxide_core::{Command, Effect, Key, KeyCode, Message, Modifiers, Side};
use masonry::accesskit::Role;
use masonry::app::RenderRoot;
use masonry::core::keyboard::KeyboardEvent;
use masonry::core::{
    ErasedAction, FromDynWidget, NewWidget, Widget, WidgetId, WidgetMut, WidgetTag,
};
use masonry::widgets::{Flex, Label};

use crate::model::{self, Lines, PanelView};
use crate::widgets::announcer::Announcer;
use crate::widgets::buttons::{ButtonPressed, ButtonRow};
use crate::widgets::choice::ChoiceAction;
use crate::widgets::panel::{PanelAction, PanelBox};
use crate::widgets::panels::Panels;
use crate::widgets::push::PushAction;
use crate::widgets::root::{Region, Root, RootAction};

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
}

/// What the widgets show now, so a refresh edits only what changed.
#[derive(Debug, Default)]
struct Shown {
    lines: Lines,
    panels: [Option<PanelView>; 2],
    arranged: Option<(diroxide_app::config::Layout, Side)>,
    dialog: Option<model::DialogView>,
    key_bar: Vec<model::ButtonView>,
    sorting: Vec<model::ButtonView>,
    /// The window's name as its accessibility node has it.
    label: String,
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
    /// Text the app put on the clipboard (the platform clipboard is G2's
    /// commit 25; until then the texts are kept here).
    clipboard: Vec<String>,
    quit: bool,
    key_test_since: Option<Instant>,
    app_tick_at: Option<Instant>,
    quick_tick_at: Option<Instant>,
    viewer_tick_at: Option<Instant>,
    hold_retry_at: Option<Instant>,
    /// The window has a native menu bar (Windows, the `menus` setting's
    /// native): the menu keys enter it.
    menu_bar: bool,
    /// What the menu keys asked of the native bar, for the window.
    menu_requests: Vec<MenuEnter>,
}

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
        Self {
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
            quit: false,
            key_test_since: None,
            app_tick_at: None,
            quick_tick_at: None,
            viewer_tick_at: None,
            hold_retry_at: None,
            menu_bar: false,
            menu_requests: Vec::new(),
        }
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
        let status = Flex::column()
            .with_fixed(NewWidget::new(Label::new(lines.status.clone())).with_tag(STATUS_LINE))
            .with_fixed(NewWidget::new(Label::new(lines.jobs.clone())).with_tag(JOBS_LINE));
        let status_bar = Region::new(NewWidget::new(status), Role::Status, "");
        let keys = model::key_bar_buttons(&self.app);
        self.shown.key_bar = keys.clone();
        let key_bar = NewWidget::new(ButtonRow::new(model::KEY_BAR_NAME, keys)).with_tag(KEY_BAR);
        let column = Flex::column()
            .with_fixed(NewWidget::new(Label::new(lines.title.clone())).with_tag(TITLE_LINE))
            .with(panels, 1.0)
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
        NewWidget::new(Root::new(main, announcer, full_passes)).with_tag(ROOT)
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
        if let Some(enter) = self.menu_key(&key) {
            // The native menu bar takes the key: the window shows the bar
            // (when it is hidden) and enters it, as Windows' own menu keys
            // would.
            self.menu_requests.push(enter);
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
            model::DialogFocus::Listing | model::DialogFocus::Control if view.local_list => {
                let total = match &view.control {
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
            model::DialogFocus::Control | model::DialogFocus::Listing => Vec::new(),
        };
        self.carry(effects);
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
            ChoiceAction::Row { index, open } => {
                self.local.on = list_focus;
                if view.local_list {
                    self.local.line = index;
                } else {
                    self.move_app_list(index);
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

    /// Moves the app's focus in the open dialog's list to the row at
    /// `index`, through the list's own moves (the app has no command to
    /// focus a row by its place; a contract request asks for one): into
    /// a field's candidates first, then Home or End, whichever is nearer,
    /// and Down or Up the rest of the way. The moves are Low lines,
    /// shown only.
    fn move_app_list(&mut self, index: usize) {
        let focus_and_total = |app: &App| -> Option<(Option<usize>, usize, bool)> {
            match app.dialog()?.1 {
                Dialog::Choose { list, .. } => Some((list.focus_index(), list.len(), true)),
                Dialog::Question { choices, .. } => {
                    Some((choices.focus_index(), choices.len(), true))
                }
                Dialog::Edit {
                    candidates: Some(list),
                    in_list,
                    ..
                } => Some((list.focus_index(), list.len(), *in_list)),
                _ => None,
            }
        };
        let Some((_, total, in_list)) = focus_and_total(&self.app) else {
            return;
        };
        if total == 0 {
            return;
        }
        let index = index.min(total - 1);
        if !in_list {
            // Down from a field enters its candidates at their focus.
            let effects = self.app.handle(AppEvent::Key(Key::plain(KeyCode::Down)));
            self.carry(effects);
        }
        let Some((Some(at), _, _)) = focus_and_total(&self.app) else {
            return;
        };
        if at == index {
            return;
        }
        let (start, from) = if index < at.abs_diff(index) {
            (Some(ids::LIST_HOME), 0)
        } else if total - 1 - index < at.abs_diff(index) {
            (Some(ids::LIST_END), total - 1)
        } else {
            (None, at)
        };
        if let Some(start) = start {
            let effects = self.app.handle(AppEvent::Command(Command::new(start)));
            self.carry(effects);
        }
        let step = if index > from {
            ids::LIST_DOWN
        } else {
            ids::LIST_UP
        };
        for _ in 0..from.abs_diff(index) {
            let effects = self
                .app
                .handle(AppEvent::Command(Command::new(step.clone())));
            self.carry(effects);
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
                .and_then(|i| model::shown_bank(&self.app)?.buttons.into_iter().nth(i))
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
                // panel: it becomes the active one, as Tab makes it.
                if side != self.app.active_side() && self.app.dialog().is_none() {
                    self.command(host, Command::new(ids::SWITCH_PANEL), now);
                }
            }
            PanelAction::Page(side, rows) => {
                if self.page[side.index()] != rows {
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
                if side != self.app.active_side() {
                    let effects = self.app.dispatch(&Command::new(ids::SWITCH_PANEL));
                    self.carry(effects);
                }
                // A plugin list's rows are moved by keys only for now
                // (`App::focus_name` moves the folder's focus).
                if self.app.panel(side).plugin_list().is_none() {
                    self.app.focus_name(side, &id);
                }
                self.after(now);
                self.refresh(host);
            }
            PanelAction::Open(side, id) => {
                if self.app.dialog().is_some() {
                    return;
                }
                if self.app.panel(side).plugin_list().is_none() {
                    self.app.focus_name(side, &id);
                }
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
                let effects = self.app.handle(event);
                self.carry(effects);
            }
        }
        self.after(now);
        self.refresh(host);
    }

    /// Waits for the workers to finish, at most `limit`, taking their
    /// results to the app; for tests and the scripted walk.
    pub fn settle(&mut self, host: &mut impl Host, now: Instant, limit: Duration) {
        let deadline = Instant::now() + limit;
        loop {
            self.wake(host, now);
            if !self.runtime.is_busy() || Instant::now() >= deadline {
                break;
            }
            let events = self.runtime.wait(Duration::from_millis(10));
            for event in events {
                let effects = self.app.handle(event);
                self.carry(effects);
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
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// After a step: the key test's clock, and the app's own timer.
    fn after(&mut self, now: Instant) {
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
                Effect::SetClipboardText(text) => self.clipboard.push(text),
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
        host.id_of(panel_tag(self.app.active_side()))
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
