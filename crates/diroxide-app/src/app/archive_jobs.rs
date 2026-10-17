//! Archive jobs in the app (plan commits 60 and 61): a copy out of an
//! archive routed through the unpack runner, so five thousand files out of
//! a tar.gz read the archive once; the plans of unpack and pack shown as
//! their rows with the question starting on No; the limit questions of a
//! plan asked first, once per kind; the run, its progress and its result
//! in the runner's own lines.

use diroxide_archive::{FolderRule, UnpackRequest};
use diroxide_core::{Command, Effect, JobKind, Message, Place, Redraw, Side};

use super::{App, Flow, Purpose, Then};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::request::{ArchivePlan, Request};

/// The line said when an archive job starts, the verb first: "Unpack
/// started: 4 members into out".
fn start_line(plan: &ArchivePlan) -> String {
    let summary = plan.question().text;
    let summary = summary.trim_end_matches("? y or n");
    match summary.split_once(' ') {
        Some((verb, rest)) => format!("{verb} started: {rest}"),
        None => format!("{summary} started"),
    }
}

impl App {
    /// A copy whose sources are inside an archive: an unpack of those
    /// members into the destination, with no folder made for the archive
    /// (W2b's notes). Never asked of a place outside an archive.
    pub(super) fn copy_out(
        &mut self,
        side: Side,
        sources: Vec<Place>,
        note: Option<String>,
        destination: Place,
    ) -> Vec<Effect> {
        let Some((container, _)) = self.panel(side).place().archive() else {
            return Vec::new();
        };
        let container = container.clone();
        let members: Vec<String> = sources
            .iter()
            .filter_map(|p| p.archive().map(|(_, member)| member.to_owned()))
            .filter(|m| !m.is_empty())
            .collect();
        let mut request = UnpackRequest::new(container, destination);
        request.members = members;
        request.folder_rule = FolderRule::Never;
        let job = self.next_job_id();
        self.flows.insert(
            job,
            Flow {
                side,
                note,
                then: Then::Report,
                confirm: self.settings.confirm.copy,
                kind: JobKind::Unpack,
            },
        );
        self.request(Request::PlanUnpack {
            job,
            request: Box::new(request),
        });
        Vec::new()
    }

    /// An archive job's plan arrived: its refusal said, or its limit
    /// questions asked first, then its confirmation (or the run, when the
    /// flow does not confirm).
    pub(super) fn archive_planned(
        &mut self,
        job: diroxide_core::JobId,
        result: Result<Box<ArchivePlan>, Message>,
    ) -> Vec<Effect> {
        let Some(flow) = self.flows.get(&job).cloned() else {
            self.request(Request::DropArchiveJob(job));
            return Vec::new();
        };
        let plan = match result {
            Ok(plan) => plan,
            Err(message) => {
                self.flows.remove(&job);
                self.jobs.finish(job);
                return vec![Effect::Redraw(Redraw::JobsLine), self.say(message)];
            }
        };
        if let ArchivePlan::Unpack { plan: unpack, .. } = plan.as_ref() {
            if let Some(refusal) = unpack.refusal() {
                self.flows.remove(&job);
                self.request(Request::DropArchiveJob(job));
                let text = format!("{} Nothing was unpacked.", refusal.text);
                return vec![self.say(Message::error(text))];
            }
            if unpack.members.is_empty() {
                self.flows.remove(&job);
                self.request(Request::DropArchiveJob(job));
                return vec![self.say(Message::info("Nothing to unpack."))];
            }
            let mut questions = unpack.questions.clone();
            if !questions.is_empty() {
                let first = questions.remove(0);
                return self.ask_plan_question(plan, first, questions);
            }
        }
        let _ = flow;
        self.confirm_archive(plan)
    }

    /// One of a plan's limit questions, before its confirmation.
    fn ask_plan_question(
        &mut self,
        plan: Box<ArchivePlan>,
        question: diroxide_archive::Question,
        rest: Vec<diroxide_archive::Question>,
    ) -> Vec<Effect> {
        let message = with_default_no(question.message());
        self.open_dialog(
            Dialog::Confirm {
                question: message,
                listing: Vec::new(),
                on_yes: Command::new(ids::YES),
            },
            Purpose::ArchiveQuestion {
                plan,
                question,
                rest,
            },
        )
    }

    /// The answer to one of a plan's limit questions: yes lets that kind
    /// pass for the job; then the next question, or the confirmation.
    pub(super) fn plan_question_answered(
        &mut self,
        mut plan: Box<ArchivePlan>,
        question: &diroxide_archive::Question,
        mut rest: Vec<diroxide_archive::Question>,
        yes: bool,
    ) -> Vec<Effect> {
        if yes && let ArchivePlan::Unpack { allow, .. } = plan.as_mut() {
            *allow = allow.with(question.kind);
        }
        if !rest.is_empty() {
            let next = rest.remove(0);
            return self.ask_plan_question(plan, next, rest);
        }
        self.confirm_archive(plan)
    }

    /// The plan's rows and its question, starting on No; or the run, when
    /// its flow does not confirm (a copy out with the copy question off).
    fn confirm_archive(&mut self, plan: Box<ArchivePlan>) -> Vec<Effect> {
        let job = plan.job();
        let confirm = self.flows.get(&job).is_none_or(|flow| flow.confirm);
        if !confirm {
            return self.run_archive(*plan);
        }
        let question = with_default_no(plan.question());
        let mut listing = Vec::new();
        if let Some(note) = self.flows.get(&job).and_then(|f| f.note.clone()) {
            listing.push(note);
        }
        let other_panel = match plan.as_ref() {
            ArchivePlan::Pack { plan: pack, .. } => {
                let other = self
                    .flows
                    .get(&job)
                    .map(|flow| self.panel(flow.side.other()).place().clone());
                pack.destination.parent() == other
            }
            ArchivePlan::Unpack { .. } => true,
        };
        listing.extend(plan.rows(other_panel));
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Purpose::RunArchive(plan),
        )
    }

    /// Runs a confirmed archive job, exactly as planned.
    pub(super) fn run_archive(&mut self, plan: ArchivePlan) -> Vec<Effect> {
        let line = start_line(&plan);
        let now = self.now();
        let job = plan.job();
        let kind = plan.job_request().kind;
        self.jobs.begin(job, kind, line.clone(), now);
        if let ArchivePlan::Pack { plan: pack, .. } = &plan
            && let Some(folder) = pack.destination.parent()
        {
            self.remember_pack_folder(&folder);
        }
        self.request(Request::RunArchive(Box::new(plan)));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info(line)),
        ]
    }

    /// A confirmation of an archive job said no: nothing changed, and its
    /// passwords are dropped.
    pub(super) fn archive_declined(&mut self, plan: &ArchivePlan) -> Vec<Effect> {
        let job = plan.job();
        self.flows.remove(&job);
        self.jobs.finish(job);
        self.request(Request::DropArchiveJob(job));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info("Nothing changed.")),
        ]
    }

    /// An archive job ended: its result in the runner's own lines, the
    /// summary first; the rest is any job's end (the undo record, the
    /// panels read again).
    pub(super) fn archive_finished(
        &mut self,
        report: diroxide_ops::JobReport,
        lines: Vec<String>,
    ) -> Vec<Effect> {
        self.finish_lines = Some(lines);
        let effects = self.job_finished(report);
        self.finish_lines = None;
        effects
    }

    /// Notes a folder a pack writes into, for the sweep of temporary
    /// archives at the next start.
    pub(super) fn remember_pack_folder(&mut self, folder: &Place) {
        let history = self
            .histories
            .entry(crate::state::keys::PACK_FOLDERS.to_owned())
            .or_default();
        crate::state::remember(history, &folder.to_string());
        history.truncate(PACK_FOLDERS_KEPT);
        let entries = history.clone();
        self.request(Request::SaveHistory {
            key: crate::state::keys::PACK_FOLDERS.to_owned(),
            entries,
        });
    }
}

/// How many folders packs wrote into the app remembers, for the sweep at
/// start.
const PACK_FOLDERS_KEPT: usize = 16;
