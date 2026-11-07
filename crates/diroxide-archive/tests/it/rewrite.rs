//! Editing inside a zip (alpha 4, R30 commit 82): Delete, Rename and
//! Make folder by a rewrite through the pack runner. The question and the
//! cost in words; members read by record and kept in order, bytes, times
//! and links; the old archive to the trash only after the new one tested
//! clean; refusals in words; one password for the job; a lock by another
//! program, a place without a trash, and an archive changed since its
//! plan, each leaving the old archive as it was.

use std::sync::Arc;

use diroxide_archive::rewrite::{
    COMMENTS_LOST, COMPRESSION_REDONE, NESTED_REFUSAL, OLD_NOT_KEPT, OLD_TO_TRASH, PASSWORD_ONCE,
    SPLIT_REFUSAL, ZIPCRYPTO_UPGRADED,
};
use diroxide_archive::{
    ArchiveError, RewriteOutcome, RewritePlan, ZIP_ONLY, plan_rewrite, run_rewrite,
};
use diroxide_core::format::within_cells;
use diroxide_core::{JobId, Place};
use diroxide_ops::{FakeProvider, FakeSwitches, JobControl};
use diroxide_vfs::{OpContext, SecretSource};

use crate::build::{noise, sevenz_of, zip_of, zipcrypto_zip};
use crate::jobs::{Answers, Sink, engine, env};
use crate::pack_zip::read_zip;
use crate::prompt::{Answer, Prompt, aes_zip};
use crate::world::World;

/// The course: two folders, a deep member, and a member whose bytes are
/// noise, so a compare means something.
fn course() -> Vec<u8> {
    let big = noise(40_000, 9);
    zip_of(&[
        ("week1/", b""),
        ("week1/notes.txt", b"the notes"),
        ("week1/deep/a/b/c.txt", b"deep down"),
        ("week2/plan.txt", b"the plan"),
        ("week2/data.bin", &big),
        ("readme.txt", b"read me"),
    ])
}

fn plan(
    world: &World,
    archive: &Place,
    removals: &[&str],
    renames: &[(&str, &str)],
    folders: &[&str],
) -> Result<RewritePlan, ArchiveError> {
    let owned = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let renames: Vec<(String, String)> = renames
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
    plan_rewrite(
        &world.provider,
        archive,
        &owned(removals),
        &renames,
        &owned(folders),
        JobId::new(41),
        &OpContext::new(),
    )
}

fn run(
    world: &World,
    plan: &RewritePlan,
    secrets: Option<Arc<dyn SecretSource>>,
) -> RewriteOutcome {
    let engine = engine(world);
    let control = JobControl::new();
    let sink = Sink::skipping();
    let answers = Answers::no();
    let env = env(world, &engine, &control, &sink, &answers, secrets);
    run_rewrite(plan, None, &env)
}

fn names(bytes: &[u8]) -> Vec<String> {
    read_zip(bytes).into_iter().map(|m| m.0).collect()
}

fn assert_fits(lines: &[String]) {
    for line in lines {
        assert!(line.chars().count() <= 40, "{line}");
    }
}

#[test]
fn removing_two_members_asks_says_the_cost_and_trashes_the_old() {
    let world = World::plain();
    let old = course();
    let archive = world.put("course.zip", &old);
    let plan = plan(
        &world,
        &archive,
        &["week1/notes.txt", "readme.txt"],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        plan.question().text,
        "Remove 2 members from course.zip? y or n"
    );
    // 33 cells with its question mark, as R29 counted it.
    assert_eq!(plan.summary().chars().count(), 32);
    let cost = plan.cost_line();
    assert!(cost.starts_with("Rewrites all of course.zip: "), "{cost}");
    assert!(within_cells(&cost, "Rewrites all of course.zip"));
    let rows = plan.rows();
    assert_eq!(
        rows,
        [
            plan.summary(),
            cost,
            COMMENTS_LOST.to_owned(),
            COMPRESSION_REDONE.to_owned(),
            OLD_TO_TRASH.to_owned(),
        ]
    );
    assert_fits(&rows);
    let outcome = run(&world, &plan, None);
    assert!(
        outcome.changed && outcome.old_in_trash,
        "{:?}",
        outcome.lines
    );
    assert_eq!(
        outcome.lines,
        [
            "Removed 2 members from course.zip",
            "Checked 8 of 8 entries, all good.",
            "Old copy of course.zip is in the trash",
        ]
    );
    assert_fits(&outcome.lines);
    assert_eq!(world.fake.trash_contents(), std::slice::from_ref(&old));
    let new = world.fake.contents(&archive).unwrap();
    assert_eq!(
        names(&new),
        [
            "week1/",
            "week1/deep/a/b/c.txt",
            "week2/plan.txt",
            "week2/data.bin",
        ]
    );
    // Every kept member's bytes and time are the old ones.
    let before = read_zip(&old);
    for member in read_zip(&new) {
        let was = before.iter().find(|m| m.0 == member.0).unwrap();
        assert_eq!(member.1, was.1, "{}", member.0);
        assert_eq!(member.3, was.3, "{}", member.0);
    }
    assert!(world.fake.unfinished().is_empty());
    // The undo record holds the new archive and the old one in the trash.
    assert_eq!(outcome.report.undo.created, std::slice::from_ref(&archive));
    assert_eq!(outcome.report.undo.trashed, std::slice::from_ref(&archive));
    // The panel reads the new archive, not a stale index.
    assert!(world.rows(&archive.clone().into_archive()).unwrap().len() == 2);
}

#[test]
fn a_folder_removed_takes_its_members_and_a_renamed_one_moves_them() {
    let world = World::plain();
    let archive = world.put("course.zip", &course());
    let plan_removal = plan(&world, &archive, &["week1"], &[], &[]).unwrap();
    assert_eq!(plan_removal.summary(), "Remove week1 from course.zip");
    assert_eq!(plan_removal.rows()[2], "Removes 3 members in all");
    let renaming = plan(&world, &archive, &[], &[("week2", "term2")], &[]).unwrap();
    assert_eq!(renaming.summary(), "Rename week2 in course.zip");
    let outcome = run(&world, &renaming, None);
    assert_eq!(outcome.lines[0], "Renamed week2 in course.zip");
    let new = world.fake.contents(&archive).unwrap();
    assert_eq!(
        names(&new),
        [
            "week1/",
            "week1/notes.txt",
            "week1/deep/a/b/c.txt",
            "term2/plan.txt",
            "term2/data.bin",
            "readme.txt",
        ]
    );
    let made = plan(&world, &archive, &[], &[], &["week1/deep/extra"]).unwrap();
    assert_eq!(made.summary(), "Make folder extra in course.zip");
    let outcome = run(&world, &made, None);
    assert_eq!(outcome.lines[0], "Made folder extra in course.zip");
    let new = world.fake.contents(&archive).unwrap();
    assert_eq!(names(&new).last().unwrap(), "week1/deep/extra/");
    assert_eq!(world.fake.trashed(), 2);
}

#[test]
fn several_changes_at_once_are_one_rewrite() {
    let world = World::plain();
    let archive = world.put("course.zip", &course());
    let plan = plan(
        &world,
        &archive,
        &["readme.txt"],
        &[("week1/notes.txt", "week1/old-notes.txt")],
        &["week3"],
    )
    .unwrap();
    assert_eq!(plan.summary(), "Change 3 members of course.zip");
    let outcome = run(&world, &plan, None);
    assert_eq!(outcome.lines[0], "Changed 3 members of course.zip");
    let new = world.fake.contents(&archive).unwrap();
    let names = names(&new);
    assert!(names.contains(&"week1/old-notes.txt".to_owned()));
    assert!(!names.contains(&"readme.txt".to_owned()));
    assert_eq!(names.last().unwrap(), "week3/");
}

#[test]
fn refusals_are_said_in_words() {
    let world = World::plain();
    let archive = world.put("course.zip", &course());
    let refusal = |result: Result<RewritePlan, ArchiveError>| match result {
        Ok(plan) => panic!("planned, not refused: {}", plan.summary()),
        Err(error) => error.to_string(),
    };
    assert_eq!(
        refusal(plan(&world, &archive, &["nope.txt"], &[], &[])),
        "nope.txt is not in course.zip"
    );
    assert_eq!(
        refusal(plan(
            &world,
            &archive,
            &[],
            &[("readme.txt", "week1/notes.txt")],
            &[]
        )),
        "week1/notes.txt is in course.zip already"
    );
    assert_eq!(
        refusal(plan(
            &world,
            &archive,
            &[],
            &[("week2/plan.txt", "week2/DATA.BIN")],
            &[]
        )),
        "week2/DATA.BIN is in course.zip already",
        "a name that differs by case only clashes on Windows"
    );
    assert_eq!(
        refusal(plan(&world, &archive, &[], &[("readme.txt", "../x")], &[])),
        "../x is not a safe name in course.zip"
    );
    assert_eq!(
        refusal(plan(&world, &archive, &[], &[], &["week9/sub"])),
        "week9 is not in course.zip"
    );
    assert_eq!(
        refusal(plan(&world, &archive, &[], &[], &["week1"])),
        "week1 is in course.zip already"
    );
    assert_eq!(
        refusal(plan(&world, &archive, &[], &[], &[])),
        "Nothing to change in course.zip"
    );
    let sevenz = world.put("course.7z", &sevenz_of(&[("a.txt", b"a")], false, None));
    assert_eq!(
        refusal(plan(&world, &sevenz, &["a.txt"], &[], &[])),
        ZIP_ONLY
    );
    let tar = world.put("course.tar", &crate::build::tar_of(&[("a.txt", b"a")]));
    assert_eq!(refusal(plan(&world, &tar, &["a.txt"], &[], &[])), ZIP_ONLY);
    let outer = world.put("outer.zip", &zip_of(&[("inner.zip", &course())]));
    let inner = world.inside(&outer, "inner.zip");
    assert_eq!(
        refusal(plan(&world, &inner, &["readme.txt"], &[], &[])),
        NESTED_REFUSAL
    );
    let part = world.put("backup.zip.001", &course());
    assert_eq!(
        refusal(plan(&world, &part, &["readme.txt"], &[], &[])),
        SPLIT_REFUSAL
    );
    for line in [ZIP_ONLY, NESTED_REFUSAL, SPLIT_REFUSAL] {
        assert!(line.chars().count() <= 40);
    }
}

#[test]
fn a_locked_zip_asks_its_password_once_and_stays_locked() {
    let world = World::plain();
    let old = aes_zip(&[("a.txt", b"alpha"), ("b.txt", b"beta")], "right");
    let archive = world.put("locked.zip", &old);
    let plan = plan(&world, &archive, &["a.txt"], &[], &[]).unwrap();
    assert!(plan.locked);
    assert!(plan.rows().contains(&PASSWORD_ONCE.to_owned()));
    let prompt = Prompt::new(vec![Answer::Type("wrong"), Answer::Type("right")]);
    let outcome = run(&world, &plan, Some(prompt.clone()));
    assert!(outcome.changed, "{:?}", outcome.lines);
    assert_eq!(prompt.asked().len(), 2, "one wrong try, then the right one");
    let new = world.fake.contents(&archive).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(new)).unwrap();
    assert_eq!(zip.len(), 1);
    assert!(zip.by_index(0).is_err(), "the new member is locked too");
    let mut file = zip.by_index_decrypt(0, b"right").unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut file, &mut text).unwrap();
    assert_eq!(text, "beta");
    // No password given: nothing changes.
    let archive = world.put("locked2.zip", &old);
    let plan2 = plan_rewrite(
        &world.provider,
        &archive,
        &["a.txt".to_owned()],
        &[],
        &[],
        JobId::new(42),
        &OpContext::new(),
    )
    .unwrap();
    let outcome = run(&world, &plan2, Some(Prompt::new(vec![Answer::Escape])));
    assert_eq!(
        outcome.lines,
        [
            "Could not change locked2.zip: a password is needed",
            "locked2.zip is unchanged",
        ]
    );
    assert_eq!(world.fake.contents(&archive).unwrap(), old);
}

#[test]
fn zipcrypto_members_are_written_with_aes() {
    let world = World::plain();
    let archive = world.put("weak.zip", &zipcrypto_zip("secret.txt", b"the plans", "pw"));
    let plan = plan(&world, &archive, &[], &[], &["drafts"]).unwrap();
    assert!(plan.rows().contains(&ZIPCRYPTO_UPGRADED.to_owned()));
    let outcome = run(&world, &plan, Some(Prompt::always("pw")));
    assert!(outcome.changed, "{:?}", outcome.lines);
    let new = world.fake.contents(&archive).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(new)).unwrap();
    let file = zip.by_name_decrypt("secret.txt", b"pw").unwrap();
    assert!(file.encrypted());
}

#[test]
fn another_program_holding_the_archive_leaves_it_unchanged() {
    let world = World::plain();
    let old = course();
    let archive = world.put("course.zip", &old);
    let plan = plan(&world, &archive, &["readme.txt"], &[], &[]).unwrap();
    world.fake.switches_mut().locked.push(archive.clone());
    let outcome = run(&world, &plan, None);
    assert!(!outcome.changed);
    assert_eq!(
        outcome.lines,
        [
            "Could not replace course.zip: another program has it open",
            "course.zip is unchanged",
        ]
    );
    assert!(within_cells(
        &outcome.lines[0],
        "Could not replace course.zip"
    ));
    assert_eq!(world.fake.contents(&archive).unwrap(), old);
    assert!(world.fake.unfinished().is_empty());
    assert_eq!(world.fake.trashed(), 0);
    let folder = archive.parent().unwrap();
    let left: Vec<_> = world
        .fake
        .snapshot()
        .into_keys()
        .filter(|k| k.contains(".diroxide-"))
        .collect();
    assert!(left.is_empty(), "{left:?} in {folder}");
}

#[test]
fn a_place_without_a_trash_says_the_old_copy_is_not_kept() {
    let switches = FakeSwitches {
        no_trash: true,
        ..FakeSwitches::default()
    };
    let world = World::with_fake(FakeProvider::with_switches(switches));
    let archive = world.put("course.zip", &course());
    let plan = plan(&world, &archive, &["readme.txt"], &[], &[]).unwrap();
    assert!(!plan.keeps_old);
    assert_eq!(plan.rows().last().unwrap(), OLD_NOT_KEPT);
    let outcome = run(&world, &plan, None);
    assert!(outcome.changed && !outcome.old_in_trash);
    assert_eq!(
        outcome.lines.last().unwrap(),
        "Old copy of course.zip was not kept"
    );
}

#[test]
fn an_archive_changed_since_its_plan_is_not_rewritten() {
    let world = World::plain();
    let archive = world.put("course.zip", &course());
    let plan = plan(&world, &archive, &["readme.txt"], &[], &[]).unwrap();
    let other = zip_of(&[("other.txt", b"someone else wrote this")]);
    world.fake.add_file(&archive, &other);
    let outcome = run(&world, &plan, None);
    assert_eq!(
        outcome.lines,
        [
            "Could not change course.zip: it changed since it was listed",
            "course.zip is unchanged",
        ]
    );
    assert_eq!(world.fake.contents(&archive).unwrap(), other);
}

#[test]
fn a_cancel_leaves_the_old_archive() {
    let world = World::plain();
    let old = course();
    let archive = world.put("course.zip", &old);
    let plan = plan(&world, &archive, &["readme.txt"], &[], &[]).unwrap();
    let engine = engine(&world);
    let control = JobControl::new();
    let mut sink = Sink::skipping();
    sink.cancel_after = Some((2, control.clone()));
    let answers = Answers::no();
    let env = env(&world, &engine, &control, &sink, &answers, None);
    let outcome = run_rewrite(&plan, None, &env);
    assert_eq!(outcome.lines, ["Cancelled: course.zip is unchanged"]);
    assert_eq!(world.fake.contents(&archive).unwrap(), old);
    assert!(world.fake.unfinished().is_empty());
}

#[test]
fn links_stay_links_and_duplicates_are_said() {
    let world = World::plain();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("a.txt", options).unwrap();
    std::io::Write::write_all(&mut writer, b"first").unwrap();
    writer.add_symlink("link", "a.txt", options).unwrap();
    writer.start_file("b.txt", options).unwrap();
    std::io::Write::write_all(&mut writer, b"b").unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    // A second copy of a.txt, by renaming b.txt in both headers.
    let at: Vec<usize> = bytes
        .windows(5)
        .enumerate()
        .filter(|(_, w)| *w == b"b.txt")
        .map(|(i, _)| i)
        .collect();
    for i in at {
        bytes[i] = b'a';
    }
    let archive = world.put("dupes.zip", &bytes);
    let plan = plan(&world, &archive, &[], &[], &["new"]).unwrap();
    assert!(
        plan.rows()
            .contains(&"Older copies of 1 name are dropped".to_owned()),
        "{:?}",
        plan.rows()
    );
    let outcome = run(&world, &plan, None);
    assert!(outcome.changed, "{:?}", outcome.lines);
    let new = world.fake.contents(&archive).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(new)).unwrap();
    assert_eq!(zip.len(), 3);
    assert!(zip.by_name("link").unwrap().is_symlink());
}

#[test]
fn a_stored_member_stays_stored_and_first() {
    // An EPUB's mimetype is first and stored; readers check both.
    let world = World::plain();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("mimetype", stored).unwrap();
    std::io::Write::write_all(&mut writer, b"application/epub+zip").unwrap();
    writer
        .start_file("META-INF/container.xml", deflated)
        .unwrap();
    std::io::Write::write_all(&mut writer, &b"<container/>".repeat(40)).unwrap();
    let book = writer.finish().unwrap().into_inner();
    let archive = world.put("book.epub", &book);
    let plan = plan(&world, &archive, &[], &[], &["OEBPS"]).unwrap();
    assert!(
        plan.rows()
            .contains(&"Compression redone: stored stays stored".to_owned())
    );
    let outcome = run(&world, &plan, None);
    assert!(outcome.changed, "{:?}", outcome.lines);
    let new = world.fake.contents(&archive).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(new)).unwrap();
    let first = zip.by_index(0).unwrap();
    assert_eq!(first.name().unwrap(), "mimetype");
    assert_eq!(first.compression(), zip::CompressionMethod::Stored);
    drop(first);
    let container = zip.by_name("META-INF/container.xml").unwrap();
    assert_eq!(container.compression(), zip::CompressionMethod::Deflated);
}
