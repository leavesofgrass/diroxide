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
