//! Pack to zip (plan commit 32): the temporary name, the finish, the test
//! and the rename; levels in words; excludes; path modes; times from the
//! sources; a cancelled pack leaves the old file untouched; and a pack
//! that fails its test leaves nothing.

use std::io::{Cursor, Read};

use diroxide_archive::{
    ArchiveFormat, CompressionLevel, PackOutcome, PackPlan, PackRequest, StorePaths, plan_pack,
    run_pack,
};
use diroxide_core::{
    Attributes, ConflictAction, ConflictAnswer, ConflictPolicy, JobId, Place, Timestamp,
};
use diroxide_ops::JobControl;
use diroxide_vfs::OpContext;

use crate::jobs::{Answers, Sink, engine, env, out};
use crate::world::World;

/// A folder of sources on the fake disk: `docs` with two files and a
/// folder of two more, each with its time and mode.
pub fn sources(world: &World) -> Place {
    let docs = world.root.parent().unwrap().join("docs").unwrap();
    let put = |path: &[&str], data: &[u8], secs: i64| {
        let mut place = docs.clone();
        for part in path {
            place = place.join(part).unwrap();
        }
        world.fake.add_file(&place, data);
        world
            .fake
            .set_modified(&place, Timestamp::from_unix(secs, 0));
        world
            .fake
            .set_attributes_now(&place, Attributes::Unix(0o640));
        place
    };
    world.fake.add_folder(&docs.join("week1").unwrap());
    put(
        &["notes.txt"],
        b"the notes, again and again and again",
        1_760_000_000,
    );
    put(&["draft.tmp"], b"scratch", 1_760_000_100);
    put(&["week1", "a.txt"], b"week one", 1_760_000_200);
    put(
        &["week1", "b.bin"],
        &crate::build::noise(70_000, 3),
        1_760_000_300,
    );
    docs
}

pub fn plan_of(world: &World, request: &PackRequest) -> PackPlan {
    plan_pack(&world.registry, request, JobId::new(41), &OpContext::new()).unwrap()
}

pub fn run_plan(world: &World, plan: &PackPlan, sink: &Sink) -> PackOutcome {
    let engine = engine(world);
    let control = JobControl::new();
    let answers = Answers::no();
    let env = env(world, &engine, &control, sink, &answers, None);
    run_pack(plan, None, &env)
}

/// One member of a packed zip: name, contents, Unix mode, DOS date.
pub type ZipMember = (String, Vec<u8>, Option<u32>, (u16, u8, u8));

/// The packed zip's members: name, contents, Unix mode, DOS time.
pub fn read_zip(bytes: &[u8]) -> Vec<ZipMember> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut members = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        let time = file.last_modified().unwrap_or_default();
        members.push((
            file.name().unwrap().into_owned(),
            data,
            file.unix_mode(),
            (time.year(), time.month(), time.day()),
        ));
    }
    members
}

#[test]
fn a_folder_packs_to_zip_through_a_temporary_name() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("reports.zip").unwrap();
    let request = PackRequest::new(vec![docs.clone()], destination.clone(), ArchiveFormat::Zip);
    let plan = plan_of(&world, &request);
    let rows = plan.rows();
    assert_eq!(rows[0], "Pack 1 item into reports.zip");
    assert_eq!(rows[1], "Format: zip, level normal (6 of 9)");
    assert_eq!(rows[2], format!("Where: {out}"));
    assert_eq!(rows[3], "Password: none (zip names stay visible)");
    assert_eq!(rows[4], "Solid: not for zip");
    assert!(rows[5].starts_with("Reads 4 files, "), "{}", rows[5]);
    assert_eq!(rows[6], "Paths: relative to their folder");
    assert!(rows.contains(&"Test after packing: on".to_owned()));
    for row in rows.iter().filter(|r| !r.starts_with("Where: ")) {
        assert!(row.chars().count() <= 40, "{row}");
    }
    assert_eq!(
        plan.rows_for(true)[2],
        format!("Where: the other panel, {out}")
    );
    assert_eq!(plan.question().text, "Pack 1 item into reports.zip? y or n");
    let request_kept = plan.job_request();
    assert_eq!(request_kept.new_name.as_deref(), Some("reports.zip"));
    let sink = Sink::skipping();
    let outcome = run_plan(&world, &plan, &sink);
    assert_eq!(
        outcome.lines,
        [
            "Pack finished: 6 done.",
            "Checked 6 of 6 entries, all good."
        ]
    );
    assert_eq!(outcome.archive.as_ref(), Some(&destination));
    assert!(!world.fake.exists(&plan.temp), "the temporary name is gone");
    assert!(world.fake.unfinished().is_empty());
    assert_eq!(
        outcome.report.undo.created,
        std::slice::from_ref(&destination)
    );
    let members = read_zip(&world.fake.contents(&destination).unwrap());
    let names: Vec<&str> = members.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "docs/",
            "docs/draft.tmp",
            "docs/notes.txt",
            "docs/week1/",
            "docs/week1/a.txt",
            "docs/week1/b.bin"
        ]
    );
    let notes = &members[2];
    assert_eq!(notes.1, b"the notes, again and again and again");
    assert_eq!(notes.2, Some(0o100_640), "the mode from the source");
    assert_eq!(notes.3, (2025, 10, 9), "the time from the source");
    let lines = sink.lines.lock().unwrap();
    assert!(lines[0].starts_with("Packing"), "{lines:?}");
    assert!(
        lines.iter().any(|l| l.starts_with("Verifying")),
        "{lines:?}"
    );
}

#[test]
fn path_modes_excludes_and_levels() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let notes = docs.join("notes.txt").unwrap();
    let week1 = docs.join("week1").unwrap();
    let cases: [(StorePaths, Vec<&str>); 2] = [
        (StorePaths::NamesOnly, vec!["notes.txt", "a.txt", "b.bin"]),
        (
            StorePaths::Relative,
            vec!["notes.txt", "week1/", "week1/a.txt", "week1/b.bin"],
        ),
    ];
    for (n, (store_paths, expected)) in cases.into_iter().enumerate() {
        let destination = out.join(&format!("mode{n}.zip")).unwrap();
        let request = PackRequest {
            store_paths,
            ..PackRequest::new(
                vec![notes.clone(), week1.clone()],
                destination.clone(),
                ArchiveFormat::Zip,
            )
        };
        let outcome = run_plan(&world, &plan_of(&world, &request), &Sink::skipping());
        assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
        let members = read_zip(&world.fake.contents(&destination).unwrap());
        let names: Vec<&str> = members.iter().map(|m| m.0.as_str()).collect();
        assert_eq!(names, expected, "{store_paths:?}");
    }
    // The full path, without its drive or root.
    let full = out.join("full.zip").unwrap();
    let request = PackRequest {
        store_paths: StorePaths::Full,
        ..PackRequest::new(vec![notes.clone()], full.clone(), ArchiveFormat::Zip)
    };
    run_plan(&world, &plan_of(&world, &request), &Sink::skipping());
    assert_eq!(
        read_zip(&world.fake.contents(&full).unwrap())[0].0,
        "fake/docs/notes.txt"
    );
    // Excludes: a name pattern and a path pattern.
    let excluded = out.join("excluded.zip").unwrap();
    let request = PackRequest {
        exclude: vec!["*.tmp".to_owned(), "docs/week1/*.bin".to_owned()],
        ..PackRequest::new(vec![docs.clone()], excluded.clone(), ArchiveFormat::Zip)
    };
    let plan = plan_of(&world, &request);
    assert!(
        plan.rows()
            .contains(&"Leaves out: *.tmp, docs/week1/*.bin".to_owned())
    );
    assert!(
        plan.rows()
            .contains(&"Skipped draft.tmp: left out by a pattern.".to_owned())
    );
    let outcome = run_plan(&world, &plan, &Sink::skipping());
    assert_eq!(outcome.lines[0], "Pack finished: 4 done, 2 skipped.");
    let names: Vec<String> = read_zip(&world.fake.contents(&excluded).unwrap())
        .into_iter()
        .map(|m| m.0)
        .collect();
    assert_eq!(
        names,
        ["docs/", "docs/notes.txt", "docs/week1/", "docs/week1/a.txt"]
    );
    // Every level makes a zip that reads back.
    for level in [
        CompressionLevel::Fastest,
        CompressionLevel::Maximum,
        CompressionLevel::Ultra,
    ] {
        let destination = out.join(&format!("{}.zip", level.word())).unwrap();
        let request = PackRequest {
            level,
            ..PackRequest::new(vec![docs.clone()], destination.clone(), ArchiveFormat::Zip)
        };
        let outcome = run_plan(&world, &plan_of(&world, &request), &Sink::skipping());
        assert_eq!(
            outcome.lines[1], "Checked 6 of 6 entries, all good.",
            "{level:?}"
        );
    }
}

#[test]
fn a_cancelled_pack_leaves_the_old_file_untouched() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("reports.zip").unwrap();
    world.fake.add_file(&destination, b"the old archive");
    let request = PackRequest::new(vec![docs], destination.clone(), ArchiveFormat::Zip);
    let plan = plan_of(&world, &request);
    let engine = engine(&world);
    let control = JobControl::new();
    let mut sink = Sink::replacing();
    sink.cancel_after = Some((0, control.clone()));
    let answers = Answers::no();
    let env = env(&world, &engine, &control, &sink, &answers, None);
    let outcome = run_pack(&plan, None, &env);
    assert!(
        outcome.report.cancelled,
        "{:?} {:?}",
        outcome.lines, outcome.report
    );
    assert_eq!(outcome.lines, ["Pack cancelled: 0 done."]);
    assert_eq!(
        world.fake.contents(&destination).unwrap(),
        b"the old archive"
    );
    assert!(!world.fake.exists(&plan.temp));
    assert!(world.fake.unfinished().is_empty());
}

#[test]
fn a_taken_name_is_settled_after_the_test() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("reports.zip").unwrap();
    world.fake.add_file(&destination, b"the old archive");
    let request = PackRequest::new(vec![docs.clone()], destination.clone(), ArchiveFormat::Zip);
    let plan = plan_of(&world, &request);
    // Skip: the old one stays, nothing new is left behind.
    let outcome = run_plan(&world, &plan, &Sink::skipping());
    assert_eq!(outcome.lines, ["Pack finished: 0 done, 1 skipped."]);
    assert_eq!(
        world.fake.contents(&destination).unwrap(),
        b"the old archive"
    );
    assert!(!world.fake.exists(&plan.temp));
    // Keep both: "reports (2).zip".
    let sink = Sink::answering(ConflictAnswer::once(ConflictAction::KeepBoth));
    let outcome = run_plan(&world, &plan, &sink);
    assert_eq!(outcome.archive, Some(out.join("reports (2).zip").unwrap()));
    // Replace: the old one goes to the trash, after the new one passed.
    let outcome = run_plan(&world, &plan, &Sink::replacing());
    assert_eq!(outcome.archive.as_ref(), Some(&destination));
    assert!(world.fake.in_trash(&destination));
    assert_eq!(
        outcome.report.undo.trashed,
        std::slice::from_ref(&destination)
    );
    assert_ne!(
        world.fake.contents(&destination).unwrap(),
        b"the old archive"
    );
    // The policy decides without asking.
    let request = PackRequest {
        conflicts: ConflictPolicy::Skip,
        ..PackRequest::new(vec![docs], destination.clone(), ArchiveFormat::Zip)
    };
    let sink = Sink::replacing();
    run_plan(&world, &plan_of(&world, &request), &sink);
    assert!(sink.conflicts.lock().unwrap().is_empty());
}

#[test]
fn a_pack_that_fails_its_test_leaves_nothing() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("reports.zip").unwrap();
    // One file, so the byte changed is in its own local header.
    let notes = docs.join("notes.txt").unwrap();
    let request = PackRequest::new(vec![notes], destination.clone(), ArchiveFormat::Zip);
    let plan = plan_of(&world, &request);
    // The drive changes the temporary archive's first byte as it stores it.
    world.fake.switches_mut().corrupt.push(plan.temp.clone());
    let outcome = run_plan(&world, &plan, &Sink::skipping());
    assert_eq!(outcome.lines[0], "Pack had 1 failure: 0 done.");
    assert!(
        outcome.report.failed[0]
            .1
            .starts_with("the new archive could not be read back"),
        "{:?}",
        outcome.report.failed
    );
    assert!(!world.fake.exists(&destination));
    assert!(!world.fake.exists(&plan.temp));
}

#[test]
fn the_phase_0_pack_call_packs() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("plain.zip").unwrap();
    let request = PackRequest::new(vec![docs], destination.clone(), ArchiveFormat::Zip);
    diroxide_archive::pack(&request, &world.registry, &OpContext::new()).unwrap();
    assert_eq!(
        read_zip(&world.fake.contents(&destination).unwrap()).len(),
        6
    );
}
