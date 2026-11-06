//! Images that hold nothing the plugin reads, through the real host: the
//! words say so, and nothing else is read.

use crate::fixture::{Fixture, words};

#[test]
fn an_image_of_nothing_known_says_no_reader_knows_it() {
    let f = Fixture::new();
    let blank = f.image("blank.img", &vec![0u8; 64 * 1024]);
    let error = f.list(&blank).unwrap_err();
    let line = words(&error);
    assert!(line.contains("no reader knows this image"), "{line}");
}

#[test]
fn a_place_that_is_not_in_an_image_is_not_found() {
    let f = Fixture::new();
    let blank = f.image("blank.iso", &vec![0u8; 40 * 1024]);
    let missing = blank.join("nothing").unwrap();
    assert!(f.stat(&missing).is_err());
}

#[test]
fn an_image_that_changes_is_opened_again() {
    let f = Fixture::new();
    let first = crate::make::fat_volume(
        fatfs::FatType::Fat12,
        2880,
        b"ONE        ",
        &[],
        &[("ONE.TXT", b"1")],
    );
    let root = f.image("changing.img", &first);
    assert_eq!(f.names(&root), ["ONE.TXT"]);
    // The plugin compares the file at most every two seconds.
    std::thread::sleep(std::time::Duration::from_millis(2_200));
    let second = crate::make::fat_volume(
        fatfs::FatType::Fat12,
        2000,
        b"TWO        ",
        &[],
        &[("TWO.TXT", b"2")],
    );
    std::fs::write(f.image_path("changing.img"), &second).unwrap();
    let error = f.list(&root).unwrap_err();
    assert_eq!(error, diroxide_vfs::VfsError::Changed);
    assert_eq!(f.names(&root), ["TWO.TXT"]);
}
