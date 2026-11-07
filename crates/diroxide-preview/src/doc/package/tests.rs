// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Write};

use super::*;
use crate::doc::{DocBuilder, DocKind};

/// A zip of these members, deflated.
pub(crate) fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    for (name, data) in members {
        out.start_file(*name, options).unwrap();
        out.write_all(data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// The deepest element nesting of an XML text.
fn depth(text: &str) -> usize {
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants()
        .map(|n| n.ancestors().count())
        .max()
        .unwrap_or(0)
}

#[test]
fn a_plain_package_passes_unchanged() {
    let bytes = zip_of(&[
        ("word/document.xml", b"<w:document><w:body/></w:document>"),
        ("word/media/image1.png", &[7u8; 4_000]),
    ]);
    let checked = check(bytes.clone(), &ContainerCaps::default(), "DOCX").unwrap();
    assert_eq!(checked.bytes, bytes);
    assert!(!checked.flattened);
    assert_eq!((checked.members, checked.unpacked), (2, 4_000 + 34));
}

#[test]
fn hostile_packages_are_refused_in_words() {
    let caps = ContainerCaps {
        max_members: 3,
        ..ContainerCaps::default()
    };
    let many: Vec<(String, Vec<u8>)> = (0..4)
        .map(|n| (format!("p{n}.xml"), b"<a/>".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = many
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    assert_eq!(
        check(zip_of(&refs), &caps, "DOCX").unwrap_err(),
        DocError::Refused("it holds more than 3 files".to_owned())
    );
    // Eight megabytes of one byte pack over a thousand times.
    let bomb = vec![b' '; 8 * 1024 * 1024];
    let refused = check(
        zip_of(&[("word/document.xml", &bomb)]),
        &ContainerCaps::default(),
        "DOCX",
    )
    .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "a part claims to unpack to over 1,000 times its size"
    );
    let tight = ContainerCaps {
        max_part: 1024,
        ..ContainerCaps::default()
    };
    assert_eq!(
        check(zip_of(&[("content.xml", &[b'a'; 2048])]), &tight, "ODT")
            .unwrap_err()
            .to_string(),
        "a part is too large to read"
    );
    let total = ContainerCaps {
        max_total: 1_000,
        ..ContainerCaps::default()
    };
    assert_eq!(
        check(
            zip_of(&[("a.png", &[1u8; 600]), ("b.png", &[2u8; 600])]),
            &total,
            "EPUB"
        )
        .unwrap_err()
        .to_string(),
        "it unpacks to more than 1.0 KB"
    );
    assert_eq!(
        check(b"PK not really".to_vec(), &ContainerCaps::default(), "DOCX").unwrap_err(),
        DocError::NotValid("DOCX".to_owned())
    );
}

#[test]
fn members_whose_bytes_overlap_are_refused() {
    let mut bytes = zip_of(&[("a.xml", b"<a>one</a>"), ("b.xml", b"<b>two</b>")]);
    // Point the second central entry at the first member's bytes.
    let central: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(at, _)| at)
        .collect();
    let second = central[1];
    bytes[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        check(bytes, &ContainerCaps::default(), "DOCX").unwrap_err(),
        DocError::Refused("its parts overlap".to_owned())
    );
}

#[test]
fn a_deep_part_is_flattened_and_the_package_rewritten() {
    let mut document = String::from(
        "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
    );
    for _ in 0..20_000 {
        document.push_str("<w:sdt><w:sdtContent>");
    }
    document.push_str("<w:p><w:r><w:t>deep</w:t></w:r></w:p>");
    for _ in 0..20_000 {
        document.push_str("</w:sdtContent></w:sdt>");
    }
    document.push_str("</w:body></w:document>");
    let picture = [9u8; 3_000];
    let bytes = zip_of(&[
        ("word/document.xml", document.as_bytes()),
        ("word/media/image1.png", &picture),
    ]);
    let checked = check(bytes, &ContainerCaps::default(), "DOCX").unwrap();
    assert!(checked.flattened);
    let mut zip = zip::ZipArchive::new(Cursor::new(checked.bytes)).unwrap();
    let mut part = String::new();
    zip.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut part)
        .unwrap();
    // The document node, the wrapper's paragraph and run, a leaf and its
    // text: a Word part's leaves go in a paragraph, so carta reads them.
    assert!(depth(&part) <= MAX_NESTING + 5, "{}", depth(&part));
    assert!(part.contains("<w:t>deep</w:t>"));
    assert!(part.contains("<w:p><w:r>"));
    let mut copied = Vec::new();
    zip.by_name("word/media/image1.png")
        .unwrap()
        .read_to_end(&mut copied)
        .unwrap();
    assert_eq!(copied, picture);
}

#[test]
fn the_seekable_path_caps_the_container_and_notes_a_flattened_part() {
    let small = ContainerCaps {
        max_container: 100,
        ..ContainerCaps::default()
    };
    let bytes = zip_of(&[("a.xml", &[b'a'; 500])]);
    let options = ReadOptions::default();
    let mut source = Cursor::new(bytes.clone());
    let refused = read_seekable_with(&mut source, &options, &small, "DOCX", |_, _| {
        Ok(DocView::default())
    })
    .unwrap_err();
    assert_eq!(refused, DocError::TooLarge);
    let mut deep = String::from("<r>");
    deep.push_str(&"<x>".repeat(1_000));
    deep.push_str(&"</x>".repeat(1_000));
    deep.push_str("</r>");
    let mut source = Cursor::new(zip_of(&[("content.xml", deep.as_bytes())]));
    let view = read_seekable_with(
        &mut source,
        &options,
        &ContainerCaps::default(),
        "ODT",
        |bytes, caps| {
            assert_eq!(caps.max_table_rows, 10_000);
            let zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let mut b = DocBuilder::new(DocKind::Document);
            b.paragraph();
            b.text(&format!("{} members", zip.len()));
            b.end();
            Ok(b.finish())
        },
    )
    .unwrap();
    assert_eq!(view.rows()[0].text, "1 members");
    assert_eq!(view.notes(), [NESTING_NOTE]);
}
