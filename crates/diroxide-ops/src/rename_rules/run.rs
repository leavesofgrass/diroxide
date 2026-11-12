//! From a plan the owner said yes to, to one job: the folders read again,
//! the plan made again against them, any change refused in words, then
//! the engine's own planning and run, so the job has the engine's order
//! (chains from their free end, a temporary name for every swap, ring and
//! case-only change), its journal and its one undo record.

use std::collections::HashSet;

use diroxide_core::{JobId, Place, UtcOffset};

use super::plan::{
    FieldLookup, FolderFacts, RenameContext, RenamePlan, RenameRefused, RenameSource,
};
use super::plan_rename_with;
use crate::engine::{Engine, JobControl, JobReport, JobSink};

/// What a run needs: the engine, the job's number, its control and the
/// sink it reports to, and the facts planning read the first time.
pub struct RenameEnv<'a> {
    /// The engine, with its providers, journal and hooks.
    pub engine: &'a Engine,
    /// The job's number.
    pub job: JobId,
    /// Cancel and pause.
    pub control: &'a JobControl,
    /// Where progress, conflicts and errors go.
    pub sink: &'a dyn JobSink,
    /// The `{meta:FIELD}` values, as at the preview.
    pub lookup: Option<&'a dyn FieldLookup>,
    /// The offset dates are written at, as at the preview.
    pub offset: UtcOffset,
}

impl std::fmt::Debug for RenameEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenameEnv")
            .field("job", &self.job)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

/// Reads each source folder's facts through the engine's providers: the
/// names there now, whether case matters, the place's name rules. A
/// folder that cannot be read gets [`FolderFacts::unknown`], the stricter
/// guess. Blocking: one listing per folder.
pub fn read_context<'a>(
    engine: &Engine,
    sources: &[RenameSource],
    control: &JobControl,
) -> RenameContext<'a> {
    let ctx = engine.context(&control.cancel);
    let mut seen: HashSet<Place> = HashSet::new();
    let mut context = RenameContext::new();
    for source in sources {
        let Some(folder) = source.place.parent() else {
            continue;
        };
        if !seen.insert(folder.clone()) {
            continue;
        }
        let facts = match engine.registry().resolve(&folder) {
            Ok(provider) => {
                let capabilities = provider.capabilities(&folder);
                match provider.list(&folder, &ctx) {
                    Ok(entries) => FolderFacts::from_capabilities(
                        entries.into_iter().map(|entry| entry.name).collect(),
                        &capabilities,
                    ),
                    Err(_) => FolderFacts::unknown(),
                }
            }
            Err(_) => FolderFacts::unknown(),
        };
        context = context.with_folder(folder, facts);
    }
    context
}

/// Reads each source again through its provider, so the plan made at the
/// owner's yes sees the entries as they are. A source that cannot be read
/// is marked gone, so its row says "source is gone".
fn fresh_sources(
    engine: &Engine,
    sources: &[RenameSource],
    control: &JobControl,
) -> Vec<RenameSource> {
    let ctx = engine.context(&control.cancel);
    sources
        .iter()
        .map(|source| {
            let entry = engine
                .registry()
                .resolve(&source.place)
                .and_then(|provider| provider.stat(&source.place, &ctx))
                .map(|entry| RenameSource::new(source.place.clone(), entry));
            entry.unwrap_or_else(|_| source.clone().gone())
        })
        .collect()
}

/// Runs a plan the owner said yes to. The folders and the sources are
/// read again and the plan is made again with the same recipe and
/// choices: a new problem, or a different set of names, refuses the run
/// before anything is touched ("The folder changed since the preview:
/// ..."). Then the engine plans the pairs, which is where the steps are
/// ordered and temporary names taken, and runs them. The report's undo
/// record is the whole batch, one Undo; the app keeps it on its undo log,
/// and the engine's journal (when it has one) has it on disk before the
/// first step, so it still works after the dialog closes or the program
/// stops.
pub fn run_rename(plan: &RenamePlan, env: &RenameEnv<'_>) -> Result<JobReport, RenameRefused> {
    plan.request(env.job)?;
    let sources = fresh_sources(env.engine, plan.sources(), env.control);
    let mut context = read_context(env.engine, &sources, env.control)
        .with_offset(env.offset)
        .portable(plan.is_portable());
    if let Some(lookup) = env.lookup {
        context = context.with_lookup(lookup);
    }
    let fresh = plan_rename_with(&sources, plan.recipe(), &context, plan.choices())
        .map_err(|error| RenameRefused::Recipe(error.line()))?;
    if fresh.problems() > 0 {
        let first = fresh
            .rows()
            .iter()
            .find(|row| row.state.is_problem())
            .map(|row| row.detail())
            .unwrap_or_default();
        return Err(RenameRefused::Changed(format!(
            "The folder changed since the preview: {first}."
        )));
    }
    if fresh.pairs() != plan.pairs() {
        return Err(RenameRefused::Changed(
            "The names changed since the preview. Preview again.".to_owned(),
        ));
    }
    let request = fresh.request(env.job)?;
    let engine_plan = env
        .engine
        .plan_with(&request, env.control, env.sink)
        .map_err(|error| RenameRefused::Engine(error.to_string()))?;
    Ok(env.engine.run(&engine_plan, env.control, env.sink))
}
