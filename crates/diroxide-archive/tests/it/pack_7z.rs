//! Pack to 7z (plan commit 34): solid blocks under the block limit, the
//! non-solid option, AES-256 with the names hidden, threads; a wrong
//! password refuses.

use std::io::Cursor;

use diroxide_archive::{
    Allowances, ArchiveError, ArchiveFormat, CompressionLevel, JobSecrets, PackRequest, run_pack,
};
use diroxide_core::{Place, Secret};
use diroxide_ops::JobControl;
use diroxide_vfs::{OpContext, Provider, VfsError};

use crate::build::noise;
use crate::jobs::{Answers, Sink, engine, env, out};
use crate::pack_zip::{plan_of, run_plan, sources};
use crate::prompt::{CANARY, Prompt};
use crate::world::World;

/// The 7z as the crate reads it: its blocks, whether it is solid, and
/// each file's name and contents.
fn read_7z(bytes: &[u8], password: &str) -> (usize, bool, Vec<(String, Vec<u8>)>) {
    let password = sevenz_rust2::Password::from(password);
    let mut reader =
        sevenz_rust2::ArchiveReader::new(Cursor::new(bytes.to_vec()), password).unwrap();
    let blocks = reader.archive().blocks.len();
    let solid = reader.archive().is_solid;
    let mut files = Vec::new();
    reader
        .for_each_entries(|entry, data| {
            let mut contents = Vec::new();
            data.read_to_end(&mut contents)?;
            if !entry.is_directory() {
                files.push((entry.name().to_owned(), contents));
            }
            Ok(true)
        })
        .unwrap();
    files.sort();
    (blocks, solid, files)
}

fn pack_with(world: &World, request: &PackRequest, password: Option<&str>) -> Vec<String> {
    let plan = plan_of(world, request);
    let engine = engine(world);
    let control = JobControl::new();
    let sink = Sink::skipping();
    let answers = Answers::no();
    let env = env(world, &engine, &control, &sink, &answers, None);
    let secret = password.map(|p| Secret::new(p.to_owned()));
    let outcome = run_pack(&plan, secret.as_ref(), &env);
    assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
    outcome.lines
}

#[test]
fn a_folder_packs_to_a_solid_7z() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let destination = out.join("docs.7z").unwrap();
    let request = PackRequest::new(vec![docs], destination.clone(), ArchiveFormat::SevenZ);
    let plan = plan_of(&world, &request);
    let rows = plan.rows();
    assert_eq!(rows[1], "Format: 7z, level normal (6 of 9)");
    assert_eq!(rows[3], "Password: none");
    assert_eq!(rows[4], "Solid: on, blocks up to 1 GiB");
    let outcome = run_plan(&world, &plan, &Sink::skipping());
    assert_eq!(
        outcome.lines,
        [
            "Pack finished: 6 done.",
            "Checked 6 of 6 entries, all good."
        ]
    );
    let (blocks, solid, files) = read_7z(&world.fake.contents(&destination).unwrap(), "");
    assert_eq!(blocks, 1);
    assert!(solid);
    assert_eq!(files.len(), 4);
    assert!(files.contains(&(
        "docs/notes.txt".to_owned(),
        b"the notes, again and again and again".to_vec()
    )));
    // Through the provider: the modes and times it stored read back.
    let root = destination.clone().into_archive();
    let notes = world
        .provider
        .stat(
            &root.join("docs").unwrap().join("notes.txt").unwrap(),
            &OpContext::new(),
        )
        .unwrap();
    assert_eq!(
        notes.attributes,
        Some(diroxide_core::Attributes::Unix(0o640))
    );
    assert_eq!(
        notes.modified.map(|t| t.unix_seconds()),
        Some(1_760_000_000)
    );
    assert!(!world.fake.exists(&plan.temp));
}

#[test]
fn the_block_limit_and_the_non_solid_option() {
    let world = World::plain();
    let folder = world.root.parent().unwrap().join("blocks").unwrap();
    for n in 0..5u64 {
        world.fake.add_file(
            &folder.join(&format!("part{n}.bin")).unwrap(),
            &noise(40_000, n + 1),
        );
    }
    let out = out(&world);
    let limited = out.join("limited.7z").unwrap();
    let request = PackRequest {
        solid_block_bytes: Some(64 << 10),
        ..PackRequest::new(vec![folder.clone()], limited.clone(), ArchiveFormat::SevenZ)
    };
    assert!(
        plan_of(&world, &request)
            .rows()
            .contains(&"Solid: on, blocks up to 64 KiB".to_owned())
    );
    pack_with(&world, &request, None);
    let (blocks, _, files) = read_7z(&world.fake.contents(&limited).unwrap(), "");
    assert_eq!(files.len(), 5);
    assert_eq!(
        blocks, 5,
        "40 KB files under a 64 KiB limit: one block each"
    );
    let request = PackRequest {
        solid_block_bytes: Some(100_000),
        ..PackRequest::new(
            vec![folder.clone()],
            out.join("two.7z").unwrap(),
            ArchiveFormat::SevenZ,
        )
    };
    pack_with(&world, &request, None);
    let (blocks, solid, _) = read_7z(
        &world.fake.contents(&out.join("two.7z").unwrap()).unwrap(),
        "",
    );
    assert_eq!(blocks, 3, "two files to a block of 100,000 bytes");
    assert!(solid);
    let plain = out.join("plain.7z").unwrap();
    let request = PackRequest {
        solid: false,
        level: CompressionLevel::Fastest,
        threads: Some(4),
        ..PackRequest::new(vec![folder], plain.clone(), ArchiveFormat::SevenZ)
    };
    assert!(
        plan_of(&world, &request)
            .rows()
            .contains(&"Solid: off".to_owned())
    );
    pack_with(&world, &request, None);
    let (blocks, solid, files) = read_7z(&world.fake.contents(&plain).unwrap(), "");
    assert_eq!((blocks, solid, files.len()), (5, false, 5));
}

#[test]
fn aes_256_with_hidden_names_and_a_wrong_password_refuses() {
    let world = World::plain();
    let docs = sources(&world);
    let out = out(&world);
    let locked = out.join("locked.7z").unwrap();
    let request = PackRequest {
        password: Some(Secret::new(CANARY.to_owned())),
        encrypt_names: true,
        ..PackRequest::new(vec![docs.clone()], locked.clone(), ArchiveFormat::SevenZ)
    };
    let plan = plan_of(&world, &request);
    assert!(
        plan.rows()
            .contains(&"Password: AES-256, names hidden".to_owned())
    );
    assert!(!format!("{plan:?}").contains(CANARY));
    let lines = pack_with(&world, &request, Some(CANARY));
    assert_eq!(lines[1], "Checked 6 of 6 entries, all good.");
    let bytes = world.fake.contents(&locked).unwrap();
    assert!(
        sevenz_rust2::ArchiveReader::new(
            Cursor::new(bytes.clone()),
            sevenz_rust2::Password::empty()
        )
        .is_err(),
        "the names need the password"
    );
    let (_, _, files) = read_7z(&bytes, CANARY);
    assert_eq!(files.len(), 4);
    // A wrong password refuses, saying both causes.
    let wrong = OpContext::new().with_secrets(Prompt::always("wrong"));
    let refused = world
        .provider
        .open_archive(&locked, &wrong, Allowances::default())
        .unwrap_err();
    assert_eq!(refused, ArchiveError::Vfs(VfsError::WrongPassword));
    // The right one opens it, asked once.
    let prompt = Prompt::always(CANARY);
    let ctx = JobSecrets::new(prompt.clone()).context(&OpContext::new());
    let opened = world
        .provider
        .open_archive(&locked, &ctx, Allowances::default())
        .unwrap();
    assert_eq!(opened.index().summary().members, 4);
    assert_eq!(prompt.asked().len(), 1);
    // Names visible: listed without the password, read with it.
    let visible = out.join("visible.7z").unwrap();
    let request = PackRequest {
        password: Some(Secret::new(CANARY.to_owned())),
        encrypt_names: false,
        ..PackRequest::new(vec![docs], visible.clone(), ArchiveFormat::SevenZ)
    };
    assert!(
        plan_of(&world, &request)
            .rows()
            .contains(&"Password: AES-256, names visible".to_owned())
    );
    pack_with(&world, &request, Some(CANARY));
    let listed = world.rows(&visible.clone().into_archive()).unwrap();
    assert!(listed[0].starts_with("docs, folder"), "{listed:?}");
    let place: Place = world.inside(&visible, "docs/notes.txt");
    assert!(
        world.read(&place).is_err(),
        "the contents need the password"
    );
}
