//! Integration tests of search's contract.

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
