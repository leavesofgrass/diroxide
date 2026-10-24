//! The trash browser's jobs (R25 3.3, commit 85; answer 98): Restore puts
//! items from the trash back where they were, never over a name that is
//! taken; Delete for good and Empty the trash delete items in the trash
//! for good, every one listed first, behind a question that starts on No
//! and asks for the typed count over 1,000 items, as every permanent
//! delete does.
//!
//! The engine reaches the trash through its [`TrashBin`]
//! ([`Engine::with_trash`]): the local provider's system trash in the app,
//! a test-only bin in the tests. A plan holds the items as the trash
//! listed them, so a run acts on exactly the rows the owner chose, even
//! when two items came from one place.

use std::collections::HashSet;

use diroxide_core::format::count;
use diroxide_core::{ConflictPolicy, JobId, JobKind, JobRequest, Place};
use diroxide_vfs::{OpContext, TrashedItem, VfsError};

use crate::engine::{Engine, JobControl, JobReport};
use crate::error::OpsError;
use crate::plan::{Plan, Step};

/// Why a restore skipped an item: its place is taken. Counted on its own
/// in the report ("1 name was taken").
pub const NAME_TAKEN: &str = "its name is taken";

/// Why a restore left out an older item from the same place: the newest
/// one comes back, and the older would find its name taken.
pub const TWIN_LEFT_OUT: &str = "a newer item from the trash comes back to this name";

/// Why a plan from a request left out a source: nothing in the trash came
/// from it.
pub const NOT_IN_TRASH: &str = "nothing in the trash came from here";

/// The report of a trash job in one line, the meaning first, when it went
/// without failures: "Restored 2 items. 1 name was taken."; "Deleted 3
/// items from the trash for good." `None` otherwise, and for other jobs:
/// the usual line says the failures first.
pub(crate) fn summary(report: &JobReport) -> Option<String> {
    let clean = !report.cancelled
        && !report.stopped
        && report.failed.is_empty()
        && report
            .reconciliation
            .as_ref()
            .is_none_or(|check| check.missing.is_empty());
    if !clean {
        return None;
    }
    let done = count(report.done.len() as u64, "item", "items");
    match report.kind {
        JobKind::Restore => {
            let taken = report
                .skipped
                .iter()
                .filter(|(_, why)| why == NAME_TAKEN)
                .count() as u64;
            let other = report.skipped.len() as u64 - taken;
            let mut line = format!("Restored {done}.");
            if taken == 1 {
                line.push_str(" 1 name was taken.");
            } else if taken > 1 {
                line.push_str(&format!(" {taken} names were taken."));
            }
            if other > 0 {
                line.push_str(&format!(" {other} skipped."));
            }
            Some(line)
        }
        JobKind::Purge => {
            let mut line = format!("Deleted {done} from the trash for good.");
            if !report.skipped.is_empty() {
                line.push_str(&format!(" {} skipped.", report.skipped.len()));
            }
            Some(line)
        }
        _ => None,
    }
}

impl Engine {
    /// Plans a restore ([`JobKind::Restore`]) or a delete for good
    /// ([`JobKind::Purge`]) of these items, as the trash browser listed
    /// them. A restore puts back one item a place: the newest, the older
    /// ones left out ([`TWIN_LEFT_OUT`]); a place that is taken now is left
    /// out too ([`NAME_TAKEN`]), and the run checks again before each. A
    /// purge lists every item, asks with No first, and asks for the typed
    /// count over the engine's limit (1,000).
    pub fn plan_trash_items(
        &self,
        job: JobId,
        kind: JobKind,
        mut items: Vec<TrashedItem>,
    ) -> Result<Plan, OpsError> {
        if !matches!(kind, JobKind::Restore | JobKind::Purge) {
            return Err(OpsError::NotBuilt(kind.verb().to_owned()));
        }
        self.trash_bin()?;
        if items.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let mut plan = Plan::new(job, kind, Vec::new(), ConflictPolicy::Skip);
        plan.facts.items = items.len() as u64;
        if kind == JobKind::Restore {
            // The newest first, so the item that comes back to a place is
            // the one deleted last.
            items.sort_by_key(|item| std::cmp::Reverse(item.deleted));
            let mut places = HashSet::new();
            let ctx = OpContext::new();
            for item in items {
                if !places.insert(item.original.clone()) {
                    plan.facts
                        .left_out
                        .push((item.original.clone(), TWIN_LEFT_OUT.to_owned()));
                    continue;
                }
                if self.is_taken(&item.original, &ctx) {
                    plan.facts
                        .left_out
                        .push((item.original.clone(), NAME_TAKEN.to_owned()));
                    continue;
                }
                plan.steps.push(Step::RestoreItem { item });
            }
        } else {
            plan.steps = items.into_iter().map(|item| Step::Purge { item }).collect();
            let entries = plan.steps.len() as u64;
            if entries > self.options().typed_count_over {
                plan.facts.typed = Some(entries.to_string());
            }
        }
        plan.facts.hook_warnings = self.before_hooks(&plan, &JobControl::new())?;
        Ok(plan)
    }

    /// Plans Empty the trash: every item the trash lists, deleted for
    /// good, as [`Engine::plan_trash_items`] plans a purge, said as
    /// emptying ("Empty the trash: delete 12 items for good? y or n").
    /// [`OpsError::TrashEmpty`] when there is nothing in it.
    pub fn plan_empty_trash(&self, job: JobId) -> Result<Plan, OpsError> {
        let items = self.list_trash(&OpContext::new())?;
        if items.is_empty() {
            return Err(OpsError::TrashEmpty);
        }
        let mut plan = self.plan_trash_items(job, JobKind::Purge, items)?;
        plan.facts.emptying = true;
        Ok(plan)
    }

    /// The whole trash, newest first, through the engine's bin; a trash
    /// that cannot be listed is [`OpsError::NoTrashList`].
    pub fn list_trash(&self, ctx: &OpContext) -> Result<Vec<TrashedItem>, OpsError> {
        let bin = self.trash_bin()?;
        match diroxide_vfs::local::list_all(&**bin, ctx) {
            Ok(items) => Ok(items),
            Err(VfsError::Unsupported { .. }) => Err(OpsError::NoTrashList),
            Err(error) => Err(error.into()),
        }
    }

    /// Plans a restore or a purge from a request (a script, `diro run`):
    /// its sources are the places the items were deleted from. A restore
    /// brings back the newest item from each; a purge deletes every item
    /// from each, all of them listed first. A source nothing in the trash
    /// came from is left out ([`NOT_IN_TRASH`]).
    pub(crate) fn plan_trash_request(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        if request.sources.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let listed = self.list_trash(&OpContext::new())?;
        let mut chosen = Vec::new();
        let mut missing = Vec::new();
        for source in &request.sources {
            let mut from: Vec<TrashedItem> = listed
                .iter()
                .filter(|item| same_place(&item.original, source))
                .cloned()
                .collect();
            if from.is_empty() {
                missing.push(source.clone());
                continue;
            }
            if request.kind == JobKind::Restore {
                // The listing is newest first.
                from.truncate(1);
            }
            chosen.extend(from);
        }
        let mut plan = if chosen.is_empty() {
            let mut plan = Plan::new(request.id, request.kind, Vec::new(), ConflictPolicy::Skip);
            plan.facts.items = request.sources.len() as u64;
            plan
        } else {
            self.plan_trash_items(request.id, request.kind, chosen)?
        };
        plan.facts.origin = request.origin.clone();
        plan.facts.error_policy = request.errors;
        plan.facts.left_out.extend(
            missing
                .into_iter()
                .map(|place| (place, NOT_IN_TRASH.to_owned())),
        );
        Ok(plan)
    }

    /// Whether something answers at a place now: a name a restore would
    /// take.
    pub(crate) fn is_taken(&self, place: &Place, ctx: &OpContext) -> bool {
        self.registry()
            .resolve(place)
            .is_ok_and(|provider| provider.stat(place, ctx).is_ok())
    }
}

/// Whether two places name the same entry for the trash: exactly, or on
/// Windows in any case, as the system's own paths compare.
fn same_place(a: &Place, b: &Place) -> bool {
    if a == b {
        return true;
    }
    cfg!(windows) && a.to_string().to_lowercase() == b.to_string().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeProvider;
    use crate::testing;
    use diroxide_core::format::within_cells;
    use diroxide_core::{ErrorAction, JobState};
    use std::sync::Arc;

    use crate::engine::{Conflict, ItemError, JobSink};
    use crate::progress::JobProgress;
    use diroxide_core::ConflictAnswer;

    /// A sink that says yes to nothing: it skips every conflict and every
    /// error, and keeps what it heard.
    #[derive(Default)]
    struct Heard(std::sync::Mutex<Vec<JobState>>);

    impl JobSink for Heard {
        fn progress(&self, progress: &JobProgress) {
            self.0.lock().unwrap().push(progress.state);
        }
        fn conflict(&self, _: &Conflict) -> ConflictAnswer {
            ConflictAnswer::skip()
        }
        fn error(&self, _: &ItemError) -> ErrorAction {
            ErrorAction::Skip
        }
    }

    /// An engine over a fake provider whose trash is its test-only bin,
    /// with report.pdf trashed twice (Tuesday's, then Friday's) and
    /// notes.txt once.
    fn trash_engine() -> (Engine, FakeProvider) {
        let fake = FakeProvider::new();
        fake.add_folder(&testing::p("/work"));
        let ctx = OpContext::new();
        for (name, bytes) in [
            ("report.pdf", &b"tuesday"[..]),
            ("notes.txt", b"notes"),
            ("report.pdf", b"friday"),
        ] {
            let place = testing::p(&format!("/work/{name}"));
            fake.add_file(&place, bytes);
            diroxide_vfs::Provider::trash(&fake, &place, &ctx).unwrap();
        }
        let engine = testing::engine(&fake).with_trash(Arc::new(fake.trash_list()));
        (engine, fake)
    }

    #[test]
    fn a_restore_brings_back_the_newest_and_never_writes_over_a_taken_name() {
        let (engine, fake) = trash_engine();
        let items = engine.list_trash(&OpContext::new()).unwrap();
        assert_eq!(items.len(), 3);
        // The collision case: a new notes.txt took the name.
        fake.add_file(&testing::p("/work/notes.txt"), b"new notes");
        let plan = engine
            .plan_trash_items(JobId::new(1), JobKind::Restore, items)
            .unwrap();
        assert_eq!(plan.steps.len(), 1, "{:?}", plan.facts.left_out);
        let left: Vec<&str> = plan
            .facts
            .left_out
            .iter()
            .map(|(_, why)| why.as_str())
            .collect();
        assert!(left.contains(&NAME_TAKEN) && left.contains(&TWIN_LEFT_OUT));
        assert_eq!(plan.summary(), "Restore 1 item from the trash");
        let question = plan.question().text;
        assert_eq!(question, "Restore 1 item? No is the default. y or n");
        assert!(within_cells(&question, "Restore 1 item?"));
        assert_eq!(plan.caution(), None);
        let report = engine.run(&plan, &JobControl::new(), &Heard::default());
        assert_eq!(
            report.summary(),
            "Restored 1 item. 1 name was taken. 1 skipped."
        );
        assert_eq!(
            fake.contents(&testing::p("/work/report.pdf")).as_deref(),
            Some(&b"friday"[..])
        );
        assert_eq!(
            fake.contents(&testing::p("/work/notes.txt")).as_deref(),
            Some(&b"new notes"[..])
        );
        assert_eq!(fake.trashed(), 2, "Tuesday's report and the old notes stay");
        assert_eq!(report.undo.created, vec![testing::p("/work/report.pdf")]);
        assert!(report.reconciliation.as_ref().is_some_and(|r| r.is_whole()));
    }

    #[test]
    fn a_name_taken_after_the_plan_is_skipped_and_counted() {
        let (engine, fake) = trash_engine();
        let items = engine.list_trash(&OpContext::new()).unwrap();
        let notes: Vec<TrashedItem> = items
            .into_iter()
            .filter(|item| item.name == "notes.txt")
            .collect();
        let plan = engine
            .plan_trash_items(JobId::new(2), JobKind::Restore, notes)
            .unwrap();
        assert_eq!(plan.steps.len(), 1);
        fake.add_file(&testing::p("/work/notes.txt"), b"made meanwhile");
        let report = engine.run(&plan, &JobControl::new(), &Heard::default());
        assert_eq!(report.summary(), "Restored no items. 1 name was taken.");
        assert_eq!(
            fake.contents(&testing::p("/work/notes.txt")).as_deref(),
            Some(&b"made meanwhile"[..])
        );
        assert_eq!(fake.trashed(), 3);
    }

    #[test]
    fn a_purge_lists_every_item_first_and_starts_on_no() {
        let (engine, fake) = trash_engine();
        let items = engine.list_trash(&OpContext::new()).unwrap();
        let reports: Vec<TrashedItem> = items
            .into_iter()
            .filter(|item| item.name == "report.pdf")
            .collect();
        let plan = engine
            .plan_trash_items(JobId::new(3), JobKind::Purge, reports)
            .unwrap();
        assert_eq!(plan.summary(), "Delete 2 items from the trash for good");
        assert_eq!(plan.question().text, "Delete 2 items for good? y or n");
        assert_eq!(
            plan.caution(),
            Some(diroxide_core::catalog::PERMANENT_DELETE_NOTE)
        );
        let line = format!(
            "delete {} from the trash for good",
            testing::p("/work/report.pdf")
        );
        assert_eq!(plan.listing(), vec![line.clone(), line]);
        let report = engine.run(&plan, &JobControl::new(), &Heard::default());
        assert_eq!(report.summary(), "Deleted 2 items from the trash for good.");
        assert_eq!(fake.trashed(), 1);
        assert!(report.undo.created.is_empty() && report.undo.trashed.is_empty());
    }

    #[test]
    fn emptying_a_large_trash_asks_for_the_typed_count() {
        let fake = FakeProvider::new();
        fake.add_folder(&testing::p("/many"));
        let ctx = OpContext::new();
        for n in 0..1_001 {
            let place = testing::p(&format!("/many/f{n}"));
            fake.add_file(&place, b"x");
            diroxide_vfs::Provider::trash(&fake, &place, &ctx).unwrap();
        }
        let engine = testing::engine(&fake).with_trash(Arc::new(fake.trash_list()));
        let plan = engine.plan_empty_trash(JobId::new(4)).unwrap();
        assert!(plan.facts.emptying);
        assert_eq!(plan.summary(), "Empty the trash, 1001 items");
        assert_eq!(plan.facts.typed.as_deref(), Some("1001"));
        assert_eq!(plan.question().text, "Type 1001 to go on, or Escape");
        let mut small = plan.clone();
        small.facts.typed = None;
        assert_eq!(
            small.question().text,
            "Empty the trash: delete 1001 items for good? y or n"
        );
        let report = engine.run(&plan, &JobControl::new(), &Heard::default());
        assert_eq!(report.done.len(), 1_001);
        assert_eq!(fake.trashed(), 0);
        assert_eq!(
            engine.plan_empty_trash(JobId::new(5)),
            Err(OpsError::TrashEmpty)
        );
    }

    #[test]
    fn without_a_trash_to_list_the_jobs_say_so() {
        let fake = FakeProvider::new();
        let engine = testing::engine(&fake);
        let request = JobRequest::new(JobId::new(6), JobKind::Restore, vec![testing::p("/a")]);
        assert_eq!(engine.plan(&request), Err(OpsError::NoTrashList));
        assert_eq!(
            engine.plan_empty_trash(JobId::new(7)),
            Err(OpsError::NoTrashList)
        );
        assert_eq!(
            engine
                .plan_trash_items(JobId::new(8), JobKind::Copy, Vec::new())
                .unwrap_err(),
            OpsError::NotBuilt("copy".to_owned())
        );
    }

    #[test]
    fn a_request_names_the_places_the_items_came_from() {
        let (engine, fake) = trash_engine();
        let request = JobRequest::new(
            JobId::new(9),
            JobKind::Restore,
            vec![testing::p("/work/report.pdf"), testing::p("/work/gone.txt")],
        );
        let plan = engine.plan(&request).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.facts.left_out,
            vec![(testing::p("/work/gone.txt"), NOT_IN_TRASH.to_owned())]
        );
        let report = engine.run(&plan, &JobControl::new(), &Heard::default());
        assert_eq!(report.summary(), "Restored 1 item. 1 skipped.");
        assert_eq!(
            fake.contents(&testing::p("/work/report.pdf")).as_deref(),
            Some(&b"friday"[..])
        );
        let purge = JobRequest::new(
            JobId::new(10),
            JobKind::Purge,
            vec![testing::p("/work/report.pdf")],
        );
        let plan = engine.plan(&purge).unwrap();
        assert_eq!(plan.steps.len(), 1, "only Tuesday's is left in the trash");
    }
}
