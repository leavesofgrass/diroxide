//! Integration tests of search's contract. Tests that write use a folder
//! under the target directory (`CARGO_TARGET_TMPDIR`); the fake place
//! never touches a disk.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod content;
mod engine;
mod fake;

use diroxide_core::Place;
use diroxide_search::dirsize;
use diroxide_vfs::{OpContext, Registry};

#[test]
fn dirsize_counts_this_crate() {
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let size = dirsize(&here, &Registry::with_local(), &OpContext::new()).unwrap();
    assert!(size.files >= 3 && size.bytes > 0);
    assert!(size.describe().contains(" in "));
}
