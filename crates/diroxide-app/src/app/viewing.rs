//! The viewer wired into the app (plan commit 63): F3 opens the focused
//! file in the viewer, read from its place on a worker (a member of a
//! locked zip asks its password first); the viewer's own keys work while
//! it is open (the viewer layer of the keymap, W2d's table), every command
//! a method of [`Viewer`] whose sentences the app says; Escape goes back
//! to the file's row. The frontend draws the viewer's rows and calls
//! [`App::viewer_tick`] while [`App::viewer_wants_tick`] says so, so a long
//! scan, the highlighter and Follow carry on without blocking a key.

use diroxide_core::{CommandId, Effect, Message, Move, Place, Redraw, RequestId, Side, Timestamp};

use super::{Action, App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::request::Request;
use crate::viewer::{
    IndentMove, Sibling, Viewer, ViewerOptions, ViewerSource, WORK_BUDGET, highlight,
};

/// A command of the viewer layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewerCommand {
    /// Close viewer
    Close,
    /// Previous token
    Left,
    /// Next token
    Right,
    /// Wrap lines
    Wrap,
    /// Line numbers
    LineNumbers,
    /// Go to line
    Jump,
    /// Search in file
    Search,
    /// Search backward
    SearchBack,
    /// Next match
    NextMatch,
    /// Previous match
    PreviousMatch,
    /// Hex mode
    Hex,
    /// Say byte
    Byte,
    /// Read more
    ReadMore,
    /// Reload file
    Reload,
    /// Follow file
    Follow,
    /// Next file
    NextFile,
    /// Previous file
    PreviousFile,
    /// Show in folder
    ShowInFolder,
    /// Copy path
    CopyPath,
    /// Copy name
    CopyName,
    /// Say token kind
    TokenKind,
    /// Say line by kind
    LineKinds,
    /// Outline
    Outline,
    /// Next symbol
    NextSymbol,
    /// Previous symbol
    PreviousSymbol,
    /// Matching bracket
    Bracket,
    /// Skip comment
    SkipComment,
    /// Say indent
    Indent,
    /// Next at this indent
    IndentNext,
    /// Previous at this indent
    IndentPrevious,
    /// Up an indent level
    IndentParent,
    /// Into the next indent
    IndentChild,
    /// Say context
    Context,
}

const VIEWER_ROUTES: [(CommandId, ViewerCommand); 33] = [
    (ids::CLOSE_VIEWER, ViewerCommand::Close),
    (ids::VIEWER_LEFT, ViewerCommand::Left),
    (ids::VIEWER_RIGHT, ViewerCommand::Right),
    (ids::VIEWER_WRAP, ViewerCommand::Wrap),
    (ids::VIEWER_LINE_NUMBERS, ViewerCommand::LineNumbers),
    (ids::VIEWER_JUMP, ViewerCommand::Jump),
    (ids::VIEWER_SEARCH, ViewerCommand::Search),
    (ids::VIEWER_SEARCH_BACK, ViewerCommand::SearchBack),
    (ids::VIEWER_NEXT_MATCH, ViewerCommand::NextMatch),
    (ids::VIEWER_PREVIOUS_MATCH, ViewerCommand::PreviousMatch),
    (ids::VIEWER_HEX, ViewerCommand::Hex),
    (ids::VIEWER_BYTE, ViewerCommand::Byte),
    (ids::VIEWER_READ_MORE, ViewerCommand::ReadMore),
    (ids::VIEWER_RELOAD, ViewerCommand::Reload),
    (ids::VIEWER_FOLLOW, ViewerCommand::Follow),
    (ids::VIEWER_NEXT_FILE, ViewerCommand::NextFile),
    (ids::VIEWER_PREVIOUS_FILE, ViewerCommand::PreviousFile),
    (ids::VIEWER_SHOW_IN_FOLDER, ViewerCommand::ShowInFolder),
    (ids::VIEWER_COPY_PATH, ViewerCommand::CopyPath),
    (ids::VIEWER_COPY_NAME, ViewerCommand::CopyName),
    (ids::VIEWER_TOKEN_KIND, ViewerCommand::TokenKind),
    (ids::VIEWER_LINE_KINDS, ViewerCommand::LineKinds),
    (ids::VIEWER_OUTLINE, ViewerCommand::Outline),
    (ids::VIEWER_NEXT_SYMBOL, ViewerCommand::NextSymbol),
    (ids::VIEWER_PREVIOUS_SYMBOL, ViewerCommand::PreviousSymbol),
    (ids::VIEWER_BRACKET, ViewerCommand::Bracket),
    (ids::VIEWER_SKIP_COMMENT, ViewerCommand::SkipComment),
    (ids::VIEWER_INDENT, ViewerCommand::Indent),
    (ids::VIEWER_INDENT_NEXT, ViewerCommand::IndentNext),
    (ids::VIEWER_INDENT_PREVIOUS, ViewerCommand::IndentPrevious),
    (ids::VIEWER_INDENT_PARENT, ViewerCommand::IndentParent),
    (ids::VIEWER_INDENT_CHILD, ViewerCommand::IndentChild),
    (ids::VIEWER_CONTEXT, ViewerCommand::Context),
];

/// The viewer command a command id runs, if it is one.
pub(super) fn viewer_route(id: &CommandId) -> Option<ViewerCommand> {
    VIEWER_ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, command)| *command)
}

/// The prompt histories of the viewer.
const JUMP_HISTORY: &str = "viewer-jump";
const SEARCH_HISTORY: &str = "viewer-search";

/// How often the viewer is given time while it works (a scan, the
/// highlighter), in milliseconds.
pub const VIEWER_WORK_TICK_MS: u64 = 20;

/// The open viewer and where it came from.
pub(crate) struct Viewing {
    /// The viewer.
    pub viewer: Box<Viewer>,
    /// The panel it was opened from.
    pub side: Side,
    /// The row it goes back to.
    pub row: String,
    /// The files it steps through, in the panel's order.
    pub siblings: Vec<Sibling>,
    /// When Follow last looked for new lines.
    pub last_follow: Option<Timestamp>,
}

impl std::fmt::Debug for Viewing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Viewing")
            .field("viewer", &self.viewer)
            .field("side", &self.side)
            .finish_non_exhaustive()
    }
}

/// Why a viewer source is being read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceFor {
    /// F3: a new viewer.
    Open {
        side: Side,
        row: String,
        siblings: Vec<Sibling>,
    },
    /// The next or the previous file, in the open viewer.
    Show,
    /// Reload, in the open viewer.
    Reload,
}

impl App {
    /// The open viewer, for the frontend to draw.
    pub fn viewer(&self) -> Option<&Viewer> {
        self.viewing.as_ref().map(|v| v.viewer.as_ref())
    }

    /// The open viewer, mutable: drawing gives it its size and reads its
    /// rows.
    pub fn viewer_mut(&mut self) -> Option<&mut Viewer> {
        self.viewing.as_mut().map(|v| v.viewer.as_mut())
    }

    /// Whether the viewer is open.
    pub fn viewer_open(&self) -> bool {
        self.viewing.is_some()
    }

    /// How soon the frontend should call [`App::viewer_tick`], in
    /// milliseconds: soon while the viewer scans or highlights, at the
    /// progress interval while it follows a file, never otherwise.
    pub fn viewer_wants_tick(&self) -> Option<u64> {
        let viewing = self.viewing.as_ref()?;
        let viewer = &viewing.viewer;
        if viewer.wants_work() || viewer.is_highlighting() {
            return Some(VIEWER_WORK_TICK_MS);
        }
        viewer.follows().then(|| self.follow_interval_ms())
    }

    fn follow_interval_ms(&self) -> u64 {
        if self.screen_reader {
            self.settings.progress_interval_seconds.saturating_mul(1000)
        } else {
            1000
        }
    }

    /// Gives the viewer its time: a pending scan or the index a slice
    /// further, the highlighter's result, and Follow's look for new lines
    /// at its interval. What they say is said; the screen is drawn again
    /// when anything changed.
    pub fn viewer_tick(&mut self) -> Vec<Effect> {
        let now = self.now();
        let interval = self.follow_interval_ms();
        let Some(viewing) = self.viewing.as_mut() else {
            return Vec::new();
        };
        let mut said = Vec::new();
        let mut changed = false;
        if viewing.viewer.wants_work() {
            said.extend(viewing.viewer.work(WORK_BUDGET));
            changed = true;
        }
        if viewing.viewer.is_highlighting() {
            said.extend(viewing.viewer.poll_highlight());
            changed = true;
        }
        if viewing.viewer.follows() {
            let due = viewing
                .last_follow
                .is_none_or(|at| crate::jobs::millis_between(at, now) >= interval);
            if due {
                viewing.last_follow = Some(now);
                said.extend(viewing.viewer.follow_tick());
                changed = true;
            }
        }
        let mut effects = Vec::new();
        if changed {
            effects.push(Effect::Redraw(Redraw::All));
        }
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// F3: the focused file in the viewer, its bytes read from its place
    /// on a worker.
    pub(super) fn view_file(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if entry.is_folder_like() && entry.kind != diroxide_core::EntryKind::Archive {
            let text = format!("Not a file: {}, folder", entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        let size = entry.size;
        let row = entry.name.clone();
        let siblings: Vec<Sibling> = panel
            .list()
            .rows()
            .iter()
            .filter_map(|r| panel.entry(&r.id))
            .filter(|e| !e.is_folder_like() || e.kind == diroxide_core::EntryKind::Archive)
            .filter_map(|e| {
                panel
                    .place()
                    .join(&e.name)
                    .ok()
                    .map(|p| Sibling::new(p, e.size))
            })
            .collect();
        self.read_viewer_source(
            place,
            size,
            SourceFor::Open {
                side,
                row,
                siblings,
            },
        )
    }

    fn read_viewer_source(
        &mut self,
        place: Place,
        size: Option<u64>,
        why: SourceFor,
    ) -> Vec<Effect> {
        let request = self.ids.request();
        let options = ViewerOptions::from_settings(&self.settings, self.screen_reader);
        self.viewer_wait = Some((request, why));
        self.request(Request::OpenViewer {
            request,
            place,
            size,
            cap: options.stream_cap(),
        });
        Vec::new()
    }

    /// A viewer source arrived: a new viewer, the next file, or the same
    /// file read again.
    pub(super) fn viewer_source(
        &mut self,
        request: RequestId,
        place: Place,
        result: Result<ViewerSource, Message>,
    ) -> Vec<Effect> {
        let Some((waited, why)) = self.viewer_wait.take() else {
            return Vec::new();
        };
        if waited != request {
            self.viewer_wait = Some((waited, why));
            return Vec::new();
        }
        let source = match result {
            Ok(source) => source,
            Err(message) => return vec![self.say(message)],
        };
        let said = match why {
            SourceFor::Open {
                side,
                row,
                siblings,
            } => {
                let options = ViewerOptions::from_settings(&self.settings, self.screen_reader);
                let (mut viewer, mut said) =
                    match Viewer::from_source(place.clone(), source, options) {
                        Ok(opened) => opened,
                        Err(error) => {
                            let name = place
                                .name()
                                .map(diroxide_core::names::display_name)
                                .unwrap_or_default()
                                .into_owned();
                            return vec![self.say(error.message(&name))];
                        }
                    };
                viewer.set_siblings(siblings.clone());
                let session = self
                    .highlighter
                    .get_or_insert_with(highlight::Session::new)
                    .clone();
                said.extend(viewer.set_highlighter(session));
                self.viewing = Some(Viewing {
                    viewer: Box::new(viewer),
                    side,
                    row,
                    siblings,
                    last_follow: None,
                });
                said
            }
            SourceFor::Show => match self.viewing.as_mut() {
                Some(viewing) => viewing.viewer.show(place, source),
                None => return Vec::new(),
            },
            SourceFor::Reload => match self.viewing.as_mut() {
                Some(viewing) => viewing.viewer.reload_with(source),
                None => return Vec::new(),
            },
        };
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// A command while the viewer is open and no dialog is: the viewer's
    /// own, a move, Say Status, Escape; the program's (the palette, the
    /// menus, help, quit) as anywhere; a panel's waits for the viewer to
    /// close. `None` hands the command to the panel's dispatch.
    pub(super) fn viewer_dispatch(
        &mut self,
        action: Option<Action>,
        id: &CommandId,
    ) -> Option<Vec<Effect>> {
        self.viewing.as_ref()?;
        if let Some(command) = viewer_route(id) {
            return Some(self.viewer_command(command));
        }
        let action = action?;
        let effects = match action {
            Action::Focus(movement) => self.viewer_move(movement),
            Action::SayStatus => {
                let line = self.viewer().map(Viewer::status).unwrap_or_default();
                vec![self.say(Message::info(line))]
            }
            Action::Cancel => self.close_viewer(),
            Action::Help
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
                let close = self.key(&ids::CLOSE_VIEWER);
                let text = if close.is_empty() {
                    "Close the viewer first.".to_owned()
                } else {
                    format!("Close the viewer first. {close} closes it.")
                };
                vec![self.say(Message::info(text))]
            }
        };
        Some(effects)
    }

    /// A viewer command with no viewer open: said, never an error.
    pub(super) fn viewer_closed_command(&mut self) -> Vec<Effect> {
        let view = self.key(&ids::VIEW);
        let text = if view.is_empty() {
            "The viewer is not open.".to_owned()
        } else {
            format!("The viewer is not open. {view} opens it.")
        };
        vec![self.say(Message::info(text))]
    }

    fn viewer_move(&mut self, movement: Move) -> Vec<Effect> {
        let Some(viewer) = self.viewer_mut() else {
            return Vec::new();
        };
        let said = match movement {
            Move::Up => viewer.up(),
            Move::Down => viewer.down(),
            Move::PageUp => viewer.page_up(),
            Move::PageDown => viewer.page_down(),
            Move::Home => viewer.home(),
            Move::End => viewer.end(),
            _ => Vec::new(),
        };
        self.viewer_said(said)
    }

    fn viewer_said(&mut self, said: Vec<Message>) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// Escape in the viewer: back to the file's row in its panel.
    pub(super) fn close_viewer(&mut self) -> Vec<Effect> {
        let Some(viewing) = self.viewing.take() else {
            return Vec::new();
        };
        let name = viewing.viewer.name().to_owned();
        self.viewer_wait = None;
        let side = viewing.side;
        // The row the viewer came from, unless Next file moved on: then
        // the file it shows now.
        let row = viewing
            .viewer
            .place()
            .name()
            .filter(|n| self.panel(side).entry(n).is_some())
            .map_or(viewing.row.clone(), str::to_owned);
        self.panel_mut(side).list_mut().focus_id(&row);
        self.active = side;
        vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::info(format!("Viewer closed: {name}"))),
        ]
    }

    fn viewer_command(&mut self, command: ViewerCommand) -> Vec<Effect> {
        use ViewerCommand as C;
        if self.viewing.is_none() {
            return self.viewer_closed_command();
        }
        match command {
            C::Close => return self.close_viewer(),
            C::Jump => {
                let field =
                    EntryField::new("Go to line, offset or percent", CompletionSource::History)
                        .with_history(JUMP_HISTORY);
                return self.open_dialog(Dialog::edit(field), Purpose::ViewerJump);
            }
            C::Search | C::SearchBack => {
                let backward = command == C::SearchBack;
                let label = if backward {
                    "Search backward for"
                } else {
                    "Search for"
                };
                let field =
                    EntryField::new(label, CompletionSource::History).with_history(SEARCH_HISTORY);
                return self.open_dialog(Dialog::edit(field), Purpose::ViewerSearch { backward });
            }
            C::Outline => {
                let Some(list) = self.viewer().map(Viewer::outline) else {
                    return Vec::new();
                };
                if list.is_empty() {
                    let text = list.empty_text().to_owned();
                    return vec![self.say(Message::info(text))];
                }
                return self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: diroxide_core::Command::new(ids::VIEWER_OUTLINE),
                    },
                    Purpose::ViewerOutline,
                );
            }
            C::NextFile | C::PreviousFile => return self.step_file(command == C::NextFile),
            C::Reload => {
                let Some(place) = self.viewer().map(|v| v.place().clone()) else {
                    return Vec::new();
                };
                return self.read_viewer_source(place, None, SourceFor::Reload);
            }
            C::ShowInFolder => return self.viewer_show_in_folder(),
            C::CopyPath | C::CopyName => {
                let Some(viewer) = self.viewer() else {
                    return Vec::new();
                };
                let (effect, message) = if command == C::CopyPath {
                    viewer.copy_path()
                } else {
                    viewer.copy_name()
                };
                return vec![effect, self.say(message)];
            }
            _ => {}
        }
        let Some(viewer) = self.viewer_mut() else {
            return Vec::new();
        };
        let said = match command {
            C::Left => viewer.left(),
            C::Right => viewer.right(),
            C::Wrap => viewer.toggle_wrap(),
            C::LineNumbers => viewer.toggle_line_numbers(),
            C::NextMatch => viewer.next_match(),
            C::PreviousMatch => viewer.previous_match(),
            C::Hex => viewer.toggle_hex(),
            C::Byte => viewer.say_byte(),
            C::ReadMore => viewer.read_more(),
            C::Follow => viewer.toggle_follow(),
            C::TokenKind => viewer.token_kind(),
            C::LineKinds => viewer.line_kinds(),
            C::NextSymbol => viewer.next_symbol(),
            C::PreviousSymbol => viewer.previous_symbol(),
            C::Bracket => viewer.bracket_jump(),
            C::SkipComment => viewer.skip_comment(),
            C::Indent => viewer.say_indent(),
            C::IndentNext => viewer.indent_move(IndentMove::NextSame),
            C::IndentPrevious => viewer.indent_move(IndentMove::PreviousSame),
            C::IndentParent => viewer.indent_move(IndentMove::Parent),
            C::IndentChild => viewer.indent_move(IndentMove::Child),
            C::Context => viewer.context(),
            _ => Vec::new(),
        };
        self.viewer_said(said)
    }

    /// Next or previous file: the sibling read on a worker, then shown in
    /// the same viewer ("report-2.txt, 2 of 4"); at the ends, said.
    fn step_file(&mut self, forward: bool) -> Vec<Effect> {
        let Some(viewing) = self.viewing.as_ref() else {
            return Vec::new();
        };
        let Some((at, of)) = viewing.viewer.file_position() else {
            return vec![self.say(Message::info("No other files to step to"))];
        };
        let next = if forward { at + 1 } else { at.wrapping_sub(1) };
        if next == 0 || next > of {
            let which = if forward { "Last" } else { "First" };
            return vec![self.say(Message::info(format!("{which} file, {at} of {of}")))];
        }
        let sibling = viewing.siblings[next - 1].clone();
        self.read_viewer_source(sibling.place, sibling.size, SourceFor::Show)
    }

    /// Show in folder: the viewer closes and the panel shows the file's
    /// folder with its row focused.
    fn viewer_show_in_folder(&mut self) -> Vec<Effect> {
        let Some(viewing) = self.viewing.take() else {
            return Vec::new();
        };
        let (place, message) = viewing.viewer.show_in_folder();
        let side = viewing.side;
        self.active = side;
        let mut effects = vec![Effect::Redraw(Redraw::All), self.say(message)];
        let (Some(folder), Some(name)) = (place.parent(), place.name().map(str::to_owned)) else {
            return effects;
        };
        if &folder == self.panel(side).place() {
            self.panel_mut(side).list_mut().focus_id(&name);
        } else {
            effects.extend(self.open(side, folder, Some(name)));
        }
        effects
    }

    /// The jump prompt answered.
    pub(super) fn viewer_jump(&mut self, text: &str) -> Vec<Effect> {
        self.remember(JUMP_HISTORY, text);
        let said = self.viewer_mut().map(|v| v.jump(text)).unwrap_or_default();
        self.viewer_said(said)
    }

    /// The search prompt answered.
    pub(super) fn viewer_search(&mut self, text: &str, backward: bool) -> Vec<Effect> {
        if !text.is_empty() {
            self.remember(SEARCH_HISTORY, text);
        }
        let said = self
            .viewer_mut()
            .map(|v| v.search(text, backward))
            .unwrap_or_default();
        self.viewer_said(said)
    }

    /// A symbol chosen from the outline.
    pub(super) fn viewer_outline_go(&mut self, id: &str) -> Vec<Effect> {
        let said = self
            .viewer_mut()
            .map(|v| v.outline_go(id))
            .unwrap_or_default();
        self.viewer_said(said)
    }

    /// The title line while the viewer is open: its title, the name first
    /// ("notes.txt, in course.zip"), cut to `cells`.
    pub(super) fn viewer_title(&self, cells: usize) -> Option<String> {
        let title = diroxide_core::names::display_name(self.viewer()?.title()).into_owned();
        Some(title.chars().take(cells).collect())
    }
}
