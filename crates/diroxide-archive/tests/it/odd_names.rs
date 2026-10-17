//! The odd-names table of research/archives.md section 7, on the index
//! and the fake provider: each kind of name lists (or does not) as the
//! table says, and copying every listed member into the fake provider by
//! the index's own paths writes nothing outside the destination root. The
//! names live in memory and in the fake; nothing here touches a disk.

use std::io::Write;

use diroxide_archive::index::{EXTRA_CARE, EXTRA_NAME, EXTRA_NOTE, ROOT};
use diroxide_archive::{Index, IndexBuilder, Limits, Locator, Member, RecordKind, member_row};
use diroxide_core::Place;
use diroxide_ops::FakeProvider;
use diroxide_vfs::{OpContext, Provider, WriteMode};

fn file(name: &str) -> Member {
    Member {
        name: name.to_owned(),
        kind: Some(RecordKind::File),
        size: 4,
        locator: Locator::Whole,
        ..Member::default()
    }
}

/// The table's names, one or two per row.
fn odd_index() -> Index {
    let lossy = String::from_utf8_lossy(b"caf\xE9.txt").into_owned();
    let mut members = vec![
        file("../x"),
        file("a/../../y"),
        file("./"),
        file("C:"),
        file("/etc/passwd"),
        file(r"C:\win\x.dll"),
        file("nul\u{0}byte.txt"),
        file("bell\u{7}.txt"),
        Member {
            not_unicode: true,
            ..file(&lossy)
        },
        file("dup.txt"),
        file("dup.txt"),
        file("README"),
        file("readme"),
        file("same"),
        file("same/inner.txt"),
        file("inner.zip!x.txt"),
        file(r"win\style\path.txt"),
        file("ok.txt"),
    ];
    let mut builder = IndexBuilder::new(Limits::default());
    for member in members.drain(..) {
        builder.add(member).unwrap();
    }
    builder.finish()
}

fn row(index: &Index, path: &str) -> String {
    let id = index
        .find(path)
        .unwrap_or_else(|| panic!("{path} is listed"));
    member_row(&index.entry(id).unwrap())
}

#[test]
fn each_odd_name_lists_as_the_table_says() {
    let index = odd_index();
    let summary = index.summary();
    // Climbs out, or nothing left: not listed, counted with the reason.
    assert_eq!(summary.refused, 4, "{:?}", summary.refused_lines);
    assert!(
        summary
            .refused_lines
            .contains(&"Skipped ../x: name leaves the archive.".to_owned())
    );
    assert!(
        summary
            .refused_lines
            .contains(&"Skipped C:: name is empty once made safe.".to_owned())
    );
    // Absolute or drive-qualified: the prefix removed, flagged.
    assert!(row(&index, "etc/passwd").contains("leading slash removed"));
    assert!(row(&index, "win/x.dll").contains("drive removed"));
    // NUL or control: listed, flagged, spelled out.
    let nul = index
        .entry(index.find("nul\u{fffd}byte.txt").unwrap())
        .unwrap();
    assert!(
        nul.extra[EXTRA_CARE].contains("contains a control character"),
        "{nul:?}"
    );
    assert_eq!(nul.extra[EXTRA_NAME], "nul[U+0000]byte.txt");
    assert!(row(&index, "bell\u{fffd}.txt").contains("control character"));
    // Not valid text: a safe display name, flagged.
    let lossy = String::from_utf8_lossy(b"caf\xE9.txt").into_owned();
    assert!(row(&index, &lossy).contains("name is not valid text"));
    // A duplicate path: the last copy, flagged with the count.
    assert!(row(&index, "dup.txt").starts_with("dup.txt, appears twice, last copy shown"));
    // A case clash: both listed, both flagged.
    assert!(row(&index, "README").contains("case clash with readme"));
    assert!(row(&index, "readme").contains("case clash with README"));
    // A file and a folder with one name: the folder, flagged.
    let same = index.entry(index.find("same").unwrap()).unwrap();
    assert_eq!(same.kind, diroxide_core::EntryKind::Folder);
    assert!(same.extra[EXTRA_NOTE].contains("file and folder share a name"));
    // An archive marker or backslashes: listed, flagged.
    assert!(row(&index, "inner.zip!x.txt").contains("contains an archive marker"));
    assert!(row(&index, "win/style/path.txt").contains("backslashes read as folders"));
    // And the plain one is plain.
    assert_eq!(row(&index, "ok.txt"), "ok.txt, file, 4 bytes");
}

/// The destination root in the fake provider: a local place in the
/// platform's style, never touched on any disk.
fn fake_root() -> Place {
    Place::parse(if cfg!(windows) {
        r"D:\fake\unpack"
    } else {
        "/fake/unpack"
    })
    .unwrap()
}

/// Copies every listed file into the fake by the index's own paths, the
/// way an unpack runner walks them; a name `join` refuses is renamed with
/// underscores, as the runner's rule will rename it.
fn copy_into(index: &Index, fake: &FakeProvider, root: &Place) -> usize {
    let ctx = OpContext::new();
    let mut renamed = 0;
    let mut stack = vec![(ROOT, root.clone())];
    while let Some((folder, place)) = stack.pop() {
        for &id in index.children(folder) {
            let name = index.name(id);
            let target = match place.join(name) {
                Ok(target) => target,
                Err(_) => {
                    renamed += 1;
                    let plain: String = name
                        .chars()
                        .map(|c| if "!\\/:".contains(c) { '_' } else { c })
                        .collect();
                    place.join(&plain).unwrap()
                }
            };
            let record = index.record(id).unwrap();
            if record.kind == RecordKind::Folder {
                fake.create_dir(&target, &ctx).unwrap();
                stack.push((id, target));
            } else {
                let mut writer = fake.write(&target, WriteMode::CreateNew, &ctx).unwrap();
                writer.write_all(b"data").unwrap();
                writer.commit(false).unwrap();
            }
        }
    }
    renamed
}

#[test]
fn nothing_is_written_outside_the_root() {
    let index = odd_index();
    let fake = FakeProvider::new();
    let root = fake_root();
    fake.add_folder(&root);
    // A local destination takes `inner.zip!x.txt` as a plain name (only an
    // archive place refuses the marker), so nothing needs renaming here.
    let renamed = copy_into(&index, &fake, &root);
    assert_eq!(renamed, 0);
    assert!(fake.exists(&root.join("inner.zip!x.txt").unwrap()));
    let mut written = 0;
    for place in fake.snapshot().keys() {
        let place = Place::parse(place).unwrap();
        assert!(
            place.is_within(&root) || root.is_within(&place),
            "{place} is outside {root}"
        );
        written += usize::from(place.is_within(&root) && place != root);
    }
    assert!(written >= 14, "{written} entries written");
}
