//! The quick view wired into the app (plan commit 64): Shift+F3 turns the
//! other panel into W2e's live preview of the file under the active
//! panel's cursor ([`QuickView`]); Tab enters it as a region with its own
//! keys (the preview layer of the keymap: headings, paragraphs, tables
//! and cells, links, find, the previous and next file), and Tab or Escape
//! comes back to the panel's row. The View menu and the palette reach
//! everything, for a terminal that keeps Shift+F3 for itself.
//!
//! What is said, one line a change (research/alpha2-plan.md part 3.6):
//! "Quick view on" and "Quick view off"; when the cursor stops, the
//! preview's one line ("Preview: report.md, 12 headings"), never the text;
//! entering, the region's title and the row under its cursor; inside,
//! what each move found ("Heading 2: Part 1, 2 of 12"). A move by line
//! says nothing of its own: the row under the cursor is its words, as in
//! a panel.
//!
//! The quick view reads on its own worker thread in real time, so its
//! timer runs on [`Instant::now`]: the frontend calls
//! [`App::quick_view_tick`] while [`App::quick_view_wants_tick`] says so.

use std::time::Instant;

use diroxide_core::{Command, CommandId, Effect, Importance, Key, Layer, Message, Move, Redraw};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::{Registry, WatchEvent};

use super::{Action, App, Purpose};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::Lookup;
use crate::quickview::{Outcome, QuickView, QuickViewOptions, QuickViewRequest};
use crate::request::{Request, SpawnKind};

/// A command of the preview layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewCommand {
    /// Leave the preview
    Leave,
    /// Next heading
    NextHeading,
    /// Previous heading
    PreviousHeading,
    /// Next paragraph
    NextParagraph,
    /// Previous paragraph
    PreviousParagraph,
    /// Next table
    NextTable,
    /// Previous table
    PreviousTable,
    /// Cell to the left
    CellLeft,
    /// Cell to the right
    CellRight,
    /// Cell above
    CellUp,
    /// Cell below
    CellDown,
    /// Links in the preview
    Links,
    /// Next link
    NextLink,
    /// Previous link
    PreviousLink,
    /// Open link
    OpenLink,
    /// Find in the preview
    Find,
    /// Next match in the preview
    NextMatch,
    /// Previous match in the preview
    PreviousMatch,
    /// Next file in the preview
    NextFile,
    /// Previous file in the preview
    PreviousFile,
}

const PREVIEW_ROUTES: [(CommandId, PreviewCommand); 20] = [
    (ids::PREVIEW_LEAVE, PreviewCommand::Leave),
    (ids::PREVIEW_NEXT_HEADING, PreviewCommand::NextHeading),
    (
        ids::PREVIEW_PREVIOUS_HEADING,
        PreviewCommand::PreviousHeading,
    ),
    (ids::PREVIEW_NEXT_PARAGRAPH, PreviewCommand::NextParagraph),
    (
        ids::PREVIEW_PREVIOUS_PARAGRAPH,
        PreviewCommand::PreviousParagraph,
    ),
    (ids::PREVIEW_NEXT_TABLE, PreviewCommand::NextTable),
    (ids::PREVIEW_PREVIOUS_TABLE, PreviewCommand::PreviousTable),
    (ids::PREVIEW_CELL_LEFT, PreviewCommand::CellLeft),
    (ids::PREVIEW_CELL_RIGHT, PreviewCommand::CellRight),
    (ids::PREVIEW_CELL_UP, PreviewCommand::CellUp),
    (ids::PREVIEW_CELL_DOWN, PreviewCommand::CellDown),
    (ids::PREVIEW_LINKS, PreviewCommand::Links),
    (ids::PREVIEW_NEXT_LINK, PreviewCommand::NextLink),
    (ids::PREVIEW_PREVIOUS_LINK, PreviewCommand::PreviousLink),
    (ids::PREVIEW_OPEN_LINK, PreviewCommand::OpenLink),
    (ids::PREVIEW_FIND, PreviewCommand::Find),
    (ids::PREVIEW_NEXT_MATCH, PreviewCommand::NextMatch),
    (ids::PREVIEW_PREVIOUS_MATCH, PreviewCommand::PreviousMatch),
    (ids::PREVIEW_NEXT_FILE, PreviewCommand::NextFile),
    (ids::PREVIEW_PREVIOUS_FILE, PreviewCommand::PreviousFile),
];

/// The preview command a command id runs, if it is one.
pub(super) fn preview_route(id: &CommandId) -> Option<PreviewCommand> {
    PREVIEW_ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, command)| *command)
}

/// The find prompt's history.
const FIND_HISTORY: &str = "preview-find";

/// How often the quick view is given time while it waits for the cursor
/// to stop, reads a file or colors code, in milliseconds.
pub const QUICK_VIEW_TICK_MS: u64 = 20;

/// The quick view's state in the app.
#[derive(Debug, Default)]
pub(crate) struct Preview {
    /// The quick view, made the first time it is turned on.
    pub view: Option<QuickView>,
    /// The registry it reads through: the runtime's, given by the
    /// frontend; the local provider alone without one.
    pub registry: Option<Registry>,
    /// The tools, for "Open in textweaver".
    pub tools: Option<ToolRegistry>,
    /// Whether Tab put the focus in the preview region.
    pub focused: bool,
}

impl App {
    /// Gives the quick view the registry it reads through: the runtime's,
    /// so a file inside an archive previews as any other. The frontend
    /// calls it once, at the start.
    pub fn set_quick_view_registry(&mut self, registry: Registry) {
        self.preview.registry = Some(registry);
    }

    /// Gives "Open in textweaver" the tools, with the user's `tools.toml`;
    /// the built-in entries on the path without it.
    pub fn set_tools(&mut self, tools: ToolRegistry) {
        self.preview.tools = Some(tools);
    }

    /// The quick view while it is on, for the frontend to draw its
    /// region: [`QuickView::region`] with the hardware cursor on the
    /// focused row's first cell when [`App::in_preview`].
    pub fn quick_view(&self) -> Option<&QuickView> {
        self.preview.view.as_ref().filter(|v| v.is_on())
    }

    /// Whether the quick view is on: the other panel shows the preview.
    pub fn quick_view_on(&self) -> bool {
        self.quick_view().is_some()
    }

    /// Whether the focus is in the preview region (Tab entered it).
    pub fn in_preview(&self) -> bool {
        self.preview.focused && self.quick_view_on()
    }

    /// The columns the preview region is drawn in, from the frontend:
    /// rows wrap there when the viewer's `wrap` setting is on.
    pub fn set_preview_width(&mut self, columns: usize) {
        if let Some(view) = self.preview.view.as_mut() {
            view.set_width(columns);
        }
    }

    /// How soon the frontend should call [`App::quick_view_tick`], in
    /// milliseconds: while the quick view waits for the cursor to stop,
    /// reads a file or colors code; never otherwise.
    pub fn quick_view_wants_tick(&self) -> Option<u64> {
        let view = self.quick_view()?;
        view.wants_tick().then_some(QUICK_VIEW_TICK_MS)
    }

    /// Gives the quick view its time: a render started once the cursor
    /// stopped, a finished one taken, colors put on code. Its one line is
    /// said; the screen is drawn again when what it shows changed.
    pub fn quick_view_tick(&mut self) -> Vec<Effect> {
        let Some(view) = self.preview.view.as_mut().filter(|v| v.is_on()) else {
            return Vec::new();
        };
        let before = signature(view);
        let said = view.tick(Instant::now());
        let changed = signature(view) != before;
        let mut effects = Vec::new();
        if changed || !said.is_empty() {
            effects.push(Effect::Redraw(Redraw::All));
        }
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// Shift+F3: the quick view on or off. On, it shows the file under
    /// the cursor at once: "Quick view on", then the preview's line.
    pub(super) fn toggle_quick_view(&mut self) -> Vec<Effect> {
        if self.quick_view_on() {
            self.preview.focused = false;
            let said = self
                .preview
                .view
                .as_mut()
                .map(QuickView::turn_off)
                .unwrap_or_default();
            return self.preview_said(said, false);
        }
        let options = QuickViewOptions::from_settings(&self.settings, self.screen_reader);
        let stale = self
            .preview
            .view
            .as_ref()
            .is_none_or(|v| v.options() != &options);
        if stale {
            let registry = self
                .preview
                .registry
                .clone()
                .unwrap_or_else(Registry::with_local);
            let mut view = QuickView::new(registry, options);
            view.set_types(self.types.clone());
            let session = self
                .highlighter
                .get_or_insert_with(crate::viewer::highlight::Session::new)
                .clone();
            view.set_highlighter(session);
            self.preview.view = Some(view);
        }
        let panel = self.panel(self.active);
        let cursor = panel.focused_place().zip(panel.focused_entry().cloned());
        let said = match self.preview.view.as_mut() {
            Some(view) => view.turn_on(cursor.as_ref().map(|(p, e)| (p, e)), Instant::now()),
            None => Vec::new(),
        };
        self.preview_said(said, false)
    }

    /// After a command or an event: the quick view follows the active
    /// panel's cursor, rendering once it stops. Says nothing.
    pub(super) fn follow_cursor(&mut self) {
        if self.preview.focused || self.viewing.is_some() || !self.quick_view_on() {
            return;
        }
        let panel = self.panel(self.active);
        let (Some(place), Some(entry)) = (panel.focused_place(), panel.focused_entry().cloned())
        else {
            return;
        };
        if let Some(view) = self.preview.view.as_mut() {
            // A move says nothing; the line comes when the render does.
            let _ = view.cursor_moved(&place, &entry, Instant::now());
        }
    }

    /// The active panel's folder watch: the file shown is read again when
    /// it changes ("Preview updated: report.md changed", after the wait).
    pub(super) fn quick_view_watch(&mut self, side: diroxide_core::Side, event: &WatchEvent) {
        if side != self.active {
            return;
        }
        if let Some(view) = self.preview.view.as_mut().filter(|v| v.is_on()) {
            // Said when the render is done, by the tick.
            let _ = view.watch_event(event, Instant::now());
        }
    }

    /// Tab with the quick view on: into the preview region, its title and
    /// the row under its cursor said as one line, one announcement for
    /// one change ("Preview, report.md, Report, heading 1, 1 of 40").
    pub(super) fn enter_preview(&mut self) -> Vec<Effect> {
        self.refresh_siblings();
        self.preview.focused = true;
        let said = self
            .preview
            .view
            .as_mut()
            .map(QuickView::enter)
            .unwrap_or_default();
        let text: Vec<&str> = said.iter().map(|m| m.text.as_str()).collect();
        let line = Message::info(text.join(", "));
        self.preview_said(vec![line], false)
    }

    /// Tab or Escape in the preview: back to the panel, the cursor on its
    /// row ("Left panel: demo, 13 items").
    pub(super) fn leave_preview(&mut self) -> Vec<Effect> {
        self.preview.focused = false;
        let side = self.active;
        let panel = self.panel(side);
        let text = format!(
            "{} panel: {}, {}",
            super::capitalized(side.word()),
            crate::panel::place_leaf(panel.place()),
            diroxide_core::format::count(panel.list().len() as u64, "item", "items")
        );
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
    }

    /// The panel's files, in its order, for the previous and next file.
    fn refresh_siblings(&mut self) {
        let panel = self.panel(self.active);
        let siblings: Vec<_> = panel
            .list()
            .rows()
            .iter()
            .filter_map(|r| panel.entry(&r.id))
            .filter_map(|e| panel.place().join(&e.name).ok().map(|p| (p, e.clone())))
            .collect();
        if let Some(view) = self.preview.view.as_mut() {
            view.set_siblings(siblings);
        }
    }

    /// Say Other Panel with the quick view on: the preview's own words
    /// ("Other panel: Preview, report.md").
    pub(super) fn say_other_preview(&mut self) -> Option<Vec<Effect>> {
        let view = self.quick_view()?;
        let text = format!("Other panel: {}", view.region_title());
        Some(vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::info(text)),
        ])
    }

    /// A preview command, or a panel's command while the focus is in the
    /// preview; `None` when it is neither.
    pub(super) fn preview_dispatch(
        &mut self,
        action: Option<Action>,
        id: &CommandId,
    ) -> Option<Vec<Effect>> {
        if let Some(command) = preview_route(id) {
            if !self.quick_view_on() {
                let key = self.key(&ids::QUICK_VIEW);
                let text = if key.is_empty() {
                    "The quick view is off. Quick view in the View menu turns it on.".to_owned()
                } else {
                    format!("The quick view is off. {key} turns it on.")
                };
                return Some(vec![self.say(Message::info(text))]);
            }
            return Some(self.preview_command(command));
        }
        if !self.in_preview() {
            return None;
        }
        let effects = match action? {
            Action::Focus(movement) => self.preview_move(movement),
            Action::SayStatus => {
                let said = self
                    .preview
                    .view
                    .as_ref()
                    .map(QuickView::region_status)
                    .unwrap_or_default();
                self.preview_said(said, false)
            }
            Action::Cancel | Action::SwitchPanel => self.leave_preview(),
            Action::SayOther => self.say_other_preview()?,
            Action::QuickView
            | Action::OpenInTextweaver
            | Action::Help
            | Action::Palette
            | Action::MenuBar
            | Action::Menu
            | Action::Quit
            | Action::QuitToFolder
            | Action::QuitKeepFolder
            | Action::SayKeyBar
            | Action::SwitchBank
            | Action::Settings
            | Action::KeyTest
            | Action::Layout
            | Action::NextTheme
            | Action::JobHistory
            | Action::Problems
            | Action::Yes
            | Action::No
            | Action::Accept => return None,
            _ => {
                let text = self.leave_first();
                vec![self.say(Message::info(text))]
            }
        };
        Some(effects)
    }

    /// A key while the focus is in the preview: the preview's own keys,
    /// then a list's, then those that work everywhere. A panel's key says
    /// how to get back to the panel instead of doing nothing.
    pub(super) fn preview_press(&mut self, key: Key) -> Vec<Effect> {
        let layers = [Layer::Preview, Layer::List, Layer::Global];
        match self.keymap.lookup(&layers, &key) {
            Lookup::Command(id) => {
                // A move keeps the last message, as in a panel.
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
            Lookup::NoMatch => match self.keymap.lookup(&[Layer::Panel], &key) {
                Lookup::Command(_) => {
                    let text = self.leave_first();
                    vec![self.say(Message::info(text))]
                }
                Lookup::NoMatch => Vec::new(),
            },
        }
    }

    /// "Leave the preview first. Escape returns to the panel."
    pub(super) fn leave_first(&self) -> String {
        let leave = self.key(&ids::PREVIEW_LEAVE);
        if leave.is_empty() {
            "Leave the preview first.".to_owned()
        } else {
            format!("Leave the preview first. {leave} returns to the panel.")
        }
    }

    fn preview_move(&mut self, movement: Move) -> Vec<Effect> {
        let said = self
            .preview
            .view
            .as_mut()
            .map(|v| v.region_move(movement))
            .unwrap_or_default();
        self.preview_said(said, true)
    }

    fn preview_command(&mut self, command: PreviewCommand) -> Vec<Effect> {
        use PreviewCommand as C;
        if command == C::Leave {
            return if self.in_preview() {
                self.leave_preview()
            } else {
                Vec::new()
            };
        }
        if command == C::Find {
            let field = EntryField::new("Find in the preview", CompletionSource::History)
                .with_history(FIND_HISTORY);
            return self.open_dialog(Dialog::edit(field), Purpose::PreviewFind);
        }
        if command == C::Links {
            let Some(view) = self.preview.view.as_ref() else {
                return Vec::new();
            };
            let list = view.links();
            return self.open_dialog(
                Dialog::Choose {
                    list,
                    on_enter: Command::new(ids::PREVIEW_LINKS),
                },
                Purpose::PreviewLinks,
            );
        }
        if matches!(command, C::NextFile | C::PreviousFile) {
            self.refresh_siblings();
        }
        let now = Instant::now();
        let Some(view) = self.preview.view.as_mut() else {
            return Vec::new();
        };
        let outcome: Outcome = match command {
            C::NextHeading => view.next_heading().into(),
            C::PreviousHeading => view.previous_heading().into(),
            C::NextParagraph => view.next_paragraph().into(),
            C::PreviousParagraph => view.previous_paragraph().into(),
            C::NextTable => view.next_table().into(),
            C::PreviousTable => view.previous_table().into(),
            C::CellLeft => view.cell_left().into(),
            C::CellRight => view.cell_right().into(),
            C::CellUp => view.cell_up().into(),
            C::CellDown => view.cell_down().into(),
            C::NextLink => view.next_link().into(),
            C::PreviousLink => view.previous_link().into(),
            C::NextMatch => view.next_match().into(),
            C::PreviousMatch => view.previous_match().into(),
            C::NextFile => view.next_file(now),
            C::PreviousFile => view.previous_file(now),
            C::OpenLink => {
                let Some(id) = view.link_here() else {
                    return vec![self.say(Message::status("No link on this line"))];
                };
                let said = view.open_link(&id);
                return self.link_question(said);
            }
            C::Leave | C::Find | C::Links => Outcome::default(),
        };
        self.preview_outcome(outcome)
    }

    /// What a command said and what it asks: the panel's cursor moved to
    /// the next file, an address opened, textweaver run.
    fn preview_outcome(&mut self, outcome: Outcome) -> Vec<Effect> {
        let mut effects = self.preview_said(outcome.said, false);
        match outcome.request {
            Some(QuickViewRequest::FocusEntry(name)) => {
                let side = self.active;
                self.panel_mut(side).list_mut().focus_id(&name);
                effects.push(Effect::Redraw(Redraw::Panel(side)));
            }
            Some(QuickViewRequest::OpenAddress(address)) => {
                effects.push(Effect::OpenAddress(address));
            }
            Some(QuickViewRequest::RunProgram { program, args }) => {
                let side = self.active;
                let kind = SpawnKind::Program {
                    name: "textweaver".to_owned(),
                    program,
                    args,
                };
                let spawn = self.spawn_for(side, kind);
                self.request(Request::Spawn(Box::new(spawn)));
            }
            None => {}
        }
        effects
    }

    /// Opening a link asks first, starting on No: "Open https://example.org
    /// in the browser? No is the default. y or n". A refused link is said.
    fn link_question(&mut self, said: Vec<Message>) -> Vec<Effect> {
        let asking = self.preview.view.as_ref().is_some_and(QuickView::is_asking);
        let Some(question) = said.into_iter().next() else {
            return Vec::new();
        };
        if !asking {
            return vec![self.say(question)];
        }
        self.open_dialog(
            Dialog::Confirm {
                question: with_default_no(question),
                listing: Vec::new(),
                on_yes: Command::new(ids::YES),
            },
            Purpose::PreviewLink,
        )
    }

    /// The answer to the link question: yes opens the address with the
    /// system's browser or mail program; no opens nothing ("Not opened").
    pub(super) fn preview_link_answered(&mut self, yes: bool) -> Vec<Effect> {
        let outcome = self
            .preview
            .view
            .as_mut()
            .map(|v| v.answer_link(yes))
            .unwrap_or_default();
        self.preview_outcome(outcome)
    }

    /// The find prompt's text: "Match 2 of 7 in the preview".
    pub(super) fn preview_find(&mut self, text: &str) -> Vec<Effect> {
        if !text.is_empty() {
            self.remember(FIND_HISTORY, text);
        }
        let said = self
            .preview
            .view
            .as_mut()
            .map(|v| v.find(text))
            .unwrap_or_default();
        self.preview_said(said, false)
    }

    /// A row of the links list: the region's cursor on the link, said
    /// ("Link 2 of 5: docs, to docs/index.md").
    pub(super) fn preview_link_chosen(&mut self, id: &str) -> Vec<Effect> {
        let said = self
            .preview
            .view
            .as_mut()
            .map(|v| v.link_go(id))
            .unwrap_or_default();
        self.preview_said(said, false)
    }

    /// Open in textweaver: the shown file handed to `tw` with the screen,
    /// with no shell; or how to install it.
    pub(super) fn open_in_textweaver(&mut self) -> Vec<Effect> {
        if !self.quick_view_on() {
            let key = self.key(&ids::QUICK_VIEW);
            let text = if key.is_empty() {
                "Open in textweaver opens the quick view's file. Turn the quick view on first."
                    .to_owned()
            } else {
                format!("Open in textweaver opens the quick view's file. {key} turns it on.")
            };
            return vec![self.say(Message::info(text))];
        }
        if self.preview.tools.is_none() {
            self.preview.tools = Some(ToolRegistry::builtin());
        }
        let (Some(view), Some(tools)) = (self.preview.view.as_ref(), self.preview.tools.as_ref())
        else {
            return Vec::new();
        };
        let outcome = view.open_in_textweaver(tools);
        self.preview_outcome(outcome)
    }

    /// The quick view's words: a move's own row is announced but not put
    /// on the status line, as a panel's are; everything else is said.
    fn preview_said(&mut self, said: Vec<Message>, moves: bool) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        for message in said {
            if moves && message.importance == Importance::Low {
                effects.push(Effect::Announce(message));
            } else {
                effects.push(self.say(message));
            }
        }
        effects
    }

    /// What the preview region shows in screen-reader mode while the
    /// focus is in the panel, in place of the document: "Tab goes into the
    /// preview", with the key that does it.
    pub fn preview_placeholder(&self) -> String {
        let key = self.key(&ids::SWITCH_PANEL);
        if key.is_empty() {
            "Switch panels goes into the preview".to_owned()
        } else {
            format!("{key} goes into the preview")
        }
    }

    /// The title line in the preview: "Preview, report.md".
    pub(super) fn preview_title(&self, cells: usize) -> Option<String> {
        if !self.in_preview() {
            return None;
        }
        let title = self.quick_view()?.region_title();
        let title = diroxide_core::names::display_name(&title).into_owned();
        Some(title.chars().take(cells).collect())
    }
}

/// What a tick may change on the screen: the file shown, its rows, and
/// whether it still waits on something.
fn signature(view: &QuickView) -> (Option<diroxide_core::Place>, usize, bool, u64) {
    (
        view.shown().map(|s| s.place.clone()),
        view.region().len(),
        view.wants_tick(),
        view.renders(),
    )
}
