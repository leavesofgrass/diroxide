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
#[derive(Debug)]
pub(crate) enum SourceFor {
    /// F3: a new viewer; `bytes` when the Hex command asked for the
    /// file's bytes, so no preview's words are read (lane VP).
    Open {
        side: Side,
        row: String,
        siblings: Vec<Sibling>,
        bytes: bool,
    },
    /// The next or the previous file, in the open viewer.
    Show,
    /// Reload, in the open viewer.
    Reload,
    /// The preview's words, read after the bytes looked binary (lane
    /// VP): where they go, the viewer over the bytes when it is not open
    /// yet, and what it said, said only when there are no words.
    Words {
        to: WordsTo,
        bytes: Option<Box<Viewer>>,
        held: Vec<Message>,
    },
}

/// Where a preview's words go once read (lane VP).
#[derive(Debug)]
pub(crate) enum WordsTo {
    /// A new viewer, the bytes behind the words.
    Open {
        side: Side,
        row: String,
        siblings: Vec<Sibling>,
    },
    /// The open viewer, stepped to the next or the previous file.
    Show,
    /// The open viewer's words, read again.
    Reload,
    /// Behind the bytes the open viewer shows, said nothing.
    Behind,
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
        // The type's own View action, when it has one (lane TA).
        if let Some(effects) = self.typed_slot(crate::typing::Slot::View) {
            return effects;
        }
        // A picture in the window opens in the picture view at once
        // (lane VP, the owner's finding).
        if let Some(effects) = self.show_focused_picture() {
            return effects;
        }
        self.view_focused(false)
    }

    /// The focused file in the viewer: its bytes as the viewer reads
    /// them, and for a file whose bytes are binary the preview's words in
    /// front of them unless `bytes` (the Hex command with no viewer open,
    /// the road to a picture's or a document's bytes).
    fn view_focused(&mut self, bytes: bool) -> Vec<Effect> {
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
                bytes,
            },
        )
    }

    /// A file in the viewer from outside a panel (a search result): its
    /// siblings are the files to step through, and the viewer goes to the
    /// line a result names once it is open.
    pub(super) fn view_place_at(
        &mut self,
        place: Place,
        size: Option<u64>,
        siblings: Vec<Sibling>,
    ) -> Vec<Effect> {
        let side = self.active;
        let row = place.name().unwrap_or_default().to_owned();
        self.read_viewer_source(
            place,
            size,
            SourceFor::Open {
                side,
                row,
                siblings,
                bytes: false,
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
            Err(message) => {
                // A result that could not open leaves the results open.
                self.take_park_on_open();
                self.take_view_line();
                return vec![self.say(message)];
            }
        };
        let said = match why {
            SourceFor::Open {
                side,
                row,
                siblings,
                bytes,
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
                // Bytes that are not text, of a file a previewer takes:
                // its words first, the bytes behind them (lane VP).
                if !bytes
                    && viewer.looks_binary()
                    && let Some(wanted) = self.words_wanted(&place)
                {
                    let to = WordsTo::Open {
                        side,
                        row,
                        siblings,
                    };
                    return self.ask_words(&place, wanted, to, Some(Box::new(viewer)), said, false);
                }
                self.install_viewer(viewer, side, row, siblings, &mut said);
                // A picture says what shows it, in the window (lane AE).
                said.extend(self.picture_hint(&place));
                said
            }
            SourceFor::Show => {
                let Some(viewing) = self.viewing.as_mut() else {
                    return Vec::new();
                };
                // Stepping on from a preview's words: its bytes' viewer
                // shows the next file; the words stay behind nothing.
                if viewing.viewer.is_preview()
                    && let Some(bytes) = viewing.viewer.take_other()
                {
                    viewing.viewer = bytes;
                }
                let _ = viewing.viewer.take_other();
                let said = viewing.viewer.show(place.clone(), source);
                let binary = viewing.viewer.looks_binary();
                match binary.then(|| self.words_wanted(&place)).flatten() {
                    Some(wanted) => {
                        return self.ask_words(&place, wanted, WordsTo::Show, None, said, false);
                    }
                    None => said,
                }
            }
            SourceFor::Reload => {
                let Some(viewing) = self.viewing.as_mut() else {
                    return Vec::new();
                };
                if viewing.viewer.is_preview() {
                    // The bytes behind the words are read again, then the
                    // words.
                    if let Some(bytes) = viewing.viewer.other_mut() {
                        let _ = bytes.reload_with(source);
                    }
                    match self.words_wanted(&place) {
                        Some(wanted) => {
                            return self.ask_words(
                                &place,
                                wanted,
                                WordsTo::Reload,
                                None,
                                Vec::new(),
                                true,
                            );
                        }
                        None => Vec::new(),
                    }
                } else {
                    let said = viewing.viewer.reload_with(source);
                    let behind = viewing.viewer.other().is_some_and(Viewer::is_preview);
                    if behind && let Some(wanted) = self.words_wanted(&place) {
                        let _ =
                            self.ask_words(&place, wanted, WordsTo::Behind, None, Vec::new(), true);
                    }
                    said
                }
            }
            SourceFor::Words { to, bytes, held } => {
                // Not an answer to a source read: put it back.
                self.viewer_wait = Some((waited, SourceFor::Words { to, bytes, held }));
                return Vec::new();
            }
        };
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// A new viewer, open: at a search result's line, with the results
    /// aside until it closes (W3a).
    fn install_viewer(
        &mut self,
        mut viewer: Viewer,
        side: Side,
        row: String,
        siblings: Vec<Sibling>,
        said: &mut Vec<Message>,
    ) {
        if let Some(line) = self.take_view_line() {
            said.extend(viewer.jump(&line.to_string()));
        }
        if self.take_park_on_open() {
            self.park_results();
        }
        self.viewing = Some(Viewing {
            viewer: Box::new(viewer),
            side,
            row,
            siblings,
            last_follow: None,
        });
    }

    /// The entry a viewed place is listed as, for its type: the panel's
    /// row where a panel lists it (its checked type), else its name.
    fn listed_entry(&self, place: &Place) -> diroxide_core::Entry {
        [self.active, self.active.other()]
            .into_iter()
            .find_map(|side| {
                let panel = self.panel(side);
                (place.parent().as_ref() == Some(panel.place()))
                    .then(|| place.name().and_then(|n| panel.entry(n)).cloned())
                    .flatten()
            })
            .unwrap_or_else(|| {
                diroxide_core::Entry::new(
                    place.name().unwrap_or_default(),
                    diroxide_core::EntryKind::File,
                )
            })
    }

    /// The quick view's readers for the viewer's words (lane VP): the
    /// app's types, quick view options, tools and previews' cache.
    fn words_preview(&self) -> crate::quickview::ViewerPreview {
        crate::quickview::ViewerPreview::new(
            std::sync::Arc::clone(&self.types),
            crate::quickview::QuickViewOptions::from_settings(&self.settings, self.screen_reader),
            self.tool_runner().0,
            std::sync::Arc::clone(&self.preview.cache.0),
        )
    }

    /// Whether a file whose bytes looked binary gets the preview's words:
    /// its entry and the previewer, when one takes it. A picture in the
    /// window (the picture view shows it), an archive, and a file no
    /// previewer takes show their bytes as before.
    fn words_wanted(
        &self,
        place: &Place,
    ) -> Option<(diroxide_core::Entry, crate::quickview::WordsFrom)> {
        let entry = self.listed_entry(place);
        let from = self.words_preview().takes(place, &entry)?;
        // The window shows a picture itself; the words are the terminal's.
        if from == crate::quickview::WordsFrom::Picture
            && self.keymap.column() != diroxide_core::KeyColumn::Terminal
        {
            return None;
        }
        Some((entry, from))
    }

    /// Asks for the preview's words: the request is sent, the bytes'
    /// viewer and what it said are held until they come.
    #[allow(clippy::too_many_arguments)]
    fn ask_words(
        &mut self,
        place: &Place,
        (entry, from): (diroxide_core::Entry, crate::quickview::WordsFrom),
        to: WordsTo,
        bytes: Option<Box<Viewer>>,
        held: Vec<Message>,
        refresh: bool,
    ) -> Vec<Effect> {
        let preview = self.words_preview();
        let request = self.ids.request();
        self.request(Request::ViewerWords(Box::new(crate::request::WordsAsk {
            request,
            place: place.clone(),
            entry: entry.clone(),
            refresh,
            preview,
        })));
        let quiet = matches!(to, WordsTo::Behind);
        self.viewer_wait = Some((request, SourceFor::Words { to, bytes, held }));
        if from.runs_a_helper() && !quiet {
            let line = format!("Reading the preview of {}", entry.display_name());
            return vec![
                self.say(Message::status(line).with_importance(diroxide_core::Importance::Normal)),
            ];
        }
        Vec::new()
    }

    /// The preview's words arrived (lane VP): the viewer opens on them, the
    /// bytes behind; without words the bytes show as before, and a read
    /// that failed says why first, never falling to hex in silence.
    pub(super) fn viewer_words(
        &mut self,
        request: RequestId,
        place: Place,
        result: Result<Option<crate::quickview::ViewerWords>, Message>,
    ) -> Vec<Effect> {
        let Some((waited, why)) = self.viewer_wait.take() else {
            return Vec::new();
        };
        let SourceFor::Words { to, bytes, held } = why else {
            self.viewer_wait = Some((waited, why));
            return Vec::new();
        };
        if waited != request {
            self.viewer_wait = Some((waited, SourceFor::Words { to, bytes, held }));
            return Vec::new();
        }
        let (words, failed) = match result {
            Ok(Some(words)) => (Some(words), None),
            Ok(None) => (None, None),
            Err(message) => (None, Some(message)),
        };
        let Some(words) = words else {
            let mut said: Vec<Message> = failed.into_iter().collect();
            match to {
                WordsTo::Open {
                    side,
                    row,
                    siblings,
                } => {
                    let Some(viewer) = bytes else {
                        return Vec::new();
                    };
                    said.extend(held);
                    self.install_viewer(*viewer, side, row, siblings, &mut said);
                }
                WordsTo::Show => said.extend(held),
                WordsTo::Reload | WordsTo::Behind => {}
            }
            return self.viewer_said(said);
        };
        let (text, spoken, starts) = self.words_text(&words);
        let options = ViewerOptions::from_settings(&self.settings, self.screen_reader);
        let (mut viewer, mut said) = match Viewer::from_preview(place.clone(), text, options) {
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
        said.extend(spoken.into_iter().map(Message::info));
        viewer.set_pages(words.pages, starts);
        match to {
            WordsTo::Open {
                side,
                row,
                siblings,
            } => {
                viewer.set_siblings(siblings.clone());
                if let Some(bytes) = bytes {
                    viewer.set_other(bytes);
                }
                self.install_viewer(viewer, side, row, siblings, &mut said);
            }
            WordsTo::Show => {
                let Some(viewing) = self.viewing.as_mut() else {
                    return Vec::new();
                };
                viewer.set_siblings(viewing.siblings.clone());
                if let (Some(first), Some((at, of))) = (said.first_mut(), viewer.file_position()) {
                    first.text = format!("{}, {at} of {of}", viewer.title());
                }
                let shown = std::mem::replace(&mut viewing.viewer, Box::new(viewer));
                viewing.viewer.set_other(shown);
            }
            WordsTo::Reload => {
                let Some(viewing) = self.viewing.as_mut() else {
                    return Vec::new();
                };
                viewer.set_siblings(viewing.siblings.clone());
                if let Some(bytes) = viewing.viewer.take_other() {
                    viewer.set_other(bytes);
                }
                if let Some(first) = said.first_mut() {
                    first.text = format!("Reloaded {}", viewer.title());
                }
                *viewing.viewer = viewer;
            }
            WordsTo::Behind => {
                if let Some(viewing) = self.viewing.as_mut() {
                    viewer.set_siblings(viewing.siblings.clone());
                    viewing.viewer.set_other(Box::new(viewer));
                }
                return vec![Effect::Redraw(Redraw::All)];
            }
        }
        // A PDF's words in the window: Enter shows its pages.
        said.extend(self.picture_hint(&place));
        self.viewer_said(said)
    }

    /// The viewer's text for a preview's words, and the lines said after
    /// its title (lane VP): the sentence, a missing helper's install line,
    /// in the terminal how a picture is shown; then the rows, a
    /// document's text and its notes, and the line that says how to reach
    /// the bytes.
    #[allow(clippy::type_complexity)]
    fn words_text(
        &self,
        words: &crate::quickview::ViewerWords,
    ) -> (String, Vec<String>, Vec<(u64, u32)>) {
        let mut spoken = vec![words.summary.clone()];
        if let Some(tool) = &words.missing
            && let Some(step) = self.tools_registry().next_step(tool)
        {
            spoken.push(match step {
                diroxide_tools::NextStep::Command(command) => format!("Install: {command}"),
                diroxide_tools::NextStep::Words(words) => words,
            });
        }
        if words.from == Some(crate::quickview::WordsFrom::Picture) {
            spoken.push(super::picture_view::IN_THE_WINDOW.to_owned());
        }
        let mut lines = spoken.clone();
        lines.extend(words.rows.iter().cloned());
        let mut starts = Vec::new();
        if !words.text.is_empty() {
            lines.push(String::new());
            for row in &words.text {
                // A PDF's page headings ("Page 2"), for the page pictures.
                if words.from == Some(crate::quickview::WordsFrom::Pdf)
                    && let Some(page) = row.strip_prefix("Page ").and_then(|n| n.parse().ok())
                {
                    starts.push((lines.len() as u64, page));
                }
                lines.push(row.clone());
            }
        }
        lines.extend(words.notes.iter().cloned());
        lines.push(String::new());
        lines.push(crate::viewer::hex_hint(&self.key(&ids::VIEWER_HEX)));
        let mut text = lines.join("\n");
        text.push('\n');
        (text, spoken, starts)
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

    /// A viewer command with no viewer open (lane VP): Hex mode opens the
    /// focused file's bytes in hex, the road to a picture's or a
    /// document's bytes; any other is said, never an error.
    pub(super) fn viewer_command_closed(&mut self, id: &CommandId) -> Vec<Effect> {
        if *id == ids::VIEWER_HEX && self.dialogs.is_empty() && !self.in_preview() {
            let file = self
                .panel(self.active)
                .focused_entry()
                .is_some_and(|e| !e.is_folder_like());
            if file {
                return self.view_focused(true);
            }
        }
        self.viewer_closed_command()
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
        // A search result's viewer leaves the panel's focus alone (W3a).
        if !self.results_parked() {
            self.panel_mut(side).list_mut().focus_id(&row);
        }
        self.active = side;
        let mut effects = vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::info(format!("Viewer closed: {name}"))),
        ];
        // The search results it came from come back (W3a).
        effects.extend(self.unpark_results());
        effects
    }

    fn viewer_command(&mut self, command: ViewerCommand) -> Vec<Effect> {
        use ViewerCommand as C;
        if self.viewing.is_none() {
            return if command == C::Hex {
                self.viewer_command_closed(&ids::VIEWER_HEX)
            } else {
                self.viewer_closed_command()
            };
        }
        if command == C::Hex && self.viewer().is_some_and(|v| v.other().is_some()) {
            return self.swap_faces();
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
                // Text diroxide made is not read from the place again
                // (Read text in picture, alpha 4).
                if self
                    .viewer()
                    .is_some_and(|v| v.is_made() && !v.is_preview())
                {
                    let line =
                        "This text was read from the picture. Read text in picture reads it again.";
                    return vec![self.say(Message::info(line))];
                }
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

    /// The Hex command over a preview's words (lane VP): the bytes come to
    /// the front in hex mode, "Hex mode: the bytes of report.pdf"; over
    /// those bytes it brings the words back, their title said.
    fn swap_faces(&mut self) -> Vec<Effect> {
        let Some(viewing) = self.viewing.as_mut() else {
            return Vec::new();
        };
        let Some(mut other) = viewing.viewer.take_other() else {
            return Vec::new();
        };
        let line = if other.is_preview() {
            other.title().to_owned()
        } else {
            other.show_hex();
            crate::viewer::hex_face_line(other.name())
        };
        let shown = std::mem::replace(&mut viewing.viewer, other);
        viewing.viewer.set_other(shown);
        self.viewer_said(vec![Message::info(line)])
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
