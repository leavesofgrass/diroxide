//! The compare and synchronize tests, over the fake provider: the statuses
//! and their words, the count line that adds up, the tolerance, contents,
//! cancel, the plan's modes, toggles and directions, the steps, a run with
//! one undo record, the checks before and during the run, and the hooks a
//! synchronize asks. Every quoted line is checked against the 40 cells.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_core::format::within_cells;
use diroxide_core::{EntryKind, JobId, JobKind, Side, Timestamp};
use diroxide_vfs::{Registry, VfsError};

use super::*;
use crate::engine::{Engine, JobControl, JobReport};
use crate::error::OpsError;
use crate::fake::{CallFault, FakeCall, FakeProvider};
use crate::hooks::{
    HookCall, HookEvent, HookReport, HookRunner, HookVerdict, OnSilence, WakingHook,
};
use crate::plan::Step;
use crate::testing::{Script, engine, p};

fn t(seconds: i64) -> Timestamp {
    Timestamp::from_unix(1_700_000_000 + seconds, 0)
}

fn file(fake: &FakeProvider, path: &str, data: &[u8], seconds: i64) {
    fake.add_file(&p(path), data);
    fake.set_modified(&p(path), t(seconds));
}

fn registry(fake: &FakeProvider) -> Registry {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    registry
}

fn quiet() -> impl CompareSink {
    |_: &CompareProgress| {}
}

fn compare_with(fake: &FakeProvider, criteria: &CompareCriteria) -> Comparison {
    compare_dirs(
        &registry(fake),
        &p("/left"),
        &p("/right"),
        criteria,
        &JobControl::new(),
        &quiet(),
    )
    .unwrap()
}

fn compare(fake: &FakeProvider) -> Comparison {
    compare_with(fake, &CompareCriteria::default())
}

fn row_of<'a>(result: &'a Comparison, path: &str) -> &'a CompareRow {
    result
        .rows
        .iter()
        .find(|row| row.names.join("/") == path)
        .unwrap_or_else(|| panic!("no row {path}"))
}

fn state_of(result: &Comparison, path: &str) -> RowState {
    row_of(result, path).state
}

fn index_of(plan: &SyncPlan, path: &str) -> usize {
    plan.rows
        .iter()
        .position(|row| row.row.names.join("/") == path)
        .unwrap_or_else(|| panic!("no row {path}"))
}

/// The first 40 cells hold `meaning`, and the line has `cells` in all.
fn cells(line: &str, meaning: &str, cells: usize) {
    assert!(within_cells(line, meaning), "{line}");
    assert_eq!(line.chars().count(), cells, "{line}");
}

/// The count line's numbers add up to the rows.
fn adds_up(result: &Comparison) {
    let counts = result.counts();
    let parts: u64 = counts.parts().iter().map(|(n, _)| n).sum();
    assert_eq!(parts + counts.same, counts.all);
    assert_eq!(parts, counts.differences());
    assert_eq!(counts.all, result.rows.len() as u64);
}

/// The plan's summary adds up to the rows.
fn plan_adds_up(plan: &SyncPlan) {
    let counts = plan.counts();
    assert_eq!(counts.sum(), counts.all);
    assert_eq!(counts.all, plan.rows.len() as u64);
}

fn env<'a>(engine: &'a Engine, job: u64, control: &'a JobControl, sink: &'a Script) -> SyncEnv<'a> {
    SyncEnv {
        engine,
        job: JobId::new(job),
        control,
        sink,
        hooks: None,
        hooks_asked: true,
    }
}

/// A tree with a row of most statuses.
fn mixed() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_folder(&p("/left"));
    fake.add_folder(&p("/right"));
    file(&fake, "/left/same.txt", b"a", 0);
    file(&fake, "/right/same.txt", b"a", 0);
    file(&fake, "/left/newer.txt", b"new", 100);
    file(&fake, "/right/newer.txt", b"old", 0);
    file(&fake, "/left/older.txt", b"old", 0);
    file(&fake, "/right/older.txt", b"new", 100);
    file(&fake, "/left/only.txt", b"x", 0);
    file(&fake, "/left/sub/inner.txt", b"y", 0);
    file(&fake, "/right/gone.txt", b"z", 0);
    file(&fake, "/left/clash", b"file", 0);
    fake.add_folder(&p("/right/clash"));
    file(&fake, "/left/differs.txt", b"one", 0);
    file(&fake, "/right/differs.txt", b"three", 0);
    fake
}

#[test]
fn every_status_has_its_row_and_the_count_line_adds_up() {
    let result = compare(&mixed());
    assert_eq!(state_of(&result, "same.txt"), RowState::Same);
    assert_eq!(state_of(&result, "newer.txt"), RowState::Different);
    assert_eq!(row_of(&result, "newer.txt").newer(), Some(Side::Left));
    assert_eq!(row_of(&result, "older.txt").newer(), Some(Side::Right));
    assert_eq!(state_of(&result, "only.txt"), RowState::OnlyLeft);
    assert_eq!(state_of(&result, "sub"), RowState::OnlyLeft);
    assert_eq!(state_of(&result, "sub/inner.txt"), RowState::OnlyLeft);
    assert_eq!(state_of(&result, "gone.txt"), RowState::OnlyRight);
    assert_eq!(state_of(&result, "clash"), RowState::TypeDiffers);
    assert_eq!(state_of(&result, "differs.txt"), RowState::Different);
    adds_up(&result);
    let line = result.count_line();
    assert_eq!(
        line,
        "8 differences in 9 items: 1 type differs, 3 different, 3 only here, 1 only there. 1 same."
    );
    cells(&line, "8 differences in 9 items", 89);
    let lines = result.lines(false);
    assert_eq!(lines[0], line);
    assert_eq!(
        lines[1],
        "Compared by size and time; times within 2 seconds count as the same."
    );
    // Problems first, then the differences; the same rows only on request.
    assert_eq!(lines[2], "Type differs, clash, file here, folder there");
    cells(&lines[2], "Type differs, clash", 44);
    let differs = "Different, differs.txt, larger there, same time";
    assert!(lines.contains(&differs.to_owned()));
    cells(differs, "Different, differs.txt", 47);
    assert!(lines.contains(&"Different, newer.txt, newer here".to_owned()));
    let inner = "Only here, inner.txt, in sub";
    assert!(lines.contains(&inner.to_owned()));
    cells(inner, inner, 28);
    assert!(!lines.iter().any(|line| line.starts_with("Same")));
    assert!(result.lines(true).contains(&"Same, same.txt".to_owned()));
    // Select the differences: what differs on each side.
    assert_eq!(result.differing(Side::Right).len(), 5);
}

#[test]
fn the_count_line_says_empty_and_no_differences() {
    let fake = FakeProvider::new();
    fake.add_folder(&p("/left"));
    fake.add_folder(&p("/right"));
    assert_eq!(compare(&fake).count_line(), "Both folders are empty.");
    file(&fake, "/left/a.txt", b"a", 0);
    file(&fake, "/right/a.txt", b"a", 0);
    let result = compare(&fake);
    assert_eq!(result.count_line(), "No differences in 1 item.");
    adds_up(&result);
}

#[test]
fn the_tolerance_is_two_seconds_and_said_when_it_decided() {
    let fake = FakeProvider::new();
    file(&fake, "/left/a.txt", b"a", 0);
    fake.add_file(&p("/right/a.txt"), b"a");
    fake.set_modified(
        &p("/right/a.txt"),
        Timestamp::from_unix(1_700_000_001, 900_000_000),
    );
    file(&fake, "/left/b.txt", b"b", 0);
    file(&fake, "/right/b.txt", b"b", 3);
    file(&fake, "/left/c.txt", b"c", 0);
    file(&fake, "/right/c.txt", b"c", 0);
    let result = compare(&fake);
    let a = row_of(&result, "a.txt");
    assert_eq!(a.state, RowState::Same);
    let line = a.line('/');
    assert_eq!(line, "Same, a.txt, times within 2 seconds");
    cells(&line, "Same, a.txt, times within 2 seconds", 35);
    assert_eq!(
        row_of(&result, "b.txt").line('/'),
        "Different, b.txt, newer there"
    );
    // Exactly equal times say nothing more.
    assert_eq!(row_of(&result, "c.txt").line('/'), "Same, c.txt");
    assert_eq!(result.criteria.tolerance, TOLERANCE);
}

#[test]
fn an_hour_apart_counts_as_the_same_only_when_asked() {
    let fake = FakeProvider::new();
    file(&fake, "/left/a.txt", b"a", 0);
    file(&fake, "/right/a.txt", b"a", 3_601);
    assert_eq!(row_of(&compare(&fake), "a.txt").newer(), Some(Side::Right));
    let criteria = CompareCriteria {
        ignore_one_hour: true,
        ..CompareCriteria::default()
    };
    let result = compare_with(&fake, &criteria);
    assert_eq!(
        row_of(&result, "a.txt").line('/'),
        "Same, a.txt, times 1 hour apart"
    );
    let note = "Times 1 hour apart count as the same: daylight saving.";
    assert_eq!(result.notes()[1], note);
    cells(note, "Times 1 hour apart count as the same", 54);
}

#[test]
fn contents_are_compared_block_by_block_when_the_sizes_match() {
    let fake = FakeProvider::new();
    file(&fake, "/left/a.txt", b"abc", 0);
    file(&fake, "/right/a.txt", b"abd", 0);
    file(&fake, "/left/b.txt", b"same", 0);
    file(&fake, "/right/b.txt", b"same", 50);
    file(&fake, "/left/c.txt", b"short", 0);
    file(&fake, "/right/c.txt", b"longer one", 0);
    let quick = compare(&fake);
    assert_eq!(state_of(&quick, "a.txt"), RowState::Same);
    assert_eq!(row_of(&quick, "b.txt").newer(), Some(Side::Right));
    let criteria = CompareCriteria {
        by: CompareBy::Content,
        ..CompareCriteria::default()
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let seen = Arc::clone(&seen);
        move |progress: &CompareProgress| seen.lock().unwrap().push(progress.clone())
    };
    let result = compare_dirs(
        &registry(&fake),
        &p("/left"),
        &p("/right"),
        &criteria,
        &JobControl::new(),
        &sink,
    )
    .unwrap();
    let a = row_of(&result, "a.txt").line('/');
    assert_eq!(a, "Different, a.txt, same size, content differs");
    cells(&a, "Different, a.txt, same size", 44);
    let b = row_of(&result, "b.txt");
    assert_eq!(b.state, RowState::Same);
    assert_eq!(b.line('/'), "Same, b.txt, times differ");
    assert_eq!(state_of(&result, "c.txt"), RowState::Different);
    assert_eq!(
        result.notes(),
        vec!["Compared by contents where the sizes match.".to_owned()]
    );
    let seen = seen.lock().unwrap();
    let last = seen.last().unwrap();
    assert!(last.done);
    assert_eq!(last.bytes_read, 14);
    assert_eq!(last.describe(), "Compare finished: 3 entries");
}

#[test]
fn progress_reads_meaning_first() {
    let progress = CompareProgress {
        entries: 1_200,
        folder: "photos".to_owned(),
        ..CompareProgress::default()
    };
    let line = progress.describe();
    assert_eq!(line, "Comparing: 1,200 entries, in photos");
    cells(&line, "Comparing: 1,200 entries", 35);
    let reading = CompareProgress {
        bytes_read: 3_100_000,
        ..progress.clone()
    };
    assert_eq!(
        reading.describe(),
        "Comparing contents: 1,200 entries, 3.1 MB read, in photos"
    );
    let paused = CompareProgress {
        paused: true,
        ..progress
    };
    assert!(within_cells(
        &paused.describe(),
        "Compare paused: 1,200 entries"
    ));
}

#[test]
fn a_cancel_keeps_what_was_compared_and_says_the_rest() {
    let fake = mixed();
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::List,
        p("/left/sub"),
        VfsError::Cancelled,
    ));
    let result = compare(&fake);
    assert!(result.cancelled);
    assert_eq!(state_of(&result, "sub"), RowState::NotCompared);
    assert!(
        result
            .rows
            .iter()
            .all(|row| row.names.join("/") != "sub/inner.txt")
    );
    adds_up(&result);
    let line = result.count_line();
    assert_eq!(
        line,
        "Cancelled, 7 differences in 8 items: 1 type differs, 1 not compared, 3 different, 1 only here, 1 only there. 1 same."
    );
    cells(&line, "Cancelled, 7 differences in 8 items", 116);
    // The plan never acts on what was not compared, and says it is partial.
    let plan = plan_sync(&result, SyncMode::UpdateThere);
    assert_eq!(
        plan.notes()[0],
        "The compare was cancelled: this plan is partial."
    );
    let sub = index_of(&plan, "sub");
    assert_eq!(plan.line(sub), "Not compared: sub");
    assert!(!plan.acts(sub));
    plan_adds_up(&plan);

    let control = JobControl::new();
    control.cancel.cancel();
    let early = compare_dirs(
        &registry(&fake),
        &p("/left"),
        &p("/right"),
        &CompareCriteria::default(),
        &control,
        &quiet(),
    )
    .unwrap_err();
    // Cancelled before the roots were read: nothing to show, said as such.
    assert!(
        matches!(early, OpsError::Vfs(VfsError::Cancelled)),
        "{early:?}"
    );
}

#[test]
fn a_folder_inside_the_other_or_a_file_is_refused() {
    let fake = mixed();
    let registry = registry(&fake);
    let control = JobControl::new();
    let inside = compare_dirs(
        &registry,
        &p("/left"),
        &p("/left/sub"),
        &CompareCriteria::default(),
        &control,
        &quiet(),
    )
    .unwrap_err();
    assert!(
        inside
            .to_string()
            .starts_with("Cannot compare: one folder is inside")
    );
    let file = compare_dirs(
        &registry,
        &p("/left/only.txt"),
        &p("/right"),
        &CompareCriteria::default(),
        &control,
        &quiet(),
    )
    .unwrap_err();
    assert!(file.to_string().starts_with("Cannot compare:"), "{file}");
    assert!(file.to_string().ends_with("is not a folder"));
}

#[test]
fn links_are_compared_by_target_and_never_followed() {
    let fake = FakeProvider::new();
    fake.add_folder(&p("/left"));
    fake.add_folder(&p("/right"));
    file(&fake, "/outside/big.txt", b"big", 0);
    fake.add_link(&p("/left/out"), "/outside", true);
    fake.add_link(&p("/right/out"), "/elsewhere", true);
    let result = compare(&fake);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        row_of(&result, "out").line('/'),
        "Different, out, the links point elsewhere"
    );
}

#[test]
fn hidden_and_filtered_entries_are_skipped_rows() {
    #[derive(Debug)]
    struct NoTemp;
    impl EntryFilter for NoTemp {
        fn keep(&self, _names: &[String], entry: &diroxide_core::Entry) -> bool {
            !entry.name.ends_with(".tmp")
        }
        fn reason(&self) -> String {
            "matches the filter *.tmp".to_owned()
        }
    }
    let fake = mixed();
    file(&fake, "/right/cache.tmp", b"t", 0);
    let criteria = CompareCriteria {
        filter: Some(Arc::new(NoTemp)),
        ..CompareCriteria::default()
    };
    let result = compare_with(&fake, &criteria);
    let line = row_of(&result, "cache.tmp").line('/');
    assert_eq!(line, "Skipped, cache.tmp, matches the filter *.tmp");
    adds_up(&result);
    // A mirror never removes what the filter skipped.
    let plan = plan_sync(&result, SyncMode::MirrorThere);
    let cache = index_of(&plan, "cache.tmp");
    assert!(!plan.acts(cache));
    assert_eq!(
        plan.line(cache),
        "Skip: cache.tmp, matches the filter *.tmp"
    );
}

#[test]
fn subfolders_off_still_lists_a_folder_on_one_side() {
    let fake = mixed();
    file(&fake, "/left/both/a.txt", b"1", 0);
    file(&fake, "/right/both/a.txt", b"2", 0);
    let criteria = CompareCriteria {
        subfolders: false,
        ..CompareCriteria::default()
    };
    let result = compare_with(&fake, &criteria);
    assert!(
        result
            .rows
            .iter()
            .all(|row| row.names.join("/") != "both/a.txt")
    );
    assert_eq!(state_of(&result, "sub/inner.txt"), RowState::OnlyLeft);
}

#[test]
fn an_unreadable_folder_is_a_row_and_the_compare_goes_on() {
    let fake = mixed();
    file(&fake, "/left/locked/a.txt", b"a", 0);
    file(&fake, "/right/locked/a.txt", b"a", 0);
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::List,
        p("/right/locked"),
        VfsError::PermissionDenied,
    ));
    let result = compare(&fake);
    let row = row_of(&result, "locked");
    assert_eq!(row.state, RowState::Unreadable);
    assert!(
        row.line('/').starts_with("Unreadable there, locked, "),
        "{}",
        row.line('/')
    );
    assert_eq!(state_of(&result, "same.txt"), RowState::Same);
    let plan = plan_sync(&result, SyncMode::MirrorThere);
    let at = index_of(&plan, "locked");
    assert!(plan.line(at).starts_with("Unreadable there: locked, "));
    assert!(
        !plan
            .steps()
            .unwrap()
            .iter()
            .any(|(_, step)| step.subject() == &p("/right/locked"))
    );
}

#[test]
fn a_compare_across_two_providers_works() {
    let local = mixed();
    let remote = FakeProvider::new();
    let root = diroxide_core::Place::parse("sftp://host/srv").unwrap();
    remote.add_folder(&root);
    remote.add_file(&root.join("same.txt").unwrap(), b"a");
    remote.set_modified(&root.join("same.txt").unwrap(), t(1));
    let mut registry = registry(&local);
    registry.register("sftp", Arc::new(remote.clone()));
    let result = compare_dirs(
        &registry,
        &p("/left"),
        &root,
        &CompareCriteria::default(),
        &JobControl::new(),
        &quiet(),
    )
    .unwrap();
    assert_eq!(result.right_facts.scheme, "sftp");
    assert_eq!(
        row_of(&result, "same.txt").line('/'),
        "Same, same.txt, times within 2 seconds"
    );
    assert_eq!(state_of(&result, "only.txt"), RowState::OnlyLeft);
    assert_eq!(row_of(&result, "only.txt").kind(), EntryKind::File);
}

// ----- the plan -----

#[test]
fn update_copies_missing_and_newer_and_says_problems_first() {
    let result = compare(&mixed());
    let plan = plan_sync(&result, SyncMode::UpdateThere);
    plan_adds_up(&plan);
    let summary = plan.summary();
    assert_eq!(
        summary,
        "Update the other panel: 2 problems, 4 to copy there, 2 skipped, 1 same; 5 bytes to copy"
    );
    cells(&summary, "Update the other panel: 2 problems", 87);
    let lines = plan.lines(false);
    assert_eq!(lines[0], summary);
    assert_eq!(
        lines[1],
        "Compared by size and time; times within 2 seconds count as the same."
    );
    assert_eq!(lines[2], "Type differs: clash, file here, folder there");
    cells(&lines[2], "Type differs: clash", 44);
    assert_eq!(
        lines[3],
        "Choose a direction: differs.txt, larger there, same time"
    );
    cells(&lines[3], "Choose a direction: differs.txt", 56);
    assert_eq!(
        lines[4],
        "Copy there: newer.txt, newer here, old one to the trash"
    );
    cells(&lines[4], "Copy there: newer.txt, newer here", 55);
    assert_eq!(lines[5], "Copy there: only.txt, only here");
    cells(&lines[5], "Copy there: only.txt, only here", 31);
    assert_eq!(lines[6], "Copy there: sub, new folder");
    assert_eq!(lines[7], "Copy there: inner.txt, in sub, only here");
    assert_eq!(lines[8], "Skip: gone.txt, only there, kept");
    assert_eq!(lines[9], "Skip: older.txt, newer there");
    assert_eq!(lines.len(), 10);
    assert!(plan.lines(true).contains(&"Same: same.txt".to_owned()));
    let here = plan_sync(&result, SyncMode::UpdateHere);
    plan_adds_up(&here);
    assert_eq!(
        here.summary(),
        "Update this panel: 2 problems, 2 to copy here, 4 skipped, 1 same; 4 bytes to copy"
    );
}

#[test]
fn both_ways_copies_each_way_and_never_removes() {
    let plan = plan_sync(&compare(&mixed()), SyncMode::BothWays);
    plan_adds_up(&plan);
    let steps = plan.steps().unwrap();
    assert!(
        steps
            .iter()
            .all(|(_, step)| matches!(step, Step::Copy { .. }))
    );
    assert_eq!(
        plan.summary(),
        "Both ways, newer wins: 2 problems, 4 to copy there, 2 to copy here, 1 same; 9 bytes to copy"
    );
    let older = index_of(&plan, "older.txt");
    assert_eq!(
        plan.line(older),
        "Copy here: older.txt, newer there, old one to the trash"
    );
    // Equal times with different sizes are never a guess.
    let differs = index_of(&plan, "differs.txt");
    assert_eq!(
        plan.status(differs),
        Some(RowStatus::Problem(SyncProblem::NeedsDirection))
    );
}

#[test]
fn a_mirror_lists_every_removal_first_and_trashes_a_folder_in_one_step() {
    let fake = mixed();
    file(&fake, "/right/old/a.txt", b"a", 0);
    file(&fake, "/right/old/b.txt", b"b", 0);
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    plan_adds_up(&plan);
    let lines = plan.lines(false);
    assert_eq!(lines[2], "Type differs: clash, file here, folder there");
    assert_eq!(lines[3], "Will be removed there: gone.txt, to the trash");
    cells(&lines[3], "Will be removed there: gone.txt", 45);
    assert_eq!(lines[4], "Will be removed there: old, to the trash");
    assert_eq!(
        lines[5],
        "Will be removed there: a.txt, in old, to the trash"
    );
    assert_eq!(
        lines[6],
        "Will be removed there: b.txt, in old, to the trash"
    );
    // Then what replaces a newer file, before the plain copies.
    assert_eq!(
        lines[7],
        "Copy there: older.txt, replaces a newer file there, old one to the trash"
    );
    cells(&lines[7], "Copy there: older.txt, replaces a", 72);
    assert!(lines.contains(
        &"Copy there: differs.txt, larger there, same time, old one to the trash".to_owned()
    ));
    assert_eq!(
        plan.summary(),
        "Mirror to the other panel: 1 problem, 4 to remove there, 6 to copy there, 1 same; 11 bytes to copy"
    );
    let trashes: Vec<_> = plan
        .steps()
        .unwrap()
        .into_iter()
        .filter_map(|(_, step)| match step {
            Step::Trash { place } => Some(place),
            _ => None,
        })
        .collect();
    assert_eq!(trashes, vec![p("/right/gone.txt"), p("/right/old")]);
}

#[test]
fn without_a_trash_removals_are_for_good_deepest_first_and_said() {
    let fake = mixed();
    fake.switches_mut().no_trash = true;
    file(&fake, "/right/old/a.txt", b"a", 0);
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    let notes = plan.notes();
    assert_eq!(notes[0], "6 changes cannot be undone: no trash there.");
    cells(&notes[0], "6 changes cannot be undone", 43);
    let line = "Will be removed there for good: a.txt, in old, no trash there";
    assert!(plan.lines(false).contains(&line.to_owned()));
    cells(line, "Will be removed there for good: a.txt", 61);
    let removes: Vec<_> = plan
        .steps()
        .unwrap()
        .into_iter()
        .filter_map(|(_, step)| match step {
            Step::Remove { place } => Some(place),
            _ => None,
        })
        .collect();
    assert_eq!(
        removes,
        vec![p("/right/old/a.txt"), p("/right/old"), p("/right/gone.txt")]
    );
    let newer = index_of(&plan, "newer.txt");
    assert_eq!(
        plan.line(newer),
        "Copy there: newer.txt, newer here, replaces for good, no trash there"
    );
    assert!(plan.summary().contains("3 to remove there for good"));
}

#[test]
fn space_leaves_out_a_folder_with_what_is_inside() {
    let mut plan = plan_sync(&compare(&mixed()), SyncMode::UpdateThere);
    let sub = index_of(&plan, "sub");
    let inner = index_of(&plan, "sub/inner.txt");
    let said = plan.toggle(sub).unwrap();
    assert_eq!(said, "Left out: sub, would copy there");
    cells(&said, "Left out: sub, would copy there", 31);
    assert_eq!(plan.status(inner), Some(RowStatus::LeftOut));
    plan_adds_up(&plan);
    // A file put back while its folder is out cannot be copied.
    plan.toggle(inner).unwrap();
    assert_eq!(
        plan.status(inner),
        Some(RowStatus::Problem(SyncProblem::FolderLeftOut))
    );
    assert_eq!(plan.line(inner), "Folder left out: inner.txt, in sub");
    assert!(!plan.steps().unwrap().iter().any(|(at, _)| *at == inner));
    let clash = index_of(&plan, "clash");
    let refused = plan.toggle(clash).unwrap_err().to_string();
    assert_eq!(
        refused,
        "Cannot include clash: a type that differs is never resolved"
    );
    let same = index_of(&plan, "same.txt");
    assert_eq!(
        plan.toggle(same).unwrap_err().to_string(),
        "Nothing to do: same.txt"
    );
}

#[test]
fn a_direction_settles_a_row_that_differs() {
    let mut plan = plan_sync(&compare(&mixed()), SyncMode::UpdateThere);
    let differs = index_of(&plan, "differs.txt");
    let said = plan.set_direction(differs, Side::Left).unwrap();
    assert_eq!(
        said,
        "Copy here: differs.txt, larger there, same time, old one to the trash"
    );
    cells(&said, "Copy here: differs.txt", 69);
    assert!(plan.acts(differs));
    let only = index_of(&plan, "only.txt");
    let refused = plan.set_direction(only, Side::Left).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Cannot copy only.txt here: it is not there"
    );
    plan.reset(differs).unwrap();
    assert!(!plan.acts(differs));
}

#[test]
fn a_mirror_folder_with_something_left_out_is_kept() {
    let fake = mixed();
    file(&fake, "/right/old/a.txt", b"a", 0);
    file(&fake, "/right/old/b.txt", b"b", 0);
    let mut plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    let old = index_of(&plan, "old");
    let a = index_of(&plan, "old/a.txt");
    plan.toggle(a).unwrap();
    assert_eq!(plan.status(old), Some(RowStatus::Kept));
    let line = plan.line(old);
    assert_eq!(
        line,
        "Kept: old, something inside is left out; would be removed there"
    );
    cells(&line, "Kept: old, something inside is left", 63);
    plan_adds_up(&plan);
    let trashes: Vec<_> = plan
        .steps()
        .unwrap()
        .into_iter()
        .filter_map(|(_, step)| match step {
            Step::Trash { place } => Some(place),
            _ => None,
        })
        .collect();
    assert_eq!(trashes, vec![p("/right/gone.txt"), p("/right/old/b.txt")]);
}

#[test]
fn a_read_only_side_is_a_problem() {
    let mut result = compare(&mixed());
    result.right_facts.writable = false;
    let mut plan = plan_sync(&result, SyncMode::UpdateThere);
    let only = index_of(&plan, "only.txt");
    assert_eq!(plan.line(only), "Read-only there: only.txt");
    assert!(plan.steps().unwrap().is_empty());
    assert!(plan.toggle(only).is_err());
}

// ----- the run -----

#[test]
fn a_mirror_runs_as_one_job_with_one_undo_record() {
    let fake = mixed();
    file(&fake, "/right/old/a.txt", b"a", 0);
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    let job = plan.job_plan(JobId::new(5)).unwrap();
    // The engine checks every step's entry against the compare's.
    assert_eq!(job.expected.len(), job.steps.len());
    assert_eq!(job.kind, JobKind::Copy);
    let engine = engine(&fake);
    let control = JobControl::new();
    let sink = Script::default();
    let report = run_sync(&plan, &env(&engine, 5, &control, &sink)).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!report.cancelled);
    assert_eq!(fake.contents(&p("/right/newer.txt")).unwrap(), b"new");
    assert_eq!(fake.contents(&p("/right/older.txt")).unwrap(), b"old");
    assert_eq!(fake.contents(&p("/right/differs.txt")).unwrap(), b"one");
    assert_eq!(fake.contents(&p("/right/sub/inner.txt")).unwrap(), b"y");
    assert!(!fake.exists(&p("/right/gone.txt")));
    assert!(!fake.exists(&p("/right/old")));
    assert!(fake.in_trash(&p("/right/gone.txt")));
    // One record: the removals, the old files replaced, the copies.
    let undo = &report.undo;
    assert_eq!(undo.job, Some(JobId::new(5)));
    assert!(undo.trashed.contains(&p("/right/gone.txt")));
    assert!(undo.trashed.contains(&p("/right/old")));
    assert!(undo.trashed.contains(&p("/right/newer.txt")));
    assert!(undo.created.contains(&p("/right/only.txt")));
    // Only the type that differs is left.
    let after = compare(&fake);
    let left: Vec<_> = after
        .rows
        .iter()
        .filter(|row| row.state != RowState::Same)
        .map(|row| row.names.join("/"))
        .collect();
    assert_eq!(left, vec!["clash".to_owned()]);
}

#[test]
fn nothing_to_do_is_refused() {
    let fake = FakeProvider::new();
    file(&fake, "/left/a.txt", b"a", 0);
    file(&fake, "/right/a.txt", b"a", 0);
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    assert_eq!(
        plan.summary(),
        "Mirror to the other panel: nothing to do, 1 same"
    );
    let engine = engine(&fake);
    let control = JobControl::new();
    let sink = Script::default();
    assert!(matches!(
        run_sync(&plan, &env(&engine, 1, &control, &sink)),
        Err(OpsError::NoChange)
    ));
}

#[test]
fn an_entry_changed_since_the_compare_is_skipped_never_overwritten() {
    let fake = mixed();
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    // After the compare: the right's newer.txt is edited, gone.txt goes,
    // and a file appears where only.txt was to go.
    file(&fake, "/right/newer.txt", b"edited", 200);
    fake.vanish(&p("/right/gone.txt"));
    file(&fake, "/right/only.txt", b"someone else's", 0);
    let engine = engine(&fake);
    let control = JobControl::new();
    let sink = Script::default();
    let report = run_sync(&plan, &env(&engine, 2, &control, &sink)).unwrap();
    let skipped = |path: &str| {
        report
            .skipped
            .iter()
            .find(|(place, _)| place == &p(path))
            .map(|(_, why)| why.clone())
    };
    assert_eq!(skipped("/left/newer.txt").as_deref(), Some(CHANGED_SINCE));
    assert_eq!(skipped("/right/gone.txt").as_deref(), Some(ALREADY_GONE));
    assert_eq!(skipped("/left/only.txt").as_deref(), Some(TAKEN_SINCE));
    assert_eq!(fake.contents(&p("/right/newer.txt")).unwrap(), b"edited");
    assert_eq!(
        fake.contents(&p("/right/only.txt")).unwrap(),
        b"someone else's"
    );
}

#[test]
fn a_replace_goes_through_the_trash_even_when_the_setting_is_off() {
    let fake = mixed();
    let plan = plan_sync(&compare(&fake), SyncMode::UpdateThere);
    let options = crate::options::EngineOptions {
        replace_keeps_old_in_trash: false,
        ..crate::options::EngineOptions::default()
    };
    let engine = engine(&fake).with_options(options);
    let control = JobControl::new();
    let sink = Script::default();
    let report = run_sync(&plan, &env(&engine, 3, &control, &sink)).unwrap();
    assert!(report.undo.trashed.contains(&p("/right/newer.txt")));
    assert!(fake.trash_contents().iter().any(|data| data == b"old"));
}

// ----- hooks -----

#[derive(Debug, Default)]
struct Recorder {
    veto: Option<HookEvent>,
    calls: Mutex<Vec<HookCall>>,
    afters: Mutex<Vec<HookReport>>,
}

impl HookRunner for Recorder {
    fn waking(&self, call: &HookCall) -> Vec<WakingHook> {
        self.calls.lock().unwrap().push(call.clone());
        vec![WakingHook {
            plugin: "guard".to_owned(),
            name: "Folder guard".to_owned(),
            index: 0,
            timeout: Duration::from_secs(2),
            on_timeout: OnSilence::Continue,
        }]
    }

    fn before(&self, _hook: &WakingHook, call: &HookCall) -> HookVerdict {
        if self.veto == Some(call.event) {
            HookVerdict::Cancel("Delete stopped: old is kept (Folder guard)".to_owned())
        } else {
            HookVerdict::Continue
        }
    }

    fn after(&self, report: &HookReport) {
        self.afters.lock().unwrap().push(report.clone());
    }
}

fn hooked(fake: &FakeProvider, runner: &Arc<Recorder>) -> Engine {
    engine(fake).with_hooks(Arc::clone(runner) as Arc<dyn HookRunner>)
}

#[test]
fn a_mirror_asks_the_copy_hooks_and_the_delete_hooks() {
    let fake = mixed();
    let runner = Arc::new(Recorder::default());
    let engine = hooked(&fake, &runner);
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    let control = JobControl::new();
    let warnings = plan.before_hooks(&engine, JobId::new(9), &control).unwrap();
    assert!(warnings.is_empty());
    let events: Vec<_> = runner
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.event)
        .collect();
    assert_eq!(events, vec![HookEvent::Copy, HookEvent::Delete]);
    let calls = plan.hook_calls(JobId::new(9)).unwrap();
    assert_eq!(calls[1].items[0].from, p("/right/gone.txt"));

    let sink = Script::default();
    let env = SyncEnv {
        hooks: Some(Arc::clone(&runner) as Arc<dyn HookRunner>),
        ..env(&engine, 9, &control, &sink)
    };
    let report = run_sync(&plan, &env).unwrap();
    let started = Instant::now();
    while runner.afters.lock().unwrap().len() < 2 && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let afters = runner.afters.lock().unwrap();
    let mut events: Vec<_> = afters.iter().map(|report| report.event).collect();
    events.sort_by_key(|event| event.before());
    assert_eq!(events, vec![HookEvent::Copy, HookEvent::Delete]);
    let delete = afters
        .iter()
        .find(|r| r.event == HookEvent::Delete)
        .unwrap();
    assert_eq!(delete.items.len(), 1);
    assert_eq!(delete.items[0].from, p("/right/gone.txt"));
    // The split P5 can reuse: each part holds only its own places.
    let job = plan.job_plan(JobId::new(9)).unwrap();
    let parts: Vec<JobReport> = split_report(&job, &report);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].kind, JobKind::Copy);
    assert!(!parts[0].done.contains(&p("/right/gone.txt")));
    assert_eq!(parts[1].kind, JobKind::Trash);
    assert_eq!(parts[1].done, vec![p("/right/gone.txt")]);
}

#[test]
fn a_delete_hook_that_stops_a_mirror_stops_it_before_anything_is_touched() {
    let fake = mixed();
    let runner = Arc::new(Recorder {
        veto: Some(HookEvent::Delete),
        ..Recorder::default()
    });
    let engine = hooked(&fake, &runner);
    let plan = plan_sync(&compare(&fake), SyncMode::MirrorThere);
    let control = JobControl::new();
    let sink = Script::default();
    let env = SyncEnv {
        hooks_asked: false,
        ..env(&engine, 4, &control, &sink)
    };
    let error = run_sync(&plan, &env).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Delete stopped: old is kept (Folder guard)"
    );
    assert!(fake.exists(&p("/right/gone.txt")));
    assert!(!fake.exists(&p("/right/only.txt")));
}

#[test]
fn every_mode_has_words_that_start_with_their_meaning() {
    let words: Vec<String> = SyncMode::ALL.iter().map(|mode| mode.words()).collect();
    assert_eq!(
        words,
        vec![
            "Update the other panel",
            "Update this panel",
            "Both ways, newer wins",
            "Mirror to the other panel",
            "Mirror to this panel",
        ]
    );
    assert!(words.iter().all(|word| word.chars().count() <= 40));
    assert!(SyncMode::MirrorHere.removes() && !SyncMode::BothWays.removes());
}
