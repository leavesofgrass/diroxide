//! The shipped table (R30's commit 25): it reads whole, agrees with
//! core's group table, finds each system's screenshots by name, and
//! types the usual files by their bytes and their names.

use std::collections::HashSet;

use diroxide_config::FileTypesConfig;
use diroxide_core::filter::{GROUPS, Unknown, group_of_extension};
use diroxide_core::format::within_cells;
use diroxide_core::{Entry, EntryKind, FixedClock, Timestamp, UtcOffset};
use diroxide_filetype::{Probe, Special, TypeRegistry, TypeStage};

fn clock() -> FixedClock {
    let now = Timestamp::from_civil(2026, 11, 6, 4, 0, 0, UtcOffset::UTC).unwrap();
    FixedClock::new(now, UtcOffset::UTC)
}

fn file(name: &str) -> Entry {
    Entry::new(name, EntryKind::File).with_size(4_096)
}

fn by_bytes(types: &TypeRegistry, name: &str, head: &[u8]) -> String {
    match types.check(&Probe::new(&file(name), Some(head)), &clock()) {
        Ok(found) => found.type_id,
        Err(special) => special.words().to_owned(),
    }
}

fn by_name(types: &TypeRegistry, name: &str) -> String {
    match types.by_name(&file(name), &clock()) {
        Ok(found) => found.type_id,
        Err(special) => special.words().to_owned(),
    }
}

#[test]
fn the_shipped_table_reads_whole() {
    let types = TypeRegistry::builtin();
    assert!(types.problems().is_empty(), "{:?}", types.problems());
    assert!(types.types().len() >= 100, "{}", types.types().len());
    let mut ids = HashSet::new();
    for t in types.types() {
        assert!(ids.insert(t.id.as_str()), "{} twice", t.id);
        assert!(!t.yours && !t.retro, "{}", t.id);
        let mime = t.mime.as_deref().unwrap_or_default();
        assert!(mime.contains('/'), "{} has no media type", t.id);
        assert!(!t.name.is_empty() && !t.short.is_empty(), "{}", t.id);
        // The row's word is short, and the details' line fits a Braille
        // line with the type first.
        assert!(t.short.chars().count() <= 12, "{}", t.short);
        let line = format!("Type: {}", t.name);
        assert!(line.chars().count() <= 40, "{line}");
        if let Some(group) = &t.group {
            assert!(types.group(group).is_some(), "{}: {group}", t.id);
        }
        assert!(
            !t.signatures.is_empty()
                || t.when.is_some()
                || !t.extensions.is_empty()
                || !t.patterns.is_empty(),
            "{} has no test",
            t.id
        );
        for extension in &t.extensions {
            assert_eq!(*extension, extension.to_lowercase(), "{}", t.id);
        }
    }
    let group_ids: Vec<&str> = types.groups().iter().map(|g| g.id.as_str()).collect();
    assert_eq!(
        group_ids,
        [
            "images",
            "music",
            "video",
            "documents",
            "archives",
            "code",
            "text",
            "screenshots"
        ]
    );
    assert_eq!(types.groups_line(), "Groups: 8 built in");
}

/// Core's seven groups moved into the table: each group's sentence and
/// every extension core's table places are where core puts them, so
/// `group:` reads as before for every name core knew.
#[test]
fn the_groups_agree_with_cores_table() {
    let types = TypeRegistry::builtin();
    for core in GROUPS {
        let group = types.group(core.name).unwrap();
        assert_eq!(group.about, core.description);
        assert_eq!(group.name.to_lowercase(), core.name);
        let first = group.about.split(':').next().unwrap_or_default();
        assert!(within_cells(&group.about, first), "{}", group.id);
    }
    for t in types.types() {
        for extension in &t.extensions {
            if let Some(core) = group_of_extension(extension) {
                assert_eq!(
                    t.group.as_deref(),
                    Some(core.name),
                    "{}: .{extension} is in {} in core's table",
                    t.id,
                    core.name
                );
            }
        }
    }
    for core in GROUPS {
        for extension in core.extensions {
            let name = format!("sample.{extension}");
            let group = types.group_for(&file(&name), &clock()).unwrap();
            assert_eq!(group.map(|g| g.id.as_str()), Some(core.name), "{name}");
        }
    }
}

/// R29 5.6: each system's screenshot names, the macOS one with its
/// narrow no-break space; a screenshot is in Images first, then in
/// Screenshots.
#[test]
fn each_systems_screenshots_are_found() {
    let types = TypeRegistry::builtin();
    for name in [
        "Screenshot (12).png",
        "Screenshot 2026-11-06 at 4.15.03\u{202F}AM.png",
        "Screenshot 2026-11-06 at 4.15.03\u{202F}AM (2).png",
        "Screen Shot 2019-05-01 at 10.15.03 PM.png",
        "Screenshot 2026-11-06 at 16.15.03.png",
        "IMG_0042.PNG",
        "Screenshot_20261106-041503.png",
        "Screenshot_20261106_041503_Chrome.jpg",
        "Screenshot from 2026-11-06 04-15-03.png",
        "Screenshot_20261106_041503.png",
    ] {
        let groups = types.groups_of(&file(name), &clock()).unwrap();
        let ids: Vec<&str> = groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["images", "screenshots"], "{name}");
    }
    for name in [
        "Screenshot.txt",
        "photo.png",
        "IMG_0042.JPG",
        "Screenshot (12).png.txt",
    ] {
        let groups = types.groups_of(&file(name), &clock()).unwrap();
        assert!(
            groups.iter().all(|g| g.id != "screenshots"),
            "{name}: {groups:?}"
        );
    }
}

#[test]
fn group_answers_wait_for_a_pending_type_and_skip_folders() {
    let types = TypeRegistry::builtin();
    assert_eq!(
        types.group_for(&file("mystery"), &clock()),
        Err(Unknown::TypeNotChecked)
    );
    let mut checked = file("mystery");
    checked.type_name = Some("unknown".to_owned());
    assert_eq!(types.group_for(&checked, &clock()), Ok(None));
    let folder = Entry::new("Pictures.png", EntryKind::Folder);
    assert!(types.groups_of(&folder, &clock()).unwrap().is_empty());
    let mut indexed = file("mystery");
    let found = types
        .check(&Probe::new(&indexed, Some(b"%PDF-1.7\n")), &clock())
        .unwrap();
    indexed.type_index = TypeRegistry::index_of(&found);
    let group = types.group_for(&indexed, &clock()).unwrap().unwrap();
    assert_eq!(group.id, "documents");
}

#[test]
fn the_owners_groups_join_and_complete() {
    let config = FileTypesConfig::from_toml(
        r#"
[[group]]
id = "scans"
name = "Scans"
about = "Scanned pages: by their names."
patterns = ["scan-*.pdf, scan-*.png"]

[[group]]
id = "receipts"
name = "Receipts"
about = "Receipts, by their names."
patterns = ['re:^receipt \d+']
"#,
    )
    .unwrap();
    let types = TypeRegistry::builtin().with_config(&config);
    assert_eq!(types.groups_line(), "Groups: 8 built in, 2 yours");
    assert!(within_cells(&types.groups_line(), "Groups"));
    let names = types.group_names();
    assert_eq!(names.len(), 10);
    assert!(names.contains(&"scans".to_owned()));
    let groups = types.groups_of(&file("scan-0001.pdf"), &clock()).unwrap();
    let ids: Vec<&str> = groups.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids, ["documents", "scans"]);
}

/// Each kind of file by its first bytes alone, and the usual names.
#[test]
fn the_usual_files_are_typed() {
    let types = TypeRegistry::builtin();
    let ogg = |at28: &[u8]| {
        let mut head = b"OggS".to_vec();
        head.resize(28, 0);
        head.extend_from_slice(at28);
        head
    };
    let ftyp = |brand: &[u8]| {
        let mut head = b"\x00\x00\x00\x18ftyp".to_vec();
        head.extend_from_slice(brand);
        head.extend_from_slice(b"\x00\x00\x02\x00");
        head
    };
    let mut deb = b"!<arch>\ndebian-binary   ".to_vec();
    deb.resize(64, b' ');
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"\x89PNG\r\n\x1a\n\x00\x00".to_vec(), "png"),
        (b"\xff\xd8\xff\xe0\x00\x10JFIF".to_vec(), "jpeg"),
        (b"GIF89a\x01\x00".to_vec(), "gif"),
        (b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec(), "webp"),
        (b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec(), "wav"),
        (b"RIFF\x24\x00\x00\x00AVI LIST".to_vec(), "avi"),
        (b"%PDF-1.7\n".to_vec(), "pdf"),
        (b"7z\xbc\xaf\x27\x1c\x00\x04".to_vec(), "7z"),
        (b"Rar!\x1a\x07\x01\x00".to_vec(), "rar"),
        (b"\x1f\x8b\x08\x00\x00\x00".to_vec(), "gzip"),
        (b"BZh91AY&SY".to_vec(), "bzip2"),
        (b"\xfd7zXZ\x00\x00".to_vec(), "xz"),
        (b"\x28\xb5\x2f\xfd\x04".to_vec(), "zstd"),
        (b"\x7fELF\x02\x01\x01".to_vec(), "elf"),
        (b"SQLite format 3\x00".to_vec(), "sqlite"),
        (ogg(b"OpusHead"), "opus"),
        (ogg(b"\x80theora"), "ogv"),
        (ogg(b"\x01vorbis"), "ogg"),
        (ftyp(b"isom"), "mp4"),
        (ftyp(b"heic"), "heic"),
        (ftyp(b"M4A "), "m4a"),
        (ftyp(b"qt  "), "mov"),
        (ftyp(b"zzzz"), "iso-media"),
        (
            b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\xf7\x81\x01\x42\xf2\x81\x04\x42\xf3\x81\x08\x42\x82\x84webm".to_vec(),
            "webm",
        ),
        (
            b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\xf7\x81\x01\x42\xf2\x81\x04\x42\xf3\x81\x08\x42\x82\x88matroska".to_vec(),
            "mkv",
        ),
        (b"%!PS-Adobe-3.0".to_vec(), "postscript"),
        (b"{\\rtf1\\ansi".to_vec(), "rtf"),
        (b"<?xml version=\"1.0\"?>".to_vec(), "xml"),
        (b"#!/bin/sh\necho".to_vec(), "shell"),
        (b"#!/usr/bin/env python3\n".to_vec(), "python"),
        (b"#!/usr/bin/awk -f\n".to_vec(), "script"),
        (b"II*\x00\x08\x00\x00\x00".to_vec(), "tiff"),
        (b"MThd\x00\x00\x00\x06".to_vec(), "midi"),
        (b"fLaC\x00\x00\x00\x22".to_vec(), "flac"),
        (b"\x00asm\x01\x00\x00\x00".to_vec(), "wasm"),
        (b"\xca\xfe\xba\xbe\x00\x00\x00\x41".to_vec(), "java-class"),
        (b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00".to_vec(), "ole"),
        (b"BEGIN:VCALENDAR\r\nVERSION:2.0".to_vec(), "calendar"),
        (b"<!DOCTYPE html>\n<html>".to_vec(), "html"),
        (deb, "deb"),
        (b"!<arch>\nlib.o/          ".to_vec(), "ar"),
        (b"#!AMR\n".to_vec(), "amr"),
    ];
    for (head, wanted) in cases {
        assert_eq!(by_bytes(&types, "file.bin", &head), wanted, "{head:?}");
    }
    // Two fixed bytes are not enough alone.
    assert_eq!(by_bytes(&types, "file.bin", b"MZ\x90\x00"), "unknown");
    assert_eq!(by_bytes(&types, "setup.exe", b"MZ\x90\x00"), "exe");
    assert_eq!(by_bytes(&types, "file.bin", b"BM\x36\x00"), "unknown");
    assert_eq!(
        by_bytes(&types, "a.doc", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"),
        "doc"
    );
    assert_eq!(by_bytes(&types, "a.svg", b"<?xml version=\"1.0\"?>"), "svg");
    assert_eq!(
        by_bytes(&types, "a.nef", b"MM\x00*\x00\x00\x00\x08"),
        "camera-raw"
    );
    // The contents speak first: a .py file that starts as a shell script
    // is a shell script; one that starts as Python is Python.
    assert_eq!(by_bytes(&types, "run.py", b"#!/bin/sh\n"), "shell");
    assert_eq!(
        by_bytes(&types, "run.py", b"#!/usr/bin/env python3\n"),
        "python"
    );
    for (name, wanted) in [
        ("report.pdf", "pdf"),
        ("notes.md", "markdown"),
        ("notes.txt", "text"),
        ("Makefile", "makefile"),
        ("Dockerfile", "dockerfile"),
        ("backup.tar.gz", "tar-gz"),
        ("backup.tgz", "tar-gz"),
        ("photos.tar.zst", "tar-zst"),
        ("lib.gz", "gzip"),
        ("app.ts", "typescript"),
        ("clip.mts", "mpeg-ts"),
        ("report.docx", "docx"),
        ("book.epub", "epub"),
        ("disk.vhdx", "vhdx"),
        ("song.flac", "flac"),
        ("main.rs", "rust"),
        ("data", "type pending"),
    ] {
        assert_eq!(by_name(&types, name), wanted, "{name}");
    }
}

/// The disagreement line, meaning first: the name's word, then the
/// contents'. A refinement is no disagreement.
#[test]
fn a_name_and_contents_that_disagree_are_said() {
    let types = TypeRegistry::builtin();
    let clock = clock();
    let photo = file("photo.png");
    let named = types.by_name(&photo, &clock).unwrap();
    let checked = types
        .check(&Probe::new(&photo, Some(b"\xff\xd8\xff\xe0")), &clock)
        .unwrap();
    assert_eq!(checked.stage, TypeStage::Signature);
    let line = types.disagreement(&named, &checked).unwrap();
    assert_eq!(line, "Name says PNG; contents say JPEG");
    assert!(line.chars().count() <= 40);
    let report = file("report.docx");
    let named = types.by_name(&report, &clock).unwrap();
    let checked = types
        .check(
            &Probe::new(&report, Some(b"PK\x03\x04\x14\x00\x00\x00")),
            &clock,
        )
        .unwrap();
    assert_eq!(types.disagreement(&named, &checked), None);
    let raw = file("photo.nef");
    let named = types.by_name(&raw, &clock).unwrap();
    let checked = types
        .check(&Probe::new(&raw, Some(b"MM\x00*\x00\x00")), &clock)
        .unwrap();
    assert_eq!(types.disagreement(&named, &checked), None);
    assert_eq!(
        types.check(&Probe::new(&file("data"), None).skipping(true), &clock),
        Err(Special::Unknown)
    );
}

/// Spike F2 on the shipped table: the same twenty-four files.
#[test]
fn the_shipped_table_passes_the_container_files() {
    crate::containers::check_family(&TypeRegistry::builtin());
}
