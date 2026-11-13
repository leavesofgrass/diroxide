//! Compare folders and synchronize in the app (alpha 5, lane SA): the
//! compare and its result, the plan view, the run, over the engine half
//! in `diroxide_ops::sync`.
//!
//! The flow, every step a dialog under the contract (named, Escape
//! closes, the focus returns):
//!
//! 1. Compare folders: a list of criteria, "By size and time, within 2
//!    seconds" first; Enter on a By row compares, Enter on a setting turns
//!    it on or off. The compare is a job: its progress is said at the
//!    interval, Escape on the panel cancels it and its rows say "Not
//!    compared".
//! 2. The result: its name is the count line that adds up; the rows,
//!    status first, differences only, problems first; the actions at the
//!    end (Synchronize, show the same rows, show one status, the
//!    differences as a list, Compare again). Enter on a Different row
//!    compares the two files.
//! 3. Synchronize: the modes in words, then the plan view: its name is the
//!    plan's summary; each row starts with its action ("Copy there:
//!    plan.md, only here", "Trash there: old.txt, only there"); Enter on a
//!    row lists what can change on it (leave out, take back, a direction,
//!    the suggestion). "Synchronize now" asks the plugins' hooks, then the
//!    question, which starts on No and lists the removals first.
//! 4. The run: one job, one undo record, every place read again first;
//!    its report is any job's.
//!
//! Rows say "here" and "there", sentences "this panel" and "the other
//! panel", never left and right. Every row and line puts its meaning in
//! its first 40 cells; no color or arrow carries meaning.

use std::sync::Arc;

use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, Effect, EntryKind, JobId, JobKind, Key, KeyCode, ListModel, ListRow,
    Message, Place, Redraw, Side,
};
use diroxide_ops::sync::{
    CompareBy, CompareRow, Comparison, RowState, RowStatus, SyncAction, SyncMode, SyncPlan, at,
    plan_sync,
};

use super::organize::Wants;
use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::jobs::{FinishedJob, SIGHTED_INTERVAL_MS};
use crate::panel::place_leaf;
use crate::request::Request;
use crate::sync_ui::{CompareOptions, HeldComparison, SyncAsk, SyncNote};

/// Said when a setting the compare used changed before the run: 34
/// cells.
pub(crate) const OPTIONS_CHANGED: &str = "Compare again: the options changed";
/// Said when a plan has nothing to run: 23 cells.
pub(crate) const NOTHING_TO_DO: &str = "Nothing to synchronize.";
/// Said when the question is answered No: 25 cells.
pub(crate) const NOTHING_DONE: &str = "Nothing was synchronized.";
/// Said when the criteria list is closed: 21 cells.
pub(crate) const NOTHING_COMPARED: &str = "Nothing was compared.";

/// The two folders and what the compare looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Setup {
    /// The panel that asked: its folder is "here".
    side: Side,
    /// This panel's folder.
    here: Place,
    /// The other panel's folder.
    there: Place,
    /// The criteria.
    options: CompareOptions,
}

/// A compare's result as the result list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResultView {
    setup: Setup,
    held: HeldComparison,
    /// Whether the same rows are listed too.
    same: bool,
    /// The one status listed, when the owner chose one.
    only: Option<RowState>,
}

/// A plan as the plan view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlanView {
    result: ResultView,
    plan: SyncPlan,
    /// Whether the same rows are listed too.
    same: bool,
}

/// What a dialog of compare and synchronize is for, hosted under
/// `Wants::Sync`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SyncWants {
    /// The criteria list before a compare.
    Criteria(Box<Setup>),
    /// The result list.
    Result(Box<ResultView>),
    /// The modes, over the result.
    Modes(Box<ResultView>),
    /// The plan view, over the result.
    Plan(Box<PlanView>),
    /// What can change on one row of the plan, over the plan view.
    RowActions(Box<PlanView>, usize),
    /// The question before the run, over the plan view.
    Confirm(Box<PlanView>, JobId),
}

impl SyncWants {
    /// The purpose's name for the window's test of every purpose.
    pub(super) fn purpose(&self) -> &'static str {
        match self {
            SyncWants::Criteria(_) => "SyncCriteria",
            SyncWants::Result(_) => "SyncResult",
            SyncWants::Modes(_) => "SyncModes",
            SyncWants::Plan(_) => "SyncPlan",
            SyncWants::RowActions(..) => "SyncRowActions",
            SyncWants::Confirm(..) => "SyncConfirm",
        }
    }
}

/// A compare or synchronize command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    CompareFolders,
    Synchronize,
    TowardHere,
    TowardThere,
    CompareAgain,
}

const ROUTES: [(CommandId, Do); 5] = [
    (ids::COMPARE_FOLDERS, Do::CompareFolders),
    (ids::SYNCHRONIZE, Do::Synchronize),
    (ids::SYNC_TOWARD_HERE, Do::TowardHere),
    (ids::SYNC_TOWARD_THERE, Do::TowardThere),
    (ids::COMPARE_AGAIN, Do::CompareAgain),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

// The rows' ids.
const BY_TIME: &str = "by-time";
const BY_SIZE: &str = "by-size";
const BY_CONTENT: &str = "by-content";
const SUBFOLDERS: &str = "subfolders";
const HIDDEN: &str = "hidden";
const HOUR: &str = "hour";
const SYNC: &str = "a:sync";
const SAME: &str = "a:same";
const ONLY: &str = "a:only";
const AS_LIST: &str = "a:list";
const AGAIN: &str = "a:again";
const RUN: &str = "a:run";
const MODE: &str = "a:mode";
const NO_MODE: &str = "m:none";
const TOGGLE: &str = "toggle";
const TOWARD_THERE: &str = "there";
const TOWARD_HERE: &str = "here";
const RESET: &str = "reset";

/// "on" or "off".
fn on(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// "Show 1 same row", "Show 1,185 same rows".
fn show_same(n: u64) -> String {
    let rows = if n == 1 { "row" } else { "rows" };
    format!("Show {} same {rows}", diroxide_core::format::grouped(n))
}

/// "2 seconds", "1 second".
fn seconds(n: u64) -> String {
    count(n, "second", "seconds")
}

/// The criteria list: the three ways to compare, then the settings.
fn criteria_list(setup: &Setup) -> ListModel {
    let options = &setup.options;
    let mark = |by: CompareBy| (options.by == by).then(|| "chosen".to_owned());
    let by = |id: &str, label: String, of: CompareBy| {
        let row = ListRow::new(id, label);
        match mark(of) {
            Some(chosen) => row.with_details([chosen]),
            None => row,
        }
    };
    let rows = vec![
        by(
            BY_TIME,
            format!(
                "By size and time, within {}",
                seconds(options.tolerance_seconds)
            ),
            CompareBy::SizeAndTime,
        ),
        by(
            BY_SIZE,
            "By size only, times not looked at".to_owned(),
            CompareBy::Size,
        ),
        by(
            BY_CONTENT,
            "By contents, reads both files".to_owned(),
            CompareBy::Content,
        ),
        ListRow::new(
            SUBFOLDERS,
            format!("Subfolders: {}", on(options.subfolders)),
        ),
        ListRow::new(HIDDEN, format!("Hidden items: {}", on(options.hidden))),
        ListRow::new(
            HOUR,
            format!("Ignore a 1 hour shift: {}", on(options.ignore_one_hour)),
        ),
    ];
    let mut list = ListModel::new("Compare folders:").with_rows(rows);
    list.set_help("Enter on a By row compares. Enter on a setting turns it on or off.");
    list
}

/// The rows the result lists, in its order: problems, differences, then
/// the skipped and the same when shown; one status only when chosen.
fn result_order(view: &ResultView) -> Vec<usize> {
    let rows = &view.held.0.rows;
    let rank = |row: &CompareRow| -> Option<u8> {
        if let Some(only) = view.only {
            return (row.state == only).then_some(0);
        }
        match row.state {
            state if state.is_problem() => Some(0),
            RowState::Same if view.same => Some(3),
            RowState::Same => None,
            RowState::Skipped if view.same => Some(2),
            RowState::Skipped => None,
            _ => Some(1),
        }
    };
    let mut order: Vec<(u8, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| rank(row).map(|rank| (rank, index)))
        .collect();
    order.sort();
    order.into_iter().map(|(_, index)| index).collect()
}

/// The statuses a result can be narrowed to, in the order Enter steps
/// through them: those its rows have, besides the same and the skipped.
fn statuses(result: &Comparison) -> Vec<RowState> {
    [
        RowState::OnlyLeft,
        RowState::OnlyRight,
        RowState::Different,
        RowState::TypeDiffers,
        RowState::Unreadable,
        RowState::NotCompared,
    ]
    .into_iter()
    .filter(|state| result.rows.iter().any(|row| row.state == *state))
    .collect()
}

/// The result list: the rows, then the actions, then how it was compared.
fn result_list(view: &ResultView) -> ListModel {
    let result = &view.held.0;
    let separator = result.separator();
    let mut rows: Vec<ListRow> = result_order(view)
        .into_iter()
        .map(|index| ListRow::new(format!("r{index}"), result.rows[index].line(separator)))
        .collect();
    let counts = result.counts();
    if counts.differences() > 0 {
        rows.push(ListRow::new(SYNC, "Synchronize: plan the changes"));
    }
    if counts.same > 0 {
        let label = if view.same {
            "Hide the same rows".to_owned()
        } else {
            show_same(counts.same)
        };
        rows.push(ListRow::new(SAME, label));
    }
    if statuses(result).len() > 1 || view.only.is_some() {
        let label = match view.only {
            Some(state) => format!("Showing only: {}", state.words().to_lowercase()),
            None => "Showing: all differences".to_owned(),
        };
        rows.push(ListRow::new(ONLY, label).with_details(["Enter shows another status"]));
    }
    if counts.differences() > 0 {
        rows.push(ListRow::new(AS_LIST, "Open the differences as a list"));
    }
    rows.push(ListRow::new(AGAIN, "Compare again"));
    for (i, note) in result.notes().into_iter().enumerate() {
        rows.push(ListRow::new(format!("n{i}"), note));
    }
    for (i, (place, why)) in result.left_out.iter().enumerate() {
        let name = diroxide_core::names::spell_out(&place.to_string()).into_owned();
        rows.push(ListRow::new(
            format!("o{i}"),
            format!("Not compared: {name}, {why}"),
        ));
    }
    let mut list = ListModel::new(result.count_line()).with_rows(rows);
    list.set_help(
        "Enter on a Different row compares the two files. The actions are at the end of the list.",
    );
    list
}

/// The modes, in the order the owner reaches for them, and Compare only.
fn modes_list() -> ListModel {
    let modes = [
        SyncMode::UpdateThere,
        SyncMode::MirrorThere,
        SyncMode::BothWays,
        SyncMode::UpdateHere,
        SyncMode::MirrorHere,
    ];
    let mut rows: Vec<ListRow> = modes
        .into_iter()
        .map(|mode| {
            let row = ListRow::new(mode_id(mode), mode.words());
            if mode.removes() {
                let side = if mode == SyncMode::MirrorThere {
                    Side::Right
                } else {
                    Side::Left
                };
                row.with_details([format!(
                    "trashes {} what is not on the other side",
                    at(side)
                )])
            } else {
                row
            }
        })
        .collect();
    rows.push(ListRow::new(NO_MODE, "Compare only"));
    let mut list = ListModel::new("Synchronize how:").with_rows(rows);
    list.set_help("Enter shows the plan; nothing changes until you answer its question.");
    list
}

fn mode_id(mode: SyncMode) -> String {
    format!("m:{mode:?}")
}

fn mode_of(id: &str) -> Option<SyncMode> {
    SyncMode::ALL.into_iter().find(|mode| mode_id(*mode) == id)
}

/// One plan row in words, its action first, in the short form (question
/// 135): "Copy there: plan.md, only here" (30), "Replace there:
/// notes.txt, newer here" (36), "Trash there: old.txt, only there" (32),
/// "Delete for good there: old.txt, no trash there". The engine's own
/// line for the rest: "Skip: thing, newer there", "Type differs: photos,
/// folder here, file there", "Left out: report.pdf, would copy there".
pub(super) fn plan_line(plan: &SyncPlan, index: usize) -> String {
    let (Some(row), Some(status)) = (plan.rows.get(index), plan.status(index)) else {
        return String::new();
    };
    if !matches!(status, RowStatus::Acts | RowStatus::Caution) {
        return plan.line(index);
    }
    let separator = if plan.left.local_path().is_some_and(|p| !p.starts_with('/')) {
        '\\'
    } else {
        '/'
    };
    let place = row.row.where_words(separator);
    match row.action {
        SyncAction::RemoveFrom(side) if plan.facts(side).trash => {
            format!("Trash {}: {place}, only {}", at(side), at(side))
        }
        SyncAction::RemoveFrom(side) => {
            format!(
                "Delete for good {}: {place}, no trash {}",
                at(side),
                at(side)
            )
        }
        SyncAction::CopyTo(to) if row.row.entry(to).is_some() && !row.row.is_folder() => {
            let newer = row.row.newer() == Some(to);
            let mut head = if newer {
                "Replace newer".to_owned()
            } else {
                "Replace".to_owned()
            };
            if !plan.facts(to).trash {
                head.push_str(" for good");
            }
            let mut parts = vec![format!("{head} {}: {place}", at(to))];
            parts.extend(row.row.reasons());
            if !plan.facts(to).trash {
                parts.push(format!("no trash {}", at(to)));
            }
            parts.join(", ")
        }
        _ => plan.line(index),
    }
}

/// The plan view: the rows in the listing's order, then the actions,
/// then the notes.
fn plan_list(view: &PlanView) -> ListModel {
    let plan = &view.plan;
    let mut rows: Vec<ListRow> = plan
        .order(view.same)
        .into_iter()
        .map(|index| ListRow::new(format!("p{index}"), plan_line(plan, index)))
        .collect();
    let counts = plan.counts();
    let acting = counts.acting();
    let run = if acting == 0 {
        NOTHING_TO_DO.trim_end_matches('.').to_owned()
    } else {
        format!("Synchronize now: {}", count(acting, "item", "items"))
    };
    rows.push(ListRow::new(RUN, run));
    rows.push(ListRow::new(MODE, format!("Mode: {}", plan.mode.words())));
    if counts.same > 0 {
        let label = if view.same {
            "Hide the same rows".to_owned()
        } else {
            show_same(counts.same)
        };
        rows.push(ListRow::new(SAME, label));
    }
    rows.push(ListRow::new(AGAIN, "Compare again"));
    for (i, note) in plan.notes().into_iter().enumerate() {
        rows.push(ListRow::new(format!("n{i}"), note));
    }
    // The title is the mode and the count, inside 40 cells: "Update the
    // other panel: 2 changes" (33); the whole summary is said as the
    // plan opens (lane AS).
    let title = format!(
        "{}: {}",
        plan.mode.words(),
        count(acting, "change", "changes")
    );
    let mut list = ListModel::new(title).with_rows(rows);
    list.set_help(
        "Enter on a row lists what can change on it. Synchronize now is after the rows; nothing changes before its question.",
    );
    list
}

/// What can change on one plan row, each a row; empty when nothing can.
fn row_actions(plan: &SyncPlan, index: usize) -> Vec<ListRow> {
    let Some(row) = plan.rows.get(index) else {
        return Vec::new();
    };
    let problem = matches!(plan.status(index), Some(RowStatus::Problem(_)));
    let mut rows = Vec::new();
    if row.action != SyncAction::Skip && !problem {
        let label = if row.included {
            "Leave out"
        } else {
            "Take back in"
        };
        rows.push(ListRow::new(TOGGLE, label));
    }
    let both_folders = row.row.is_folder() && row.row.left.is_some() && row.row.right.is_some();
    if !row.row.state.is_problem() && !both_folders {
        if row.row.left.is_some() && row.action != SyncAction::CopyTo(Side::Right) {
            rows.push(ListRow::new(TOWARD_THERE, "Copy toward the other panel"));
        }
        if row.row.right.is_some() && row.action != SyncAction::CopyTo(Side::Left) {
            rows.push(ListRow::new(TOWARD_HERE, "Copy toward this panel"));
        }
    }
    if row.action != row.suggested || !row.included {
        rows.push(ListRow::new(RESET, "Put back the suggestion"));
    }
    rows
}

/// The progress interval: the setting's in screen-reader mode, half a
/// second otherwise.
fn interval_ms(app: &App) -> u64 {
    if app.screen_reader {
        app.settings
            .progress_interval_seconds
            .max(1)
            .saturating_mul(1000)
    } else {
        SIGHTED_INTERVAL_MS
    }
}

/// A row id's number: "p12" is 12.
fn row_index(id: &str, prefix: char) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
}

impl App {
    /// The compare and synchronize commands; `None` for any other.
    pub(super) fn sync_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::CompareFolders => self.compare_folders(command),
            Do::Synchronize => self.synchronize(command),
            Do::TowardHere | Do::TowardThere | Do::CompareAgain => self.sync_list_action(command),
        })
    }

    /// The two panels' folders, this panel's first, or why they cannot be
    /// compared.
    fn sync_setup(&self) -> Result<Setup, String> {
        let side = self.active;
        let here = self.panel(side).place().clone();
        let there = self.panel(side.other()).place().clone();
        if here == there {
            return Err(format!(
                "Cannot compare: both panels show {}",
                place_leaf(&here)
            ));
        }
        Ok(Setup {
            side,
            here,
            there,
            options: CompareOptions::from_settings(&self.settings),
        })
    }

    /// The top dialog's purpose, when it is one of these.
    fn top_sync(&self) -> Option<SyncWants> {
        match self.opens.last().map(|open| &open.purpose) {
            Some(Purpose::Organize(wants)) => match wants.as_ref() {
                Wants::Sync(wants) => Some(wants.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Replaces the top dialog's list and purpose, keeping the focus on
    /// the same row, or near where it was.
    fn replace_sync_top(&mut self, list: ListModel, wants: SyncWants) {
        if let Some((_, Dialog::Choose { list: open, .. })) = self.dialogs.last_mut() {
            let focus = open.focused().map(|row| row.id.clone());
            let position = open.focus_index();
            *open = list;
            let kept = focus.is_some_and(|id| open.focus_id(&id));
            if !kept && let Some(position) = position {
                let last = open.len().saturating_sub(1);
                open.move_focus(diroxide_core::Move::To(position.min(last)));
            }
        }
        if let Some(open) = self.opens.last_mut() {
            open.purpose = Purpose::Organize(Box::new(Wants::Sync(wants)));
        }
    }

    /// Opens a list of these, saying `line` instead of its name when
    /// given.
    fn open_sync_list(
        &mut self,
        list: ListModel,
        wants: SyncWants,
        line: Option<String>,
    ) -> Vec<Effect> {
        let dialog = Dialog::Choose {
            list,
            on_enter: Command::new(ids::ACCEPT),
        };
        let purpose = Purpose::Organize(Box::new(Wants::Sync(wants)));
        match line {
            Some(line) => self.open_dialog_saying(dialog, purpose, Message::info(line)),
            None => self.open_dialog(dialog, purpose),
        }
    }

    /// Closes every dialog of these on top of the stack.
    fn close_sync_dialogs(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        while self.top_sync().is_some() {
            effects.extend(self.close_top());
        }
        effects
    }

    /// Compare folders: the two panels' places, then the criteria.
    fn compare_folders(&mut self, _command: &Command) -> Vec<Effect> {
        if !self.dialogs.is_empty() {
            return vec![self.say(Message::status("Close this dialog first."))];
        }
        let setup = match self.sync_setup() {
            Ok(setup) => setup,
            Err(line) => return vec![self.say(Message::info(line))],
        };
        let list = criteria_list(&setup);
        self.open_sync_list(list, SyncWants::Criteria(Box::new(setup)), None)
    }

    /// Starts a compare as a job: "Comparing photos here with backup
    /// there".
    fn start_folder_compare(&mut self, setup: Setup, then_plan: bool) -> Vec<Effect> {
        let job = self.next_job_id();
        let now = self.now();
        let line = format!(
            "Comparing {} here with {} there",
            place_leaf(&setup.here),
            place_leaf(&setup.there)
        );
        self.jobs.begin(job, JobKind::Compare, line.clone(), now);
        let every_ms = interval_ms(self);
        self.request(Request::Sync(Box::new(SyncAsk::Compare {
            job,
            here: setup.here,
            there: setup.there,
            options: setup.options,
            every_ms,
            then_plan,
        })));
        // Said with its period (lane AS); the jobs line keeps none.
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info(format!("{line}."))),
        ]
    }

    /// Synchronize: from the panel, a compare with the settings' options
    /// and then the modes; over a result, the modes.
    fn synchronize(&mut self, _command: &Command) -> Vec<Effect> {
        match self.top_sync() {
            Some(SyncWants::Result(view)) => self.open_modes(*view),
            Some(SyncWants::Plan(_)) => {
                vec![self.say(Message::status("Synchronize now is after the plan's rows."))]
            }
            Some(_) => vec![self.say(Message::status("Close this dialog first."))],
            None if !self.dialogs.is_empty() => {
                vec![self.say(Message::status("Close this dialog first."))]
            }
            None => match self.sync_setup() {
                Ok(setup) => self.start_folder_compare(setup, true),
                Err(line) => vec![self.say(Message::info(line))],
            },
        }
    }

    /// A command of the result or the plan view: a row's direction,
    /// Compare again.
    fn sync_list_action(&mut self, command: &Command) -> Vec<Effect> {
        let toward = if command.id == ids::SYNC_TOWARD_HERE {
            Some(Side::Left)
        } else if command.id == ids::SYNC_TOWARD_THERE {
            Some(Side::Right)
        } else {
            None
        };
        let mut effects = Vec::new();
        if matches!(self.top_sync(), Some(SyncWants::RowActions(..))) && toward.is_some() {
            effects.extend(self.close_top());
        }
        match (toward, self.top_sync()) {
            (Some(to), Some(SyncWants::Plan(view))) => {
                let index = match self.dialog_list().and_then(|list| list.focused()) {
                    Some(row) => row_index(&row.id, 'p'),
                    None => None,
                };
                let Some(index) = index else {
                    effects.push(self.say(Message::status("Move to a row of the plan first.")));
                    return effects;
                };
                effects.extend(self.change_row(*view, index, |plan| plan.set_direction(index, to)));
            }
            (Some(_), _) => {
                effects.push(self.say(Message::status("Open a synchronize plan first.")));
            }
            (None, Some(SyncWants::Result(view) | SyncWants::Modes(view))) => {
                effects.extend(self.compare_again(view.setup));
            }
            (None, Some(SyncWants::Plan(view) | SyncWants::RowActions(view, _))) => {
                effects.extend(self.compare_again(view.result.setup));
            }
            (None, _) => {
                effects.push(self.say(Message::status("Compare folders first.")));
            }
        }
        effects
    }

    /// The list of the dialog on top, when it is a choice list.
    fn dialog_list(&self) -> Option<&ListModel> {
        match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => Some(list),
            _ => None,
        }
    }

    /// Compare again: the views close and the same two folders are
    /// compared with the settings as they are now.
    fn compare_again(&mut self, mut setup: Setup) -> Vec<Effect> {
        let mut effects = self.close_sync_dialogs();
        let now = CompareOptions::from_settings(&self.settings);
        setup.options.tolerance_seconds = now.tolerance_seconds;
        setup.options.ignore_one_hour = now.ignore_one_hour;
        effects.extend(self.start_folder_compare(setup, false));
        effects
    }

    /// Enter or a typed answer in one of these dialogs.
    pub(super) fn sync_accept(
        &mut self,
        wants: SyncWants,
        row: Option<String>,
        _text: &str,
    ) -> Vec<Effect> {
        let Some(row) = row else {
            return Vec::new();
        };
        match wants {
            SyncWants::Criteria(setup) => self.criteria_chosen(*setup, &row),
            SyncWants::Result(view) => self.result_chosen(*view, &row),
            SyncWants::Modes(view) => self.mode_chosen(*view, &row),
            SyncWants::Plan(view) => self.plan_chosen(*view, &row),
            SyncWants::RowActions(view, index) => self.row_action_chosen(*view, index, &row),
            SyncWants::Confirm(..) => Vec::new(),
        }
    }

    /// Enter in the criteria: a By row compares, a setting turns.
    fn criteria_chosen(&mut self, mut setup: Setup, row: &str) -> Vec<Effect> {
        let by = match row {
            BY_TIME => Some(CompareBy::SizeAndTime),
            BY_SIZE => Some(CompareBy::Size),
            BY_CONTENT => Some(CompareBy::Content),
            _ => None,
        };
        if let Some(by) = by {
            setup.options.by = by;
            let mut effects = self.close_top();
            effects.extend(self.start_folder_compare(setup, false));
            return effects;
        }
        match row {
            SUBFOLDERS => setup.options.subfolders = !setup.options.subfolders,
            HIDDEN => setup.options.hidden = !setup.options.hidden,
            HOUR => setup.options.ignore_one_hour = !setup.options.ignore_one_hour,
            _ => return Vec::new(),
        }
        let list = criteria_list(&setup);
        let said = list
            .rows()
            .iter()
            .find(|r| r.id == row)
            .map(|r| r.label.clone())
            .unwrap_or_default();
        self.replace_sync_top(list, SyncWants::Criteria(Box::new(setup)));
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))]
    }

    /// The result list, opened over whatever is open.
    fn show_result(&mut self, view: ResultView) -> Vec<Effect> {
        let list = result_list(&view);
        let line = view.held.0.count_line();
        self.open_sync_list(list, SyncWants::Result(Box::new(view)), Some(line))
    }

    /// Enter in the result list.
    fn result_chosen(&mut self, mut view: ResultView, row: &str) -> Vec<Effect> {
        match row {
            SYNC => return self.open_modes(view),
            AGAIN => return self.compare_again(view.setup),
            AS_LIST => return self.differences_as_list(&view),
            SAME | ONLY => {
                if row == SAME {
                    view.same = !view.same;
                    view.only = None;
                } else {
                    let all = statuses(&view.held.0);
                    view.only = match view.only {
                        None => all.first().copied(),
                        Some(state) => all
                            .iter()
                            .position(|s| *s == state)
                            .and_then(|at| all.get(at + 1).copied()),
                    };
                    view.same = false;
                }
                let list = result_list(&view);
                let shown = result_order(&view).len() as u64;
                let said = match view.only {
                    Some(state) => format!(
                        "Showing only {}: {}",
                        state.words().to_lowercase(),
                        count(shown, "row", "rows")
                    ),
                    None if view.same => format!("Showing all {}", count(shown, "row", "rows")),
                    None => format!("Showing all differences: {}", count(shown, "row", "rows")),
                };
                self.replace_sync_top(list, SyncWants::Result(Box::new(view)));
                return vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
            }
            _ => {}
        }
        let Some(index) = row_index(row, 'r') else {
            // A note: said again in full.
            let line = self
                .dialog_list()
                .and_then(|list| list.focused())
                .map(|r| r.label.clone())
                .unwrap_or_default();
            return vec![self.say(Message::info(line))];
        };
        let result = Arc::clone(&view.held.0);
        let Some(compared) = result.rows.get(index) else {
            return Vec::new();
        };
        let files = compared.state == RowState::Different
            && compared
                .left
                .as_ref()
                .is_some_and(|e| e.kind == EntryKind::File)
            && compared
                .right
                .as_ref()
                .is_some_and(|e| e.kind == EntryKind::File);
        if !files {
            let line = compared.line(result.separator());
            return vec![self.say(Message::info(line))];
        }
        let (Ok(after), Ok(before)) = (
            result.place_of(compared, Side::Left),
            result.place_of(compared, Side::Right),
        ) else {
            return Vec::new();
        };
        // The file compare's own start (lane AS), so its keys, the viewer
        // at the change among them, have their files.
        let options = super::comparing::file_options(&self.settings);
        let pair = super::comparing::Pair {
            before,
            after,
            panels: true,
        };
        self.start_compare(pair, options)
    }

    /// The differing rows' files in the results lister, each with its
    /// status after its name; a row only there is the file there.
    fn differences_as_list(&mut self, view: &ResultView) -> Vec<Effect> {
        use super::searching::{ListKind, ListOpen};
        let result = &view.held.0;
        let mut hits = Vec::new();
        let mut notes = Vec::new();
        for index in result_order(view) {
            let row = &result.rows[index];
            if matches!(row.state, RowState::Same | RowState::Skipped) {
                continue;
            }
            let side = if row.left.is_some() {
                Side::Left
            } else {
                Side::Right
            };
            let (Some(entry), Ok(place)) = (row.entry(side), result.place_of(row, side)) else {
                continue;
            };
            let mut note = row.state.words().to_owned();
            for reason in row.reasons() {
                note.push_str(", ");
                note.push_str(&reason);
            }
            notes.push((place.clone(), note));
            hits.push(diroxide_search::SearchHit::new(place, entry.clone()));
        }
        let n = hits.len() as u64;
        let open = ListOpen {
            kind: ListKind::Compared,
            side: view.setup.side,
            title: format!(
                "Differences, {} and {}",
                place_leaf(&view.setup.here),
                place_leaf(&view.setup.there)
            ),
            root: Some(view.setup.here.clone()),
            hits,
            gone: Vec::new(),
            notes,
            start: format!("Differences as a list, {}", count(n, "file", "files")),
            streaming: false,
        };
        self.open_list(open).0
    }

    /// The modes, over the result.
    fn open_modes(&mut self, view: ResultView) -> Vec<Effect> {
        if view.held.0.counts().differences() == 0 {
            return vec![self.say(Message::info(NOTHING_TO_DO))];
        }
        self.open_sync_list(modes_list(), SyncWants::Modes(Box::new(view)), None)
    }

    /// Enter on a mode: its plan, in place of the modes.
    fn mode_chosen(&mut self, view: ResultView, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let Some(mode) = mode_of(row) else {
            // Compare only: back to the result.
            return effects;
        };
        let plan = plan_sync(&view.held.0, mode);
        let line = plan.summary();
        let view = PlanView {
            result: view,
            plan,
            same: false,
        };
        let list = plan_list(&view);
        effects.extend(self.open_sync_list(list, SyncWants::Plan(Box::new(view)), Some(line)));
        effects
    }

    /// Enter in the plan view.
    fn plan_chosen(&mut self, mut view: PlanView, row: &str) -> Vec<Effect> {
        match row {
            RUN => return self.ask_hooks(view),
            AGAIN => return self.compare_again(view.result.setup),
            MODE => {
                let mut effects = self.close_top();
                effects.extend(self.open_modes(view.result));
                return effects;
            }
            SAME => {
                view.same = !view.same;
                let list = plan_list(&view);
                let shown = view.plan.order(view.same).len() as u64;
                let said = if view.same {
                    format!("Showing all {}", count(shown, "row", "rows"))
                } else {
                    format!("Same rows hidden: {}", count(shown, "row", "rows"))
                };
                self.replace_sync_top(list, SyncWants::Plan(Box::new(view)));
                return vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
            }
            _ => {}
        }
        let Some(index) = row_index(row, 'p') else {
            let line = self
                .dialog_list()
                .and_then(|list| list.focused())
                .map(|r| r.label.clone())
                .unwrap_or_default();
            return vec![self.say(Message::info(line))];
        };
        let actions = row_actions(&view.plan, index);
        if actions.is_empty() {
            let line = format!("Nothing to change: {}", plan_line(&view.plan, index));
            return vec![self.say(Message::info(line))];
        }
        let name = view.plan.rows[index].row.name();
        let mut list = ListModel::new(format!("{name}:")).with_rows(actions);
        list.set_help("Enter changes the row; Escape leaves it as it is.");
        self.open_sync_list(list, SyncWants::RowActions(Box::new(view), index), None)
    }

    /// Enter on what can change on a row: the change, then the plan view
    /// again with the row's new line said.
    fn row_action_chosen(&mut self, view: PlanView, index: usize, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let change: fn(&mut SyncPlan, usize) -> Result<String, diroxide_ops::OpsError> = match row {
            TOGGLE => |plan, index| plan.toggle(index),
            TOWARD_THERE => |plan, index| plan.set_direction(index, Side::Right),
            TOWARD_HERE => |plan, index| plan.set_direction(index, Side::Left),
            RESET => |plan, index| plan.reset(index),
            _ => return effects,
        };
        effects.extend(self.change_row(view, index, |plan| change(plan, index)));
        effects
    }

    /// A key in a synchronize list (lane AS): Space on a plan row leaves
    /// it out or takes it back in, and the row's new line is said once;
    /// `None` for any other key or list.
    pub(super) fn sync_key(&mut self, wants: &SyncWants, key: &Key) -> Option<Vec<Effect>> {
        let SyncWants::Plan(view) = wants else {
            return None;
        };
        if key.code != KeyCode::Char(' ') || !key.mods.is_empty() {
            return None;
        }
        let index = row_index(&self.top_row_id()?, 'p')?;
        Some(self.change_row((**view).clone(), index, |plan| plan.toggle(index)))
    }

    /// Changes one row of the plan view on top, says its new line, or the
    /// refusal.
    fn change_row(
        &mut self,
        mut view: PlanView,
        index: usize,
        change: impl FnOnce(&mut SyncPlan) -> Result<String, diroxide_ops::OpsError>,
    ) -> Vec<Effect> {
        match change(&mut view.plan) {
            Ok(_) => {
                let line = plan_line(&view.plan, index);
                let list = plan_list(&view);
                self.replace_sync_top(list, SyncWants::Plan(Box::new(view)));
                vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
            }
            Err(diroxide_ops::OpsError::Refused(line)) => vec![self.say(Message::info(line))],
            Err(error) => vec![self.say(Message::info(error.to_string()))],
        }
    }

    /// Synchronize now: refused when nothing acts or the options the
    /// compare used changed; else the hooks are asked before the
    /// question.
    fn ask_hooks(&mut self, view: PlanView) -> Vec<Effect> {
        if view.plan.counts().acting() == 0 {
            return vec![self.say(Message::info(NOTHING_TO_DO))];
        }
        let used = view.result.setup.options;
        let now = CompareOptions::from_settings(&self.settings);
        if used.tolerance_seconds != now.tolerance_seconds
            || used.ignore_one_hour != now.ignore_one_hour
        {
            return vec![self.say(Message::info(OPTIONS_CHANGED))];
        }
        let job = self.next_job_id();
        self.request(Request::Sync(Box::new(SyncAsk::Hooks {
            job,
            plan: Box::new(view.plan),
        })));
        Vec::new()
    }

    /// The question before the run: "Synchronize 6 items? No is the
    /// default. y or n", the listing under it: what the hooks said, what
    /// cannot be undone, then the rows that act, removals first.
    fn ask_to_run(&mut self, view: PlanView, job: JobId, said: Vec<String>) -> Vec<Effect> {
        let plan = &view.plan;
        let acting = plan.counts().acting();
        let question = with_default_no(Message::question(&format!(
            "Synchronize {}",
            count(acting, "item", "items")
        )));
        let mut listing = said;
        listing.extend(
            plan.notes()
                .into_iter()
                .filter(|note| *note != plan.verified),
        );
        listing.extend(
            plan.order(false)
                .into_iter()
                .filter(|index| plan.acts(*index))
                .map(|index| plan_line(plan, index)),
        );
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Purpose::Organize(Box::new(Wants::Sync(SyncWants::Confirm(
                Box::new(view),
                job,
            )))),
        )
    }

    /// Yes to one of these questions ("Synchronize 2 items? y or n"): the
    /// views close and the run starts as one job.
    pub(super) fn sync_yes(&mut self, wants: SyncWants) -> Vec<Effect> {
        let SyncWants::Confirm(view, job) = wants else {
            return Vec::new();
        };
        let mut effects = self.close_sync_dialogs();
        let acting = view.plan.counts().acting();
        let line = format!("Synchronize started: {}", count(acting, "item", "items"));
        let now = self.now();
        self.jobs
            .begin(job, JobKind::Synchronize, line.clone(), now);
        self.request(Request::Sync(Box::new(SyncAsk::Run {
            job,
            plan: Box::new(view.plan),
        })));
        effects.push(Effect::Redraw(Redraw::JobsLine));
        effects.push(self.say(Message::info(line)));
        effects
    }

    /// Escape on one of these dialogs, after it closed.
    pub(super) fn sync_escape(&mut self, wants: SyncWants) -> Vec<Effect> {
        match wants {
            SyncWants::Criteria(_) => vec![self.say(Message::info(NOTHING_COMPARED))],
            SyncWants::Confirm(..) => vec![self.say(Message::info(NOTHING_DONE))],
            _ => Vec::new(),
        }
    }

    /// What the runtime answered a [`crate::sync_ui::SyncAsk`].
    pub(super) fn sync_note(&mut self, note: SyncNote) -> Vec<Effect> {
        match note {
            SyncNote::Progress { job, line } => {
                if self.jobs.ids().contains(&job) {
                    vec![self.say(Message::progress(line))]
                } else {
                    Vec::new()
                }
            }
            SyncNote::Compared {
                job,
                here,
                there,
                options,
                then_plan,
                result,
            } => self.folders_compared(job, here, there, options, then_plan, result),
            SyncNote::Files {
                after,
                options,
                result,
                ..
            } => match result {
                Ok(result) => {
                    let name = place_leaf(&after);
                    self.show_documents(crate::plugins::diff::ShownDiff {
                        title: name.clone(),
                        before: format!("{name} there"),
                        after: format!("{name} here"),
                        result: *result,
                        options,
                    })
                }
                Err(message) => vec![self.say(message)],
            },
            SyncNote::Hooked { job, plan, result } => {
                let view = match self.top_sync() {
                    Some(SyncWants::Plan(view)) if view.plan == *plan => view,
                    // The owner moved on: nothing is asked.
                    _ => return Vec::new(),
                };
                match result {
                    Ok(said) => self.ask_to_run(*view, job, said),
                    Err(line) => vec![self.say(Message::info(line))],
                }
            }
        }
    }

    /// A compare ended: the job is done; the result list opens, and the
    /// modes over it when the owner asked for Synchronize.
    fn folders_compared(
        &mut self,
        job: JobId,
        here: Place,
        there: Place,
        options: CompareOptions,
        then_plan: bool,
        result: Result<Box<Comparison>, String>,
    ) -> Vec<Effect> {
        let side = if self.panel(self.active).place() == &here {
            self.active
        } else {
            self.active.other()
        };
        let (summary, done, cancelled, failed) = match &result {
            Ok(result) => (
                result.count_line(),
                result.rows.len() as u64,
                result.cancelled,
                Vec::new(),
            ),
            Err(line) => (line.clone(), 0, false, vec![line.clone()]),
        };
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::Compare,
            summary,
            failed,
            done,
            cancelled,
            ..FinishedJob::default()
        });
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        let result = match result {
            Ok(result) => result,
            Err(line) => {
                effects.push(self.say(Message::info(line)));
                return effects;
            }
        };
        let view = ResultView {
            setup: Setup {
                side,
                here,
                there,
                options,
            },
            held: HeldComparison(Arc::from(result)),
            same: false,
            only: None,
        };
        let differences = view.held.0.counts().differences();
        effects.extend(self.show_result(view.clone()));
        if then_plan && differences > 0 {
            effects.extend(self.open_modes(view));
        }
        effects
    }
}

#[cfg(test)]
mod tests;
