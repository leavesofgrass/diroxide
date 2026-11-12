//! The runner against a fixed clock (spike H2: R16's scenario list as the
//! runner's own tests), over real files in the build's scratch folder and
//! a fake tool. No real program runs and nothing plays sound; hostile
//! names stay in memory (the ledger's and the log's tests).

use std::fs;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diroxide_vfs::WatchEvent;

use super::actions::ActionResult;
use super::approval::Approval;
use super::ledger::Outcome;
use super::once::{EXIT_COULD_NOT_RUN, EXIT_DONE, EXIT_PARTLY, run_once};
use super::runner::{Checkpoint, FileOutcome, HotNote, RunnerOptions, StartError};
use super::store::LOCK_FILE;
use super::task::CatchUpChoice;
use super::testing::{Act, World, file_lines, place, said};

fn paused(notes: &[HotNote]) -> Vec<String> {
    notes
        .iter()
        .filter_map(|n| match n {
            HotNote::Paused(line) => Some(line.clone()),
            _ => None,
        })
        .collect()
}

fn outcomes(notes: &[HotNote]) -> Vec<(String, FileOutcome)> {
    notes
        .iter()
        .filter_map(|n| match n {
            HotNote::File(file) => Some((file.name.clone(), file.outcome)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_settled_pdf_is_converted_and_its_original_moved() {
    let world = World::new();
    let mut runner = world.runner(world.task(""));
    let notes = runner.start(world.now()).unwrap();
    assert!(notes.contains(&HotNote::Started {
        waiting: 0,
        polled: None
    }));
    world.drop_in("scan-004.pdf", b"page one");
    world.drop_in("notes.txt", b"not a pdf");
    let mut notes = runner.rescan(world.now());
    // Not yet: two seconds of quiet first.
    notes.extend(world.run_for(&mut runner, 1));
    assert!(file_lines(&notes).is_empty());
    notes.extend(world.run_for(&mut runner, 2));
    assert_eq!(file_lines(&notes), ["Done: scan-004.pdf, under a second"]);
    assert_eq!(world.names("in"), ["notes.txt"]);
    assert_eq!(world.names("out"), ["scan-004.txt"]);
    assert_eq!(world.names("processed"), ["scan-004.pdf"]);
    assert_eq!(
        fs::read(world.folder("out").join("scan-004.txt")).unwrap(),
        b"made from page one"
    );
    assert_eq!(world.tools.ran(), ["scan-004.pdf"]);
    // The summary comes once, after the pace, and nothing more.
    let notes = world.run_for(&mut runner, 11);
    assert_eq!(said(&notes), ["Scans: 1 done"]);
    assert!(said(&world.run_for(&mut runner, 20)).is_empty());
    // The log says it with the outcome first and the time last.
    let log = runner.store().log().newest(10);
    assert!(
        log[0].starts_with("done  scan-004.pdf  pdf-to-text  under a second  "),
        "{log:?}"
    );
    assert!(log[0].contains("out"), "{log:?}");
    assert!(
        log.last().unwrap().starts_with("started  Scans. Watches "),
        "{log:?}"
    );
    // The ledger has the made line, then the done line.
    let ledger = runner.store().ledger().unwrap();
    assert_eq!(
        ledger.newest("scan-004.pdf").map(|l| l.outcome),
        Some(Outcome::Done)
    );
}

#[test]
fn a_file_written_in_parts_waits_for_its_quiet_period() {
    let world = World::new();
    let mut runner = world.runner(world.task("[run]\nretries = 3\n"));
    runner.start(world.now()).unwrap();
    // Written for 5 seconds with a pause of 1.5 seconds, under the quiet.
    let mut notes = Vec::new();
    let path = world.folder("in").join("scan.pdf");
    for part in 0..5u8 {
        let mut bytes = fs::read(&path).unwrap_or_default();
        bytes.extend_from_slice(&[b'a' + part; 100]);
        fs::write(&path, &bytes).unwrap();
        notes.extend(runner.event(&WatchEvent::Changed(place(&path)), world.now()));
        let wait = if part == 2 { 3 } else { 2 };
        for _ in 0..wait {
            notes.extend(world.settle_now(&mut runner));
            world.pass(Duration::from_millis(500));
        }
    }
    assert!(file_lines(&notes).is_empty(), "{notes:?}");
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    assert_eq!(world.tools.ran(), ["scan.pdf"]);
    assert_eq!(
        fs::read(world.folder("out").join("scan.txt"))
            .unwrap()
            .len(),
        "made from ".len() + 500
    );
}

#[test]
fn a_file_renamed_in_settles_after_half_a_second() {
    let world = World::new();
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    let outside = world.dir.path().join("scan.pdf");
    fs::write(&outside, b"finished elsewhere").unwrap();
    let inside = world.folder("in").join("scan.pdf");
    fs::rename(&outside, &inside).unwrap();
    let mut notes = runner.event(&WatchEvent::RenamedIn(place(&inside)), world.now());
    notes.extend(world.run_for(&mut runner, 1));
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
}

#[test]
fn a_name_that_goes_before_it_settles_is_never_run() {
    let world = World::new();
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("brief.pdf", b"x");
    let path = world.folder("in").join("brief.pdf");
    let mut notes = runner.event(&WatchEvent::Created(place(&path)), world.now());
    notes.extend(world.run_for(&mut runner, 1));
    fs::remove_file(&path).unwrap();
    notes.extend(runner.event(&WatchEvent::Removed(place(&path)), world.now()));
    notes.extend(world.run_for(&mut runner, 5));
    assert!(file_lines(&notes).is_empty());
    assert!(world.tools.ran().is_empty());
    assert!(runner.is_idle());
}

#[test]
fn a_file_in_use_is_tried_again_after_ten_seconds() {
    let world = World::new();
    world.tools.on("scan.pdf", Act::Fail("the file is in use"));
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Retrying scan.pdf in 10 seconds"]);
    assert_eq!(world.names("in"), ["scan.pdf"]);
    assert!(world.names("out").is_empty(), "the half output is gone");
    world.tools.on("scan.pdf", Act::Write);
    // Ran at two seconds in; the retry is due at twelve.
    let notes = world.run_for(&mut runner, 8);
    assert!(file_lines(&notes).is_empty());
    let notes = world.run_for(&mut runner, 2);
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    assert_eq!(world.tools.ran(), ["scan.pdf", "scan.pdf"]);
}

#[test]
fn retries_wait_longer_each_time_and_end_in_failed() {
    let world = World::new();
    world.tools.on("slow.pdf", Act::TimedOut);
    let mut runner = world.runner(world.task("[run]\nretries = 2\n"));
    runner.start(world.now()).unwrap();
    world.drop_in("slow.pdf", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    notes.extend(world.run_for(&mut runner, 10));
    notes.extend(world.run_for(&mut runner, 61));
    assert_eq!(
        file_lines(&notes),
        [
            "Retrying slow.pdf in 10 seconds",
            "Retrying slow.pdf in 1 minute",
            "Failed: slow.pdf, pdftotext took longer than 600 seconds and was stopped",
        ]
    );
    assert_eq!(world.names("failed"), ["slow.pdf"]);
    assert_eq!(world.tools.ran().len(), 3);
    let log = runner.store().log().newest(1);
    assert!(
        log[0].starts_with("failed  slow.pdf  pdftotext took longer"),
        "{log:?}"
    );
}

#[test]
fn a_damaged_file_goes_to_failed_at_once_and_the_next_goes_on() {
    let world = World::new();
    world.tools.on(
        "bad.pdf",
        Act::Fail("Syntax Error: Couldn't read xref table"),
    );
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("bad.pdf", b"x");
    world.drop_in("good.pdf", b"y");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(
        file_lines(&notes),
        [
            "Failed: bad.pdf, pdftotext ended with code 1: Syntax Error: Couldn't read xref table",
            "Done: good.pdf, under a second",
        ]
    );
    assert_eq!(world.names("failed"), ["bad.pdf"]);
    assert_eq!(world.names("processed"), ["good.pdf"]);
    assert_eq!(world.names("out"), ["good.txt"]);
    let notes = world.run_for(&mut runner, 11);
    assert_eq!(said(&notes), ["Scans: 1 done, 1 failed"]);
}

#[test]
fn a_missing_tool_pauses_the_task_and_resume_goes_on() {
    let world = World::new();
    *world.tools.has.lock().unwrap() = false;
    let mut runner = world.runner(world.task(""));
    world.drop_in("scan.pdf", b"x");
    let notes = runner
        .start_with(world.now(), Some(CatchUpChoice::All))
        .unwrap();
    assert_eq!(paused(&notes), ["Scans paused: pdftotext is not installed"]);
    assert!(said(&notes).contains(&"Scans paused: pdftotext is not installed".to_owned()));
    let notes = world.run_for(&mut runner, 5);
    assert!(file_lines(&notes).is_empty());
    assert_eq!(world.names("in"), ["scan.pdf"]);
    assert_eq!(
        runner.store().state().paused.as_deref(),
        Some("pdftotext is not installed")
    );
    // Still missing: resume pauses again, once.
    let notes = runner.resume(world.now());
    assert_eq!(paused(&notes), ["Scans paused: pdftotext is not installed"]);
    *world.tools.has.lock().unwrap() = true;
    let mut notes = runner.resume(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert!(notes.contains(&HotNote::Resumed));
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
}

#[test]
fn ten_failures_in_a_row_with_one_cause_pause_the_task() {
    let world = World::new();
    for n in 1..=11 {
        let name = format!("bad-{n:02}.pdf");
        world.tools.on(
            &name,
            Act::Fail("Syntax Error: Couldn't find trailer dictionary"),
        );
        world.drop_in(&name, b"x");
    }
    let mut runner = world.runner(world.task(""));
    let mut notes = runner
        .start_with(world.now(), Some(CatchUpChoice::All))
        .unwrap();
    assert!(said(&notes).contains(&"11 files were waiting in Scans".to_owned()));
    notes.extend(world.run_for(&mut runner, 3));
    let failed = outcomes(&notes)
        .iter()
        .filter(|(_, o)| *o == FileOutcome::Failed)
        .count();
    assert_eq!(failed, 10);
    assert_eq!(paused(&notes), ["Scans paused: 10 failures in a row"]);
    assert_eq!(world.names("in"), ["bad-11.pdf"]);
    assert_eq!(world.names("failed").len(), 10);
    // Resume starts the count again: the eleventh fails alone.
    let mut notes = runner.resume(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert!(paused(&notes).is_empty());
    assert_eq!(world.names("failed").len(), 11);
}

#[test]
fn one_bad_file_retried_never_trips_the_breaker_and_a_success_resets_it() {
    let world = World::new();
    // In natural order: nine bad, a good one, a busy one tried four
    // times, nine bad.
    let mut names = Vec::new();
    for n in 1..=9 {
        names.push(format!("a-{n}.pdf"));
        names.push(format!("d-{n}.pdf"));
    }
    for name in &names {
        world.tools.on(
            name,
            Act::Fail("Syntax Error: Couldn't find trailer dictionary"),
        );
        world.drop_in(name, b"x");
    }
    world.drop_in("b-good.pdf", b"x");
    world
        .tools
        .on("c-busy.pdf", Act::Fail("the file is in use"));
    world.drop_in("c-busy.pdf", b"x");
    let mut runner = world.runner(world.task(""));
    let mut notes = runner
        .start_with(world.now(), Some(CatchUpChoice::All))
        .unwrap();
    notes.extend(world.run_for(&mut runner, 3));
    notes.extend(world.run_for(&mut runner, 10));
    notes.extend(world.run_for(&mut runner, 60));
    notes.extend(world.run_for(&mut runner, 600));
    assert!(paused(&notes).is_empty(), "{:?}", paused(&notes));
    assert_eq!(world.names("failed").len(), 19);
    assert_eq!(world.names("processed"), ["b-good.pdf"]);
    assert_eq!(
        world
            .tools
            .ran()
            .iter()
            .filter(|n| *n == "c-busy.pdf")
            .count(),
        4
    );
}

#[test]
fn catch_up_asks_once_then_follows_the_answer() {
    let world = World::new();
    world.drop_in("one.pdf", b"1");
    world.drop_in("two.pdf", b"2");
    let mut runner = world.runner(world.task(""));
    match runner.start(world.now()) {
        Err(StartError::AskCatchUp {
            waiting,
            question,
            yes_first,
        }) => {
            assert_eq!(waiting, 2);
            assert_eq!(question, "Process the 2 files in Scans? y or n");
            assert!(yes_first);
        }
        other => panic!("{other:?}"),
    }
    // The question took no lock.
    assert!(runner.store().lock().unwrap().is_some());
    runner
        .store()
        .remember_catch_up(CatchUpChoice::None)
        .unwrap();
    let notes = runner.start(world.now()).unwrap();
    assert!(notes.contains(&HotNote::Started {
        waiting: 0,
        polled: None
    }));
    let notes = world.run_for(&mut runner, 5);
    assert!(file_lines(&notes).is_empty());
    assert_eq!(world.names("in"), ["one.pdf", "two.pdf"]);
    // A new arrival still runs.
    world.drop_in("three.pdf", b"3");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Done: three.pdf, under a second"]);
}

#[test]
fn a_restart_catches_up_on_what_came_while_it_was_off() {
    let world = World::new();
    let mut runner = world.runner(world.task(""));
    runner
        .store()
        .remember_catch_up(CatchUpChoice::All)
        .unwrap();
    runner.start(world.now()).unwrap();
    drop(runner);
    world.drop_in("scan-10.pdf", b"10");
    world.drop_in("scan-9.pdf", b"9");
    let mut runner = world.runner(world.task(""));
    let mut notes = runner.start(world.now()).unwrap();
    assert!(said(&notes).contains(&"2 files were waiting in Scans".to_owned()));
    notes.extend(world.run_for(&mut runner, 3));
    // Natural order: 9 before 10.
    assert_eq!(world.tools.ran(), ["scan-9.pdf", "scan-10.pdf"]);
    assert_eq!(world.names("processed"), ["scan-10.pdf", "scan-9.pdf"]);
}

/// Spike H3 in process: the runner stops as a crash would after the
/// output is committed and before the original moves; the next start
/// finishes the move and converts nothing twice, which the ledger proves.
#[test]
fn a_crash_between_commit_and_move_is_finished_and_never_converted_twice() {
    let world = World::new();
    let crash = Arc::new(AtomicBool::new(true));
    let armed = Arc::clone(&crash);
    let mut runner = world.runner(world.task("")).with_checkpoint(Arc::new(
        move |at: Checkpoint, _name: &str| {
            if at == Checkpoint::Committed && armed.load(Ordering::SeqCst) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        },
    ));
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"page");
    runner.rescan(world.now());
    world.run_for(&mut runner, 3);
    assert!(runner.is_stopped());
    assert_eq!(world.names("out"), ["scan.txt"]);
    assert_eq!(world.names("in"), ["scan.pdf"]);
    let ledger = runner.store().ledger().unwrap();
    let unfinished: Vec<&str> = ledger
        .unfinished()
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(unfinished, ["scan.pdf"]);
    drop(runner);
    crash.store(false, Ordering::SeqCst);
    let mut runner = world.runner(world.task(""));
    let mut notes = runner.start(world.now()).unwrap();
    notes.extend(world.run_for(&mut runner, 5));
    assert_eq!(
        file_lines(&notes),
        ["Done: scan.pdf, finished after a restart"]
    );
    assert_eq!(world.tools.ran(), ["scan.pdf"], "converted once");
    assert_eq!(world.names("out"), ["scan.txt"]);
    assert_eq!(world.names("processed"), ["scan.pdf"]);
    let text = fs::read_to_string(runner.store().file("ledger.txt")).unwrap();
    let words: Vec<&str> = text
        .lines()
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(words, ["made", "done"]);
}

#[test]
fn a_second_runner_is_refused_while_the_first_holds_the_task() {
    let world = World::new();
    let mut first = world.runner(world.task(""));
    first.start(world.now()).unwrap();
    let mut second = world.runner(world.task(""));
    assert_eq!(
        second.start(world.now()),
        Err(StartError::Locked(
            "Scans is run by another diro".to_owned()
        ))
    );
    assert!(
        world
            .dir
            .path()
            .join("state")
            .join("hot")
            .join("scans")
            .join(LOCK_FILE)
            .exists()
    );
    drop(first);
    assert!(second.start(world.now()).is_ok());
}

#[test]
fn the_tool_version_comes_from_the_engines_tools_first() {
    use super::approval::Resolved;
    let world = World::new();
    let version = |runner: &super::runner::TaskRunner| match runner.resolve().unwrap() {
        Resolved::Recipe { version, .. } => version,
        other => panic!("{other:?}"),
    };
    // The runner reads no version: the frontend's answer stands.
    let runner = world.bare_runner(world.task(""));
    assert_eq!(version(&runner).as_deref(), Some("25.03.0"));
    // The engine's tools runner reads one: it wins.
    *world.tools.version.lock().unwrap() = Some("4.00".to_owned());
    let runner = world.bare_runner(world.task(""));
    assert_eq!(version(&runner).as_deref(), Some("4.00"));
    // A new version changes the approval key: asked again.
    let (_, before) = world.bare_runner(world.task("")).approval().unwrap();
    *world.tools.version.lock().unwrap() = Some("4.01".to_owned());
    let (_, after) = world.bare_runner(world.task("")).approval().unwrap();
    assert_ne!(before, after);
}

#[test]
fn a_task_runs_only_as_approved() {
    let world = World::new();
    let mut never = world.bare_runner(world.task(""));
    match never.start(world.now()) {
        Err(StartError::NotApproved {
            approval, question, ..
        }) => {
            assert_eq!(approval, Approval::NotYet);
            assert_eq!(question, "Turn on Scans? y or n");
        }
        other => panic!("{other:?}"),
    }
    let runner = world.runner(world.task(""));
    assert_eq!(runner.approval().unwrap().0, Approval::Approved);
    drop(runner);
    // The task changed in its file: off until approved again.
    let mut skip = world.bare_runner(world.task("[run]\nconflicts = \"skip\"\n"));
    match skip.start(world.now()) {
        Err(StartError::NotApproved {
            approval, question, ..
        }) => {
            assert_eq!(approval, Approval::Changed);
            assert_eq!(question, "Scans changed. Turn it on again? y or n");
        }
        other => panic!("{other:?}"),
    }
    let mut skip = world.runner(world.task("[run]\nconflicts = \"skip\"\n"));
    assert!(skip.start(world.now()).is_ok());
}

#[test]
fn a_folder_that_goes_away_pauses_the_task_until_it_comes_back() {
    let world = World::new();
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    fs::remove_dir(world.folder("in")).unwrap();
    let notes = runner.event(&WatchEvent::Gone, world.now());
    assert_eq!(paused(&notes), ["Scans paused: folder is missing"]);
    let notes = world.run_for(&mut runner, 31);
    assert!(paused(&notes).is_empty());
    fs::create_dir(world.folder("in")).unwrap();
    world.drop_in("back.pdf", b"x");
    let mut notes = world.run_for(&mut runner, 31);
    notes.extend(world.run_for(&mut runner, 3));
    assert!(notes.contains(&HotNote::Resumed), "{notes:?}");
    assert_eq!(file_lines(&notes), ["Done: back.pdf, under a second"]);
}

#[test]
fn a_file_replaced_during_the_run_runs_again() {
    let world = World::new();
    world.tools.on("scan.pdf", Act::Grow);
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"part");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Changed during the run: scan.pdf"]);
    assert_eq!(world.names("in"), ["scan.pdf"]);
    world.tools.on("scan.pdf", Act::Write);
    let notes = world.run_for(&mut runner, 4);
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    // The first output was made from half a file; nothing was replaced.
    assert_eq!(world.names("out"), ["scan (2).txt", "scan.txt"]);
    assert_eq!(world.names("processed"), ["scan.pdf"]);
}

#[test]
fn a_thousand_arrivals_in_one_tick_run_in_natural_order() {
    let world = World::new();
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    for n in (1..=1000).rev() {
        world.drop_in(&format!("p-{n}.pdf"), b"x");
    }
    // The overflow: one rescan for all of them.
    let mut notes = runner.event(&WatchEvent::Rescan, world.now());
    notes.extend(world.run_for(&mut runner, 3));
    let ran = world.tools.ran();
    assert_eq!(ran.len(), 1000);
    assert_eq!(ran[0], "p-1.pdf");
    assert_eq!(ran[8], "p-9.pdf");
    assert_eq!(ran[9], "p-10.pdf");
    assert_eq!(ran[999], "p-1000.pdf");
    assert_eq!(world.names("processed").len(), 1000);
    let notes = world.run_for(&mut runner, 11);
    assert_eq!(said(&notes), ["Scans: 1000 done"]);
}

#[test]
fn the_queue_cap_holds_files_in_in_and_says_so_once() {
    let world = World::new();
    let mut runner = world.runner(world.task("")).with_options(RunnerOptions {
        queue_cap: 3,
        space_floor: 0,
        space_percent: 0,
        ..RunnerOptions::default()
    });
    runner.start(world.now()).unwrap();
    for n in 1..=5 {
        world.drop_in(&format!("p-{n}.pdf"), b"x");
    }
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(said(&notes), ["Scans: 3 files waiting"]);
    assert_eq!(world.names("processed").len(), 5);
}

#[test]
fn low_space_where_results_go_pauses_the_task() {
    let world = World::new();
    let mut runner = world.runner(world.task("")).with_options(RunnerOptions {
        space_floor: u64::MAX,
        ..RunnerOptions::default()
    });
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    let lines = paused(&notes);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("Scans paused: only "), "{lines:?}");
    assert!(lines[0].ends_with(" free"), "{lines:?}");
    assert_eq!(world.names("in"), ["scan.pdf"]);
}

#[test]
fn leave_mode_handles_a_file_once_and_again_when_it_changes() {
    let world = World::new();
    let mut runner = world.runner(world.task("[run]\nafter = \"leave\"\n"));
    world.drop_in("scan.pdf", b"one");
    runner
        .start_with(world.now(), Some(CatchUpChoice::All))
        .unwrap();
    let notes = world.run_for(&mut runner, 3);
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    assert_eq!(world.names("in"), ["scan.pdf"]);
    drop(runner);
    let mut runner = world.runner(world.task("[run]\nafter = \"leave\"\n"));
    runner
        .start_with(world.now(), Some(CatchUpChoice::All))
        .unwrap();
    let notes = world.run_for(&mut runner, 3);
    assert!(file_lines(&notes).is_empty(), "handled once");
    std::thread::sleep(Duration::from_millis(20));
    world.drop_in("scan.pdf", b"one, edited");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    assert_eq!(world.names("out"), ["scan (2).txt", "scan.txt"]);
}

#[test]
fn skip_leaves_an_existing_output_and_files_the_original() {
    let world = World::new();
    fs::write(world.folder("out").join("scan.txt"), b"older").unwrap();
    let mut runner = world.runner(world.task("[run]\nconflicts = \"skip\"\n"));
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Skipped scan.pdf: output exists"]);
    assert_eq!(
        fs::read(world.folder("out").join("scan.txt")).unwrap(),
        b"older"
    );
    assert_eq!(world.names("processed"), ["scan.pdf"]);
    assert!(world.tools.ran().is_empty());
}

#[test]
fn the_dry_run_touches_nothing_and_says_why() {
    let world = World::new();
    world.drop_in("scan-2.pdf", b"x");
    world.drop_in("scan-10.pdf", b"x");
    world.drop_in("notes.txt", b"x");
    world.drop_in("big.pdf.part", b"x");
    let runner = world.runner(world.task(""));
    let dry = runner.preview().unwrap();
    assert_eq!(
        dry.lines,
        [
            "Skipped big.pdf.part: unfinished name",
            "Skipped notes.txt: not a match",
            "Would convert scan-2.pdf",
            "Would convert scan-10.pdf",
        ]
    );
    assert_eq!(dry.summary, "Dry run: 2 would convert, 2 skipped");
    assert!(world.tools.ran().is_empty());
    assert!(!runner.store().file("ledger.txt").exists());
    assert!(!runner.store().file("log.txt").exists());
}

#[test]
fn once_runs_what_waits_and_ends_with_the_familys_codes() {
    let world = World::new();
    world.drop_in("good.pdf", b"x");
    let runner = world.runner(world.task(""));
    let clock = world.clock.clone();
    let mut sleep = |d: Duration| world.pass(d);
    let report = run_once(runner, &*clock, &mut sleep);
    assert_eq!(report.exit_code(), EXIT_DONE);
    assert_eq!(report.lines, ["Done: good.pdf, under a second"]);
    assert_eq!(report.summary(), "Scans: 1 done");

    world.tools.on("bad.pdf", Act::Fail("Syntax Error"));
    world.tools.on("busy.pdf", Act::Fail("the file is in use"));
    world.drop_in("bad.pdf", b"x");
    world.drop_in("busy.pdf", b"x");
    world.drop_in("fine.pdf", b"x");
    let runner = world.runner(world.task(""));
    let report = run_once(runner, &*clock, &mut |d: Duration| world.pass(d));
    assert_eq!(report.exit_code(), EXIT_PARTLY);
    assert_eq!((report.done, report.failed, report.left), (1, 1, 1));
    assert!(
        report
            .lines
            .contains(&"Left for the next run: busy.pdf".to_owned())
    );
    assert_eq!(world.names("in"), ["busy.pdf"]);

    // Changed since approved: it could not run.
    let changed = world.bare_runner(world.task("[run]\nparallel = 3\n"));
    let report = run_once(changed, &*clock, &mut |d: Duration| world.pass(d));
    assert_eq!(report.exit_code(), EXIT_COULD_NOT_RUN);
    assert_eq!(report.summary(), "Scans changed. Turn it on again? y or n");
}

#[test]
fn a_tell_task_says_each_arrival_once_and_touches_nothing() {
    let world = World::new();
    let tell = world.task_from(
        "name = \"scans\"\nkind = \"tell\"\n[trigger]\nhome = '{home}'\n[say]\nannounce = \"each\"\n",
    );
    let mut runner = world.runner(tell.clone());
    runner.start(world.now()).unwrap();
    world.drop_in("photo.jpg", b"12345678");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Arrived: photo.jpg, 8 bytes"]);
    assert_eq!(said(&notes), ["Arrived: photo.jpg, 8 bytes"]);
    assert_eq!(world.names("in"), ["photo.jpg"]);
    assert!(world.tools.ran().is_empty());
    drop(runner);
    let mut runner = world.runner(tell);
    runner
        .start_with(world.now(), Some(CatchUpChoice::All))
        .unwrap();
    let notes = world.run_for(&mut runner, 3);
    assert!(file_lines(&notes).is_empty());
}

#[test]
fn a_move_task_sorts_into_its_folder_and_keeps_both_on_a_taken_name() {
    let world = World::new();
    let software = world.dir.path().join("software");
    fs::create_dir_all(&software).unwrap();
    fs::write(software.join("tool.zip"), b"older").unwrap();
    let task = world.task_from(&format!(
        "name = \"scans\"\nkind = \"move\"\nto = '{}'\n[trigger]\nhome = '{{home}}'\nmatch = [\"*.zip\"]\n",
        software.display()
    ));
    let mut runner = world.runner(task);
    runner.start(world.now()).unwrap();
    world.drop_in("tool.zip", b"newer");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Done: tool.zip, under a second"]);
    let mut names: Vec<String> = fs::read_dir(&software)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["tool (2).zip", "tool.zip"]);
    assert_eq!(fs::read(software.join("tool.zip")).unwrap(), b"older");
    assert!(world.names("in").is_empty());
}

#[test]
fn a_script_task_runs_through_the_frontend_with_the_tasks_name() {
    let world = World::new();
    let task = world.task_from(
        "name = \"scans\"\nkind = \"script\"\nscript = \"script.ocr\"\n[trigger]\nhome = '{home}'\n",
    );
    let mut runner = world.runner(task);
    runner.start(world.now()).unwrap();
    world.drop_in("page.png", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Done: page.png, under a second"]);
    let runs = world.actions.scripts.lock().unwrap().clone();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].task, "scans");
    assert_eq!(runs[0].command, "script.ocr");
    assert_eq!(runs[0].input.name(), Some("page.png"));
    assert_eq!(world.names("processed"), ["page.png"]);
    // A command that cannot run pauses the task.
    *world.actions.script.lock().unwrap() = Some(ActionResult::Pause(
        "no script command is named script.ocr".to_owned(),
    ));
    world.drop_in("next.png", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(
        paused(&notes),
        ["Scans paused: no script command is named script.ocr"]
    );
}

#[test]
fn the_filter_comes_from_the_frontend_and_a_bad_one_pauses() {
    let world = World::new();
    let text = "name = \"scans\"\nrecipe = \"pdf-to-text\"\n[trigger]\nhome = '{home}'\nmatch = [\"*.pdf\"]\nfilter = \"keep\"\n";
    let mut runner = world.runner(world.task_from(text));
    runner.start(world.now()).unwrap();
    world.drop_in("keep-1.pdf", b"x");
    world.drop_in("drop-1.pdf", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(world.tools.ran(), ["keep-1.pdf"]);
    assert_eq!(world.names("in"), ["drop-1.pdf"]);
    drop(runner);
    let bad = text.replace("filter = \"keep\"", "filter = \"unreadable\"");
    let mut runner = world.runner(world.task_from(&bad));
    runner
        .start_with(world.now(), Some(CatchUpChoice::None))
        .unwrap();
    world.drop_in("keep-2.pdf", b"x");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(
        paused(&notes),
        ["Scans paused: the filter cannot be read: it is not a filter"]
    );
}

#[test]
fn partial_outputs_a_crash_left_are_said_and_never_removed() {
    let world = World::new();
    fs::write(world.folder("out").join(".diro-part-0badf00d.txt"), b"half").unwrap();
    let mut runner = world.runner(world.task(""));
    let notes = runner.start(world.now()).unwrap();
    assert!(said(&notes).contains(&"Found 1 partial file in out".to_owned()));
    world.run_for(&mut runner, 5);
    assert_eq!(world.names("out"), [".diro-part-0badf00d.txt"]);
}

#[test]
fn put_back_returns_a_failed_file_as_a_new_arrival() {
    let world = World::new();
    world.tools.on("scan.pdf", Act::Fail("Syntax Error"));
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"x");
    runner.rescan(world.now());
    world.run_for(&mut runner, 3);
    assert_eq!(world.names("failed"), ["scan.pdf"]);
    world.tools.on("scan.pdf", Act::Write);
    let lines = runner.put_back(&["scan.pdf".to_owned()]);
    assert_eq!(lines, ["Put back: scan.pdf"]);
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    assert_eq!(world.names("processed"), ["scan.pdf"]);
}

#[test]
fn the_folders_a_task_needs_are_made_with_the_make_folder_job() {
    let world = World::new();
    let home = world.dir.path().join("new-home");
    let text = format!(
        "name = \"scans\"\nrecipe = \"pdf-to-text\"\n[trigger]\nhome = '{}'\n",
        home.display()
    );
    let mut runner = world.runner(world.task_from(&text));
    let notes = runner.start(world.now()).unwrap();
    assert_eq!(paused(&notes), ["Scans paused: folder is missing"]);
    drop(runner);
    let runner = world.runner(world.task_from(&text));
    assert_eq!(
        runner.make_folders().unwrap(),
        ["home", "in", "out", "processed", "failed"]
    );
    assert!(home.join("processed").is_dir());
    assert_eq!(runner.make_folders().unwrap(), Vec::<String>::new());
}

#[test]
fn several_files_run_at_once_when_the_task_allows() {
    let world = World::new();
    let mut runner = world.runner(world.task("[run]\nparallel = 4\n"));
    runner.start(world.now()).unwrap();
    for n in 1..=9 {
        world.drop_in(&format!("p-{n}.pdf"), b"x");
    }
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    assert_eq!(world.names("processed").len(), 9);
    assert_eq!(world.names("out").len(), 9);
    assert_eq!(outcomes(&notes).len(), 9);
}

#[test]
fn a_file_swapped_for_another_during_the_run_runs_again() {
    let world = World::new();
    world.tools.on("scan.pdf", Act::Replace);
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("scan.pdf", b"same size");
    let mut notes = runner.rescan(world.now());
    notes.extend(world.run_for(&mut runner, 3));
    // Same size, a new file: the identity tells them apart, where the
    // platform reports one.
    let lines = file_lines(&notes);
    if lines == ["Changed during the run: scan.pdf"] {
        world.tools.on("scan.pdf", Act::Write);
        let notes = world.run_for(&mut runner, 4);
        assert_eq!(file_lines(&notes), ["Done: scan.pdf, under a second"]);
    } else {
        assert_eq!(lines, ["Done: scan.pdf, under a second"]);
    }
    assert_eq!(world.names("processed"), ["scan.pdf"]);
}

#[test]
fn an_interrupted_run_leaves_its_file_in_in_and_no_partial_output() {
    let world = World::new();
    world.tools.on("long.pdf", Act::Hang);
    let mut runner = world.runner(world.task(""));
    runner.start(world.now()).unwrap();
    world.drop_in("long.pdf", b"x");
    runner.rescan(world.now());
    world.run_for(&mut runner, 1);
    let stopper = runner.interrupter();
    let interrupt = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stopper.interrupt();
    });
    world.run_for(&mut runner, 2);
    interrupt.join().unwrap();
    assert!(runner.is_stopped());
    assert_eq!(world.names("in"), ["long.pdf"]);
    assert!(world.names("out").is_empty(), "{:?}", world.names("out"));
    assert!(
        runner
            .store()
            .ledger()
            .unwrap()
            .newest("long.pdf")
            .is_none()
    );
}

/// The real thread over the local provider's watch and the real clock.
#[test]
fn a_spawned_task_converts_what_arrives() {
    let world = World::new();
    let task = world.task_from(
        "name = \"scans\"\nrecipe = \"pdf-to-text\"\n[trigger]\nhome = '{home}'\nmatch = [\"*.pdf\"]\nsettle = \"500ms\"\n",
    );
    let runner = world
        .runner(task)
        .with_clock(Arc::new(diroxide_core::SystemClock::utc()));
    let (send, receive) = std::sync::mpsc::channel::<HotNote>();
    let send = std::sync::Mutex::new(send);
    let run = super::spawn::spawn_task(
        runner,
        Arc::new(diroxide_core::SystemClock::utc()),
        Box::new(move |note| {
            let _ = send.lock().unwrap().send(note);
        }),
    )
    .unwrap();
    world.drop_in("live.pdf", b"x");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut done = false;
    while std::time::Instant::now() < deadline {
        if let Ok(HotNote::File(file)) = receive.recv_timeout(Duration::from_millis(200))
            && file.outcome == FileOutcome::Done
        {
            done = true;
            break;
        }
    }
    run.stop();
    assert!(done, "the spawned task did not convert live.pdf");
    assert_eq!(world.names("processed"), ["live.pdf"]);
}
