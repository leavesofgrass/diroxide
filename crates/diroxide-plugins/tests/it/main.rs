//! Integration tests of the plugin protocol's contract.

mod spikes;

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
