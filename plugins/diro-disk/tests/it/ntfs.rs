//! NTFS through the real host, from the fixture mkntfs and ntfscp made:
//! one row a file (the DOS twin left out), the system's files not rows,
//! a second data stream counted in the details and never a row.

use crate::fixture::{Fixture, at};
use crate::fixtures::fixture;

#[test]
fn an_ntfs_volume_lists_one_row_a_file_and_counts_its_streams() {
    let f = Fixture::new();
    let root = f.image("ntfs.img", &fixture("ntfs.img"));
    let mut names = f.names(&root);
    names.sort();
    assert_eq!(names, ["A Long File Name.txt", "hello.txt"]);
    assert_eq!(
        f.read(&at(&root, &["hello.txt"])).unwrap(),
        b"Hello, world."
    );
    assert_eq!(
        f.read(&at(&root, &["A Long File Name.txt"])).unwrap(),
        b"a long name"
    );
    let hello = f.stat(&at(&root, &["hello.txt"])).unwrap();
    assert_eq!(
        hello.extra.get("disk.about").map(String::as_str),
        Some("Has alternate streams: 1")
    );
    assert_eq!(hello.size, Some(13));
}
