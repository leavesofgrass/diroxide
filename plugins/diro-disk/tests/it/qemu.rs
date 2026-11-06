//! `qemu-img` through the real host: a QCOW2 whose data is in its backing
//! file opens as the raw disk qemu-img converts it to, in the spool, where
//! qemu-img is (the development image); elsewhere it says why in words.

use crate::fixture::{Fixture, at, words};
use crate::fixtures::fixture;

fn has_qemu_img() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let name = if cfg!(windows) {
        "qemu-img.exe"
    } else {
        "qemu-img"
    };
    std::env::split_paths(&path).any(|folder| folder.join(name).is_file())
}

#[test]
fn a_qcow2_with_a_backing_file_opens_through_qemu_img_or_says_why() {
    let f = Fixture::new();
    std::fs::write(f.image_path("fat.qcow2"), fixture("fat.qcow2")).unwrap();
    let root = f.image("overlay.qcow2", &fixture("overlay.qcow2"));
    if has_qemu_img() {
        let mut names = f.names(&root);
        names.sort();
        assert_eq!(names, ["docs", "hello.txt"]);
        assert_eq!(
            f.read(&at(&root, &["hello.txt"])).unwrap(),
            b"Hello, world."
        );
    } else {
        eprintln!("qemu-img is not on this system: the words are checked instead");
        let line = words(&f.list(&root).unwrap_err());
        assert!(line.contains("a QCOW2 with a backing file"), "{line}");
    }
}
