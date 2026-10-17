//! Zip reading through the provider (commit 14): list, stat and read in
//! place, zip64, AES and ZipCrypto, method errors, and the regression list
//! of research/archives.md section 13 for zip.

use std::io::{Cursor, Write};

use diroxide_archive::{Allowances, ArchiveError, member_row};
use diroxide_core::EntryKind;
use diroxide_vfs::{OpContext, Provider, VfsError};

use crate::build::{Method, RawMember, RawZip, deflate, zip_of};
use crate::world::World;

#[test]
fn a_zip_lists_and_reads_in_place() {
    let world = World::plain();
    let zip = world.put(
        "course.zip",
        &zip_of(&[
            ("week1/", b""),
            ("week1/notes.txt", b"hello"),
            ("week1/deep/more.txt", b"more"),
            ("readme.md", b"# Course"),
        ]),
    );
    let root = zip.clone().into_archive();
    let rows = world.rows(&root).unwrap();
    assert_eq!(rows, ["week1, folder", "readme.md, file, 8 bytes"]);
    let week1 = world.inside(&zip, "week1");
    let names: Vec<String> = world.list(&week1).into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["notes.txt", "deep"]);
    let notes = world.inside(&zip, "week1/notes.txt");
    assert_eq!(world.read(&notes).unwrap(), b"hello");
    let entry = world.provider.stat(&notes, &OpContext::new()).unwrap();
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size, Some(5));
    assert_eq!(entry.crc32, Some(crate::build::crc(b"hello")));
    assert!(entry.modified.is_some());
    let opened = world
        .provider
        .open_archive(&zip, &OpContext::new(), Allowances::default())
        .unwrap();
    assert_eq!(
        opened.opened_line(),
        "Opened course.zip: 5 items, 17 bytes."
    );
    assert!(matches!(
        world.provider.list(&notes, &OpContext::new()),
        Err(VfsError::NotAFolder)
    ));
    assert!(matches!(
        world
            .provider
            .stat(&world.inside(&zip, "nothing"), &OpContext::new()),
        Err(VfsError::NotFound)
    ));
}

#[test]
fn a_zip_named_dat_opens_by_its_bytes() {
    let world = World::plain();
    let data = world.put("data.dat", &zip_of(&[("inside.txt", b"found")]));
    let root = data.into_archive();
    assert_eq!(world.rows(&root).unwrap(), ["inside.txt, file, 5 bytes"]);
    // `data.dat!inside.txt` reads back as a local name (".dat" is no
    // archive extension); the root place joins its member instead.
    let typed = diroxide_core::Place::parse(&format!("{root}inside.txt")).unwrap();
    assert_eq!(typed.kind(), diroxide_core::PlaceKind::Local);
    let inside = root.join("inside.txt").unwrap();
    assert_eq!(world.read(&inside).unwrap(), b"found");
}

#[test]
fn the_index_is_cached_until_the_archive_changes() {
    let world = World::plain();
    let zip = world.put("a.zip", &zip_of(&[("one.txt", b"1")]));
    let ctx = OpContext::new();
    let first = world
        .provider
        .open_archive(&zip, &ctx, Allowances::default())
        .unwrap();
    let again = world
        .provider
        .open_archive(&zip, &ctx, Allowances::default())
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    world
        .fake
        .add_file(&zip, &zip_of(&[("one.txt", b"1"), ("two.txt", b"22")]));
    let changed = world
        .provider
        .open_archive(&zip, &ctx, Allowances::default())
        .unwrap();
    assert_eq!(changed.index().summary().members, 2);
}

/// A zip with an AES password, by the zip crate, or a ZipCrypto one, by
/// hand (the crate reads ZipCrypto and will not write it).
fn locked_zip(aes: bool) -> Vec<u8> {
    if !aes {
        return crate::build::zipcrypto_zip(
            "secret.txt",
            b"the plans",
            "canary-7f3a-not-a-password",
        );
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .with_aes_encryption(zip::AesMode::Aes256, "canary-7f3a-not-a-password");
    writer.start_file("secret.txt", options).unwrap();
    writer.write_all(b"the plans").unwrap();
    writer.finish().unwrap().into_inner()
}

struct Password(&'static str);

impl diroxide_vfs::SecretSource for Password {
    fn ask(
        &self,
        request: &diroxide_core::SecretRequest,
        _cancel: &diroxide_vfs::CancelToken,
    ) -> Option<diroxide_core::Secret> {
        assert!(request.prompt_line().contains("locked.zip"));
        Some(diroxide_core::Secret::new(self.0.to_owned()))
    }
}

fn read_locked(
    world: &World,
    place: &diroxide_core::Place,
    password: Option<&'static str>,
) -> Result<Vec<u8>, VfsError> {
    let mut ctx = OpContext::new();
    if let Some(password) = password {
        ctx = ctx.with_secrets(std::sync::Arc::new(Password(password)));
    }
    let mut reader = world.provider.read(place, &ctx)?;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut out).map_err(diroxide_archive::guard::from_io)?;
    Ok(out)
}

#[test]
fn aes_and_zipcrypto_read_with_their_password() {
    for aes in [true, false] {
        let world = World::plain();
        let zip = world.put("locked.zip", &locked_zip(aes));
        let secret = world.inside(&zip, "secret.txt");
        let row = &world.rows(&zip.clone().into_archive()).unwrap()[0];
        assert!(row.starts_with("secret.txt, locked"), "{row}");
        assert_eq!(row.contains("weak encryption"), !aes, "{row}");
        assert_eq!(
            read_locked(&world, &secret, None),
            Err(VfsError::PasswordRequired)
        );
        assert_eq!(
            read_locked(&world, &secret, Some("canary-7f3a-not-a-password")).unwrap(),
            b"the plans"
        );
        let wrong = read_locked(&world, &secret, Some("wrong"));
        assert!(wrong.is_err(), "aes {aes}: {wrong:?}");
        let opened = world
            .provider
            .open_archive(&zip, &OpContext::new(), Allowances::default())
            .unwrap();
        let notes = &opened.index().summary().notes;
        assert_eq!(
            notes.contains(&"Weak encryption in locked.zip (ZipCrypto).".to_owned()),
            !aes
        );
    }
}

#[test]
fn an_unknown_method_is_said_not_panicked() {
    let world = World::plain();
    let mut member = RawMember::new("odd.bin", b"data", Method::Stored);
    member.method = 77;
    let zip = world.put("odd.zip", &RawZip::new().member(&member).finish(b""));
    let odd = world.inside(&zip, "odd.bin");
    let row = &world.rows(&zip.clone().into_archive()).unwrap()[0];
    assert!(row.contains("cannot be read here"), "{row}");
    let error = world.read(&odd).unwrap_err();
    assert_eq!(
        error,
        "it uses compression method 77, which is not supported"
    );
}

#[test]
fn zip64_end_records_are_read() {
    // Seventy thousand members: past the classic 65,535, so the end
    // record is zip64's.
    let mut zip = RawZip::new();
    for i in 0..70_000u32 {
        zip = zip.member(&RawMember::new(&format!("f{i:05}"), b"", Method::Stored));
    }
    let world = World::plain();
    let place = world.put("big.zip", &zip.finish(b""));
    let opened = world
        .provider
        .open_archive(&place, &OpContext::new(), Allowances::default())
        .unwrap();
    assert_eq!(opened.index().summary().members, 70_000);
}

#[test]
fn the_regression_list_for_zip() {
    let world = World::plain();
    // A symlink whose stated size is huge: listed, its target read capped.
    let mut link = RawMember::symlink("link", "target.txt");
    link.size = 0xFFFF_FFF0;
    let zip = world.put("link.zip", &RawZip::new().member(&link).finish(b""));
    let rows = world.rows(&zip.clone().into_archive()).unwrap();
    assert!(
        rows[0].starts_with("link, link to target.txt, not followed"),
        "{rows:?}"
    );
    // A read that runs past its stated size stops with an error.
    let zip = world.put("small.zip", &crate::hostile::claims_too_little());
    let error = world.read(&world.inside(&zip, "small.bin")).unwrap_err();
    assert_eq!(
        error,
        "it is damaged: small.bin is longer than its stated size of 10 bytes"
    );
    // The quoted-overlap zip: listed with its note, nothing read.
    let zip = world.put("bomb.zip", &crate::hostile::overlap_bomb());
    let opened = world
        .provider
        .open_archive(&zip, &OpContext::new(), Allowances::default())
        .unwrap();
    assert!(opened.index().summary().notes[0].contains("overlap, as a zip bomb's do"));
    let error = world.read(&world.inside(&zip, "0005.bin")).unwrap_err();
    assert_eq!(
        error,
        "it was refused: its members overlap, as a zip bomb's do"
    );
    // The end-of-directory signature inside the comment.
    let comment = b"PK\x05\x06 a false end record in the comment";
    let zip = world.put(
        "comment.zip",
        &RawZip::new()
            .member(&RawMember::new("a.txt", b"real", Method::Stored))
            .finish(comment),
    );
    assert_eq!(world.read(&world.inside(&zip, "a.txt")).unwrap(), b"real");
    // A self-extracting prefix: accepted, with a note.
    let zip = world.put(
        "setup.zip",
        &RawZip::new()
            .prefix(&[b'M'; 600])
            .member(&RawMember::new(
                "a.txt",
                b"after the stub",
                Method::Deflated,
            ))
            .finish(b""),
    );
    assert_eq!(
        world.read(&world.inside(&zip, "a.txt")).unwrap(),
        b"after the stub"
    );
    let opened = world
        .provider
        .open_archive(&zip, &OpContext::new(), Allowances::default())
        .unwrap();
    assert!(
        opened
            .index()
            .summary()
            .notes
            .iter()
            .any(|n| n.contains("600 bytes before"))
    );
    // A data descriptor, with and without its signature.
    for signature in [true, false] {
        let data = b"described later";
        let mut member = RawMember::new("dd.txt", data, Method::Deflated);
        member.flags |= 1 << 3;
        let mut descriptor = Vec::new();
        if signature {
            descriptor.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
        }
        descriptor.extend_from_slice(&crate::build::crc(data).to_le_bytes());
        descriptor.extend_from_slice(&(member.data.len() as u32).to_le_bytes());
        descriptor.extend_from_slice(&(data.len() as u32).to_le_bytes());
        let mut zip = RawZip::new().member(&member).raw(&descriptor);
        zip = zip.member(&RawMember::new("next.txt", b"next", Method::Stored));
        let place = world.put(&format!("dd-{signature}.zip"), &zip.finish(b""));
        assert_eq!(world.read(&world.inside(&place, "dd.txt")).unwrap(), data);
        assert_eq!(
            world.read(&world.inside(&place, "next.txt")).unwrap(),
            b"next"
        );
    }
    // A central directory whose entry count lies: a note, and it lists.
    let zip = world.put(
        "count.zip",
        &RawZip::new()
            .member(&RawMember::new("a.txt", b"a", Method::Stored))
            .finish_claiming(b"", 2),
    );
    let opened = world
        .provider
        .open_archive(&zip, &OpContext::new(), Allowances::default());
    match opened {
        Ok(opened) => assert!(
            opened
                .index()
                .summary()
                .notes
                .iter()
                .any(|n| n.contains("says it holds 2 entries and holds 1")),
            "{:?}",
            opened.index().summary().notes
        ),
        Err(error) => assert!(error.to_string().contains("damaged"), "{error}"),
    }
    // A local header whose name differs from the central one.
    let zip = world.put("names.zip", &crate::hostile::names_disagree());
    let error = world.read(&world.inside(&zip, "b.txt")).unwrap_err();
    assert_eq!(error, "it was refused: its local and central names differ");
    let row = &world.rows(&zip.into_archive()).unwrap()[0];
    assert!(row.contains("its two headers disagree"), "{row}");
}

#[test]
fn end_records_that_lie_are_refused_before_anything_is_allocated() {
    let world = World::plain();
    // A claim of a billion entries in a 100-byte directory.
    let mut bytes = RawZip::new()
        .member(&RawMember::new("a.txt", b"a", Method::Stored))
        .finish_claiming(b"", 1_000_000_000);
    let zip = world.put("claims.zip", &bytes);
    let error = world.rows(&zip.into_archive()).unwrap_err();
    assert_eq!(error, "it passes the limit for entries: 1,000,000");
    // Under the entry limit, but more than the directory can hold.
    bytes = RawZip::new()
        .member(&RawMember::new("a.txt", b"a", Method::Stored))
        .finish_claiming(b"", 500);
    let zip = world.put("short.zip", &bytes);
    let error = world.rows(&zip.into_archive()).unwrap_err();
    assert!(error.contains("more than its directory holds"), "{error}");
    // No end record at all.
    let zip = world.put("cut.zip", &deflate(b"not a zip at all")[..]);
    let error = world.rows(&zip.into_archive()).unwrap_err();
    assert!(error.starts_with("it is damaged"), "{error}");
}

#[test]
fn a_member_over_the_size_limit_asks_first() {
    let world = World::new(diroxide_archive::Limits::strict());
    let zip = world.put("big.zip", &crate::hostile::claims_four_gb());
    let big = world.inside(&zip, "big.bin");
    let asked = world.read_with(&big, Allowances::default()).unwrap_err();
    let ArchiveError::Ask(question) = asked else {
        panic!("a question first: {asked:?}");
    };
    assert_eq!(
        question.message().text,
        "big.bin is 4.3 GB. Read it? y or n"
    );
    let refused = world.read_with(&big, Allowances::all()).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("big.bin is shorter than its stated size"),
        "{refused}"
    );
    let row = member_row(&world.list(&zip.into_archive())[0]);
    assert_eq!(row, "big.bin, file, 4.3 GB");
}
