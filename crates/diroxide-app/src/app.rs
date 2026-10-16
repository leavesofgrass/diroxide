//! The app: two panels, one active; keys become commands through the
//! keymap; commands change state and return effects.
//!
//! Wave 1 (W1d) owns this module. Phase 0 handles moving, switching,
//! entering, going up, reloading, selecting, hidden files, help, Say Status
//! and Quit; every other command says it is not built yet, and nothing in
//! Phase 0 touches a file.

use std::sync::Arc;

use diroxide_config::Settings;
use diroxide_core::format::count;
use diroxide_core::{
    Clock, Command, CommandId, DialogId, Effect, Entry, EntryKind, IdSource, JobId, Key, Layer,
    Message, Move, Place, Redraw, RequestId, Side, SystemClock,
};
use diroxide_filetype::TypeRegistry;
use diroxide_ops::{JobProgress, JobReport, UndoLog};
use diroxide_vfs::VfsError;

use crate::commands::{self, ids};
use crate::dialog::{Answer, Dialog};
use crate::jobs::JobTable;
use crate::keymap::{Keymap, Lookup};
use crate::panel::Panel;

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
}

impl AppOptions {
    /// Two places with default settings, keys and types, and the system
    /// clock in UTC (the app supplies the local offset once it reads it).
    pub fn new(left: Place, right: Place) -> Self {
        Self {
            left,
            right,
            settings: Settings::default(),
            keymap: Keymap::defaults(),
            types: TypeRegistry::builtin(),
            clock: Arc::new(SystemClock::utc()),
        }
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
    /// A job made progress.
    JobProgress {
        /// The job.
        job: JobId,
        /// How far it got.
        progress: JobProgress,
    },
    /// A job finished.
    JobFinished(JobReport),
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

#[derive(Clone, Copy, Debug)]
enum Action {
    Focus(Move),
    SwitchPanel,
    Enter,
    Up,
    Reload,
    ToggleSelect,
    SelectAll,
    SelectNone,
    SelectInvert,
    ToggleHidden,
    Help,
    SayStatus,
    Quit,
}

const ROUTES: [(CommandId, Action); 18] = [
    (ids::LIST_UP, Action::Focus(Move::Up)),
    (ids::LIST_DOWN, Action::Focus(Move::Down)),
    (ids::LIST_PAGE_UP, Action::Focus(Move::PageUp)),
    (ids::LIST_PAGE_DOWN, Action::Focus(Move::PageDown)),
    (ids::LIST_HOME, Action::Focus(Move::Home)),
    (ids::LIST_END, Action::Focus(Move::End)),
    (ids::SWITCH_PANEL, Action::SwitchPanel),
    (ids::ENTER, Action::Enter),
    (ids::UP, Action::Up),
    (ids::RELOAD, Action::Reload),
    (ids::SELECT_TOGGLE, Action::ToggleSelect),
    (ids::SELECT_ALL, Action::SelectAll),
    (ids::SELECT_NONE, Action::SelectNone),
    (ids::SELECT_INVERT, Action::SelectInvert),
    (ids::TOGGLE_HIDDEN, Action::ToggleHidden),
    (ids::HELP, Action::Help),
    (ids::SAY_STATUS, Action::SayStatus),
    (ids::QUIT, Action::Quit),
];

/// The app core.
#[derive(Debug)]
pub struct App {
    panels: [Panel; 2],
    active: Side,
    keymap: Keymap,
    settings: Settings,
    types: TypeRegistry,
    clock: Arc<dyn Clock>,
    ids: IdSource,
    dialogs: Vec<(DialogId, Dialog)>,
    jobs: JobTable,
    undo: UndoLog,
    status: String,
    quit: bool,
}

impl App {
    /// An app with two panels, the left one active. Call [`App::start`]
    /// for the first listings.
    pub fn new(options: AppOptions) -> Self {
        let keymap = if options.settings.single_letter_keys {
            options.keymap
        } else {
            options.keymap.without_single_characters()
        };
        Self {
            panels: [
                Panel::new(Side::Left, options.left, &options.settings),
                Panel::new(Side::Right, options.right, &options.settings),
            ],
            active: Side::Left,
            keymap,
            settings: options.settings,
            types: options.types,
            clock: options.clock,
            ids: IdSource::new(),
            dialogs: Vec::new(),
            jobs: JobTable::new(),
            undo: UndoLog::default(),
            status: String::new(),
            quit: false,
        }
    }

    /// The first effects: list both panels.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for side in Side::BOTH {
            let place = self.panel(side).place().clone();
            effects.extend(self.open(side, place, None));
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

    /// Whether the app asked to quit.
    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    /// The title line, the active place first: "photos, in D:\, left
    /// panel".
    pub fn title_line(&self) -> String {
        let panel = self.active_panel();
        let loading = if panel.is_loading() { ", loading" } else { "" };
        format!(
            "{}, {} panel{loading}",
            panel.list().title(),
            self.active.word()
        )
    }

    /// The status line, the key fact first: the last message, or the
    /// active panel's status.
    pub fn status_line(&self) -> String {
        if self.status.is_empty() {
            self.active_panel().list().status()
        } else {
            self.status.clone()
        }
    }

    /// Handles an event and returns what to do about it.
    pub fn handle(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::Key(key) => self.press(key),
            AppEvent::Command(command) => self.dispatch(&command),
            AppEvent::Listed {
                request,
                place,
                result,
            } => self.listed(request, &place, result),
            AppEvent::JobProgress { job, progress } => {
                let text = progress.describe();
                self.jobs.update(job, text.clone());
                vec![
                    Effect::Redraw(Redraw::JobsLine),
                    Effect::Announce(Message::progress(text)),
                ]
            }
            AppEvent::JobFinished(report) => {
                self.jobs.finish(report.job);
                self.undo.record(report.undo.clone());
                let mut effects = vec![
                    Effect::Redraw(Redraw::JobsLine),
                    self.say(Message::done(report.summary())),
                ];
                for side in Side::BOTH {
                    let place = self.panel(side).place().clone();
                    let focus = self.panel(side).list().focused().map(|r| r.id.clone());
                    effects.extend(self.open(side, place, focus));
                }
                effects
            }
            AppEvent::Answer { dialog, .. } => {
                self.dialogs.retain(|(id, _)| *id != dialog);
                vec![Effect::CloseDialog(dialog), Effect::Redraw(Redraw::All)]
            }
            AppEvent::Problem(message) => vec![self.say(message)],
        }
    }

    /// Runs a command and returns what to do about it.
    pub fn dispatch(&mut self, command: &Command) -> Vec<Effect> {
        let Some(action) = ROUTES
            .iter()
            .find(|(id, _)| *id == command.id)
            .map(|(_, action)| *action)
        else {
            return self.not_built(&command.id);
        };
        let side = self.active;
        match action {
            Action::Focus(movement) => self.move_focus(movement),
            Action::SwitchPanel => {
                self.active = side.other();
                let panel = self.active_panel();
                let text = format!(
                    "{} panel: {}",
                    capitalized(self.active.word()),
                    panel.list().status()
                );
                vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
            }
            Action::Enter => self.enter(),
            Action::Up => {
                let panel = self.panel(side);
                match panel.place().parent() {
                    Some(parent) => {
                        let focus = panel.place().name().map(str::to_owned);
                        self.open(side, parent, focus)
                    }
                    None => vec![self.say(Message::status("At the top already"))],
                }
            }
            Action::Reload => {
                let panel = self.panel(side);
                let place = panel.place().clone();
                let focus = panel.list().focused().map(|r| r.id.clone());
                self.open(side, place, focus)
            }
            Action::ToggleSelect => {
                let list = self.panel_mut(side).list_mut();
                let Some(selected) = list.toggle_selected() else {
                    return Vec::new();
                };
                let label = list.focused().map(|r| r.label.clone()).unwrap_or_default();
                list.move_focus(Move::Down);
                let state = if selected { "selected" } else { "not selected" };
                vec![
                    Effect::Redraw(Redraw::Panel(side)),
                    self.say(Message::info(format!("{label} {state}"))),
                ]
            }
            Action::SelectAll | Action::SelectNone | Action::SelectInvert => {
                let list = self.panel_mut(side).list_mut();
                match action {
                    Action::SelectAll => list.select_all(),
                    Action::SelectNone => list.select_none(),
                    _ => list.invert_selection(),
                }
                let selected = list.selected_count() as u64;
                let text = if selected == 0 {
                    "Nothing selected".to_owned()
                } else {
                    format!("{} selected", count(selected, "item", "items"))
                };
                vec![
                    Effect::Redraw(Redraw::Panel(side)),
                    self.say(Message::info(text)),
                ]
            }
            Action::ToggleHidden => {
                let panel = &mut self.panels[side.index()];
                let show = !panel.shows_hidden();
                panel.set_show_hidden(show, self.clock.as_ref());
                let text = if show {
                    "Hidden files shown"
                } else {
                    "Hidden files not shown"
                };
                vec![
                    Effect::Redraw(Redraw::Panel(side)),
                    self.say(Message::info(text)),
                ]
            }
            Action::Help => {
                let help = self.active_panel().list().help().to_owned();
                vec![self.say(Message::info(help))]
            }
            Action::SayStatus => {
                let mut text = self.active_panel().list().status();
                if !self.jobs.is_empty() {
                    text.push_str(". ");
                    text.push_str(&self.jobs.line());
                }
                vec![self.say(Message::info(text))]
            }
            Action::Quit => {
                self.quit = true;
                vec![Effect::Quit]
            }
        }
    }

    fn not_built(&mut self, id: &CommandId) -> Vec<Effect> {
        let message = match commands::info(id) {
            Some(info) => {
                let targets = self.active_panel().targets().len() as u64;
                Message::info(format!(
                    "{} is not built yet; {} would be acted on.",
                    info.name,
                    count(targets, "item", "items")
                ))
            }
            None => Message::could_not("run", id.as_str(), "no command has that id", ""),
        };
        vec![self.say(message)]
    }

    fn say(&mut self, message: Message) -> Effect {
        self.status = message.text.clone();
        Effect::Announce(message)
    }

    fn layers(&self) -> [Layer; 3] {
        [Layer::Panel, Layer::List, Layer::Global]
    }

    fn press(&mut self, key: Key) -> Vec<Effect> {
        let top = self.dialogs.last().map(|(id, _)| *id);
        if let Some(id) = top
            && key == Key::plain(diroxide_core::KeyCode::Escape)
        {
            self.dialogs.pop();
            return vec![Effect::CloseDialog(id), Effect::Redraw(Redraw::All)];
        }
        match self.keymap.lookup(&self.layers(), &key) {
            Lookup::Command(id) => {
                self.status.clear();
                self.dispatch(&Command::new(id))
            }
            Lookup::NoMatch => match key.typed_char() {
                Some(letter) => {
                    let side = self.active;
                    if self.panel_mut(side).list_mut().letter_jump(letter) {
                        self.status.clear();
                        self.focus_effects(side)
                    } else {
                        vec![self.say(Message::status(format!("No name starts with {letter}")))]
                    }
                }
                None => Vec::new(),
            },
        }
    }

    fn move_focus(&mut self, movement: Move) -> Vec<Effect> {
        let side = self.active;
        if self.panel_mut(side).list_mut().move_focus(movement) {
            self.status.clear();
            self.focus_effects(side)
        } else if self.panel(side).list().is_empty() {
            let empty = self.panel(side).list().empty_text().to_owned();
            vec![self.say(Message::status(empty))]
        } else {
            let edge = if matches!(movement, Move::Up | Move::PageUp | Move::Home) {
                "Top of the list"
            } else {
                "Bottom of the list"
            };
            vec![self.say(Message::status(edge))]
        }
    }

    fn focus_effects(&self, side: Side) -> Vec<Effect> {
        vec![
            Effect::Redraw(Redraw::Panel(side)),
            Effect::Announce(Message::status(self.panel(side).list().announce_focus())),
        ]
    }

    fn enter(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let (kind, container) = (entry.kind, entry.is_container());
        if entry.name_not_unicode {
            let message =
                Message::from_error("open", &entry.display_name(), &VfsError::NameNotUnicode);
            return vec![self.say(message)];
        }
        match kind {
            EntryKind::Archive => self.open(side, place.into_archive(), None),
            _ if container => self.open(side, place, None),
            _ => vec![Effect::OpenExternally(place)],
        }
    }

    fn open(&mut self, side: Side, place: Place, focus: Option<String>) -> Vec<Effect> {
        let request = self.ids.request();
        self.panel_mut(side)
            .begin_load(request, place.clone(), focus);
        vec![
            Effect::Redraw(Redraw::TitleLine),
            Effect::List { request, place },
        ]
    }

    fn listed(
        &mut self,
        request: RequestId,
        place: &Place,
        result: Result<Vec<Entry>, VfsError>,
    ) -> Vec<Effect> {
        let Some(side) = Side::BOTH
            .into_iter()
            .find(|s| self.panel(*s).waits_for(request))
        else {
            return Vec::new();
        };
        match result {
            Ok(entries) => {
                self.panels[side.index()].finish_load(entries, &self.types, self.clock.as_ref());
                let mut effects = vec![Effect::Redraw(Redraw::Panel(side))];
                if side == self.active {
                    let list = self.panel(side).list();
                    let text = format!(
                        "{}, {}",
                        list.title(),
                        count(list.len() as u64, "item", "items")
                    );
                    effects.push(Effect::Redraw(Redraw::TitleLine));
                    effects.push(self.say(Message::info(text)));
                }
                effects
            }
            Err(error) => {
                self.panel_mut(side).cancel_load();
                vec![
                    Effect::Redraw(Redraw::TitleLine),
                    self.say(Message::from_error("open", &place.to_string(), &error)),
                ]
            }
        }
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
mod tests {
    use super::*;
    use crate::runtime::Runtime;
    use diroxide_core::{FixedClock, Timestamp, UtcOffset};

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    fn here() -> Place {
        Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap()
    }

    /// Runs effects through a runtime until nothing is left, returning the
    /// announcements in order.
    fn settle(app: &mut App, runtime: &mut Runtime, effects: Vec<Effect>) -> Vec<String> {
        let mut said = Vec::new();
        let mut pending = effects;
        while !pending.is_empty() {
            for effect in &pending {
                if let Effect::Announce(message) = effect {
                    said.push(message.text.clone());
                }
                runtime.execute(effect);
            }
            pending = runtime
                .poll()
                .into_iter()
                .flat_map(|event| app.handle(event))
                .collect();
        }
        said
    }

    fn app() -> (App, Runtime) {
        let mut options = AppOptions::new(here(), here().join("src").unwrap());
        options.clock = Arc::new(FixedClock::new(
            Timestamp::from_civil(2026, 10, 10, 15, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        ));
        (App::new(options), Runtime::with_local())
    }

    #[test]
    fn starting_lists_both_panels() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        let said = settle(&mut app, &mut runtime, start);
        assert!(said[0].starts_with("diroxide-app, in "), "{said:?}");
        assert!(app.panel(Side::Left).list().len() >= 3);
        assert!(
            app.panel(Side::Right)
                .list()
                .rows()
                .iter()
                .any(|r| r.id == "app.rs")
        );
        assert!(app.title_line().starts_with("diroxide-app, in "));
        assert!(app.title_line().ends_with(", left panel"));
    }

    #[test]
    fn keys_move_switch_and_enter() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        settle(&mut app, &mut runtime, start);
        let effects = app.handle(AppEvent::Key(key("tab")));
        let said = settle(&mut app, &mut runtime, effects);
        assert_eq!(app.active_side(), Side::Right);
        assert!(said[0].starts_with("Right panel: src, in "), "{said:?}");
        let effects = app.handle(AppEvent::Key(key("backspace")));
        let said = settle(&mut app, &mut runtime, effects);
        assert!(said[0].starts_with("diroxide-app, in "), "{said:?}");
        assert_eq!(app.active_panel().list().focused().unwrap().id, "src");
        let effects = app.handle(AppEvent::Key(key("enter")));
        settle(&mut app, &mut runtime, effects);
        assert_eq!(app.active_panel().place().name(), Some("src"));
        assert_eq!(app.active_panel().history().len(), 2);
    }

    #[test]
    fn letters_jump_and_space_selects() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        settle(&mut app, &mut runtime, start);
        let effects = app.handle(AppEvent::Key(key("c")));
        settle(&mut app, &mut runtime, effects);
        assert_eq!(
            app.active_panel().list().focused().unwrap().id,
            "Cargo.toml"
        );
        let effects = app.handle(AppEvent::Key(key("space")));
        let said = settle(&mut app, &mut runtime, effects);
        assert_eq!(said, vec!["Cargo.toml selected".to_owned()]);
        assert_eq!(app.active_panel().targets().len(), 1);
        let effects = app.handle(AppEvent::Key(key("f5")));
        let said = settle(&mut app, &mut runtime, effects);
        assert_eq!(
            said,
            vec!["Copy to other panel is not built yet; 1 item would be acted on.".to_owned()]
        );
        assert!(app.status_line().starts_with("Copy to other panel"));
    }

    #[test]
    fn a_failed_listing_keeps_the_panel_where_it_was() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        settle(&mut app, &mut runtime, start);
        let missing = here().join("no-such-folder").unwrap();
        let effects = app.open(Side::Left, missing, None);
        let said = settle(&mut app, &mut runtime, effects);
        assert!(said[0].starts_with("Could not open "), "{said:?}");
        assert!(
            said[0].ends_with("it does not exist. Press F5 to reload the folder."),
            "{said:?}"
        );
        assert_eq!(app.panel(Side::Left).place(), &here());
    }

    #[test]
    fn quit_asks_the_frontend_to_leave() {
        let (mut app, _) = app();
        assert_eq!(app.handle(AppEvent::Key(key("ctrl+q"))), vec![Effect::Quit]);
        assert!(app.wants_quit());
    }
}
