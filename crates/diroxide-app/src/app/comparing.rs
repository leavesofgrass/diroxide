//! Compare files in the app (W3a, plan commit 40, over W3d's crate):
//! which two, the verdict and the change list, one change in full, the
//! unified view, Swap, the viewer at a change, compare properties, and
//! select the differences.
//!
//! - **Which two**: two marked files in the active panel, the first as
//!   before; otherwise the focused file here against the file of the same
//!   name in the other panel, else the other panel's focused file. Read
//!   from the other panel's file (before) to this panel's (after), as
//!   question 40 settled. "Comparing report.txt here with old.txt there"
//!   is said before any work.
//! - **The report**: the verdict line (at most 40 cells), the counts, the
//!   direction, what was ignored and the files' facts, said once; then the
//!   change list, one row a change ("Changed line 40, was: total due").
//!   F7 and Shift+F7, or N and P while letters are on, go to the next and
//!   previous change; Enter says one change in full; U opens the unified
//!   view, whose Enter opens the viewer at its line; S swaps the
//!   direction; the view key opens the viewer at the change; the context
//!   menu's key lists every action.
//! - A file over the text limit asks: the first part as text, byte by
//!   byte, or cancel, the focus on cancel.

use std::sync::Arc;

use diroxide_core::format::{count, human_size};
use diroxide_core::{
    Command, CommandId, Effect, Entry, EntryKind, Key, KeyCode, Layer, ListModel, ListRow, Message,
    Move, Place, Redraw, RequestId, Side, Timestamp,
};
use diroxide_diff::{
    ChangeKind, CompareOptions, Context, FileComparison, FileOptions, Ignore, LargeFiles,
    LineAlgorithm, Outcome, Sides, UnifiedKind, verdict_sentence,
};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::jobs::{SIGHTED_INTERVAL_MS, millis_between};
use crate::keymap::Lookup as KeyLookup;
use crate::panel::place_leaf;
use crate::request::Request;

/// One MiB, for the text limit.
const MIB: u64 = 1024 * 1024;

/// A compare on its way.
#[derive(Clone, Debug)]
struct Wait {
    request: RequestId,
    pair: Pair,
    said_at: Timestamp,
}

/// The two files of a compare, before and after, with their names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pair {
    pub before: Place,
    pub after: Place,
    /// Whether the two came from the panels (there and here), not two
    /// marked files of one panel.
    pub panels: bool,
}

impl Pair {
    fn names(&self) -> (String, String) {
        (place_leaf(&self.before), place_leaf(&self.after))
    }
}

/// A finished compare, as its dialogs show it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Compared {
    pub result: FileComparison,
    pub pair: Pair,
    pub swapped: bool,
    pub options: FileOptions,
}

impl Compared {
    /// The sides as the report says them.
    fn sides(&self) -> Sides {
        let sides = if self.pair.panels {
            Sides::panels()
        } else {
            let (b, a) = self.pair.names();
            Sides::files(&b, &a)
        };
        if self.swapped { sides.swapped() } else { sides }
    }

    /// The names, before first, as read now.
    fn names(&self) -> (String, String) {
        let (b, a) = self.pair.names();
        if self.swapped { (a, b) } else { (b, a) }
    }

    /// The report before the change list, one string, the verdict with
    /// the whole names for speech.
    fn report(&self) -> String {
        let (before, after) = self.names();
        let sides = self.sides();
        let mut lines = self.result.summary(&before, &after, &sides, &self.options);
        let (first, second) = if sides.before_first() {
            (&before, &after)
        } else {
            (&after, &before)
        };
        if let Some(verdict) = lines.first_mut() {
            *verdict = verdict_sentence(self.result.verdict(), first, second);
        }
        join_sentences(&lines)
    }

    /// The verdict line, for the list's title.
    fn title(&self) -> String {
        let (before, after) = self.names();
        self.result
            .summary(&before, &after, &self.sides(), &self.options)
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// The change list.
    fn list(&self) -> ListModel {
        let rows = self
            .result
            .rows(&self.options)
            .into_iter()
            .enumerate()
            .map(|(i, row)| ListRow::new(i.to_string(), row))
            .collect();
        let mut list = ListModel::new(self.title()).with_rows(rows);
        list.set_empty_text("No changes to list");
        list.set_help(
            "Up and Down or the next and previous change keys move by change, Enter says one in full, the context menu's key lists every action, Escape closes.",
        );
        list
    }

    /// The place a change's line is in, and the line, counted from 1: the
    /// after file's for a change or an addition, the before file's for a
    /// removal.
    fn change_line(&self, index: usize) -> Option<(Place, u64)> {
        let Outcome::Text { content, .. } = &self.result.outcome else {
            return None;
        };
        let change = content.text().changes().get(index)?;
        let (before, after) = if self.swapped {
            (&self.pair.after, &self.pair.before)
        } else {
            (&self.pair.before, &self.pair.after)
        };
        let lines = change.lines();
        let place = if change.kind == ChangeKind::Removed {
            before
        } else {
            after
        };
        Some((place.clone(), lines.start as u64 + 1))
    }
}

/// Lines said as one: each ends with its own period.
fn join_sentences(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| {
            if line.ends_with(['.', '?']) {
                line.clone()
            } else {
                format!("{line}.")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the app keeps for compare: the one on its way.
#[derive(Clone, Debug, Default)]
pub(super) struct CompareState {
    wait: Option<Wait>,
}

/// The compare options from the settings (`[compare]`).
pub(super) fn file_options(settings: &diroxide_config::Settings) -> FileOptions {
    use diroxide_config::{CompareAlgorithm, LargeFilePolicy};
    let c = settings.compare;
    FileOptions {
        compare: CompareOptions {
            ignore: Ignore {
                line_endings: c.ignore_line_endings,
                trailing_space: c.ignore_trailing_space,
                space_amount: c.ignore_space_amount,
                all_space: c.ignore_all_space,
                blank_lines: c.ignore_blank_lines,
                case: c.ignore_case,
            },
            algorithm: match c.algorithm {
                CompareAlgorithm::Patience => LineAlgorithm::Patience,
                CompareAlgorithm::Histogram => LineAlgorithm::Histogram,
                CompareAlgorithm::Myers => LineAlgorithm::Myers,
            },
            deadline: std::time::Duration::from_millis(c.deadline_ms),
        },
        max_text_bytes: c.text_limit_mib.saturating_mul(MIB),
        large: match c.large_files {
            LargeFilePolicy::Ask => LargeFiles::Ask,
            LargeFilePolicy::Head => LargeFiles::Head,
            LargeFilePolicy::Bytes => LargeFiles::Bytes,
        },
    }
}

/// A command of compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Files,
    Properties,
    Differences,
    Next,
    Previous,
}

const ROUTES: [(CommandId, Do); 5] = [
    (ids::COMPARE_FILES, Do::Files),
    (ids::COMPARE_PROPERTIES, Do::Properties),
    (ids::SELECT_DIFFERENCES, Do::Differences),
    (ids::COMPARE_NEXT, Do::Next),
    (ids::COMPARE_PREVIOUS, Do::Previous),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// Whether a compare's change list is on the screen, so the key bar
    /// shows its keys.
    pub fn compare_shown(&self) -> bool {
        matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Compare(_))
        )
    }

    /// Runs a compare command; `None` when the command is another's.
    pub(super) fn compare_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        if matches!(action, Do::Next | Do::Previous) {
            return Some(self.step_change(action == Do::Next));
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(match action {
            Do::Files => self.compare_files(),
            Do::Properties => self.compare_properties(),
            Do::Differences => self.select_differences(),
            Do::Next | Do::Previous => Vec::new(),
        })
    }

    /// The two files to compare, by the which-two rule, or why not.
    fn compare_pair(&self) -> Result<Pair, String> {
        let side = self.active;
        let here = self.panel(side);
        let there = self.panel(side.other());
        let marked: Vec<&Entry> = here
            .selected_entries()
            .into_iter()
            .filter(|e| !e.is_folder_like())
            .collect();
        if here.list().selected_count() == 2 && marked.len() == 2 {
            let before = here
                .place()
                .join(&marked[0].name)
                .map_err(|e| e.to_string())?;
            let after = here
                .place()
                .join(&marked[1].name)
                .map_err(|e| e.to_string())?;
            return Ok(Pair {
                before,
                after,
                panels: false,
            });
        }
        let Some(focused) = here.focused_entry().filter(|e| !e.is_folder_like()) else {
            return Err("Mark two files, or focus a file in each panel, to compare.".to_owned());
        };
        let after = here
            .place()
            .join(&focused.name)
            .map_err(|e| e.to_string())?;
        let twin = there
            .all_entries()
            .iter()
            .find(|e| e.name == focused.name && !e.is_folder_like())
            .or_else(|| there.focused_entry().filter(|e| !e.is_folder_like()));
        let Some(other) = twin else {
            return Err(format!(
                "No file to compare {} with in the other panel. Focus one there, or mark two here.",
                focused.display_name()
            ));
        };
        let before = there.place().join(&other.name).map_err(|e| e.to_string())?;
        Ok(Pair {
            before,
            after,
            panels: true,
        })
    }

    /// Compare files: "Comparing report.txt here with old.txt there",
    /// then the compare on a worker.
    fn compare_files(&mut self) -> Vec<Effect> {
        let pair = match self.compare_pair() {
            Ok(pair) => pair,
            Err(line) => return vec![self.say(Message::status(line))],
        };
        let options = file_options(&self.settings);
        self.start_compare(pair, options)
    }

    fn start_compare(&mut self, pair: Pair, options: FileOptions) -> Vec<Effect> {
        let (before, after) = pair.names();
        let line = if pair.panels {
            format!("Comparing {after} here with {before} there")
        } else {
            format!("Comparing {before} with {after}")
        };
        let request = self.ids.request();
        self.comparing.wait = Some(Wait {
            request,
            pair: pair.clone(),
            said_at: self.now(),
        });
        self.request(Request::Compare {
            request,
            before: pair.before,
            after: pair.after,
            options,
        });
        vec![self.say(Message::info(line))]
    }

    /// How far a compare got: "Comparing, 40 percent", at the progress
    /// interval.
    pub(super) fn compare_progress(
        &mut self,
        request: RequestId,
        done: u64,
        total: Option<u64>,
    ) -> Vec<Effect> {
        let now = self.now();
        let pace = if self.screen_reader {
            self.settings
                .progress_interval_seconds
                .max(1)
                .saturating_mul(1000)
        } else {
            SIGHTED_INTERVAL_MS
        };
        let Some(wait) = self
            .comparing
            .wait
            .as_mut()
            .filter(|w| w.request == request)
        else {
            return Vec::new();
        };
        if millis_between(wait.said_at, now) < pace {
            return Vec::new();
        }
        wait.said_at = now;
        let line = match total {
            Some(total) if total > 0 => {
                let percent = (u128::from(done) * 100 / u128::from(total)).min(99);
                format!("Comparing, {percent} percent")
            }
            _ => format!("Comparing, {} read", human_size(done)),
        };
        vec![self.say(Message::progress(line))]
    }

    /// The compare ended: the report, then the change list; a file too
    /// large asks first.
    pub(super) fn compared(
        &mut self,
        request: RequestId,
        result: Result<Box<FileComparison>, Message>,
    ) -> Vec<Effect> {
        let Some(wait) = self.comparing.wait.take_if(|w| w.request == request) else {
            return Vec::new();
        };
        let result = match result {
            Ok(result) => *result,
            Err(message) => return vec![self.say(message)],
        };
        let options = file_options(&self.settings);
        let compared = Compared {
            result,
            pair: wait.pair,
            swapped: false,
            options,
        };
        if compared.result.outcome == Outcome::TooLarge {
            return self.ask_large(compared);
        }
        self.show_compared(compared, None)
    }

    /// The change list's dialog, the report said once.
    fn show_compared(&mut self, compared: Compared, said: Option<String>) -> Vec<Effect> {
        let list = compared.list();
        let line = said.unwrap_or_else(|| compared.report());
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::COMPARE_FILES),
            },
            Purpose::Compare(Box::new(compared)),
            Message::info(line),
        )
    }

    /// A file over the text limit: the first part, byte by byte, or
    /// cancel, the focus on cancel.
    fn ask_large(&mut self, compared: Compared) -> Vec<Effect> {
        let limit = compared.options.max_text_bytes / MIB;
        let rows = vec![
            ListRow::new("head", format!("Compare the first {limit} MiB as text")),
            ListRow::new("bytes", "Compare byte by byte"),
            ListRow::new("cancel", "Cancel, compare nothing"),
        ];
        let mut choices = ListModel::new("Too large").with_rows(rows);
        choices.move_focus(Move::End);
        let (before, after) = compared.names();
        let lines = compared
            .result
            .summary(&before, &after, &compared.sides(), &compared.options);
        self.open_dialog(
            Dialog::Question { lines, choices },
            Purpose::CompareLarge(Box::new(compared.pair)),
        )
    }

    /// The choice for a file too large.
    pub(super) fn large_chosen(&mut self, pair: Pair, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let mut options = file_options(&self.settings);
        options.large = match row {
            "head" => LargeFiles::Head,
            "bytes" => LargeFiles::Bytes,
            _ => {
                effects.push(self.say(Message::info("Nothing was compared.")));
                return effects;
            }
        };
        effects.extend(self.start_compare(pair, options));
        effects
    }

    /// The compare's own keys in its lists, before the list's: N and P
    /// (while letters are on), S to swap, U for the unified view, the view
    /// key for the viewer at the change, the context menu's key for every
    /// action.
    pub(super) fn compare_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let purpose = self.opens.last().map(|o| o.purpose.clone())?;
        let in_unified = matches!(purpose, Purpose::Unified(_));
        let (Purpose::Compare(compared) | Purpose::Unified(compared)) = purpose else {
            return None;
        };
        let letters = self.settings.single_letters != diroxide_config::SingleLetters::Off;
        if letters && key.mods.is_empty() {
            match key.code {
                KeyCode::Char('n') if !in_unified => return Some(self.step_change(true)),
                KeyCode::Char('p') if !in_unified => return Some(self.step_change(false)),
                KeyCode::Char('s') if !in_unified => return Some(self.swap_compare(*compared)),
                KeyCode::Char('u') if !in_unified => return Some(self.unified_view(*compared)),
                _ => {}
            }
        }
        let command = match self.keymap.lookup(&[Layer::Panel], key) {
            KeyLookup::Command(id) => id,
            KeyLookup::NoMatch => return None,
        };
        if command == ids::VIEW {
            return Some(if in_unified {
                self.unified_view_line(*compared)
            } else {
                self.view_change(*compared)
            });
        }
        if command == ids::CONTEXT_MENU && !in_unified {
            return Some(self.compare_menu());
        }
        None
    }

    /// The next or previous change in the change list.
    fn step_change(&mut self, forward: bool) -> Vec<Effect> {
        if !matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Compare(_))
        ) {
            return vec![self.say(Message::status("No compare is open."))];
        }
        let movement = if forward { Move::Down } else { Move::Up };
        let moved = match self.dialogs.last_mut() {
            Some((_, Dialog::Choose { list, .. })) => list.move_focus(movement),
            _ => false,
        };
        if !moved {
            let edge = if forward {
                "No change after this one"
            } else {
                "No change before this one"
            };
            return vec![self.say(Message::status(edge))];
        }
        let line = self
            .dialogs
            .last()
            .map(|(_, d)| d.focus_line())
            .unwrap_or_default();
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
    }

    /// Enter on a change: before and after in full, as a list to read.
    pub(super) fn change_detail(&mut self, compared: Compared, row: &str) -> Vec<Effect> {
        let Ok(index) = row.parse::<usize>() else {
            return Vec::new();
        };
        let lines = match &compared.result.outcome {
            Outcome::Text { content, .. } => content.detail(index, &compared.options.compare),
            _ => compared
                .result
                .rows(&compared.options)
                .get(index)
                .cloned()
                .into_iter()
                .collect(),
        };
        if lines.is_empty() {
            return Vec::new();
        }
        let total = compared.result.rows(&compared.options).len();
        let title = format!("Change {} of {total}", index + 1);
        let said = format!("{title}. {}", join_sentences(&lines));
        self.open_dialog_saying(
            Dialog::Read { title, lines },
            Purpose::Read,
            Message::info(said),
        )
    }

    /// Swap: the same compare read the other way; nothing is compared
    /// again, and the direction sentence says the new way.
    fn swap_compare(&mut self, mut compared: Compared) -> Vec<Effect> {
        compared.result = compared.result.swapped();
        compared.swapped = !compared.swapped;
        let direction = compared.sides().direction();
        let list = compared.list();
        if let Some((_, Dialog::Choose { list: open, .. })) = self.dialogs.last_mut() {
            let focus = open.focus_index();
            *open = list;
            if let Some(focus) = focus {
                open.move_focus(Move::To(focus));
            }
        }
        if let Some(open) = self.opens.last_mut() {
            open.purpose = Purpose::Compare(Box::new(compared));
        }
        vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::info(format!("Swapped. {direction}"))),
        ]
    }

    /// The unified view: every change with the same lines around it, one
    /// row a line ("Added 52: total due now"); Enter opens the viewer at
    /// a row's line.
    fn unified_view(&mut self, compared: Compared) -> Vec<Effect> {
        let Outcome::Text { content, .. } = &compared.result.outcome else {
            return vec![self.say(Message::status("A unified view is for text files."))];
        };
        let context = Context::Lines(usize::try_from(self.settings.compare.context).unwrap_or(2));
        let rows: Vec<ListRow> = content
            .unified(context)
            .iter()
            .enumerate()
            .map(|(i, row)| ListRow::new(i.to_string(), row.spoken()))
            .collect();
        let (_, after) = compared.names();
        let mut list = ListModel::new(format!("Unified view, {after}")).with_rows(rows);
        list.set_empty_text("No changes");
        list.set_help("Up and Down move by line, Enter or the view key opens the viewer at the line, Escape goes back.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::COMPARE_FILES),
            },
            Purpose::Unified(Box::new(compared)),
        )
    }

    /// Enter in the unified view: the viewer at the row's line, in the
    /// file that has it.
    pub(super) fn unified_view_line(&mut self, compared: Compared) -> Vec<Effect> {
        let Some(row) = self.top_row_id().and_then(|r| r.parse::<usize>().ok()) else {
            return Vec::new();
        };
        let Outcome::Text { content, .. } = &compared.result.outcome else {
            return Vec::new();
        };
        let context = Context::Lines(usize::try_from(self.settings.compare.context).unwrap_or(2));
        let rows = content.unified(context);
        let Some(found) = rows.get(row) else {
            return Vec::new();
        };
        let (before, after) = if compared.swapped {
            (&compared.pair.after, &compared.pair.before)
        } else {
            (&compared.pair.before, &compared.pair.after)
        };
        let (place, line) = match (found.kind, found.after_line, found.before_line) {
            (UnifiedKind::Removed, _, Some(line)) => (before.clone(), line),
            (_, Some(line), _) => (after.clone(), line),
            (_, None, Some(line)) => (before.clone(), line),
            _ => (after.clone(), 1),
        };
        self.view_compared(&compared, place, line as u64)
    }

    /// The view key on a change: the viewer at its line.
    fn view_change(&mut self, compared: Compared) -> Vec<Effect> {
        let index = self
            .top_row_id()
            .and_then(|r| r.parse::<usize>().ok())
            .unwrap_or(0);
        match compared.change_line(index) {
            Some((place, line)) => self.view_compared(&compared, place, line),
            None => {
                let (_, after) = (compared.pair.before.clone(), compared.pair.after.clone());
                self.view_compared(&compared, after, 1)
            }
        }
    }

    /// The viewer on one of the compared files at a line, the compare's
    /// lists aside until it closes; Next and Previous file step between
    /// the two.
    fn view_compared(&mut self, compared: &Compared, place: Place, line: u64) -> Vec<Effect> {
        let siblings = vec![
            crate::viewer::Sibling::new(compared.pair.before.clone(), compared.result.before.size),
            crate::viewer::Sibling::new(compared.pair.after.clone(), compared.result.after.size),
        ];
        let size = if place == compared.pair.before {
            compared.result.before.size
        } else {
            compared.result.after.size
        };
        self.searching_view_at(place, size, siblings, line)
    }

    /// The compare's actions as a list (the context menu's key).
    fn compare_menu(&mut self) -> Vec<Effect> {
        let rows = vec![
            ListRow::new("next", "Next change").with_details([self.key(&ids::COMPARE_NEXT)]),
            ListRow::new("previous", "Previous change")
                .with_details([self.key(&ids::COMPARE_PREVIOUS)]),
            ListRow::new("detail", "One change in full").with_details(["Enter".to_owned()]),
            ListRow::new("unified", "Unified view").with_details(["U".to_owned()]),
            ListRow::new("swap", "Swap the direction").with_details(["S".to_owned()]),
            ListRow::new("view", "View at the change").with_details([self.key(&ids::VIEW)]),
        ];
        let mut list = ListModel::new("Actions on the compare").with_rows(rows);
        list.set_help("Up and Down choose, Enter runs it, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::CONTEXT_MENU),
            },
            Purpose::CompareMenu,
        )
    }

    /// An action from the compare's list, with the change list on top
    /// again.
    pub(super) fn compare_action(&mut self, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let Some(Purpose::Compare(compared)) = self.opens.last().map(|o| o.purpose.clone()) else {
            return effects;
        };
        let more = match row {
            "next" => self.step_change(true),
            "previous" => self.step_change(false),
            "detail" => match self.top_row_id() {
                Some(row) => self.change_detail(*compared, &row),
                None => Vec::new(),
            },
            "unified" => self.unified_view(*compared),
            "swap" => self.swap_compare(*compared),
            "view" => self.view_change(*compared),
            _ => Vec::new(),
        };
        effects.extend(more);
        effects
    }

    /// Compare properties: the facts that differ between the two files,
    /// from the listings in hand, and those that agree.
    fn compare_properties(&mut self) -> Vec<Effect> {
        let pair = match self.compare_pair() {
            Ok(pair) => pair,
            Err(line) => return vec![self.say(Message::status(line))],
        };
        let entry_of = |place: &Place| -> Option<Entry> {
            let name = place.name()?;
            [Side::Left, Side::Right].iter().find_map(|side| {
                let panel = self.panel(*side);
                (panel.place() == &place.parent()?)
                    .then(|| panel.all_entries().iter().find(|e| e.name == name).cloned())
                    .flatten()
            })
        };
        let (Some(before), Some(after)) = (entry_of(&pair.before), entry_of(&pair.after)) else {
            return vec![self.say(Message::status("The two files are not listed."))];
        };
        let (here, there) = if pair.panels {
            ("here", "there")
        } else {
            ("in the second", "in the first")
        };
        let clock = Arc::clone(&self.clock);
        let when = |t: Option<Timestamp>| {
            t.map_or_else(
                || "not known".to_owned(),
                |t| diroxide_core::clock::describe_time(t, clock.as_ref()),
            )
        };
        let facts: Vec<(&str, String, String)> = vec![
            (
                "Size",
                after
                    .size
                    .map_or_else(|| "not known".to_owned(), human_size),
                before
                    .size
                    .map_or_else(|| "not known".to_owned(), human_size),
            ),
            ("Modified", when(after.modified), when(before.modified)),
            ("Created", when(after.created), when(before.created)),
            (
                "Type",
                after
                    .type_name
                    .clone()
                    .unwrap_or_else(|| after.kind.word().to_owned()),
                before
                    .type_name
                    .clone()
                    .unwrap_or_else(|| before.kind.word().to_owned()),
            ),
            (
                "Permissions",
                after
                    .attributes
                    .map(|a| a.short_words(after.kind))
                    .unwrap_or_default(),
                before
                    .attributes
                    .map(|a| a.short_words(before.kind))
                    .unwrap_or_default(),
            ),
            (
                "Owner",
                after.owner.clone().unwrap_or_default(),
                before.owner.clone().unwrap_or_default(),
            ),
        ];
        let mut lines = Vec::new();
        let mut same = Vec::new();
        for (name, a, b) in facts {
            if a == b {
                if !a.is_empty() {
                    same.push(name.to_lowercase());
                }
            } else {
                lines.push(format!("{name}: {a} {here}, {b} {there}"));
            }
        }
        let differ = lines.len();
        if !same.is_empty() {
            lines.push(format!("Same: {}", same.join(", ")));
        }
        let (b, a) = pair.names();
        let title = format!("Properties of {a} and {b}");
        let said = if differ == 0 {
            format!("{title}: the same. {}", lines.join(". "))
        } else {
            format!(
                "{title}: {} differ. {}",
                count(differ as u64, "property", "properties"),
                lines.join(". ")
            )
        };
        self.open_dialog_saying(
            Dialog::Read { title, lines },
            Purpose::Read,
            Message::info(said),
        )
    }

    /// Select the differences: in both panels, the entries only on one
    /// side, and those on both that differ in size or are newer, from the
    /// listings in hand. "Marked 9: 4 only here, 2 only there, 3
    /// different".
    fn select_differences(&mut self) -> Vec<Effect> {
        let side = self.active;
        let other = side.other();
        let here: Vec<Entry> = self.panel(side).entries().to_vec();
        let there: Vec<Entry> = self.panel(other).entries().to_vec();
        let find = |list: &[Entry], name: &str| list.iter().find(|e| e.name == name).cloned();
        let mut mark_here = std::collections::HashSet::new();
        let mut mark_there = std::collections::HashSet::new();
        let (mut only_here, mut only_there, mut different) = (0u64, 0u64, 0u64);
        for entry in &here {
            match find(&there, &entry.name) {
                None => {
                    only_here += 1;
                    mark_here.insert(entry.name.clone());
                }
                Some(twin) if entry.kind == EntryKind::File || twin.kind == EntryKind::File => {
                    let sizes = entry.size != twin.size;
                    let newer_here = entry.modified > twin.modified;
                    let newer_there = twin.modified > entry.modified;
                    if sizes || newer_here || newer_there {
                        different += 1;
                        if sizes || newer_here {
                            mark_here.insert(entry.name.clone());
                        }
                        if sizes || newer_there {
                            mark_there.insert(twin.name.clone());
                        }
                    }
                }
                Some(_) => {}
            }
        }
        for entry in &there {
            if find(&here, &entry.name).is_none() {
                only_there += 1;
                mark_there.insert(entry.name.clone());
            }
        }
        self.keep_selection(side);
        self.keep_selection(other);
        self.panel_mut(side)
            .list_mut()
            .select_matching(|row| mark_here.contains(&row.id));
        self.panel_mut(other)
            .list_mut()
            .select_matching(|row| mark_there.contains(&row.id));
        let total = mark_here.len() + mark_there.len();
        let line = if total == 0 {
            "No differences: both panels hold the same names, sizes and times".to_owned()
        } else {
            format!(
                "Marked {total}: {only_here} only here, {only_there} only there, {different} different"
            )
        };
        vec![
            self.redraw_panel(side),
            self.redraw_panel(other),
            self.say(Message::info(line)),
        ]
    }
}
