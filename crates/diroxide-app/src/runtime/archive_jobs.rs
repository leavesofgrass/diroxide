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
    Allowances, ArchiveError, ArchiveFormat, ArchiveProvider, ArchiveQuestions, JobEnv, JobSecrets,
    PackRequest, UnpackRequest, ZIP_ONLY, plan_pack, plan_rewrite, plan_unpack, run_pack,
    run_rewrite, run_unpack, test_archive,
};
use diroxide_core::{ConflictAnswer, ErrorAction, JobId, JobKind, Message, Place};
use diroxide_ops::{
    Conflict, ConnectionAnswer, ConnectionQuestion, Engine, ItemError, JobControl, JobNotice,
    JobProgress, JobSink,
};
use diroxide_vfs::{CancelToken, OpContext, Registry, SecretSource, ToolRunner, VfsError};

use super::{ForwardSecrets, Shared, forward_sink, lock};
use crate::app::AppEvent;
use crate::request::{ArchivePlan, Question, Reply, RewriteAsk};

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
    tools: Arc<dyn ToolRunner>,
) {
    let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
        job: Some(job),
        shared: Arc::clone(shared),
    });
    let secrets = JobSecrets::new(source);
    let ctx = OpContext::new().with_tools(tools);
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
        Err(error) => Err(needs_seven_zip(&error, &request.source)
            .unwrap_or_else(|| error.message("unpack", &name_of(&request.source)))),
    };
    shared.send(AppEvent::ArchivePlanned { job, result });
}

/// Plans a pack and sends the plan, or why not.
pub(super) fn plan_pack_job(
    shared: &Arc<Shared>,
    registry: &Registry,
    job: JobId,
    request: &PackRequest,
    tools: Arc<dyn ToolRunner>,
) {
    let ctx = OpContext::new().with_tools(tools);
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
    // A rewrite's rows say the rewrite, not the pack that runs it.
    let rewriting = RewriteSink(&sink);
    let job_sink: &dyn JobSink = match plan {
        ArchivePlan::Rewrite { .. } => &rewriting,
        _ => &sink,
    };
    let env = JobEnv::new(engine, archives, job, control, job_sink)
        .with_questions(&questions)
        .with_secrets(source)
        .with_tools(Arc::clone(engine.tools()));
    let (report, lines) = match plan {
        ArchivePlan::Unpack { plan, allow } => {
            let outcome = run_unpack(plan, None, *allow, &env);
            (outcome.report, outcome.lines)
        }
        ArchivePlan::Pack { plan, password } => {
            let outcome = run_pack(plan, password.as_ref(), &env);
            (outcome.report, outcome.lines)
        }
        ArchivePlan::Rewrite { plan } => {
            let mut outcome = run_rewrite(plan, None, &env);
            outcome.report.kind = JobKind::Rewrite;
            if outcome.report.undo.kind.is_some() {
                outcome.report.undo.kind = Some(JobKind::Rewrite);
            }
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
        .with_secrets(source)
        .with_tools(Arc::clone(engine.tools()));
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

/// Plans a rewrite of a zip (Delete, Rename or Make folder inside it) and
/// sends the plan, or why not in words: a refusal said whole; 7z and tar
/// the old read-only sentence ("Archives are read only here. Use Pack to
/// make a new one."); a format read through 7-Zip its own ("RAR is read
/// only: diroxide never writes it").
pub(super) fn plan_rewrite_job(
    shared: &Arc<Shared>,
    archives: &ArchiveProvider,
    ask: &RewriteAsk,
    tools: Arc<dyn ToolRunner>,
) {
    let ctx = OpContext::new().with_tools(tools);
    let planned = plan_rewrite(
        archives,
        &ask.archive,
        &ask.removals,
        &ask.renames,
        &ask.folders,
        ask.job,
        &ctx,
    );
    let result = match planned {
        Ok(plan) => Ok(Box::new(ArchivePlan::Rewrite {
            plan: Box::new(plan),
        })),
        Err(ArchiveError::Refused(sentence)) if sentence == ZIP_ONLY => {
            Err(not_rewritten(archives, &ask.archive, &ctx))
        }
        Err(error) => Err(needs_seven_zip(&error, &ask.archive)
            .unwrap_or_else(|| error.message("change", &name_of(&ask.archive)))),
    };
    shared.send(AppEvent::ArchivePlanned {
        job: ask.job,
        result,
    });
}

/// Why an archive that is not a zip is not changed: a format read through
/// 7-Zip is never written; any other keeps the read-only sentence.
fn not_rewritten(archives: &ArchiveProvider, archive: &Place, ctx: &OpContext) -> Message {
    let format = archives
        .open_archive(archive, ctx, Allowances::all())
        .map(|opened| opened.format())
        .ok();
    match format {
        Some(ArchiveFormat::Tool(signature)) => {
            Message::error(diroxide_archive::sniff::never_written(signature))
        }
        _ => Message::error(diroxide_archive::READ_ONLY),
    }
}

/// The 7-Zip need said with the format first and the way to get it:
/// "RAR needs 7-Zip: not installed. Get it from Manage optional
/// components, then try again." `None` for any other error.
fn needs_seven_zip(error: &ArchiveError, archive: &Place) -> Option<Message> {
    let needs = matches!(
        error,
        ArchiveError::Vfs(VfsError::NeedsHelper { .. })
            | ArchiveError::Vfs(VfsError::UnsupportedFeature { .. })
    );
    let container = archive.archive().map_or(archive, |(c, _)| c);
    let signature = container
        .name()
        .and_then(diroxide_archive::sniff::tool_signature_of_name)?;
    if !needs || signature.is_disk_image() {
        return None;
    }
    Some(Message::error(format!(
        "{}. Get it from Manage optional components, then try again.",
        diroxide_archive::sniff::not_installed_line(signature)
    )))
}

/// A job's sink that says a rewrite's progress as the rewrite's, not the
/// pack's that runs it ("Rewriting course.zip", lane AW).
struct RewriteSink<'a>(&'a dyn JobSink);

impl JobSink for RewriteSink<'_> {
    fn progress(&self, progress: &JobProgress) {
        let mut progress = progress.clone();
        if progress.kind == JobKind::Pack {
            progress.kind = JobKind::Rewrite;
        }
        self.0.progress(&progress);
    }

    fn conflict(&self, conflict: &Conflict) -> ConflictAnswer {
        self.0.conflict(conflict)
    }

    fn error(&self, error: &ItemError) -> ErrorAction {
        self.0.error(error)
    }

    fn connection(&self, question: &ConnectionQuestion) -> ConnectionAnswer {
        self.0.connection(question)
    }

    fn notice(&self, notice: &JobNotice) {
        self.0.notice(notice);
    }
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
