//! Integration tests of the plugin protocol's contract.

use diroxide_plugins::protocol::{self, Incoming, codes};
use diroxide_plugins::{Id, Response};

#[test]
fn a_plugin_answer_reads_back() {
    let answer = Response::failure(Some(Id::Text("a".into())), codes::INVALID_PARAMS, "bad");
    let line = protocol::to_line(&answer).unwrap();
    match protocol::parse_line(&line).unwrap() {
        Incoming::Response(back) => assert_eq!(back, answer),
        other => panic!("{other:?}"),
    }
}
