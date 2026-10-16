//! Integration tests of the SFTP provider's contract.

use std::sync::Arc;

use diroxide_core::Place;
use diro_sftp::{HostKeyStatus, SftpProvider};
use diroxide_vfs::Registry;

#[test]
fn sftp_places_resolve_to_the_provider() {
    let mut registry = Registry::with_local();
    registry.register("sftp", Arc::new(SftpProvider::new()));
    let place = Place::parse("sftp://example.org/home").unwrap();
    assert_eq!(registry.resolve(&place).unwrap().scheme(), "sftp");
    assert!(
        HostKeyStatus::Changed
            .describe()
            .starts_with("Host key changed")
    );
}
