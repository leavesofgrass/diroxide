//! Spike K1, the password prompt (research/alpha2-plan.md 4.4): an AES
//! zip and a header-encrypted 7z built in memory; the provider says a
//! password is needed; the request names the archive and hides the
//! answer; three tries, then nothing more is asked; one password serves
//! the whole job and is forgotten after it; Escape or a cancel at the
//! prompt leaves nothing half done; the canary never shows; ZipCrypto is
//! read with its note; and the 7z question says both causes.

use std::io::Read;

use diroxide_archive::{
    Allowances, ArchiveError, JobSecrets, PackRequest, UnpackRequest, guard, member_row,
    wrong_password,
};
use diroxide_core::{Place, Secret, SecretKind, SecretRequest};
use diroxide_vfs::{CancelToken, OpContext, VfsError};

use crate::build::{sevenz_of, zipcrypto_zip};
use crate::prompt::{Answer, CANARY, Prompt, aes_zip};
use crate::world::World;

/// Reads a member whole through the provider's own call.
fn read(world: &World, place: &Place, ctx: &OpContext) -> Result<Vec<u8>, ArchiveError> {
    let mut reader = world
        .provider
        .read_member(place, ctx, Allowances::default(), None)?;
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| ArchiveError::from(guard::from_io(e)))?;
    Ok(out)
}

fn course(world: &World) -> Place {
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

fn locked_7z(world: &World) -> Place {
    world.put(
        "a.7z",
        &sevenz_of(&[("a.txt", b"one"), ("b.txt", b"two")], false, Some(CANARY)),
    )
}

#[test]
fn locked_archives_say_a_password_is_needed() {
    let world = World::plain();
    let zip = course(&world);
    let member = world.inside(&zip, "notes.txt");
    let refused = read(&world, &member, &OpContext::new()).unwrap_err();
    assert_eq!(refused, ArchiveError::Vfs(VfsError::PasswordRequired));
    let sevenz = locked_7z(&world);
    let refused = world
        .provider
        .open_archive(&sevenz, &OpContext::new(), Allowances::default())
        .unwrap_err();
    assert_eq!(
        refused.message("open", "a.7z").text,
        "Could not open a.7z: a password is needed. Enter the password and try again."
    );
}

#[test]
fn the_prompt_names_the_archive_and_hides_the_answer() {
    let world = World::plain();
    let zip = course(&world);
    let prompt = Prompt::always(CANARY);
    let secrets = JobSecrets::new(prompt.clone());
    let member = world.inside(&zip, "notes.txt");
    let data = secrets
        .with_tries(&OpContext::new(), |ctx| read(&world, &member, ctx))
        .unwrap();
    assert_eq!(data, b"third");
    let asked = prompt.asked();
    assert_eq!(asked.len(), 1);
    let request = &asked[0];
    assert_eq!(request.kind, SecretKind::ArchivePassword);
    assert_eq!(request.prompt_line(), "Password for course.zip (hidden): ");
    assert_eq!(request.title(), "Password for course.zip");
    assert!(!request.echo && !request.new_secret);
    assert_eq!((request.attempt, request.attempts_allowed), (1, 3));
}

#[test]
fn three_tries_then_stop() {
    let world = World::plain();
    let zip = course(&world);
    let sevenz = locked_7z(&world);
    for place in [world.inside(&zip, "notes.txt"), sevenz.clone()] {
        let prompt = Prompt::new(vec![Answer::Type("wrong"); 5]);
        let secrets = JobSecrets::new(prompt.clone());
        let result = secrets.with_tries(&OpContext::new(), |ctx| {
            if place == sevenz {
                world
                    .provider
                    .open_archive(&place, ctx, Allowances::default())
                    .map(|_| Vec::new())
            } else {
                read(&world, &place, ctx)
            }
        });
        assert_eq!(result, Err(ArchiveError::Vfs(VfsError::WrongPassword)));
        let tries: Vec<u32> = prompt.asked().iter().map(|r| r.attempt).collect();
        assert_eq!(tries, [1, 2, 3], "{place}");
        assert!(secrets.gave_up());
        let subject = secrets.subject().unwrap();
        assert_eq!(
            secrets.gave_up_line(&subject, 1).text,
            format!("Gave up on {subject} after 3 wrong passwords. Skipped.")
        );
    }
}

#[test]
fn a_wrong_password_then_the_right_one() {
    let world = World::plain();
    let zip = course(&world);
    let prompt = Prompt::new(vec![Answer::Type("wrong"), Answer::Type(CANARY)]);
    let secrets = JobSecrets::new(prompt.clone());
    let member = world.inside(&zip, "week1/a.txt");
    let data = secrets
        .with_tries(&OpContext::new(), |ctx| read(&world, &member, ctx))
        .unwrap();
    assert_eq!(data, b"first");
    let asked = prompt.asked();
    assert_eq!(asked.iter().map(|r| r.attempt).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(
        wrong_password(&asked[1]).text,
        "Wrong password for course.zip. Try again? y or n"
    );
    let sevenz = locked_7z(&world);
    let prompt = Prompt::new(vec![Answer::Type("wrong"), Answer::Type(CANARY)]);
    let secrets = JobSecrets::new(prompt.clone());
    let opened = secrets
        .with_tries(&OpContext::new(), |ctx| {
            world
                .provider
                .open_archive(&sevenz, ctx, Allowances::default())
        })
        .unwrap();
    assert_eq!(opened.index().summary().members, 2);
    assert_eq!(
        wrong_password(&prompt.asked()[1]).text,
        "Wrong password or damaged data. Try again? y or n"
    );
}

#[test]
fn one_password_serves_the_job_and_is_forgotten_after_it() {
    let world = World::plain();
    let zip = course(&world);
    let prompt = Prompt::always(CANARY);
    let job = JobSecrets::new(prompt.clone());
    for member in ["week1/a.txt", "week1/b.txt", "notes.txt"] {
        let place = world.inside(&zip, member);
        job.with_tries(&OpContext::new(), |ctx| read(&world, &place, ctx))
            .unwrap();
    }
    assert_eq!(prompt.asked().len(), 1, "asked once for three members");
    assert!(job.holds_a_password());
    drop(job);
    // The next job knows nothing of the last one's password.
    let next = JobSecrets::new(prompt.clone());
    assert!(!next.holds_a_password());
    let place = world.inside(&zip, "notes.txt");
    next.with_tries(&OpContext::new(), |ctx| read(&world, &place, ctx))
        .unwrap();
    assert_eq!(prompt.asked().len(), 2);
    next.forget();
    assert!(!next.holds_a_password());
}

#[test]
fn escape_or_cancel_at_the_prompt_leaves_nothing_half_done() {
    let world = World::plain();
    let sevenz = locked_7z(&world);
    let before = world.fake.snapshot();
    let token = CancelToken::new();
    let prompt = Prompt::new(vec![Answer::Cancel(token.clone())]);
    let secrets = JobSecrets::new(prompt.clone());
    let ctx = OpContext::new().with_cancel(token);
    let refused = secrets
        .with_tries(&ctx, |ctx| {
            world
                .provider
                .open_archive(&sevenz, ctx, Allowances::default())
        })
        .unwrap_err();
    assert_eq!(refused, ArchiveError::Vfs(VfsError::PasswordRequired));
    assert!(secrets.gave_up(), "nothing more is asked in the job");
    assert_eq!(world.fake.snapshot(), before, "nothing was written");
    // Nothing half done: the archive opens on the next try, asking again.
    let again = Prompt::always(CANARY);
    let ctx = OpContext::new().with_secrets(again.clone());
    let opened = world
        .provider
        .open_archive(&sevenz, &ctx, Allowances::default())
        .unwrap();
    assert_eq!(opened.index().summary().members, 2);
    assert_eq!(again.asked().len(), 1);
    // Escape on a zip member: the member is refused, not read in part.
    let zip = course(&world);
    let prompt = Prompt::new(vec![Answer::Escape]);
    let secrets = JobSecrets::new(prompt);
    let place = world.inside(&zip, "notes.txt");
    let refused = secrets
        .with_tries(&OpContext::new(), |ctx| read(&world, &place, ctx))
        .unwrap_err();
    assert_eq!(refused, ArchiveError::Vfs(VfsError::PasswordRequired));
}

#[test]
fn the_canary_never_shows() {
    let world = World::plain();
    let zip = course(&world);
    let sevenz = locked_7z(&world);
    let prompt = Prompt::new(vec![
        Answer::Type(CANARY),
        Answer::Type(CANARY),
        Answer::Type("wrong"),
    ]);
    let secrets = JobSecrets::new(prompt.clone());
    let place = world.inside(&zip, "notes.txt");
    secrets
        .with_tries(&OpContext::new(), |ctx| read(&world, &place, ctx))
        .unwrap();
    let opened = secrets
        .with_tries(&OpContext::new(), |ctx| {
            world
                .provider
                .open_archive(&sevenz, ctx, Allowances::default())
        })
        .unwrap();
    let wrong = JobSecrets::new(Prompt::always("wrong"));
    let refused = wrong
        .with_tries(&OpContext::new(), |ctx| read(&world, &place, ctx))
        .unwrap_err();
    let mut said = vec![
        format!("{secrets:?}"),
        format!("{wrong:?}"),
        format!("{opened:?}"),
        format!("{:?}", world.provider),
        format!("{refused:?}"),
        refused.to_string(),
        refused.message("read", "notes.txt").text,
        opened.opened_line(),
        secrets.gave_up_line("course.zip", 3).text,
    ];
    for request in prompt.asked() {
        said.push(format!("{request:?}"));
        said.push(serde_json::to_string(&request).unwrap());
        said.push(request.prompt_line());
        said.push(wrong_password(&request).text);
    }
    for entry in world.list(&zip.clone().into_archive()) {
        said.push(member_row(&entry));
    }
    let destination = world.root.join("out").unwrap();
    let mut pack = PackRequest::new(vec![place.clone()], destination.clone(), opened.format());
    pack.password = Some(Secret::new(CANARY.to_owned()));
    let mut unpack = UnpackRequest::new(zip.clone(), destination);
    unpack.password = Some(Secret::new(CANARY.to_owned()));
    said.push(format!("{pack:?}"));
    said.push(format!("{unpack:?}"));
    for text in &said {
        assert!(!text.contains(CANARY), "the canary in {text:?}");
    }
    assert!(
        SecretRequest::new(SecretKind::ArchivePassword, "x.zip")
            .prompt_line()
            .ends_with("(hidden): ")
    );
}

#[test]
fn zipcrypto_lists_with_its_note() {
    let world = World::plain();
    let zip = world.put("a.zip", &zipcrypto_zip("secret.txt", b"the plans", CANARY));
    let opened = world
        .provider
        .open_archive(&zip, &OpContext::new(), Allowances::default())
        .unwrap();
    assert!(
        opened
            .index()
            .summary()
            .notes
            .contains(&"Weak encryption in a.zip (ZipCrypto).".to_owned())
    );
    let row = &world.rows(&zip.clone().into_archive()).unwrap()[0];
    assert!(
        row.starts_with("secret.txt, locked, weak encryption"),
        "{row}"
    );
    let place = world.inside(&zip, "secret.txt");
    let secrets = JobSecrets::new(Prompt::always(CANARY));
    let data = secrets
        .with_tries(&OpContext::new(), |ctx| read(&world, &place, ctx))
        .unwrap();
    assert_eq!(data, b"the plans");
}

#[test]
fn the_7z_wrong_password_says_both_causes() {
    let world = World::plain();
    let sevenz = locked_7z(&world);
    let ctx = OpContext::new().with_secrets(Prompt::always("wrong"));
    let refused = world
        .provider
        .open_archive(&sevenz, &ctx, Allowances::default())
        .unwrap_err();
    assert_eq!(refused, ArchiveError::Vfs(VfsError::WrongPassword));
    assert!(
        refused
            .message("open", "a.7z")
            .text
            .starts_with("Could not open a.7z: the password is wrong, or the data is damaged")
    );
    let request = SecretRequest::new(SecretKind::ArchivePassword, "a.7z").next_attempt();
    assert_eq!(
        wrong_password(&request).text,
        "Wrong password or damaged data. Try again? y or n"
    );
}
