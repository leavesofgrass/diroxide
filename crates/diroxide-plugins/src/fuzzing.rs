//! The bodies of the plugin fuzz targets, as plain functions, so the
//! libFuzzer entry points in `fuzz/fuzz_targets/` and the ordinary tests
//! call the same code: the seeds under `fixtures/plugins/seeds/` and the
//! seeds built here replay on every system in normal CI, with no fuzzer.
//! Each body panics only when a property fails; bad input is an error,
//! never a panic.

use crate::framing::{Frame, LineReader, STRAY_PREVIEW_CHARS, classify};
use crate::protocol::{Incoming, parse_line, to_line};

/// `plugin_framing`: any byte stream through the bounded reader, the
/// sorting and the message parser. The properties: no line holds more
/// than the limit; a stray or invalid preview is cut; when no line is over
/// the limit the lines join back into the input; and every message that
/// parses writes back to a line that parses to the same message.
pub fn framing(data: &[u8]) {
    // A small limit, chosen by the first byte, so the fuzzer reaches the
    // over-limit path as often as the others.
    let limit = 16 + usize::from(data.first().copied().unwrap_or(0));
    let mut reader = LineReader::with_limit(data, limit);
    let mut joined = Vec::with_capacity(data.len());
    let mut over = false;
    while let Ok(Some(line)) = reader.read_raw() {
        assert!(
            line.bytes.len() <= limit,
            "a line holds more than the limit"
        );
        over |= line.over_limit();
        joined.extend_from_slice(&line.bytes);
        if line.terminated {
            joined.push(b'\n');
        }
        match classify(&line) {
            Frame::Message(text) => round_trip(&text),
            Frame::Stray(text) | Frame::NotUtf8(text) => {
                assert!(text.chars().count() <= STRAY_PREVIEW_CHARS);
            }
            Frame::TooLong { length, .. } => assert!(length > limit as u64),
            Frame::Blank => {}
        }
    }
    if !over {
        assert_eq!(joined, data, "the lines do not join back into the input");
    }
}

fn round_trip(text: &str) {
    let Ok(message) = parse_line(text) else {
        return;
    };
    let line = match &message {
        Incoming::Request(m) => to_line(m),
        Incoming::Notification(m) => to_line(m),
        Incoming::Response(m) => to_line(m),
    };
    let Ok(line) = line else {
        return;
    };
    assert!(!line.contains('\n'), "a written message holds a line break");
    match parse_line(&line) {
        Ok(back) => assert_eq!(back, message, "a message changed on the way back"),
        Err(e) => panic!("a written message does not parse: {}", e.message),
    }
}

/// Seeds for `plugin_framing` that cannot live in a text file: line
/// endings, bytes that are not UTF-8, a byte-order mark and long lines.
pub fn framing_seeds() -> Vec<(&'static str, Vec<u8>)> {
    let mut long = vec![b'{'];
    long.extend(std::iter::repeat_n(b'a', 600));
    long.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"method\":\"log\"}\n");
    vec![
        (
            "crlf",
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":null}\r\nstray\r\n".to_vec(),
        ),
        (
            "not-utf8",
            vec![
                b'c', b'a', b'f', 0xE9, b'\n', b'{', b'"', 0xFF, b'"', b'}', b'\n',
            ],
        ),
        (
            "bom",
            [
                &[0xEF, 0xBB, 0xBF][..],
                b"{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}\n",
            ]
            .concat(),
        ),
        ("long", long),
        (
            "partial",
            b"{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"result\":true}".to_vec(),
        ),
        ("empty", Vec::new()),
        (
            "batch",
            b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}]\n".to_vec(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn seed_folder(target: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/plugins/seeds")
            .join(target)
    }

    #[test]
    fn the_framing_seeds_replay() {
        for (name, data) in framing_seeds() {
            framing(&data);
            assert!(!name.is_empty());
        }
        let folder = seed_folder("plugin_framing");
        let mut count = 0;
        for item in fs::read_dir(&folder).unwrap() {
            let path = item.unwrap().path();
            framing(&fs::read(&path).unwrap());
            count += 1;
        }
        assert!(count > 0, "no seeds in {}", folder.display());
    }
}
