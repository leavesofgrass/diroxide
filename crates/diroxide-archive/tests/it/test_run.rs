//! Test archive (plan commit 31): every member read, every checksum held
//! to, the verdict by name, nothing written; and the same check after a
//! pack, against what the packer wrote.

use std::sync::Arc;

use diroxide_archive::test_archive::Written;
use diroxide_archive::{Allowances, JobSecrets, check_written, test_archive};
use diroxide_ops::JobControl;
use diroxide_vfs::{OpContext, ReadSeek};

use crate::build::{crc, gzip, sevenz_of, tar_of, xz, zip_of};
use crate::jobs::{Answers, Sink, engine, env};
use crate::prompt::{Answer, CANARY, Prompt, aes_zip};
use crate::unpack_plan::course;
use crate::world::World;

fn test(
    world: &World,
    archive: &diroxide_core::Place,
    prompt: Option<Arc<Prompt>>,
) -> diroxide_archive::TestOutcome {
    let engine = engine(world);
    let control = JobControl::new();
    let sink = Sink::skipping();
    let answers = Answers::no();
    let secrets = prompt.map(|p| p as Arc<dyn diroxide_vfs::SecretSource>);
    let env = env(world, &engine, &control, &sink, &answers, secrets);
    test_archive(archive, None, &env)
}

/// Makes the stored CRC of the `nth` central record wrong.
fn break_crc(zip: &mut [u8], nth: usize) {
    let at = zip
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == [b'P', b'K', 1, 2])
        .map(|(at, _)| at)
        .nth(nth)
        .unwrap();
    zip[at + 16] ^= 0x55;
}

#[test]
fn a_sound_archive_says_so_in_every_format() {
    let world = World::plain();
    let content: [(&str, &[u8]); 3] = [("a.txt", b"one"), ("dir/b.txt", b"two"), ("c.txt", b"")];
    let archives = [
        world.put("course.zip", &course()),
        world.put("solid.7z", &sevenz_of(&content, true, None)),
        world.put("plain.7z", &sevenz_of(&content, false, None)),
        world.put("logs.tar", &tar_of(&content)),
        world.put("logs.tar.gz", &gzip(&tar_of(&content))),
        world.put("logs.tar.xz", &xz(&tar_of(&content), 1)),
        world.put("note.txt.xz", &xz(b"one compressed file", 1)),
    ];
    let before = world.fake.snapshot();
    for archive in &archives {
        let outcome = test(&world, archive, None);
        assert!(outcome.is_sound(), "{archive}: {:?}", outcome.lines());
        assert!(outcome.sentence().text.starts_with("Archive is sound: "));
    }
    assert_eq!(
        test(&world, &archives[0], None).sentence().text,
        "Archive is sound: 10 members."
    );
    assert_eq!(world.fake.snapshot(), before, "a test writes nothing");
}

#[test]
fn damaged_members_are_named_first_in_archive_order() {
    let world = World::plain();
    let mut zip = course();
    // The central records: week1/, a, b, c, week2/, a, b, c, notes, README.
    break_crc(&mut zip, 1);
    break_crc(&mut zip, 6);
    let place = world.put("course.zip", &zip);
    let outcome = test(&world, &place, None);
    assert_eq!(
        outcome.sentence().text,
        "2 members damaged: first is week1/a.txt"
    );
    assert!(
        outcome.lines()[1].starts_with("week1/a.txt: it is damaged"),
        "{:?}",
        outcome.lines()
    );
    assert_eq!(outcome.report.failed.len(), 2);
    assert_eq!(outcome.report.done.len(), 8);
    assert!(outcome.report.summary().starts_with("Test had 2 failures"));
}

#[test]
fn a_cut_stream_is_damaged_from_where_it_ends() {
    let world = World::plain();
    let content: Vec<(String, Vec<u8>)> = (0..6)
        .map(|n| (format!("part{n}.bin"), crate::build::noise(40_000, n + 1)))
        .collect();
    let slices: Vec<(&str, &[u8])> = content
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let whole = gzip(&tar_of(&slices));
    let place = world.put("cut.tar.gz", &whole[..whole.len() * 2 / 3]);
    let outcome = test(&world, &place, None);
    assert!(!outcome.is_sound());
    assert!(
        outcome.sentence().text.contains("damaged") || outcome.failed.is_some(),
        "{:?}",
        outcome.lines()
    );
}

#[test]
fn a_locked_archive_asks_and_says_what_it_could_not_check() {
    let world = World::plain();
    let zip = world.put(
        "locked.zip",
        &aes_zip(&[("a.txt", b"one"), ("b.txt", b"two")], CANARY),
    );
    let prompt = Prompt::always(CANARY);
    let outcome = test(&world, &zip, Some(prompt.clone()));
    assert!(outcome.is_sound(), "{:?}", outcome.lines());
    assert_eq!(prompt.asked().len(), 1);
    let escape = Prompt::new(vec![Answer::Escape]);
    let outcome = test(&world, &zip, Some(escape));
    assert_eq!(outcome.sentence().text, "Not checked: 2 of 2 members.");
    assert!(
        outcome.lines()[1].ends_with("No password given. Skipped 2 members of locked.zip."),
        "{:?}",
        outcome.lines()
    );
    let locked = world.put(
        "locked.7z",
        &sevenz_of(&[("a.txt", b"one")], false, Some(CANARY)),
    );
    let outcome = test(&world, &locked, None);
    assert_eq!(
        outcome.sentence().text,
        "Could not test locked.7z: a password is needed."
    );
    let wrong = Prompt::new(vec![
        Answer::Type("no"),
        Answer::Type("nope"),
        Answer::Type("never"),
    ]);
    let outcome = test(&world, &locked, Some(wrong.clone()));
    assert_eq!(wrong.asked().len(), 3);
    assert!(
        outcome
            .failed
            .unwrap()
            .contains("the password is wrong, or the data is damaged")
    );
}

#[test]
fn the_check_after_packing_holds_the_archive_to_what_was_written() {
    let world = World::plain();
    let content: [(&str, &[u8]); 2] = [("a.txt", b"one"), ("dir/b.txt", b"two")];
    let bytes = zip_of(&content);
    let written: Vec<Written> = content
        .iter()
        .map(|(name, data)| Written {
            path: (*name).to_owned(),
            size: data.len() as u64,
            crc32: crc(data),
        })
        .collect();
    let place = world.root.join("new.zip").unwrap();
    let open = || {
        let reader: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(bytes.clone()));
        world
            .provider
            .open_bytes(reader, &place, &OpContext::new(), Allowances::default())
            .unwrap()
    };
    let ctx = JobSecrets::new(Prompt::always(CANARY)).context(&OpContext::new());
    assert_eq!(
        check_written(&world.provider, &open(), &written, &ctx).unwrap(),
        "Checked 3 of 3 entries, all good."
    );
    let mut wrong = written.clone();
    wrong[1].crc32 ^= 1;
    assert_eq!(
        check_written(&world.provider, &open(), &wrong, &ctx).unwrap_err(),
        "dir/b.txt reads back differently in the new archive"
    );
    wrong.pop();
    assert!(
        check_written(&world.provider, &open(), &wrong, &ctx)
            .unwrap_err()
            .starts_with("the new archive holds 2 files where 1 were written")
    );
}
