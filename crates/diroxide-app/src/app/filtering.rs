//! The panel filter on the one filter language (W3a, plan commit 35):
//! the filter prompt (the Filter command), its pause and count line, the
//! title's read-back, Say filter, stash and restore, the quick kind
//! toggles, and clearing.
//!
//! What is said, one line a change (research/alpha3-plan.md, "The
//! screen-reader path"):
//!
//! - Typing narrows the panel silently; after the pause
//!   (`filter_pause_ms`, 400 milliseconds by default) one line says the
//!   count first and the first match: "37 matches, first report.pdf",
//!   "No matches". The focus is on the first match.
//! - A filter that cannot be read leaves the panel as it was; after the
//!   pause its first sentence is said ("Filter problem: size needs a
//!   number"), and after one more pause its second ("Word 3. Try size
//!   over 10 MB."), each once.
//! - What probably was not meant (the lints) is said once a prompt,
//!   after the count.
//! - A filter that reads files asks first, on Enter: "Read the contents
//!   of 12 files to filter? No is the default. y or n".
//! - Clearing says "Filter off, 120 shown".
//!
//! The pause runs on the app's clock: a frontend calls [`App::app_tick`]
//! when [`App::app_wants_tick`] says so.

use std::collections::HashSet;
use std::sync::Arc;

use diroxide_core::filter::{CompletionContext, Filter, FilterError, LintContext, Unknown};
use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, Effect, Explain, Message, Move, Place, Redraw, RequestId, Side, Timestamp,
};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::millis_between;
use crate::panel::ContentAnswers;
use crate::request::{Request, SharedClock};
use crate::state;

/// The filter's state beyond the panels: the pause after typing, the
/// lints said in this prompt, the stashed filters, the filter a kind
/// toggle replaced, and a content read on its way.
#[derive(Clone, Debug, Default)]
pub(super) struct FilterState {
    pause: Option<Pause>,
    said_lints: HashSet<String>,
    stash: [Vec<Filter>; 2],
    before_kind: [Option<Option<Filter>>; 2],
    reading: Option<Reading>,
}

/// The pause after the last key in the filter prompt.
#[derive(Clone, Debug)]
struct Pause {
    side: Side,
    text: String,
    at: Timestamp,
    /// Whether the first sentence of a problem was said, so the second
    /// comes next.
    second: bool,
}

/// A content read for a panel's filter, on its way.
#[derive(Clone, Debug)]
struct Reading {
    request: RequestId,
    side: Side,
    place: Place,
    filter: Filter,
    previous: Option<Filter>,
}

/// What the runtime read for a panel's content filter: the files whose
/// contents matched, and what could not be decided, by reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterContents {
    /// The names of the files that matched the whole filter.
    pub matched: Vec<String>,
    /// The entries that could not be decided, by reason, with counts.
    pub unknown: Vec<(Unknown, u64)>,
}

/// A command of the filter's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Say,
    Stash,
    Restore,
    OnlyFolders,
    OnlyFiles,
}

const ROUTES: [(CommandId, Do); 5] = [
    (ids::SAY_FILTER, Do::Say),
    (ids::FILTER_STASH, Do::Stash),
    (ids::FILTER_RESTORE, Do::Restore),
    (ids::ONLY_FOLDERS, Do::OnlyFolders),
    (ids::ONLY_FILES, Do::OnlyFiles),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

/// The unknown part of a summary, meaning first: "3 unknown, binary file
/// not searched", or with several reasons "5 unknown, 3 binary file not
/// searched, 2 online-only file not read".
pub(super) fn unknown_words(unknown: &[(Unknown, u64)]) -> Option<String> {
    let mut reasons: Vec<(Unknown, u64)> =
        unknown.iter().filter(|(_, n)| *n > 0).copied().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    match reasons.as_slice() {
        [] => None,
        [(why, n)] => Some(format!("{n} unknown, {why}")),
        many => {
            let total: u64 = many.iter().map(|(_, n)| n).sum();
            let parts: Vec<String> = many.iter().map(|(why, n)| format!("{n} {why}")).collect();
            Some(format!("{total} unknown, {}", parts.join(", ")))
        }
    }
}

impl App {
    /// The filter settings: `filter_case` and `week_starts_on`.
    pub(super) fn filter_options(&self) -> diroxide_core::filter::FilterOptions {
        crate::settings::filter_options(&self.settings)
    }

    /// Reads filter text with the owner's settings; empty text is no
    /// filter.
    pub(super) fn read_filter(&self, text: &str) -> Result<Option<Filter>, FilterError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        Filter::parse_with(text, &self.filter_options()).map(Some)
    }

    /// Runs a command of the filter's own: Say filter, stash and restore,
    /// and the kind toggles; `None` when the command is another's.
    pub(super) fn filtering_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        if action == Do::Say {
            return Some(self.say_filter());
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        let side = self.active;
        Some(match action {
            Do::Say => Vec::new(),
            Do::Stash => self.stash_filter(side),
            Do::Restore => self.restore_filter(side),
            Do::OnlyFolders => self.only_kind(side, "folder", "Folders only"),
            Do::OnlyFiles => self.only_kind(side, "file", "Files only"),
        })
    }

    /// The Filter command: with a `pattern` argument the filter is set at
    /// once; otherwise the prompt opens with the panel's filter in it, and
    /// narrows the panel as you type.
    pub(super) fn filter_prompt(&mut self, command: &Command) -> Vec<Effect> {
        let side = self.active;
        if let Some(text) = command.text("pattern") {
            let text = text.to_owned();
            return self.filter_set(side, &text, None);
        }
        let previous = self.panel(side).filter().cloned();
        let text = previous
            .as_ref()
            .map(|f| f.text().to_owned())
            .unwrap_or_default();
        let field = EntryField::new("Filter", CompletionSource::Filter)
            .with_text(text)
            .with_history(state::keys::FILTER);
        self.filtering.pause = None;
        self.filtering.said_lints.clear();
        self.open_dialog(Dialog::edit(field), Purpose::Filter(side, previous))
    }

    /// Puts a filter on a panel with the facts the app holds (the
    /// bookmarks, and contents when they were read); the focus goes to
    /// the first match.
    fn put_filter(&mut self, side: Side, filter: Option<Filter>, content: Option<ContentAnswers>) {
        let clock = Arc::clone(&self.clock);
        let bookmarks = filter.as_ref().map(|_| self.organize.bookmarks.clone());
        let panel = &mut self.panels[side.index()];
        panel.set_filter_and_facts(filter, bookmarks, content, clock.as_ref());
        panel.list_mut().move_focus(Move::Home);
    }

    /// The text in the filter prompt changed: the panel narrows silently
    /// when the text reads, and the pause starts again.
    pub(super) fn filter_typed(&mut self, side: Side, text: &str) -> Vec<Effect> {
        let now = self.now();
        self.filtering.pause = Some(Pause {
            side,
            text: text.to_owned(),
            at: now,
            second: false,
        });
        let mut effects = Vec::new();
        match self.read_filter(text) {
            Ok(None) => {
                self.put_filter(side, None, None);
                effects.push(self.redraw_panel(side));
            }
            Ok(Some(filter)) if filter.panel_problem().is_none() && !filter.reads_content() => {
                self.put_filter(side, Some(filter), None);
                effects.push(self.redraw_panel(side));
            }
            // The panel stays as it was until the text reads.
            _ => {}
        }
        if self.settings.filter_pause_ms == 0 {
            effects.extend(self.pause_over());
        }
        effects
    }

    /// How soon the frontend should call [`App::app_tick`], in
    /// milliseconds: while the filter prompt waits for its pause; never
    /// otherwise. The pause runs on the app's clock.
    pub fn app_wants_tick(&self) -> Option<u64> {
        let pause = self.filtering.pause.as_ref()?;
        let waited = millis_between(pause.at, self.now());
        Some(self.settings.filter_pause_ms.saturating_sub(waited))
    }

    /// Gives the app's own timers their time: after the filter's pause,
    /// its count line or its problem is said.
    pub fn app_tick(&mut self) -> Vec<Effect> {
        let due = self
            .filtering
            .pause
            .as_ref()
            .is_some_and(|p| millis_between(p.at, self.now()) >= self.settings.filter_pause_ms);
        if !due {
            return Vec::new();
        }
        let effects = self.pause_over();
        self.follow_cursor();
        self.mark_requests(effects)
    }

    /// The text in the filter prompt now, when the prompt is on top.
    fn prompt_text(&self) -> Option<(Side, String)> {
        let side = match self.opens.last().map(|o| &o.purpose) {
            Some(Purpose::Filter(side, _)) => *side,
            _ => return None,
        };
        match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => Some((side, field.text().to_owned())),
            _ => None,
        }
    }

    /// The pause is over: the count line, or the problem's next sentence.
    fn pause_over(&mut self) -> Vec<Effect> {
        let Some(pause) = self.filtering.pause.take() else {
            return Vec::new();
        };
        if self.prompt_text() != Some((pause.side, pause.text.clone())) {
            return Vec::new();
        }
        let problem = match self.read_filter(&pause.text) {
            Err(error) => Some(error),
            Ok(Some(filter)) => filter.panel_problem(),
            Ok(None) => None,
        };
        if let Some(error) = problem {
            if pause.second {
                return vec![self.say(Message::info(error.next_step()))];
            }
            self.filtering.pause = Some(Pause {
                at: self.now(),
                second: true,
                ..pause
            });
            return vec![self.say(Message::info(error.line()))];
        }
        let filter = self.read_filter(&pause.text).ok().flatten();
        if filter
            .as_ref()
            .is_some_and(|f| f.reads_content() && !self.panel(pause.side).filter_contents_read())
        {
            let line = "Content test: Enter asks to read files";
            return vec![self.say(Message::info(line))];
        }
        let mut line = match &filter {
            None => self.filter_off_line(pause.side),
            Some(_) => self.match_line(pause.side),
        };
        if let Some(filter) = &filter {
            for lint in self.new_lints(filter) {
                line.push_str(&format!(". {}", lint.trim_end_matches('.')));
            }
        }
        vec![self.say(Message::info(line))]
    }

    /// The lints of a filter not yet said in this prompt.
    fn new_lints(&mut self, filter: &Filter) -> Vec<String> {
        let labels = self.label_names();
        let context = LintContext { labels: &labels };
        let lints = filter.lints(&context, self.clock.as_ref());
        let mut fresh = Vec::new();
        for lint in lints {
            let said = lint.sentences();
            if self.filtering.said_lints.insert(said.clone()) {
                fresh.push(said);
            }
        }
        fresh
    }

    /// The count line after the pause, the count first: "37 matches,
    /// first report.pdf", "1 match, report.pdf", "No matches", then what
    /// could not be decided.
    fn match_line(&self, side: Side) -> String {
        let panel = self.panel(side);
        let (shown, _) = panel.filter_counts();
        let first = panel.list().rows().first().map(|r| r.label.clone());
        let mut line = match (shown, first) {
            (0, _) | (_, None) => "No matches".to_owned(),
            (1, Some(first)) => format!("1 match, {first}"),
            (n, Some(first)) => format!("{n} matches, first {first}"),
        };
        if let Some(unknown) = panel.filter_tally().unknown_sentence() {
            line.push_str(&format!(". {unknown}"));
        }
        line
    }

    /// "37 of 120 shown".
    fn shown_words(&self, side: Side) -> String {
        let (shown, all) = self.panel(side).filter_counts();
        format!("{shown} of {all} shown")
    }

    /// "Filter off, 120 shown".
    fn filter_off_line(&self, side: Side) -> String {
        let shown = self.panel(side).list().len();
        format!("Filter off, {shown} shown")
    }

    /// Enter in the filter prompt: a filter that reads is set; one that
    /// cannot is said and the prompt stays open; one that reads files
    /// asks first.
    pub(super) fn filter_enter(
        &mut self,
        side: Side,
        text: &str,
        previous: Option<Filter>,
    ) -> Vec<Effect> {
        let problem = match self.read_filter(text) {
            Err(error) => Some(error),
            Ok(Some(filter)) => filter.panel_problem(),
            Ok(None) => None,
        };
        if let Some(error) = problem {
            // The first sentence at once; the second after the pause.
            self.filtering.pause = Some(Pause {
                side,
                text: text.to_owned(),
                at: self.now(),
                second: true,
            });
            return vec![self.say(Message::error(error.line()))];
        }
        self.filtering.pause = None;
        let mut effects = self.close_top();
        effects.extend(self.filter_set(side, text, previous));
        effects
    }

    /// Sets a panel's filter from its text, as Enter in the prompt does,
    /// and says the result.
    fn filter_set(&mut self, side: Side, text: &str, previous: Option<Filter>) -> Vec<Effect> {
        let filter = match self.read_filter(text) {
            Ok(filter) => filter,
            Err(error) => return vec![self.say(Message::error(error.sentences()))],
        };
        let Some(filter) = filter else {
            return self.clear_filter_on(side);
        };
        if let Some(problem) = filter.panel_problem() {
            return vec![self.say(Message::error(problem.sentences()))];
        }
        self.remember(state::keys::FILTER, text);
        if filter.reads_content() {
            return self.ask_filter_contents(side, filter, previous);
        }
        self.put_filter(side, Some(filter.clone()), None);
        let mut line = format!("Filter on, {}", self.shown_words(side));
        if let Some(unknown) = self.panel(side).filter_tally().unknown_sentence() {
            line.push_str(&format!(". {unknown}"));
        }
        for lint in self.new_lints(&filter) {
            line.push_str(&format!(". {}", lint.trim_end_matches('.')));
        }
        vec![
            self.redraw_panel(side),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    /// Escape in the filter prompt: the filter it opened with comes back.
    pub(super) fn filter_escaped(&mut self, side: Side, previous: Option<Filter>) -> Vec<Effect> {
        self.filtering.pause = None;
        let had = previous.is_some();
        self.put_filter(side, previous, None);
        let line = if had {
            format!("Filter as it was, {}", self.shown_words(side))
        } else {
            let shown = self.panel(side).list().len();
            format!("No filter, {shown} shown")
        };
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Clear filter: "Filter off, 120 shown".
    pub(super) fn filter_clear(&mut self) -> Vec<Effect> {
        let side = self.active;
        self.clear_filter_on(side)
    }

    /// Clears a panel's filter and says so.
    pub(super) fn clear_filter_on(&mut self, side: Side) -> Vec<Effect> {
        self.put_filter(side, None, None);
        self.filtering.before_kind[side.index()] = None;
        let line = self.filter_off_line(side);
        vec![
            self.redraw_panel(side),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    /// Say filter: the filter read back in one sentence, the date words
    /// with their real days ("modified last week, October 4 to 10"), the
    /// exact byte counts after it. In the filter prompt it reads the text
    /// being typed.
    pub(super) fn say_filter(&mut self) -> Vec<Effect> {
        let (side, text) = match self.prompt_text() {
            Some(found) => found,
            None if !self.dialogs.is_empty() => {
                return vec![self.say(Message::status("Close this dialog first."))];
            }
            None => {
                let side = self.active;
                let text = self
                    .panel(side)
                    .filter()
                    .map(|f| f.text().to_owned())
                    .unwrap_or_default();
                (side, text)
            }
        };
        let line = match self.read_filter(&text) {
            Err(error) => error.sentences(),
            Ok(None) => {
                let shown = self.panel(side).list().len();
                format!("No filter, {shown} shown")
            }
            Ok(Some(filter)) => {
                let read = filter.read_back(self.clock.as_ref());
                let mut line = read.sentence().to_owned();
                let details = read.details_line();
                if !details.is_empty() {
                    line.push_str(&format!(". {}", details.trim_end_matches('.')));
                }
                if self
                    .panel(side)
                    .filter()
                    .is_some_and(|f| f.text() == filter.text())
                {
                    line.push_str(&format!(". {}", self.shown_words(side)));
                }
                line
            }
        };
        vec![self.say(Message::info(line))]
    }

    /// What Say Status says in the filter prompt: the text, how it reads,
    /// and the count.
    pub(super) fn filter_prompt_status(&self) -> Option<String> {
        let (side, text) = self.prompt_text()?;
        let mut line = if text.is_empty() {
            "Filter, empty".to_owned()
        } else {
            format!("Filter: {text}")
        };
        match self.read_filter(&text) {
            Ok(Some(filter)) => {
                let read = filter.read_back(self.clock.as_ref());
                line.push_str(&format!(". Reads as {}", read.sentence()));
            }
            Ok(None) => {}
            Err(error) => line.push_str(&format!(". {}", error.line())),
        }
        line.push_str(&format!(". {}", self.shown_words(side)));
        Some(line)
    }

    /// Stash filter: the filter is kept and taken off, the newest last.
    fn stash_filter(&mut self, side: Side) -> Vec<Effect> {
        let Some(filter) = self.panel(side).filter().cloned() else {
            return vec![self.say(Message::status("No filter to stash"))];
        };
        self.filtering.stash[side.index()].push(filter);
        self.put_filter(side, None, None);
        let stashed = self.filtering.stash[side.index()].len() as u64;
        let line = format!(
            "Filter stashed, {} shown. {} stashed",
            self.panel(side).list().len(),
            stashed
        );
        vec![
            self.redraw_panel(side),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    /// Restore filter: the newest stashed filter comes back.
    fn restore_filter(&mut self, side: Side) -> Vec<Effect> {
        let Some(filter) = self.filtering.stash[side.index()].pop() else {
            return vec![self.say(Message::status("No stashed filter"))];
        };
        let sentence = filter.read_back(self.clock.as_ref()).sentence().to_owned();
        self.put_filter(side, Some(filter), None);
        let line = format!("Filter restored, {}: {sentence}", self.shown_words(side));
        vec![
            self.redraw_panel(side),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    /// Folders only and Files only: on, the panel shows only that kind;
    /// again, the filter before it comes back.
    fn only_kind(&mut self, side: Side, kind: &str, words: &str) -> Vec<Effect> {
        let text = format!("kind {kind}");
        let on = self.panel(side).filter().is_some_and(|f| f.text() == text);
        let line = if on {
            let before = self.filtering.before_kind[side.index()].take().flatten();
            self.put_filter(side, before, None);
            match self.panel(side).filter() {
                Some(_) => format!("{words} off, {}", self.shown_words(side)),
                None => format!("{words} off, {} shown", self.panel(side).list().len()),
            }
        } else {
            let current = self.panel(side).filter().cloned();
            if self.filtering.before_kind[side.index()].is_none() {
                self.filtering.before_kind[side.index()] = Some(current);
            }
            let filter = self.read_filter(&text).ok().flatten();
            self.put_filter(side, filter, None);
            format!("{words}, {}", self.shown_words(side))
        };
        vec![
            self.redraw_panel(side),
            Effect::Redraw(Redraw::TitleLine),
            self.say(Message::info(line)),
        ]
    }

    /// The question before a filter reads files, starting on No.
    fn ask_filter_contents(
        &mut self,
        side: Side,
        filter: Filter,
        previous: Option<Filter>,
    ) -> Vec<Effect> {
        let panel = self.panel(side);
        let hidden = panel.shows_hidden() || filter.shows_hidden();
        let files = panel
            .all_entries()
            .iter()
            .filter(|e| (hidden || !e.hidden) && !e.is_folder_like())
            .count() as u64;
        let question = with_default_no(Message::question(&format!(
            "Read the contents of {} to filter",
            count(files, "file", "files")
        )));
        let sentence = filter.read_back(self.clock.as_ref()).sentence().to_owned();
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing: vec![format!("Filter: {sentence}")],
                on_yes: Command::new(ids::YES),
            },
            Purpose::FilterContents {
                side,
                filter: Box::new(filter),
                previous: previous.map(Box::new),
            },
        )
    }

    /// Yes to the content question: the runtime reads the files on a
    /// worker.
    pub(super) fn read_filter_contents(
        &mut self,
        side: Side,
        filter: Filter,
        previous: Option<Filter>,
    ) -> Vec<Effect> {
        let request = self.ids.request();
        let place = self.panel(side).place().clone();
        let hidden = self.panel(side).shows_hidden();
        self.filtering.reading = Some(Reading {
            request,
            side,
            place: place.clone(),
            filter: filter.clone(),
            previous,
        });
        self.request(Request::FilterContents {
            request,
            side,
            place,
            filter: Box::new(filter),
            hidden,
            clock: SharedClock(Arc::clone(&self.clock)),
        });
        vec![self.say(Message::info("Reading the files for the filter"))]
    }

    /// No to the content question: nothing is read.
    pub(super) fn filter_contents_declined(
        &mut self,
        side: Side,
        previous: Option<Filter>,
    ) -> Vec<Effect> {
        self.put_filter(side, previous, None);
        let line = "Nothing was read. The filter is as it was.";
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// The runtime read the files for a panel's content filter.
    pub(super) fn filter_contents_arrived(
        &mut self,
        request: RequestId,
        result: Result<FilterContents, Message>,
    ) -> Vec<Effect> {
        let Some(reading) = self.filtering.reading.take_if(|r| r.request == request) else {
            return Vec::new();
        };
        let side = reading.side;
        if self.panel(side).place() != &reading.place {
            return Vec::new();
        }
        match result {
            Ok(contents) => {
                let answers = ContentAnswers {
                    matched: contents.matched.into_iter().collect(),
                    undecided: std::collections::HashMap::new(),
                };
                self.put_filter(side, Some(reading.filter), Some(answers));
                let mut line = format!("Filter on, {}", self.shown_words(side));
                if let Some(unknown) = unknown_words(&contents.unknown) {
                    line.push_str(&format!(". {unknown}"));
                }
                vec![
                    self.redraw_panel(side),
                    Effect::Redraw(Redraw::TitleLine),
                    self.say(Message::info(line)),
                ]
            }
            Err(message) => {
                self.put_filter(side, reading.previous, None);
                vec![self.redraw_panel(side), self.say(message)]
            }
        }
    }

    /// The filter language's completion for the filter prompt: the words
    /// the parser expects at the cursor, with where the word being
    /// completed starts.
    pub(super) fn filter_completions(
        &self,
        text: &str,
        cursor: usize,
    ) -> (usize, Vec<crate::entry::Candidate>) {
        let labels = self.label_names();
        let types: Vec<String> = self.types.types().iter().map(|t| t.short.clone()).collect();
        let context = CompletionContext {
            labels: &labels,
            types: &types,
        };
        let found = diroxide_core::filter::complete(text, cursor, &self.filter_options(), &context);
        let candidates = found
            .candidates
            .into_iter()
            .map(|c| crate::entry::Candidate::new(c.text, c.kind))
            .collect();
        (found.start, candidates)
    }

    /// A panel's filter as the title line says it: "filter: size over 1
    /// MB and name matches *.log", the date words with their days.
    pub(super) fn filter_title(&self, side: Side) -> Option<String> {
        let filter = self.panel(side).filter()?;
        let sentence = filter.read_back(self.clock.as_ref()).sentence().to_owned();
        Some(format!(
            "filter: {}",
            diroxide_core::names::display_name(&sentence)
        ))
    }
}
