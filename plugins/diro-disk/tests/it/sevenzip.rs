//! The 7-Zip fallback through the real host and the 7z the system has
//! (skipped, with the reason printed, where it has none): an image no
//! built-in reader opens, a partition no built-in reader opens, and a
//! member too large for one answer.

use crate::fixture::{Fixture, at, seven_zip_tools, words};
use crate::fixtures::fixture;
use crate::make::{gpt_with, guid};

#[test]
fn an_xz_squashfs_is_read_with_7zip() {
    let Some(tools) = seven_zip_tools() else {
        return;
    };
    let f = Fixture::with_tools(tools);
    let root = f.image("xz.squashfs", &fixture("xz.sqfs"));
    let mut names = f.names(&root);
    names.sort();
    assert!(names.contains(&"hello.txt".to_owned()), "{names:?}");
    assert!(names.contains(&"docs".to_owned()), "{names:?}");
    assert_eq!(
        f.read(&at(&root, &["hello.txt"])).unwrap(),
        b"Hello, world."
    );
    assert_eq!(
        f.read(&at(&root, &["docs", "inner.txt"])).unwrap(),
        b"inside"
    );
}

#[test]
fn a_partition_no_reader_opens_is_handed_to_7zip() {
    let Some(tools) = seven_zip_tools() else {
        return;
    };
    let f = Fixture::with_tools(tools);
    let disk = gpt_with(4 * 1024 * 1024, &[(guid::LINUX, "", &fixture("xz.sqfs"))]);
    let root = f.image("disk.img", &disk);
    let part = at(&root, &["partition-1"]);
    let names = f.names(&part);
    assert!(names.contains(&"hello.txt".to_owned()), "{names:?}");
    assert_eq!(
        f.read(&at(&part, &["hello.txt"])).unwrap(),
        b"Hello, world."
    );
}

#[test]
fn a_member_too_large_for_one_answer_is_unpacked_or_refused_in_words() {
    let Some(tools) = seven_zip_tools() else {
        return;
    };
    let f = Fixture::with_tools(tools);
    let root = f.image("big.squashfs", &fixture("big-xz.sqfs"));
    match f.read(&at(&root, &["big.txt"])) {
        Ok(bytes) => {
            assert_eq!(bytes.len(), 7_340_032);
            assert!(bytes.starts_with(b"Seven megabytes of the same line"));
        }
        Err(error) => {
            // A 7-Zip older than 25.01 never unpacks (R7's traversal fix).
            let line = words(&error);
            assert!(line.contains("is too old: needs 25.01"), "{line}");
        }
    }
}

#[test]
fn without_7zip_an_xz_squashfs_says_it_needs_7zip() {
    let f = Fixture::with_tools(diroxide_plugins::tools::ToolBox::with_path(None));
    let root = f.image("xz.squashfs", &fixture("xz.sqfs"));
    let line = words(&f.list(&root).unwrap_err());
    assert!(
        line.contains("it needs 7-Zip to read squashfs with xz"),
        "{line}"
    );
}
