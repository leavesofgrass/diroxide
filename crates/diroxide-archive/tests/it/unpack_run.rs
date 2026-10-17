//! The unpack runner (plan commit 30): one forward pass, conflicts through
//! the engine's question, links, devices and mode bits handled, times
//! kept, the report in words; the symlink-then-write trap writes nothing
//! outside; a tar.xz of 5,000 files unpacks in one pass; and the plan
//! equals the run on the fake provider.

use diroxide_archive::{
    Allowances, FolderRule, Limits, UnpackOutcome, UnpackPlan, UnpackRequest, plan_unpack,
    run_unpack,
};
use diroxide_core::{ConflictAction, ConflictAnswer, EntryKind, JobId, Timestamp};
use diroxide_ops::JobControl;
use diroxide_vfs::OpContext;
use proptest::prelude::*;

use crate::build::{tar_of, xz, zip_of};
use crate::hostile::{hostile_names, links_out_of_root, tar_links_out_of_root};
use crate::jobs::{Answers, Sink, engine, env, out, outside, tree};
use crate::unpack_plan::course;
use crate::world::World;

fn plan(world: &World, request: &UnpackRequest) -> UnpackPlan {
    plan_unpack(
        &world.provider,
        &world.registry,
        request,
        JobId::new(41),
        &OpContext::new(),
        Allowances::default(),
    )
    .unwrap()
}

fn run(world: &World, plan: &UnpackPlan, sink: &Sink, answers: &Answers) -> UnpackOutcome {
    let engine = engine(world);
    let control = JobControl::new();
    let env = env(world, &engine, &control, sink, answers, None);
    run_unpack(plan, None, Allowances::default(), &env)
}

#[test]
fn course_zip_unpacks_whole_with_its_times() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    let plan = plan(&world, &UnpackRequest::new(zip, out.clone()));
    let sink = Sink::skipping();
    let outcome = run(&world, &plan, &sink, &Answers::no());
    assert_eq!(outcome.lines[0], "Unpack finished: 10 done.");
    let files = tree(&world, &out);
    assert_eq!(files.len(), 11, "{files:?}");
    assert!(files.contains(&("course/week1/a.txt".to_owned(), b"week one, a".to_vec())));
    assert!(files.contains(&("course/README.md".to_owned(), b"# Course".to_vec())));
    let notes = world
        .fake
        .entry(&plan.destination.join("notes.txt").unwrap())
        .unwrap();
    assert_eq!(notes.modified, Some(Timestamp::from_unix(1_792_152_000, 0)));
    let report = &outcome.report;
    assert_eq!(report.undo.created.len(), 11, "the folder made and its ten");
    assert!(report.reconciliation.as_ref().unwrap().is_whole());
    let lines = sink.lines.lock().unwrap();
    assert!(lines.first().unwrap().starts_with("Unpacking"), "{lines:?}");
    assert_eq!(lines.last().unwrap(), "Unpacking finished");
    assert!(world.fake.unfinished().is_empty());
}

#[test]
fn odd_names_land_inside_with_their_lines() {
    let world = World::plain();
    world.fake.switches_mut().reserved_names = true;
    let zip = world.put("hostile.zip", &hostile_names());
    let out = out(&world);
    let before: Vec<String> = world.fake.snapshot().into_keys().collect();
    let plan = plan(&world, &UnpackRequest::new(zip, out.clone()));
    let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
    assert!(
        outcome
            .lines
            .contains(&"Skipped ../x: name leaves the archive.".to_owned()),
        "{:?}",
        outcome.lines
    );
    assert!(
        outcome
            .lines
            .iter()
            .any(|l| l.starts_with("Renamed ") && l.ends_with("names: unsafe characters."))
    );
    assert!(outside(&world, &out, &before).is_empty());
    let files = tree(&world, &out);
    assert!(
        files
            .iter()
            .any(|(name, _)| name.ends_with("inner.zip_x.txt"))
    );
    assert!(files.iter().any(|(name, _)| name.ends_with("CON_")));
}

#[test]
fn the_symlink_then_write_trap_writes_nothing_outside() {
    for restore_links in [false, true] {
        let world = World::plain();
        let out = out(&world);
        let zip = world.put("links.zip", &links_out_of_root());
        let tar = world.put("links.tar", &tar_links_out_of_root());
        // The destination already holds a link that leads out.
        world
            .fake
            .add_link(&out.join("x").unwrap(), "../../elsewhere", true);
        let trap = world.put("trap.tar", &tar_of(&[("x/evil.txt", b"gotcha")]));
        let before: Vec<String> = world.fake.snapshot().into_keys().collect();
        for archive in [zip, tar, trap] {
            let request = UnpackRequest {
                restore_links,
                folder_rule: FolderRule::Never,
                ..UnpackRequest::new(archive, out.clone())
            };
            let plan = plan(&world, &request);
            let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
            assert!(!outcome.report.cancelled);
        }
        assert!(
            outside(&world, &out, &before).is_empty(),
            "restore links {restore_links}"
        );
        let files = tree(&world, &out);
        assert!(
            !files.iter().any(|(name, _)| name == "x/evil.txt"),
            "nothing written through the link: {files:?}"
        );
        assert!(files.iter().any(|(name, _)| name == "link/passwd"));
    }
    // The skip says why.
    let world = World::plain();
    let out = out(&world);
    world
        .fake
        .add_link(&out.join("x").unwrap(), "../../elsewhere", true);
    let trap = world.put("trap.tar", &tar_of(&[("x/evil.txt", b"gotcha")]));
    let plan = plan(
        &world,
        &UnpackRequest {
            folder_rule: FolderRule::Never,
            ..UnpackRequest::new(trap, out.clone())
        },
    );
    let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
    assert!(
        outcome
            .report
            .skipped
            .iter()
            .any(|(_, why)| why == "a folder on the way is a link"),
        "{:?}",
        outcome.report.skipped
    );
}

#[test]
fn a_tar_xz_of_5000_files_unpacks_in_one_pass() {
    let world = World::plain();
    let names: Vec<(String, Vec<u8>)> = (0..5_000)
        .map(|n| {
            (
                format!("logs/day{:02}/entry{n:04}.log", n / 250),
                format!("entry {n}\n").into_bytes(),
            )
        })
        .collect();
    let members: Vec<(&str, &[u8])> = names
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let place = world.put("logs.tar.xz", &xz(&tar_of(&members), 1));
    let out = out(&world);
    let plan = plan(&world, &UnpackRequest::new(place.clone(), out.clone()));
    assert_eq!(plan.facts.files, 5_000);
    let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
    assert!(
        outcome.lines[0].starts_with("Unpack finished: "),
        "{:?}",
        outcome.lines
    );
    let opened = world
        .provider
        .open_archive(&place, &OpContext::new(), Allowances::default())
        .unwrap();
    assert_eq!(opened.passes(), 1, "one pass over the archive");
    let files = tree(&world, &out);
    assert_eq!(
        files.iter().filter(|(n, _)| n.ends_with(".log")).count(),
        5_000
    );
}

#[test]
fn solid_7z_and_tar_gz_unpack_too() {
    let world = World::plain();
    let content: [(&str, &[u8]); 3] = [
        ("a.txt", b"first"),
        ("dir/b.txt", b"second"),
        ("dir/c.txt", b"third"),
    ];
    let out = out(&world);
    for (name, bytes) in [
        ("solid.7z", crate::build::sevenz_of(&content, true, None)),
        ("plain.tar.gz", crate::build::gzip(&tar_of(&content))),
    ] {
        let archive = world.put(name, &bytes);
        let dest = out.join(name.split('.').next().unwrap()).unwrap();
        world.fake.add_folder(&dest);
        let plan = plan(
            &world,
            &UnpackRequest {
                folder_rule: FolderRule::Never,
                ..UnpackRequest::new(archive, dest.clone())
            },
        );
        let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
        assert!(
            outcome.report.failed.is_empty(),
            "{name}: {:?}",
            outcome.report
        );
        let files = tree(&world, &dest);
        assert!(
            files.contains(&("dir/c.txt".to_owned(), b"third".to_vec())),
            "{name}: {files:?}"
        );
    }
}

#[test]
fn conflicts_ask_through_the_engine() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    let request = UnpackRequest {
        folder_rule: FolderRule::Never,
        ..UnpackRequest::new(zip, out.clone())
    };
    let notes = out.join("notes.txt").unwrap();
    world.fake.add_file(&notes, b"old notes");
    // Skip: the old one stays.
    let plan = plan(&world, &request);
    let sink = Sink::skipping();
    let outcome = run(&world, &plan, &sink, &Answers::no());
    assert_eq!(outcome.lines[0], "Unpack finished: 9 done, 1 skipped.");
    assert_eq!(world.fake.contents(&notes).unwrap(), b"old notes");
    assert_eq!(sink.conflicts.lock().unwrap().len(), 1);
    // Keep both: the new one beside it.
    let sink = Sink::answering(ConflictAnswer::once(ConflictAction::KeepBoth));
    let outcome = run(&world, &plan, &sink, &Answers::no());
    assert!(world.fake.exists(&out.join("notes (2).txt").unwrap()));
    assert!(outcome.report.failed.is_empty());
    // Replace for all: the old one goes to the trash first.
    let sink = Sink::replacing();
    let outcome = run(&world, &plan, &sink, &Answers::no());
    assert_eq!(world.fake.contents(&notes).unwrap(), b"the notes");
    assert!(world.fake.in_trash(&notes));
    assert!(outcome.report.undo.trashed.contains(&notes));
    assert_eq!(
        sink.conflicts.lock().unwrap().len(),
        1,
        "asked once for all"
    );
}

#[test]
fn too_little_room_writes_nothing() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    world.fake.switches_mut().free_space = Some(40);
    let plan = plan(&world, &UnpackRequest::new(zip, out.clone()));
    let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
    assert!(
        outcome.lines[0].starts_with("Unpack had 1 failure"),
        "{:?}",
        outcome.lines
    );
    assert!(outcome.lines[1].starts_with("Needs "));
    assert!(tree(&world, &out).is_empty());
}

#[test]
fn a_cancel_leaves_no_half_file() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    // A file in the way of week2/a.txt: its question comes in the middle
    // of the job, and the cancel with it.
    let request = UnpackRequest {
        folder_rule: FolderRule::Never,
        ..UnpackRequest::new(zip, out.clone())
    };
    world.fake.add_folder(&out.join("week2").unwrap());
    world
        .fake
        .add_file(&out.join("week2").unwrap().join("a.txt").unwrap(), b"old");
    let plan = plan(&world, &request);
    let engine = engine(&world);
    let control = JobControl::new();
    let mut sink = Sink::skipping();
    sink.cancel_on_conflict = Some(control.clone());
    let answers = Answers::no();
    let env = env(&world, &engine, &control, &sink, &answers, None);
    let outcome = run_unpack(&plan, None, Allowances::default(), &env);
    assert!(outcome.report.cancelled, "{:?}", outcome.lines);
    assert!(
        outcome.lines[0].starts_with("Unpack cancelled: "),
        "{:?}",
        outcome.lines
    );
    assert!(world.fake.unfinished().is_empty());
    let files = tree(&world, &out);
    assert!(files.contains(&("week1/c.txt".to_owned(), b"week one, c".to_vec())));
    assert!(
        !files.iter().any(|(name, _)| name == "notes.txt"),
        "{files:?}"
    );
}

#[test]
fn cancel_at_a_conflict_question_stops_the_job() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    world.fake.add_file(&out.join("notes.txt").unwrap(), b"old");
    let request = UnpackRequest {
        folder_rule: FolderRule::Never,
        ..UnpackRequest::new(zip, out.clone())
    };
    let plan = plan(&world, &request);
    let sink = Sink::answering(ConflictAnswer::once(ConflictAction::Cancel));
    let outcome = run(&world, &plan, &sink, &Answers::no());
    assert!(outcome.report.cancelled);
    assert!(world.fake.unfinished().is_empty());
    assert_eq!(
        world
            .fake
            .contents(&out.join("notes.txt").unwrap())
            .unwrap(),
        b"old"
    );
}

#[test]
fn a_damaged_member_fails_alone() {
    let world = World::plain();
    let mut bytes = zip_of(&[("good.txt", b"good"), ("bad.txt", b"bad data here")]);
    // The stored CRC of bad.txt, in its central record, made wrong.
    let at = bytes
        .windows(4)
        .rposition(|w| w == [b'P', b'K', 1, 2])
        .unwrap();
    bytes[at + 16] ^= 0xFF;
    let zip = world.put("damaged.zip", &bytes);
    let out = out(&world);
    let plan = plan(
        &world,
        &UnpackRequest {
            folder_rule: FolderRule::Never,
            ..UnpackRequest::new(zip, out.clone())
        },
    );
    let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
    assert_eq!(outcome.lines[0], "Unpack had 1 failure: 1 done.");
    assert!(
        outcome.report.failed[0].1.contains("checksum"),
        "{:?}",
        outcome.report.failed
    );
    assert_eq!(
        tree(&world, &out),
        [("good.txt".to_owned(), b"good".to_vec())]
    );
}

#[test]
fn an_expansion_question_is_asked_once_and_no_skips() {
    let zeros = vec![0u8; 3 << 20];
    for yes in [false, true] {
        let world = World::new(Limits::strict());
        let zip = world.put(
            "zeros.zip",
            &zip_of(&[
                ("zeros.bin", &zeros),
                ("more.bin", &zeros),
                ("small.txt", b"s"),
            ]),
        );
        let out = out(&world);
        let plan = plan(
            &world,
            &UnpackRequest {
                folder_rule: FolderRule::Never,
                ..UnpackRequest::new(zip, out.clone())
            },
        );
        let answers = if yes { Answers::yes() } else { Answers::no() };
        let outcome = run(&world, &plan, &Sink::skipping(), &answers);
        let asked = answers.asked.lock().unwrap();
        assert_eq!(asked.len(), 1, "once for the job: {asked:?}");
        assert!(
            asked[0].message().text.starts_with("Expands to "),
            "{:?}",
            asked[0]
        );
        let files = tree(&world, &out);
        if yes {
            assert_eq!(files.len(), 3);
        } else {
            assert_eq!(files, [("small.txt".to_owned(), b"s".to_vec())]);
            assert_eq!(outcome.lines[0], "Unpack finished: 1 done, 2 skipped.");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// The plan equals the run: every planned target is there after the
    /// run, with the member's contents, and nothing else is.
    #[test]
    fn the_plan_equals_the_run(
        names in prop::collection::btree_set("[a-c]{1,2}(/[a-c]{1,2}){0,2}", 1..12),
        flatten in any::<bool>(),
    ) {
        let world = World::plain();
        // A name that is also a folder of another would be a clash the
        // index settles; keep the sets plain.
        let names: Vec<String> = names
            .iter()
            .filter(|n| !names.iter().any(|o| o.starts_with(&format!("{n}/"))))
            .cloned()
            .collect();
        let members: Vec<(String, Vec<u8>)> = names
            .iter()
            .map(|n| (n.clone(), format!("data of {n}").into_bytes()))
            .collect();
        let slices: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        let zip = world.put("random.zip", &zip_of(&slices));
        let out = out(&world);
        let request = UnpackRequest {
            flatten,
            folder_rule: FolderRule::Never,
            conflicts: diroxide_core::ConflictPolicy::KeepBoth,
            ..UnpackRequest::new(zip, out.clone())
        };
        let plan = plan(&world, &request);
        let outcome = run(&world, &plan, &Sink::skipping(), &Answers::no());
        prop_assert!(outcome.report.failed.is_empty(), "{:?}", outcome.report.failed);
        let found = tree(&world, &out);
        let files: Vec<&(String, Vec<u8>)> = found.iter().filter(|(_, d)| !d.is_empty()).collect();
        prop_assert_eq!(files.len() as u64, plan.facts.files);
        if !flatten {
            for (name, data) in &members {
                prop_assert!(found.contains(&(name.clone(), data.clone())), "{} missing", name);
            }
            for member in &plan.members {
                prop_assert!(world.fake.exists(&member.target), "{}", member.target);
                let kind = world.fake.entry(&member.target).unwrap().kind;
                prop_assert_eq!(kind == EntryKind::Folder, member.kind == diroxide_archive::RecordKind::Folder);
            }
        }
    }
}
