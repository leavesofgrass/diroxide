//! The file operations through the job engine: copy and move to the other
//! panel or a place picked from a list, make folder, rename (several
//! marked entries one after another), duplicate, the trash with its "no
//! trash here" question and a separate permanent delete, permanent delete,
//! undo, the file clipboard, and every job's plan, confirmation, progress,
//! questions and result.
//!
//! The order is always the same: the selection is frozen, the engine plans
//! without touching anything, the plan is shown (every path, hidden
//! characters spelled out, the count in words, the question starting on
//! No), and the very plan that was shown runs on Yes. A refresh while the
//! question is open cannot change what Yes does.

use std::sync::Arc;

use diroxide_core::format::count;
use diroxide_core::{
    Command, Effect, JobId, JobKind, JobOrigin, JobRequest, ListModel, ListRow, Message, Place,
    Redraw, Side, catalog,
};
use diroxide_ops::{JobProgress, JobReport, OpsError, Plan, Preflight};

use super::{App, Flow, Purpose, Then, capitalized};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{Candidate, CompletionSource, EntryField};
use crate::jobs::{FinishedJob, SIGHTED_INTERVAL_MS};
use crate::panel::{Load, place_leaf};
use crate::request::{AskId, Question, Reply, Request};
use crate::state;

/// The file clipboard of alpha.1: internal only (the owner's answer 74).
/// A cut is pending until it is pasted; a copy stays for the next paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clipboard {
    /// What was copied or cut.
    pub places: Vec<Place>,
    /// Whether a paste moves them.
    pub cut: bool,
}

/// The line said when a job starts, the verb first: "Copy started: 3
/// items to Backup".
fn start_line(plan: &Plan) -> String {
    let head = match plan.kind {
        JobKind::Copy => "Copy",
        JobKind::Move => "Move",
        JobKind::Trash => "Trash",
        JobKind::DeletePermanently => "Delete",
        JobKind::Duplicate => "Duplicate",
        JobKind::Rename => "Rename",
        JobKind::MakeFolder => "Make folder",
        JobKind::Undo => "Undo",
        JobKind::Split => "Split",
        JobKind::Join => "Join",
        JobKind::Link => "Link",
        JobKind::SetAttributes => "Change permissions",
        JobKind::SetTimes => "Change times",
        _ => "Job",
    };
    let items = count(plan.entries(), "item", "items");
    let to = plan
        .facts
        .destination
        .as_ref()
        .map(|d| format!(" to {}", place_leaf(d)))
        .unwrap_or_default();
    format!("{head} started: {items}{to}")
}

/// The confirmation's lines, in order (W1b's notes): the frozen
/// selection's note first, the summary, the unusual things, each
/// preflight's lines, the listing grouped by folder, and the honest
/// sentence of a permanent delete.
fn confirmation_lines(plan: &Plan) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(note) = &plan.note {
        lines.push(note.clone());
    }
    lines.push(plan.summary());
    if let Some(unusual) = plan.unusual() {
        lines.push(unusual);
    }
    for question in &plan.facts.questions {
        lines.extend(question.lines());
    }
    for (place, reason) in &plan.facts.left_out {
        lines.push(format!(
            "Left out: {}, {reason}",
            diroxide_core::names::spell_out(place.name().unwrap_or_default())
        ));
    }
    for group in plan.groups() {
        lines.push(group.title);
        lines.extend(group.lines);
    }
    if let Some(caution) = plan.caution() {
        lines.push(caution.to_owned());
    }
    // What a split, a join or a change of permissions will do, in its own
    // rows (W2c's notes): "12 parts of 20 MB, the last 3.1 MB".
    if let Some(split) = &plan.facts.split {
        lines.extend(split.rows());
        lines.extend(
            split
                .files
                .iter()
                .map(diroxide_ops::SplitFile::details_line),
        );
    }
    if let Some(join) = &plan.facts.join {
        lines.extend(join.rows());
    }
    if let Some(attributes) = &plan.facts.attributes {
        lines.extend(attributes.rows.iter().cloned());
    }
    lines
}

/// The plan's question with a copy's or a move's destination named by its
/// folder's own name, so the question keeps its fact inside 40 cells
/// ("Copy 3 items to Backup, 312 bytes? y or n"); the listing under it
/// still says the whole path first.
fn short_question(plan: &Plan) -> Message {
    let question = plan.question();
    if !matches!(plan.kind, JobKind::Copy | JobKind::Move) || plan.facts.typed.is_some() {
        return question;
    }
    let destination = plan.facts.destination.clone().or_else(|| {
        plan.steps.iter().find_map(|step| match step {
            diroxide_ops::Step::Copy { to, .. } | diroxide_ops::Step::Move { to, .. } => {
                to.parent()
            }
            _ => None,
        })
    });
    let Some(destination) = destination else {
        return question;
    };
    let Some(leaf) = destination.name() else {
        return question;
    };
    let full = diroxide_core::names::spell_out(&destination.to_string()).into_owned();
    let leaf = diroxide_core::names::spell_out(leaf).into_owned();
    Message {
        text: question
            .text
            .replacen(&format!(" to {full}"), &format!(" to {leaf}"), 1),
        ..question
    }
}

/// "name: reason" rows for the job history.
fn named(places: &[(Place, String)]) -> Vec<String> {
    places
        .iter()
        .map(|(place, reason)| {
            let name = place
                .name()
                .map_or_else(|| place.to_string(), str::to_owned);
            format!("{}: {reason}", diroxide_core::names::display_name(&name))
        })
        .collect()
}

impl App {
    /// The marked entries, or the focused one, frozen now: their places,
    /// and the plan's first line when the focused row is not among them.
    pub(super) fn frozen(&self, side: Side) -> Option<(Vec<Place>, Option<String>)> {
        let panel = self.panel(side);
        let frozen = panel.list().freeze();
        if frozen.ids.is_empty() {
            return None;
        }
        let sources = frozen
            .ids
            .iter()
            .filter_map(|id| panel.place().join(id).ok())
            .collect();
        Some((sources, frozen.focus_note()))
    }

    fn nothing_here(&mut self, side: Side) -> Vec<Effect> {
        let empty = self.panel(side).list().empty_text().to_owned();
        vec![self.say(Message::status(empty))]
    }

    /// Asks the engine for a plan; what happens next is the flow's.
    pub(super) fn plan_job(
        &mut self,
        side: Side,
        request: JobRequest,
        note: Option<String>,
        then: Then,
        confirm: bool,
    ) -> Vec<Effect> {
        self.flows.insert(
            request.id,
            Flow {
                side,
                note,
                then,
                confirm,
                kind: request.kind,
            },
        );
        self.request(Request::Plan(Box::new(request)));
        Vec::new()
    }

    pub(super) fn copy_or_move(&mut self, kind: JobKind) -> Vec<Effect> {
        let side = self.active;
        let Some((sources, note)) = self.frozen(side) else {
            return self.nothing_here(side);
        };
        let destination = self.panel(side.other()).place().clone();
        self.copy_or_move_to(side, kind, sources, note, destination)
    }

    fn copy_or_move_to(
        &mut self,
        side: Side,
        kind: JobKind,
        sources: Vec<Place>,
        note: Option<String>,
        destination: Place,
    ) -> Vec<Effect> {
        // Nothing is written into an archive here (copying in is alpha.8):
        // say so, and name Pack (plan commit 60).
        if destination.kind() == diroxide_core::PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        // Out of an archive, a copy is an unpack of those members: one pass
        // over a sequential archive, whatever the count.
        if kind == JobKind::Copy
            && self.panel(side).place().kind() == diroxide_core::PlaceKind::Archive
        {
            return self.copy_out(side, sources, note, destination);
        }
        if destination == *self.panel(side).place() && kind == JobKind::Move {
            let text = format!(
                "Move needs another folder: both panels show {}.",
                place_leaf(&destination)
            );
            return vec![self.say(Message::info(text))];
        }
        if destination == *self.panel(side).place() {
            let text = format!(
                "Copy needs another folder: both panels show {}. Duplicate makes copies here.",
                place_leaf(&destination)
            );
            return vec![self.say(Message::info(text))];
        }
        let job = self.next_job_id();
        let request = JobRequest::new(job, kind, sources).to(destination);
        let confirm = match kind {
            JobKind::Move => self.settings.confirm.move_items,
            _ => self.settings.confirm.copy,
        };
        self.plan_job(side, request, note, Then::Report, confirm)
    }

    /// Copy or move to a place picked from a list: recent destinations
    /// first, then the other panel, then the places used most.
    pub(super) fn pick_destination(&mut self, kind: JobKind) -> Vec<Effect> {
        let side = self.active;
        let targets = self.panel(side).targets().len() as u64;
        if targets == 0 {
            return self.nothing_here(side);
        }
        let verb = if kind == JobKind::Move {
            "Move"
        } else {
            "Copy"
        };
        let label = format!("{verb} {} to", count(targets, "item", "items"));
        let field = EntryField::new(label, CompletionSource::Places)
            .with_history(state::keys::DESTINATIONS);
        let mut effects =
            self.open_dialog(Dialog::edit(field), Purpose::PickDestination { side, kind });
        effects.extend(self.refresh_candidates());
        effects
    }

    /// The candidates for a place field: recent destinations first (for
    /// the picker), the other panel, the places, then the folders used
    /// most.
    pub(super) fn place_candidates(&self, side: Side, destinations: bool) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        let mut push = |text: String, kind: String| {
            if !out.iter().any(|c| c.text == text) {
                out.push(Candidate::new(text, kind));
            }
        };
        if destinations {
            for entry in self.history_of(state::keys::DESTINATIONS) {
                push(entry, "recent destination".to_owned());
            }
            push(
                self.panel(side.other()).place().to_string(),
                "other panel".to_owned(),
            );
        }
        if let Some(home) = &self.home {
            push(home.to_string(), "home folder".to_owned());
        }
        for place in &self.places {
            push(place.place.to_string(), place.words.clone());
        }
        let now = self.now().unix_seconds();
        for visit in self.visits.ranked(now).into_iter().take(50) {
            push(
                visit.place.to_string(),
                format!("visited {}", count(u64::from(visit.count), "time", "times")),
            );
        }
        out
    }

    pub(super) fn destination_chosen(
        &mut self,
        side: Side,
        kind: JobKind,
        text: &str,
    ) -> Vec<Effect> {
        let destination = match self.typed_place(side, text) {
            Ok(place) => place,
            Err(message) => return vec![self.say(message)],
        };
        let Some((sources, note)) = self.frozen(side) else {
            return self.nothing_here(side);
        };
        self.remember(state::keys::DESTINATIONS, &destination.to_string());
        self.copy_or_move_to(side, kind, sources, note, destination)
    }

    /// Make folder: a command with a `name` makes it; otherwise the name
    /// prompt opens.
    pub(super) fn make_folder(&mut self, command: &Command) -> Vec<Effect> {
        let side = self.active;
        if let Some(name) = command.text("name") {
            let name = name.to_owned();
            return self.make_folder_named(side, &name);
        }
        let label = format!("New folder in {}", place_leaf(self.panel(side).place()));
        let field =
            EntryField::new(label, CompletionSource::None).with_history(state::keys::MAKE_FOLDER);
        self.open_dialog(Dialog::edit(field), Purpose::MakeFolder(side))
    }

    /// Checks a new name against the place's rules: a name the place
    /// refuses is said in words and nothing is made.
    pub(super) fn name_refusal(&self, side: Side, name: &str, old: Option<&str>) -> Option<String> {
        let rules = match self.panel(side).capabilities() {
            Some(caps) => diroxide_vfs::NameRules::from_capabilities(&caps),
            None => diroxide_vfs::NameRules::native(),
        };
        let siblings: Vec<&str> = self
            .panel(side)
            .names()
            .into_iter()
            .filter(|n| Some(*n) != old)
            .collect();
        let issues = diroxide_vfs::check_name_with(name, &siblings, &rules);
        issues
            .iter()
            .find(|issue| issue.refused(&rules))
            .or_else(|| {
                issues
                    .iter()
                    .find(|issue| matches!(issue, diroxide_vfs::NameIssue::TrailingDotOrSpace))
            })
            .map(diroxide_vfs::NameIssue::line)
    }

    pub(super) fn make_folder_named(&mut self, side: Side, name: &str) -> Vec<Effect> {
        let name = name.trim_matches(['\r', '\n']).to_owned();
        if let Some(line) = self.name_refusal(side, &name, None) {
            return vec![self.say(Message::error(format!("{line}. Nothing was made.")))];
        }
        self.remember(state::keys::MAKE_FOLDER, &name);
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::MakeFolder, Vec::new())
            .to(self.panel(side).place().clone())
            .named(name.clone());
        self.plan_job(side, request, None, Then::MadeFolder(name), false)
    }

    pub(super) fn trash(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((sources, note)) = self.frozen(side) else {
            return self.nothing_here(side);
        };
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Trash, sources);
        let confirm = self.settings.confirm.trash;
        self.plan_job(side, request, note, Then::Report, confirm)
    }

    pub(super) fn delete_permanently(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((sources, note)) = self.frozen(side) else {
            return self.nothing_here(side);
        };
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::DeletePermanently, sources);
        self.plan_job(side, request, note, Then::Report, true)
    }

    /// Rename: the focused entry, or each marked entry in turn (R10); the
    /// name prompt opens with the name in it and the cursor before the
    /// extension. A command with a `name` renames the focused entry.
    pub(super) fn rename(&mut self, command: &Command) -> Vec<Effect> {
        let side = self.active;
        let targets = self.panel(side).targets();
        let Some((first, rest)) = targets.split_first() else {
            return self.nothing_here(side);
        };
        if let Some(name) = command.text("name")
            && rest.is_empty()
        {
            let name = name.to_owned();
            return self.rename_answer(side, first.clone(), Vec::new(), Vec::new(), &name);
        }
        self.rename_prompt(side, first.clone(), rest.to_vec(), Vec::new())
    }

    pub(super) fn rename_prompt(
        &mut self,
        side: Side,
        current: Place,
        rest: Vec<Place>,
        done: Vec<(Place, String)>,
    ) -> Vec<Effect> {
        let name = current.name().unwrap_or_default().to_owned();
        let shown = diroxide_core::names::display_name(&name).into_owned();
        let label = format!("Rename {shown} to");
        let mut field = EntryField::new(label, CompletionSource::None)
            .with_text(name.clone())
            .with_history(state::keys::RENAME);
        let (_, extension) = diroxide_core::names::split_extension(&name);
        if !extension.is_empty() && extension.len() < name.len() {
            for _ in 0..extension.chars().count() {
                field.left();
            }
        }
        self.open_dialog(
            Dialog::edit(field),
            Purpose::Rename {
                side,
                current,
                rest,
                done,
            },
        )
    }

    /// One rename answered: checked, then the next entry, or the job.
    pub(super) fn rename_answer(
        &mut self,
        side: Side,
        current: Place,
        rest: Vec<Place>,
        mut done: Vec<(Place, String)>,
        text: &str,
    ) -> Vec<Effect> {
        let old = current.name().unwrap_or_default().to_owned();
        let new = text.trim_matches(['\r', '\n']).to_owned();
        if new != old {
            if let Some(line) = self.name_refusal(side, &new, Some(&old)) {
                let mut effects =
                    vec![self.say(Message::error(format!("{line}. Nothing renamed.")))];
                effects.extend(self.rename_prompt(side, current, rest, done));
                return effects;
            }
            self.remember(state::keys::RENAME, &new);
            done.push((current, new));
        }
        if let Some((next, more)) = rest.split_first() {
            return self.rename_prompt(side, next.clone(), more.to_vec(), done);
        }
        if done.is_empty() {
            return vec![self.say(Message::info("Name unchanged. Nothing renamed."))];
        }
        self.rename_finish(side, done)
    }

    /// The renames answered so far, as one job.
    pub(super) fn rename_finish(&mut self, side: Side, done: Vec<(Place, String)>) -> Vec<Effect> {
        if done.is_empty() {
            return vec![self.say(Message::info("Nothing renamed."))];
        }
        let pairs: Vec<(String, String)> = done
            .iter()
            .map(|(place, new)| (place.name().unwrap_or_default().to_owned(), new.clone()))
            .collect();
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Rename, Vec::new()).renaming(done);
        self.plan_job(side, request, None, Then::Renamed(pairs), false)
    }

    /// Duplicate: each marked entry in turn, beside itself, under a name
    /// that is free ("report (2).pdf"), the prompt filled with it.
    pub(super) fn duplicate(&mut self) -> Vec<Effect> {
        let side = self.active;
        let targets = self.panel(side).targets();
        let Some((first, rest)) = targets.split_first() else {
            return self.nothing_here(side);
        };
        self.duplicate_prompt(side, first.clone(), rest.to_vec())
    }

    pub(super) fn duplicate_prompt(
        &mut self,
        side: Side,
        current: Place,
        rest: Vec<Place>,
    ) -> Vec<Effect> {
        let name = current.name().unwrap_or_default().to_owned();
        let free = diroxide_vfs::names::unused_name(&name, &self.panel(side).names())
            .unwrap_or_else(|| name.clone());
        let label = format!("Duplicate {} as", diroxide_core::names::display_name(&name));
        let field = EntryField::new(label, CompletionSource::None)
            .with_text(free)
            .with_history(state::keys::DUPLICATE);
        self.open_dialog(
            Dialog::edit(field),
            Purpose::Duplicate {
                side,
                current,
                rest,
            },
        )
    }

    pub(super) fn duplicate_answer(
        &mut self,
        side: Side,
        current: Place,
        rest: Vec<Place>,
        text: &str,
    ) -> Vec<Effect> {
        let new = text.to_owned();
        let mut effects = Vec::new();
        if let Some(line) = self.name_refusal(side, &new, None) {
            effects.push(self.say(Message::error(format!("{line}. Nothing duplicated."))));
            effects.extend(self.duplicate_prompt(side, current, rest));
            return effects;
        }
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Duplicate, vec![current]).named(new);
        effects.extend(self.plan_job(side, request, None, Then::Report, false));
        if let Some((next, more)) = rest.split_first() {
            effects.extend(self.duplicate_prompt(side, next.clone(), more.to_vec()));
        }
        effects
    }

    pub(super) fn undo(&mut self) -> Vec<Effect> {
        // A label change newer than the last job is undone first (W3l).
        if let Some(effects) = self.undo_label_change() {
            return effects;
        }
        let Some(record) = self.undo.last().cloned() else {
            return vec![self.say(Message::info("Nothing to undo"))];
        };
        let side = self.active;
        let job = self.next_job_id();
        self.flows.insert(
            job,
            Flow {
                side,
                note: None,
                then: Then::Undid(record.kind),
                confirm: false,
                kind: JobKind::Undo,
            },
        );
        self.request(Request::PlanUndo {
            job,
            record: Box::new(record),
        });
        Vec::new()
    }

    /// Copy or cut to the file clipboard: the marks, or the focused row.
    pub(super) fn clip(&mut self, cut: bool) -> Vec<Effect> {
        let side = self.active;
        let Some((places, _)) = self.frozen(side) else {
            return self.nothing_here(side);
        };
        let items = places.len() as u64;
        self.clipboard = Some(Clipboard { places, cut });
        let paste = self.key(&ids::CLIPBOARD_PASTE);
        let message = if cut {
            crate::messages::cut_pending(items, &paste)
        } else {
            crate::messages::copied_to_clipboard(items, &paste)
        };
        vec![self.say(message)]
    }

    /// Paste into the active panel's folder: a copy stays on the
    /// clipboard; a cut moves and leaves it empty.
    pub(super) fn paste(&mut self) -> Vec<Effect> {
        let Some(clipboard) = self.clipboard.clone() else {
            return vec![self.say(Message::info("The file clipboard is empty"))];
        };
        let side = self.active;
        let destination = self.panel(side).place().clone();
        if destination.kind() == diroxide_core::PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        if !clipboard.cut
            && let Some(first) = clipboard.places.first()
            && let Some((container, _)) = first.archive()
        {
            // Copies out of one archive: an unpack of those members.
            let container = container.clone();
            let request = clipboard
                .places
                .iter()
                .all(|p| p.archive().is_some_and(|(c, _)| *c == container));
            if request {
                return self.copy_out(side, clipboard.places, None, destination);
            }
        }
        if clipboard
            .places
            .iter()
            .all(|p| p.parent().as_ref() == Some(&destination))
        {
            let text = if clipboard.cut {
                "Nothing to paste: these items are already here."
            } else {
                "These items are already here. Duplicate makes copies here."
            };
            return vec![self.say(Message::info(text))];
        }
        let kind = if clipboard.cut {
            JobKind::Move
        } else {
            JobKind::Copy
        };
        let job = self.next_job_id();
        let request = JobRequest::new(job, kind, clipboard.places).to(destination);
        let confirm = if clipboard.cut {
            self.settings.confirm.move_items
        } else {
            self.settings.confirm.copy
        };
        let then = if clipboard.cut {
            Then::ClearClipboard
        } else {
            Then::Report
        };
        self.plan_job(side, request, None, then, confirm)
    }

    /// A plan arrived: questions answered by the rules (an item too large
    /// or a name refused is left out and said), the trash's "no trash
    /// here" part split off as its own permanent delete, then the
    /// confirmation, or the run.
    pub(super) fn planned(
        &mut self,
        job: JobId,
        result: Result<Box<Plan>, OpsError>,
    ) -> Vec<Effect> {
        let Some(flow) = self.flows.get(&job).cloned() else {
            return Vec::new();
        };
        let mut plan = match result {
            Ok(plan) => plan,
            Err(error) => {
                self.flows.remove(&job);
                self.jobs.finish(job);
                let message = match &error {
                    OpsError::Refused(sentence)
                    | OpsError::NameRefused(sentence)
                    | OpsError::TrashNotConfirmed(sentence) => {
                        use diroxide_core::Explain;
                        Message::error(format!("{sentence}. {}", error.next_step()))
                    }
                    OpsError::NothingToUndo => Message::info("Nothing to undo"),
                    // These say the whole sentence themselves: "Part 004 is
                    // missing. Nothing was joined."
                    OpsError::CannotJoin(sentence)
                    | OpsError::TooManyParts(sentence)
                    | OpsError::CannotLink(sentence) => Message::error(sentence.clone()),
                    // The read-only refusal is a sentence of its own (P2b).
                    OpsError::Vfs(inner) if inner.sentence().is_some() => {
                        Message::error(inner.sentence().unwrap_or_default())
                    }
                    _ => Message::from_error(flow.kind.verb(), "", &error),
                };
                return vec![Effect::Redraw(Redraw::JobsLine), self.say(message)];
            }
        };
        if flow.kind == JobKind::Undo {
            self.undo.pop();
        }
        plan.note = flow.note.clone();
        let mut effects = Vec::new();
        let mut forced = false;
        let questions = std::mem::take(&mut plan.facts.questions);
        let mut kept = Vec::new();
        let mut asked = Vec::new();
        for question in questions {
            match &question {
                Preflight::TooLarge { place, .. } => {
                    plan.leave_out(place, "too large for this drive");
                }
                Preflight::NameRefused { place, line } => {
                    let line = line.clone();
                    plan.leave_out(place, &line);
                }
                Preflight::NoTrash { gate, places } => {
                    let items = places.len() as u64;
                    if let Some(question) = gate.question(items) {
                        let delete = self.next_job_id();
                        let request =
                            JobRequest::new(delete, JobKind::DeletePermanently, places.clone());
                        effects.extend(self.plan_job(
                            flow.side,
                            request,
                            None,
                            Then::NoTrash(question),
                            true,
                        ));
                    }
                    continue;
                }
                Preflight::NotEnoughRoom { .. } => forced = true,
                // Asked one at a time before the confirmation, each with
                // what its No does (W2c's notes).
                Preflight::ManyParts { .. }
                | Preflight::WhichParts { .. }
                | Preflight::OddPart { .. }
                | Preflight::RunsAsOwner { .. }
                | Preflight::EveryoneCanChange { .. }
                | Preflight::LosesAccess { .. }
                | Preflight::TrashCopies { .. } => {
                    asked.push(question);
                    continue;
                }
                _ => forced = true,
            }
            kept.push(question);
        }
        plan.facts.questions = kept;
        if !asked.is_empty() {
            let first = asked.remove(0);
            effects.extend(self.ask_plan_preflight(plan, first, asked, forced));
            return effects;
        }
        effects.extend(self.finish_planning(job, plan, forced));
        effects
    }

    /// One of a plan's questions, before its confirmation: the line that
    /// asks, starting on No, with the plan's other lines under it.
    fn ask_plan_preflight(
        &mut self,
        plan: Box<Plan>,
        question: Preflight,
        rest: Vec<Preflight>,
        forced: bool,
    ) -> Vec<Effect> {
        let lines = question.lines();
        let asks = lines
            .iter()
            .rposition(|l| l.ends_with("y or n"))
            .unwrap_or(0);
        let text = lines.get(asks).cloned().unwrap_or_default();
        let listing: Vec<String> = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != asks)
            .map(|(_, l)| l.clone())
            .collect();
        let message = with_default_no(Message::new(
            text,
            diroxide_core::Importance::High,
            diroxide_core::MessageKind::Question,
        ));
        self.open_dialog(
            Dialog::Confirm {
                question: message,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Purpose::PlanQuestion {
                plan,
                question: Box::new(question),
                rest,
                forced,
            },
        )
    }

    /// The answer to one of a plan's questions: a file left out, the job
    /// cancelled, or the join planned again with the other set; then the
    /// next question, or the confirmation.
    pub(super) fn plan_preflight_answered(
        &mut self,
        mut plan: Box<Plan>,
        question: &Preflight,
        mut rest: Vec<Preflight>,
        forced: bool,
        yes: bool,
    ) -> Vec<Effect> {
        let job = plan.job;
        let cancel = |app: &mut App, words: &str| {
            app.flows.remove(&job);
            app.jobs.finish(job);
            vec![
                Effect::Redraw(Redraw::JobsLine),
                app.say(Message::info(words.to_owned())),
            ]
        };
        match question {
            Preflight::ManyParts { place, .. } if !yes => {
                plan.leave_out(place, "too many parts");
            }
            Preflight::RunsAsOwner { place } if !yes => {
                plan.leave_out(place, "would run as its owner");
            }
            Preflight::TrashCopies { places, .. } if !yes => {
                for place in places {
                    plan.leave_out(place, "left where it is");
                }
            }
            // A part is never left out of a join: No cancels it.
            Preflight::OddPart { .. } if !yes => return cancel(self, "Nothing was joined."),
            Preflight::EveryoneCanChange { .. } | Preflight::LosesAccess { .. } if !yes => {
                return cancel(self, "Nothing changed.");
            }
            Preflight::WhichParts { yes: parts, .. } if yes => {
                // Yes joins the other set: planned again with it.
                let destination = plan
                    .facts
                    .join
                    .as_ref()
                    .and_then(|j| j.output.parent())
                    .or_else(|| plan.facts.destination.clone());
                let mut request = JobRequest::new(job, JobKind::Join, parts.clone());
                if let Some(destination) = destination {
                    request = request.to(destination);
                }
                self.request(Request::Plan(Box::new(request)));
                return Vec::new();
            }
            _ => {}
        }
        if !rest.is_empty() {
            let next = rest.remove(0);
            return self.ask_plan_preflight(plan, next, rest, forced);
        }
        self.finish_planning(job, plan, forced)
    }

    /// A plan whose questions are answered: nothing left to do said, the
    /// typed confirmation of a big plan, the confirmation, or the run.
    fn finish_planning(&mut self, job: JobId, plan: Box<Plan>, forced: bool) -> Vec<Effect> {
        let Some(flow) = self.flows.get(&job).cloned() else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        if plan.steps.is_empty() {
            self.flows.remove(&job);
            self.jobs.finish(job);
            effects.push(Effect::Redraw(Redraw::JobsLine));
            if let Some((place, reason)) = plan.facts.left_out.first() {
                let name = place
                    .name()
                    .map(diroxide_core::names::display_name)
                    .unwrap_or_default();
                let text = format!("Nothing to {}: {name}, {reason}.", flow.kind.verb());
                effects.push(self.say(Message::info(text)));
            } else if !matches!(flow.kind, JobKind::Trash) {
                let text = format!("Nothing to {}.", flow.kind.verb());
                effects.push(self.say(Message::info(text)));
            }
            return effects;
        }
        if let Some(typed) = plan.facts.typed.clone() {
            effects.extend(self.typed_confirmation(plan, typed));
            return effects;
        }
        if flow.confirm || forced {
            let question = match &flow.then {
                Then::NoTrash(question) => question.clone(),
                _ => match plan.facts.questions.first() {
                    Some(Preflight::NotEnoughRoom { .. }) => {
                        let summary = plan.summary();
                        Message::question(&format!("{summary}. Not enough room. Go on anyway"))
                    }
                    _ => short_question(&plan),
                },
            };
            let question = with_default_no(question);
            let listing = confirmation_lines(&plan);
            effects.extend(self.open_dialog(
                Dialog::Confirm {
                    question,
                    listing,
                    on_yes: Command::new(ids::YES),
                },
                Purpose::RunPlan(plan),
            ));
            return effects;
        }
        effects.extend(self.run_plan(*plan));
        effects
    }

    /// A plan above the surprise threshold: its listing to read, and the
    /// count (or a protected folder's name) to type.
    fn typed_confirmation(&mut self, plan: Box<Plan>, typed: String) -> Vec<Effect> {
        let label = plan.question().text;
        let field = EntryField::new(label, CompletionSource::None);
        let rows = confirmation_lines(&plan)
            .into_iter()
            .enumerate()
            .map(|(i, line)| ListRow::new(i.to_string(), line))
            .collect();
        let mut listing = ListModel::new(plan.summary()).with_rows(rows);
        listing.set_help("Up and Down read the listing; type the text and Enter to go on.");
        let dialog = Dialog::Edit {
            field,
            candidates: Some(listing),
            in_list: false,
        };
        self.open_dialog(dialog, Purpose::TypedPlan(plan, typed))
    }

    /// Runs a confirmed plan, exactly as shown.
    pub(super) fn run_plan(&mut self, plan: Plan) -> Vec<Effect> {
        let line = start_line(&plan);
        let now = self.now();
        self.jobs.begin(plan.job, plan.kind, line.clone(), now);
        let loud = matches!(
            plan.kind,
            JobKind::Copy | JobKind::Move | JobKind::Trash | JobKind::DeletePermanently
        ) || plan.entries() > 20;
        self.request(Request::Run(Box::new(plan)));
        let message = if loud {
            Message::info(line)
        } else {
            Message::status(line)
        };
        vec![Effect::Redraw(Redraw::JobsLine), self.say(message)]
    }

    pub(super) fn job_progress(&mut self, job: JobId, progress: &JobProgress) -> Vec<Effect> {
        let interval = if self.screen_reader {
            self.settings.progress_interval_seconds.saturating_mul(1000)
        } else {
            SIGHTED_INTERVAL_MS
        };
        use diroxide_core::JobState;
        // A job's end is said once, by its result line ("Unpack finished:
        // 4 done."), which always follows; the progress line's "Unpacking
        // finished" is neither shown nor said, as in the CLI.
        if progress.state == JobState::Finished {
            return Vec::new();
        }
        let now = self.now();
        let Some((line, changed)) = self.jobs.progress(job, progress, now, interval) else {
            return Vec::new();
        };
        let message = match progress.state {
            JobState::Paused | JobState::Stalled | JobState::Verifying if changed => {
                Message::info(line)
            }
            _ => Message::progress(line),
        };
        vec![Effect::Redraw(Redraw::JobsLine), Effect::Announce(message)]
    }

    pub(super) fn job_finished(&mut self, report: JobReport) -> Vec<Effect> {
        let flow = self.flows.remove(&report.job);
        self.jobs.finished(FinishedJob {
            job: report.job,
            kind: report.kind,
            summary: report.summary(),
            failed: named(&report.failed),
            skipped: named(&report.skipped),
            warnings: named(&report.warnings),
        });
        // A job that made, moved or trashed nothing (Test archive) leaves
        // the last undo where it was.
        if report.kind != JobKind::Undo
            && report.undo.origin != JobOrigin::Undo
            && !(report.kind == JobKind::TestArchive && report.undo.is_empty())
        {
            self.undo.record(report.undo.clone());
        }
        let missing = report
            .reconciliation
            .as_ref()
            .is_some_and(|r| !r.missing.is_empty());
        let clean = report.failed.is_empty() && !report.cancelled && !missing;
        let done = report.done.len() as u64;
        let then = flow.as_ref().map_or(Then::Report, |f| f.then.clone());
        let mut focus: Option<String> = None;
        let text = match &then {
            Then::MadeFolder(name) if clean && done > 0 => {
                focus = Some(name.clone());
                format!(
                    "Created folder {}",
                    diroxide_core::names::display_name(name)
                )
            }
            Then::Renamed(pairs) if clean && pairs.len() == 1 && done == 1 => {
                let (old, new) = &pairs[0];
                focus = Some(new.clone());
                format!(
                    "Renamed {} to {}",
                    diroxide_core::names::display_name(old),
                    diroxide_core::names::display_name(new)
                )
            }
            Then::Undid(kind) if clean => {
                let noun = match kind {
                    Some(JobKind::Trash) => "trash",
                    Some(JobKind::Move) => "move",
                    Some(JobKind::Rename) => "rename",
                    Some(JobKind::Copy) => "copy",
                    Some(JobKind::Duplicate) => "duplicate",
                    Some(JobKind::MakeFolder) => "make folder",
                    Some(JobKind::Unpack) => "unpack",
                    Some(JobKind::Pack) => "pack",
                    Some(JobKind::Split) => "split",
                    Some(JobKind::Join) => "join",
                    _ => "job",
                };
                let how = match kind {
                    Some(JobKind::Copy | JobKind::Duplicate) => {
                        format!("{} in the trash", count(done, "copy is", "copies are"))
                    }
                    Some(JobKind::Unpack | JobKind::Pack | JobKind::Split | JobKind::Join) => {
                        format!("{} in the trash", count(done, "item is", "items are"))
                    }
                    Some(JobKind::MakeFolder) => {
                        format!("{} removed", count(done, "folder", "folders"))
                    }
                    _ => format!("{} back", count(done, "item is", "items are")),
                };
                format!("Undid {noun}: {how}")
            }
            _ if clean && report.kind == JobKind::Trash => catalog::moved_to_trash(done).text,
            _ if self.finish_lines.is_some() => {
                self.finish_lines.take().unwrap_or_default().join(" ")
            }
            // A join says its verdict: "Joined big.bin: checksum matches."
            _ if report.joined.is_some() => report
                .joined
                .as_ref()
                .map(diroxide_ops::JoinOutcome::sentence)
                .unwrap_or_default(),
            _ => {
                let mut text = report.summary();
                if let Some(reconciliation) = report.reconciliation.as_ref().filter(|_| missing) {
                    text.push(' ');
                    text.push_str(&reconciliation.summary());
                    text.push('.');
                }
                text
            }
        };
        if then == Then::ClearClipboard && done > 0 {
            self.clipboard = None;
        }
        let message = if clean {
            Message::done(text)
        } else {
            Message::warning(text)
        };
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine), self.say(message)];
        let side = flow.as_ref().map_or(self.active, |f| f.side);
        let folder = self.panel(side).place().clone();
        let failed: Vec<String> = report
            .failed
            .iter()
            .filter(|(place, _)| place.parent().as_ref() == Some(&folder))
            .filter_map(|(place, _)| place.name().map(str::to_owned))
            .collect();
        if report.kind != JobKind::MakeFolder && report.kind != JobKind::Rename {
            self.keep_selection(side);
            let list = self.panel_mut(side).list_mut();
            list.select_none();
            list.select_matching(|row| failed.contains(&row.id));
        }
        for each in Side::BOTH {
            let panel = self.panel(each);
            let place = panel.place().clone();
            let keep = if each == side { focus.clone() } else { None }
                .or_else(|| panel.list().focused().map(|r| r.id.clone()));
            effects.extend(self.open_as(each, place, keep, Load::AfterJob));
        }
        if clean && let Some(outcome) = report.joined.clone() {
            effects.extend(self.offer_trash_parts(&outcome));
        }
        // A link Windows refused for the privilege: a junction instead,
        // which needs none (folders only).
        let refused = report.failed.iter().any(|(_, why)| {
            why.contains(diroxide_ops::links::NEEDS_DEVELOPER_MODE.trim_end_matches('.'))
        });
        if refused
            && let Then::Linked {
                targets, folder, ..
            } = then
            && !targets.is_empty()
        {
            effects.extend(self.offer_junction(targets, folder));
        }
        effects
    }

    /// A worker's question: a conflict or an item's error as a question
    /// dialog, the focus on the choice that changes nothing; a password as
    /// the masked prompt.
    pub(super) fn asks(&mut self, ask: AskId, question: Question) -> Vec<Effect> {
        match question {
            Question::Conflict {
                conflict,
                replace_undoable,
                ..
            } => {
                let choices = conflict.choices();
                if choices.is_empty() {
                    self.request(Request::Answer {
                        ask,
                        reply: Reply::Conflict(diroxide_core::ConflictAnswer::skip()),
                    });
                    return Vec::new();
                }
                let clock = Arc::clone(&self.clock);
                let mut lines = conflict.lines(clock.as_ref());
                if let Some(folder) = conflict.target.parent() {
                    lines.insert(2.min(lines.len()), format!("In {}", place_leaf(&folder)));
                }
                if !replace_undoable {
                    lines.push("Replacing cannot be undone.".to_owned());
                }
                let rows = choices
                    .iter()
                    .enumerate()
                    .map(|(i, c)| ListRow::new(i.to_string(), c.label.clone()))
                    .collect();
                let mut list = ListModel::new(lines[0].clone()).with_rows(rows);
                list.set_help("Up and Down choose, Enter answers, Escape skips this one.");
                self.open_dialog(
                    Dialog::Question {
                        lines,
                        choices: list,
                    },
                    Purpose::Conflict {
                        ask,
                        conflict,
                        choices,
                    },
                )
            }
            Question::Error { error, .. } => {
                let choices = error.choices();
                let line = error.message().text;
                let rows = choices
                    .iter()
                    .enumerate()
                    .map(|(i, c)| ListRow::new(i.to_string(), c.label()))
                    .collect();
                let mut list = ListModel::new(line.clone()).with_rows(rows);
                list.set_help("Up and Down choose, Enter answers, Escape skips this one.");
                if let Some(skip) = choices
                    .iter()
                    .position(|c| *c == diroxide_core::ErrorAction::Skip)
                {
                    list.move_focus(diroxide_core::Move::To(skip));
                }
                self.open_dialog(
                    Dialog::Question {
                        lines: vec![line],
                        choices: list,
                    },
                    Purpose::ItemError { ask, choices },
                )
            }
            Question::Limit { question, .. } => self.ask_limit(ask, &question),
            Question::Secret { job, request } if request.attempt >= 2 => {
                // The next try: say the last one was wrong first, and ask
                // whether to try again, starting on No (W2b's notes).
                let _ = job;
                let question = with_default_no(diroxide_archive::wrong_password(&request));
                self.open_dialog(
                    Dialog::Confirm {
                        question,
                        listing: Vec::new(),
                        on_yes: Command::new(ids::YES),
                    },
                    Purpose::WrongPassword { ask, request },
                )
            }
            Question::Secret { job, request } => self.secret_prompt(ask, request, job.is_some()),
        }
    }

    /// The masked prompt for a password a worker waits on (ADR-0009): for a
    /// job, the jobs line's "Waiting for the password for course.zip."
    /// first, as a change of state.
    pub(super) fn secret_prompt(
        &mut self,
        ask: AskId,
        request: diroxide_core::SecretRequest,
        for_a_job: bool,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        if for_a_job {
            effects.push(self.say(catalog::waiting_for_secret(&request.subject)));
        }
        let mut dialog = Dialog::secret(request);
        if let Dialog::Secret { shown, .. } = &mut dialog {
            *shown = self.settings.secret_show_toggle;
        }
        effects.extend(self.open_dialog(dialog, Purpose::Secret { ask }));
        effects
    }

    /// Escape outside a dialog: stops the job in front.
    pub(super) fn cancel_front_job(&mut self) -> Vec<Effect> {
        match self.jobs.newest() {
            Some(job) => {
                let word = self
                    .jobs
                    .kind_of(job)
                    .map(|kind| kind.progress_word().to_lowercase())
                    .unwrap_or_else(|| "the job".to_owned());
                vec![
                    Effect::CancelJob(job),
                    self.say(Message::info(format!("{}: cancelling", capitalized(&word)))),
                ]
            }
            None => {
                self.quick = None;
                Vec::new()
            }
        }
    }
}
