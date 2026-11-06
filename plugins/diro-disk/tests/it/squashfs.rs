//! squashfs through the real host, from the fixtures mksquashfs made:
//! gzip read here, xz handed to 7-Zip in words.

use diroxide_core::EntryKind;

use crate::fixture::{Fixture, at, words};
use crate::fixtures::fixture;

#[test]
fn a_gzip_squashfs_lists_reads_and_shows_its_link() {
    let f = Fixture::new();
    let root = f.image("root.squashfs", &fixture("gzip.sqfs"));
    let mut names = f.names(&root);
    names.sort();
    assert_eq!(
        names,
        ["A Long File Name.txt", "deep", "docs", "hello.txt", "link"]
    );
    assert_eq!(
        f.read(&at(&root, &["hello.txt"])).unwrap(),
        b"Hello, world."
    );
    let deep = at(
        &root,
        &[
            "deep", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "deep.txt",
        ],
    );
    assert_eq!(f.read(&deep).unwrap(), b"deep down");
    let link = f.stat(&at(&root, &["link"])).unwrap();
    assert_eq!(link.kind, EntryKind::Link);
    assert_eq!(link.link.map(|l| l.target), Some("hello.txt".to_owned()));
}

#[test]
fn an_xz_squashfs_says_it_needs_7zip() {
    let f = Fixture::with_tools(diroxide_plugins::tools::ToolBox::with_path(None));
    let root = f.image("xz.squashfs", &fixture("xz.sqfs"));
    let line = words(&f.list(&root).unwrap_err());
    assert!(
        line.contains("it needs 7-Zip to read squashfs with xz"),
        "{line}"
    );
}
