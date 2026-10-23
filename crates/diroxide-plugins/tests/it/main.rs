//! Integration tests of the plugin protocol's contract.

// Test helpers may unwrap: a failure is the test's failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(feature = "fake-plugin")]
mod fake_plugin;
mod spikes;
mod spikes_more;

use diroxide_plugins::protocol::{self, Incoming};
use diroxide_plugins::{ErrorKind, Id, Response, RpcError};

#[test]
fn a_plugin_answer_reads_back() {
    let answer = Response::failure(
        Some(Id::Text("a".into())),
        RpcError::new(ErrorKind::AuthRequired, "the password was refused"),
    );
    let line = protocol::to_line(&answer).unwrap();
    match protocol::parse_line(&line).unwrap() {
        Incoming::Response(back) => {
            assert_eq!(back.error.as_ref().unwrap().code, -32019);
            assert_eq!(back, answer);
        }
        other => panic!("{other:?}"),
    }
}
