//! The archive jobs on workers (plan commits 60 to 62): an unpack or a copy
//! out of an archive, a pack and Test archive, run by the archive crate's
//! runners in the engine's terms (its sink, control and report).
//!
//! A job's passwords live in one [`JobSecrets`], made when it is planned
//! and handed to its run, so a locked 7z asks once per job; it is
//! forgotten when the job ends. A limit's question that comes up while a
//! job plans or runs is asked through the app once per kind
//! ([`Question::Limit`]), starting on No; a password through the masked
//! prompt, "Wrong password" first from the second try.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use diroxide_archive::{
    Allowances, ArchiveError, ArchiveProvider, ArchiveQuestions, JobEnv, JobSecrets, PackRequest,
    UnpackRequest, plan_pack, plan_unpack, run_pack, run_unpack, test_archive,
};
use diroxide_core::{JobId, Message, Place};
use diroxide_ops::{Engine, JobControl};
use diroxide_vfs::{CancelToken, OpContext, Registry, SecretSource};

use super::{ForwardSecrets, Shared, forward_sink, lock};
use crate::app::AppEvent;
use crate::request::{ArchivePlan, Question, Reply};

/// The secrets of the archive jobs between their plan and their run.
pub(super) type JobSecretsMap = Arc<Mutex<HashMap<JobId, Arc<JobSecrets>>>>;

/// Asks a running job's limit questions through the app.
struct ForwardQuestions {
    job: JobId,
    shared: Arc<Shared>,
    cancel: CancelToken,
}

impl ArchiveQuestions for ForwardQuestions {
    fn ask(&self, question: &diroxide_archive::Question) -> bool {
        let asked = Question::Limit {
            job: Some(self.job),
            question: question.clone(),
        };
        matches!(
            self.shared.ask(asked, &self.cancel, Reply::Confirm(false)),
            Reply::Confirm(true)
        )
    }
}

/// The secrets of a job: the ones its plan made, or new ones that ask the
/// app.
fn secrets_of(shared: &Arc<Shared>, map: &JobSecretsMap, job: JobId) -> Arc<JobSecrets> {
    lock(map).remove(&job).unwrap_or_else(|| {
        let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
            job: Some(job),
            shared: Arc::clone(shared),
        });
        JobSecrets::new(source)
    })
}

/// Runs `op` again after each limit question the user answers yes, until
/// it is done or the user says no.
fn asking<T>(
    shared: &Arc<Shared>,
    job: JobId,
    cancel: &CancelToken,
    mut op: impl FnMut(Allowances) -> Result<T, ArchiveError>,
) -> Result<(T, Allowances), ArchiveError> {
    let mut allow = Allowances::default();
    loop {
        match op(allow) {
            Err(ArchiveError::Ask(question)) => {
                let asked = Question::Limit {
                    job: Some(job),
                    question: question.clone(),
                };
                match shared.ask(asked, cancel, Reply::Confirm(false)) {
                    Reply::Confirm(true) => allow = allow.with(question.kind),
                    _ => return Err(ArchiveError::Vfs(question.as_error())),
                }
            }
            other => return other.map(|done| (done, allow)),
        }
    }
}

/// The archive file's name, as the sentences say it.
fn name_of(place: &Place) -> String {
    let container = place.archive().map_or(place, |(c, _)| c);
    container.name().map_or_else(
        || container.to_string(),
        |n| diroxide_core::names::display_name(n).into_owned(),
    )
}

/// Plans an unpack and sends the plan, or why not.
pub(super) fn plan_unpack_job(
    shared: &Arc<Shared>,
    registry: &Registry,
    archives: &ArchiveProvider,
    map: &JobSecretsMap,
    job: JobId,
    request: &UnpackRequest,
) {
    let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
        job: Some(job),
        shared: Arc::clone(shared),
    });
    let secrets = JobSecrets::new(source);
    let ctx = OpContext::new();
    let planned = asking(shared, job, &ctx.cancel, |allow| {
        secrets.with_tries(&ctx, |ctx| {
            plan_unpack(archives, registry, request, job, ctx, allow)
        })
    });
    let result = match planned {
        Ok((plan, allow)) => {
            lock(map).insert(job, Arc::clone(&secrets));
            Ok(Box::new(ArchivePlan::Unpack {
                plan: Box::new(plan),
                allow,
            }))
        }
        Err(error) => Err(error.message("unpack", &name_of(&request.source))),
    };
    shared.send(AppEvent::ArchivePlanned { job, result });
}

/// Plans a pack and sends the plan, or why not.
pub(super) fn plan_pack_job(
    shared: &Arc<Shared>,
    registry: &Registry,
    job: JobId,
    request: &PackRequest,
) {
    let ctx = OpContext::new();
    let result = match plan_pack(registry, request, job, &ctx) {
        Ok(plan) => Ok(Box::new(ArchivePlan::Pack {
            plan: Box::new(plan),
            password: request.password.clone(),
        })),
        Err(error) => {
            let name = request.destination.name().map_or_else(
                || request.destination.to_string(),
                |n| diroxide_core::names::display_name(n).into_owned(),
            );
            Err(error.message("pack", &name))
        }
    };
    shared.send(AppEvent::ArchivePlanned { job, result });
}

/// Runs a confirmed archive job to its end and sends its report and
/// lines.
pub(super) fn run_archive_job(
    shared: &Arc<Shared>,
    engine: &Engine,
    archives: &ArchiveProvider,
    map: &JobSecretsMap,
    control: &JobControl,
    plan: &ArchivePlan,
) {
    let job = plan.job();
    let sink = forward_sink(job, shared, engine, control);
    let questions = ForwardQuestions {
        job,
        shared: Arc::clone(shared),
        cancel: control.cancel.clone(),
    };
    let secrets = secrets_of(shared, map, job);
    // The plan's secrets hold a password only when the plan needed one and
    // it worked (a locked 7z's names): the run reuses it, so the job asks
    // once. Otherwise the run asks the app itself, so each wrong try is
    // forgotten before the next (the runner's own JobSecrets counts them).
    let source: Arc<dyn SecretSource> = if secrets.holds_a_password() {
        secrets.clone()
    } else {
        Arc::new(ForwardSecrets {
            job: Some(job),
            shared: Arc::clone(shared),
        })
    };
    let env = JobEnv::new(engine, archives, job, control, &sink)
        .with_questions(&questions)
        .with_secrets(source);
    let (report, lines) = match plan {
        ArchivePlan::Unpack { plan, allow } => {
            let outcome = run_unpack(plan, None, *allow, &env);
            (outcome.report, outcome.lines)
        }
        ArchivePlan::Pack { plan, password } => {
            let outcome = run_pack(plan, password.as_ref(), &env);
            (outcome.report, outcome.lines)
        }
    };
    secrets.forget();
    shared.send(AppEvent::ArchiveFinished {
        report: Box::new(report),
        lines,
    });
}

/// Tests an archive to its end: every member read and checked, nothing
/// written; sends the report and the verdict's lines.
pub(super) fn test_archive_job(
    shared: &Arc<Shared>,
    engine: &Engine,
    archives: &ArchiveProvider,
    control: &JobControl,
    job: JobId,
    archive: &Place,
) {
    let sink = forward_sink(job, shared, engine, control);
    let questions = ForwardQuestions {
        job,
        shared: Arc::clone(shared),
        cancel: control.cancel.clone(),
    };
    let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
        job: Some(job),
        shared: Arc::clone(shared),
    });
    let env = JobEnv::new(engine, archives, job, control, &sink)
        .with_questions(&questions)
        .with_secrets(source);
    let outcome = test_archive(archive, None, &env);
    let mut lines = vec![outcome.sentence().text];
    for line in outcome.lines() {
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    shared.send(AppEvent::ArchiveFinished {
        report: Box::new(outcome.report),
        lines,
    });
}

/// A plan's sentence when the job could not even be planned.
pub(super) fn no_archives(verb: &str) -> Message {
    Message::could_not(
        verb,
        "the archive",
        "this runtime has no archive provider",
        "",
    )
}
