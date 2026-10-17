//! Passwords in unpack and pack through the secret prompt (plan commit
//! 35, spike K1 through the runners): the first locked member asks, three
//! tries at most, the same password for the rest of the job and gone with
//! it; Escape and a cancel at the prompt leave nothing half done; a zip
//! password is AES-256, never ZipCrypto; and the canary never shows in a
//! plan, a row, a progress line, a report or a question.

use std::sync::Arc;

use diroxide_archive::{
    Allowances, ArchiveFormat, FolderRule, JobSecrets, PackRequest, UnpackOutcome, UnpackPlan,
    UnpackRequest, new_password_request, plan_unpack, run_pack, run_unpack,
};
use diroxide_core::{JobId, Place, Secret};
use diroxide_ops::JobControl;
use diroxide_vfs::{OpContext, SecretSource};

use crate::build::sevenz_of;
use crate::jobs::{Answers, Sink, engine, env, out, tree};
use crate::pack_zip::{plan_of, sources};
use crate::prompt::{Answer, CANARY, Prompt, aes_zip};
use crate::world::World;

fn locked_course(world: &World) -> Place {
    world.put(
        "course.zip",
        &aes_zip(
            &[
                ("week1/a.txt", b"first"),
                ("week1/b.txt", b"second"),
                ("notes.txt", b"third"),
            ],
            CANARY,
        ),
    )
}

fn plan_for(world: &World, archive: &Place, ctx: &OpContext) -> UnpackPlan {
    let out = out(world);
    let request = UnpackRequest {
        folder_rule: FolderRule::Never,
        ..UnpackRequest::new(archive.clone(), out)
    };
    plan_unpack(
        &world.provider,
        &world.registry,
        &request,
        JobId::new(41),
        ctx,
        Allowances::default(),
    )
    .unwrap()
}

fn unpack(
    world: &World,
    plan: &UnpackPlan,
    secrets: Arc<dyn SecretSource>,
    control: &JobControl,
    sink: &Sink,
) -> UnpackOutcome {
    let engine = engine(world);
    let answers = Answers::no();
    let env = env(world, &engine, control, sink, &answers, Some(secrets));
    run_unpack(plan, None, Allowances::default(), &env)
}

#[test]
fn unpack_asks_once_and_tries_three_times() {
    // A wrong password, then the right one: every member, asked twice.
    let world = World::plain();
    let zip = locked_course(&world);
    let plan = plan_for(&world, &zip, &OpContext::new());
    assert!(
        plan.rows()
            .contains(&"Password: asked for 3 locked members".to_owned())
    );
    let prompt = Prompt::new(vec![Answer::Type("wrong"), Answer::Type(CANARY)]);
    let outcome = unpack(
        &world,
        &plan,
        prompt.clone(),
        &JobControl::new(),
        &Sink::skipping(),
    );
    assert_eq!(outcome.lines[0], "Unpack finished: 4 done.");
    let attempts: Vec<u32> = prompt.asked().iter().map(|r| r.attempt).collect();
    assert_eq!(attempts, [1, 2]);
    assert_eq!(
        prompt.asked()[0].prompt_line(),
        "Password for course.zip (hidden): "
    );
    let files = tree(&world, &plan.destination);
    assert!(files.contains(&("notes.txt".to_owned(), b"third".to_vec())));
    // Three wrong: nothing more is asked, the locked members are skipped.
    let world = World::plain();
    let zip = locked_course(&world);
    let plan_two = plan_for(&world, &zip, &OpContext::new());
    let prompt = Prompt::new(vec![
        Answer::Type("a"),
        Answer::Type("b"),
        Answer::Type("c"),
        Answer::Type(CANARY),
    ]);
    let outcome = unpack(
        &world,
        &plan_two,
        prompt.clone(),
        &JobControl::new(),
        &Sink::skipping(),
    );
    assert_eq!(prompt.asked().len(), 3);
    assert!(
        outcome
            .lines
            .contains(&"Gave up on course.zip after 3 wrong passwords. Skipped.".to_owned()),
        "{:?}",
        outcome.lines
    );
    assert!(
        tree(&world, &plan_two.destination)
            .iter()
            .all(|(name, data)| data.is_empty() || name.is_empty())
    );
    assert!(world.fake.unfinished().is_empty());
}

#[test]
fn escape_and_cancel_at_the_prompt_leave_nothing_half_done() {
    let world = World::plain();
    let zip = locked_course(&world);
    let plan = plan_for(&world, &zip, &OpContext::new());
    let outcome = unpack(
        &world,
        &plan,
        Prompt::new(vec![Answer::Escape]),
        &JobControl::new(),
        &Sink::skipping(),
    );
    assert!(
        outcome
            .lines
            .contains(&"No password given. Skipped 3 members of course.zip.".to_owned()),
        "{:?}",
        outcome.lines
    );
    assert!(!outcome.report.cancelled);
    let files: Vec<String> = tree(&world, &plan.destination)
        .into_iter()
        .filter(|(_, data)| !data.is_empty())
        .map(|(name, _)| name)
        .collect();
    assert!(files.is_empty(), "{files:?}");
    let control = JobControl::new();
    let outcome = unpack(
        &world,
        &plan,
        Prompt::new(vec![Answer::Cancel(control.cancel.clone())]),
        &control,
        &Sink::skipping(),
    );
    assert!(outcome.report.cancelled, "{:?}", outcome.lines);
    assert!(
        outcome.lines[0].starts_with("Unpack cancelled"),
        "{:?}",
        outcome.lines
    );
    assert!(world.fake.unfinished().is_empty());
}

#[test]
fn a_locked_7z_asks_once_for_the_plan_and_the_run() {
    let world = World::plain();
    let sevenz = world.put(
        "locked.7z",
        &sevenz_of(
            &[("a.txt", b"one"), ("dir/b.txt", b"two")],
            true,
            Some(CANARY),
        ),
    );
    let prompt = Prompt::always(CANARY);
    let job = JobSecrets::new(prompt.clone());
    let plan = plan_for(&world, &sevenz, &job.context(&OpContext::new()));
    let outcome = unpack(
        &world,
        &plan,
        job.clone(),
        &JobControl::new(),
        &Sink::skipping(),
    );
    assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
    assert_eq!(prompt.asked().len(), 1, "asked once for the whole job");
    let files = tree(&world, &plan.destination);
    assert!(
        files.contains(&("dir/b.txt".to_owned(), b"two".to_vec())),
        "{files:?}"
    );
    drop(job);
}

#[test]
fn pack_locks_a_zip_with_aes_never_zipcrypto() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("locked.zip").unwrap();
    let request = PackRequest {
        password: Some(Secret::new(CANARY.to_owned())),
        ..PackRequest::new(vec![docs], destination.clone(), ArchiveFormat::Zip)
    };
    assert_eq!(
        new_password_request("locked.zip").prompt_line(),
        "New password for locked.zip (hidden): "
    );
    let plan = plan_of(&world, &request);
    assert!(
        plan.rows()
            .contains(&"Password: AES-256 (zip names visible)".to_owned())
    );
    let engine = engine(&world);
    let control = JobControl::new();
    let sink = Sink::skipping();
    let answers = Answers::no();
    let env = env(&world, &engine, &control, &sink, &answers, None);
    let outcome = run_pack(&plan, request.password.as_ref(), &env);
    assert_eq!(outcome.lines[1], "Checked 6 of 6 entries, all good.");
    let bytes = world.fake.contents(&destination).unwrap();
    // Every file's central record says method 99 (AES) with the AES extra
    // field (0x9901); none is ZipCrypto (method 8 with the encryption bit).
    let mut aes = 0;
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(4).position(|w| w == [b'P', b'K', 1, 2]) {
        let record = at + found;
        let flags = u16::from_le_bytes([bytes[record + 8], bytes[record + 9]]);
        let method = u16::from_le_bytes([bytes[record + 10], bytes[record + 11]]);
        if flags & 1 == 1 {
            assert_eq!(method, 99, "an encrypted member is AES, never ZipCrypto");
            aes += 1;
        }
        at = record + 4;
    }
    assert_eq!(aes, 4, "the four files");
    assert!(bytes.windows(2).any(|w| w == [0x01, 0x99]));
    let opened = world
        .provider
        .open_archive(&destination, &OpContext::new(), Allowances::default())
        .unwrap();
    assert!(
        opened.index().summary().notes.is_empty(),
        "no weak-encryption note"
    );
    let secrets = JobSecrets::new(Prompt::always(CANARY));
    let place = world.inside(&destination, "docs/notes.txt");
    let ctx = secrets.context(&OpContext::new());
    let mut reader = world
        .provider
        .read_member(&place, &ctx, Allowances::default(), None)
        .unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut reader, &mut text).unwrap();
    assert_eq!(text, "the notes, again and again and again");
    // Without the password, the bytes are not the text.
    assert!(read_zip_locked_fails(&bytes));
}

/// The zip crate refuses a locked member without its password.
fn read_zip_locked_fails(bytes: &[u8]) -> bool {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    zip.by_name("docs/notes.txt").is_err()
}

#[test]
fn the_canary_never_shows_in_a_job() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let mut said = Vec::new();
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let destination = out.join(&format!("canary{}", format.extension())).unwrap();
        let request = PackRequest {
            password: Some(Secret::new(CANARY.to_owned())),
            encrypt_names: true,
            ..PackRequest::new(vec![docs.clone()], destination.clone(), format)
        };
        let plan = plan_of(&world, &request);
        let engine = engine(&world);
        let control = JobControl::new();
        let sink = Sink::skipping();
        let answers = Answers::no();
        let env = env(&world, &engine, &control, &sink, &answers, None);
        let outcome = run_pack(&plan, request.password.as_ref(), &env);
        said.push(format!("{plan:?}"));
        said.extend(plan.rows());
        said.push(plan.question().text);
        said.push(serde_json::to_string(&plan.job_request()).unwrap());
        said.push(serde_json::to_string(&request.job_request(JobId::new(3))).unwrap());
        said.push(format!("{request:?}"));
        said.push(format!("{env:?}"));
        said.push(format!("{outcome:?}"));
        said.extend(outcome.lines.clone());
        said.extend(sink.lines.lock().unwrap().iter().cloned());
        // And unpacked again, through the prompt.
        let prompt = Prompt::always(CANARY);
        let job = JobSecrets::new(prompt.clone());
        let unpack_plan = plan_unpack(
            &world.provider,
            &world.registry,
            &UnpackRequest::new(destination.clone(), out.clone()),
            JobId::new(42),
            &job.context(&OpContext::new()),
            Allowances::default(),
        )
        .unwrap();
        let sink = Sink::skipping();
        let outcome = unpack(&world, &unpack_plan, job.clone(), &JobControl::new(), &sink);
        assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
        said.push(format!("{unpack_plan:?}"));
        said.extend(unpack_plan.rows());
        said.push(serde_json::to_string(&unpack_plan.job_request()).unwrap());
        said.push(format!("{outcome:?}"));
        said.push(format!("{job:?}"));
        said.extend(outcome.lines.clone());
        said.extend(sink.lines.lock().unwrap().iter().cloned());
        for request in prompt.asked() {
            said.push(serde_json::to_string(&request).unwrap());
            said.push(diroxide_archive::wrong_password(&request.next_attempt()).text);
        }
    }
    assert!(said.len() > 40);
    for text in &said {
        assert!(!text.contains(CANARY), "the canary in {text:?}");
    }
}
