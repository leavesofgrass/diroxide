//! ext through the real host: a hand-written ext2 volume, on every
//! system; real ext4 images are the container's (docker/disk).

use diroxide_core::EntryKind;

use crate::fixture::{Fixture, at};
use crate::make::{ext2, gpt_with, guid, opened};

#[test]
fn an_ext2_volume_lists_reads_and_shows_a_link_as_text() {
    let f = Fixture::new();
    let root = f.image("home.img", &ext2());
    let mut names = f.names(&root);
    names.sort();
    assert_eq!(names, ["docs", "hello.txt", "link"]);
    assert_eq!(
        f.read(&at(&root, &["hello.txt"])).unwrap(),
        b"Hello, world."
    );
    assert_eq!(
        f.read(&at(&root, &["docs", "inner.txt"])).unwrap(),
        b"inside"
    );
    let link = f.stat(&at(&root, &["link"])).unwrap();
    assert_eq!(link.kind, EntryKind::Link);
    assert_eq!(link.link.map(|l| l.target), Some("hello.txt".to_owned()));
    let inner = f.stat(&at(&root, &["docs", "inner.txt"])).unwrap();
    assert_eq!(
        inner.attributes,
        Some(diroxide_core::Attributes::Unix(0o600))
    );
}

#[test]
fn an_ext2_partition_says_its_label_and_file_system() {
    let f = Fixture::new();
    let disk = gpt_with(4 * 1024 * 1024, &[(guid::LINUX, "", &ext2())]);
    let root = f.image("disk.img", &disk);
    let entries = f.list(&root).unwrap();
    assert_eq!(
        entries[0].extra.get("disk.about").map(String::as_str),
        Some("Partition 1, home, ext2, 1.0 MB")
    );
    let (image, _) = opened(ext2(), "home.img");
    assert_eq!(image.layout_words(), "ext2");
}

#[test]
fn an_ext4_volume_reads_a_file_stored_by_an_extent() {
    let f = Fixture::new();
    let root = f.image("root.img", &crate::make::ext4());
    assert_eq!(
        f.read(&at(&root, &["hello.txt"])).unwrap(),
        b"Hello, world."
    );
    let (image, _) = opened(crate::make::ext4(), "root.img");
    assert_eq!(image.layout_words(), "ext4");
}
