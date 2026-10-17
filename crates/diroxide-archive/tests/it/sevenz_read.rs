//! 7z reading through the provider (commit 18): blocks and solid blocks,
//! the in-order visit, the dictionary pre-scan, passwords and encrypted
//! headers; and the in-order visit of the other formats.

use std::io::Read;
use std::sync::Arc;

use diroxide_archive::{Allowances, ArchiveError, ArchiveProvider, Limits};
use diroxide_vfs::{OpContext, Provider, Registry, VfsError};

use crate::build::{gzip, sevenz_of, tar_of, zip_of};
use crate::memory::{Peak, mib};
use crate::world::World;

fn members() -> Vec<(String, Vec<u8>)> {
    (0..6)
        .map(|i| {
            (
                format!("dir/file{i}.txt"),
                format!("contents of {i} ").repeat(1000).into_bytes(),
            )
        })
        .collect()
}

fn sevenz(solid: bool, password: Option<&str>) -> Vec<u8> {
    let owned = members();
    let list: Vec<(&str, &[u8])> = owned
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    sevenz_of(&list, solid, password)
}

#[test]
fn a_7z_lists_and_reads_in_place_solid_or_not() {
    for solid in [false, true] {
        let world = World::plain();
        let place = world.put("course.7z", &sevenz(solid, None));
        let rows = world.rows(&place.clone().into_archive()).unwrap();
        assert_eq!(rows, ["dir, folder"], "solid {solid}");
        let dir = world.list(&world.inside(&place, "dir"));
        assert_eq!(dir.len(), 6);
        assert!(dir.iter().all(|e| e.crc32.is_some()));
        // Out of order, back and forth.
        for i in [4, 0, 5, 2] {
            let read = world
                .read(&world.inside(&place, &format!("dir/file{i}.txt")))
                .unwrap();
            assert_eq!(read, members()[i].1, "solid {solid}, member {i}");
        }
        let random = world
            .provider
            .capabilities(&world.inside(&place, "dir"))
            .random_read;
        assert_eq!(random, !solid);
    }
}

#[test]
fn reading_far_into_a_solid_block_asks_first() {
    let world = World::plain();
    let provider = Arc::new(ArchiveProvider::new(Limits::default()).with_cost_question(10_000));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(world.fake.clone()));
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    let place = world.put("solid.7z", &sevenz(true, None));
    let last = world.inside(&place, "dir/file5.txt");
    let asked = provider
        .read_member(&last, &OpContext::new(), Allowances::default(), None)
        .err()
        .unwrap();
    let ArchiveError::Ask(question) = asked else {
        panic!("{asked:?}");
    };
    assert_eq!(
        question.message().text,
        "Read 70.0 KB first to open file5.txt? y or n"
    );
}

#[test]
fn the_in_order_visit_reads_a_solid_archive_in_one_pass() {
    let world = World::plain();
    let archives = [
        ("solid.7z", sevenz(true, None)),
        ("plain.7z", sevenz(false, None)),
        ("course.zip", {
            let owned = members();
            let list: Vec<(&str, &[u8])> = owned
                .iter()
                .map(|(n, d)| (n.as_str(), d.as_slice()))
                .collect();
            zip_of(&list)
        }),
        ("course.tar.gz", {
            let owned = members();
            let list: Vec<(&str, &[u8])> = owned
                .iter()
                .map(|(n, d)| (n.as_str(), d.as_slice()))
                .collect();
            gzip(&tar_of(&list))
        }),
    ];
    for (name, bytes) in archives {
        let place = world.put(name, &bytes);
        let mut seen = Vec::new();
        world
            .provider
            .visit(
                &place,
                &OpContext::new(),
                Allowances::default(),
                None,
                &mut |index, id, reader| {
                    let mut data = Vec::new();
                    reader
                        .read_to_end(&mut data)
                        .map_err(diroxide_archive::guard::from_io)?;
                    seen.push((index.path(id), data));
                    Ok(true)
                },
            )
            .unwrap();
        let files: Vec<_> = seen.iter().filter(|(_, d)| !d.is_empty()).collect();
        assert_eq!(files.len(), 6, "{name}");
        for (path, data) in files {
            let want = members().into_iter().find(|(n, _)| n == path).unwrap().1;
            assert_eq!(*data, want, "{name}: {path}");
        }
    }
}

struct Password(&'static str);

impl diroxide_vfs::SecretSource for Password {
    fn ask(
        &self,
        _request: &diroxide_core::SecretRequest,
        _cancel: &diroxide_vfs::CancelToken,
    ) -> Option<diroxide_core::Secret> {
        Some(diroxide_core::Secret::new(self.0.to_owned()))
    }
}

#[test]
fn encrypted_headers_need_the_password() {
    let world = World::plain();
    let place = world.put(
        "secret.7z",
        &sevenz(false, Some("canary-7f3a-not-a-password")),
    );
    let refused = world
        .provider
        .open_archive(&place, &OpContext::new(), Allowances::default())
        .unwrap_err();
    assert_eq!(
        refused.message("open", "secret.7z").text,
        "Could not open secret.7z: a password is needed. Enter the password and try again."
    );
    let with = OpContext::new().with_secrets(Arc::new(Password("canary-7f3a-not-a-password")));
    let opened = world
        .provider
        .open_archive(&place, &with, Allowances::default())
        .unwrap();
    assert!(opened.index().summary().notes[0].contains("hides its names"));
    let mut reader = world
        .provider
        .read(&world.inside(&place, "dir/file3.txt"), &with)
        .unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, members()[3].1);
    let other = World::plain();
    let place = other.put(
        "secret.7z",
        &sevenz(false, Some("canary-7f3a-not-a-password")),
    );
    let wrong = OpContext::new().with_secrets(Arc::new(Password("wrong")));
    let refused = other
        .provider
        .open_archive(&place, &wrong, Allowances::default())
        .unwrap_err();
    assert_eq!(refused, ArchiveError::Vfs(VfsError::WrongPassword));
    assert!(
        refused
            .message("open", "secret.7z")
            .text
            .contains("the password is wrong, or the data is damaged")
    );
}

#[test]
fn a_4_gib_dictionary_is_refused_before_the_allocation() {
    let world = World::new(Limits::strict());
    let peak = Peak::start();
    let header = world.put("header.7z", &crate::hostile::sevenz_four_gib_dictionary());
    let error = world.rows(&header.into_archive()).unwrap_err();
    assert!(
        error.starts_with(
            "it passes the limit for dictionary size: 64 MiB, and header.7z asks for 3.9 GiB"
        ),
        "{error}"
    );
    let content = world.put(
        "content.7z",
        &crate::hostile::sevenz_four_gib_content_dictionary(),
    );
    let error = world.rows(&content.into_archive()).unwrap_err();
    assert!(error.contains("dictionary size"), "{error}");
    let peak = peak.stop();
    assert!(peak < 512 << 20, "{:.0} MiB", mib(peak));
    // The default limits ask between 256 MiB and 1.5 GiB, refuse above.
    let world = World::plain();
    let content = world.put(
        "content.7z",
        &crate::hostile::sevenz_four_gib_content_dictionary(),
    );
    let error = world.rows(&content.into_archive()).unwrap_err();
    assert!(error.contains("1.5 GiB"), "{error}");
}

#[test]
fn a_7z_named_zip_says_so_in_its_title() {
    let world = World::plain();
    let place = world.put("a.zip", &sevenz(false, None));
    let root = place.clone().into_archive();
    let _ = world.list(&root);
    let folder = world.root.to_string();
    assert_eq!(
        world.provider.title(&root),
        format!("a.zip, 7z archive, in {folder}")
    );
}
