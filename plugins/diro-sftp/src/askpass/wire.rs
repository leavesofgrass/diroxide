//! The frames between the helper and the plugin on the local endpoint:
//! a four-byte length, a kind byte, then parts, each with its own
//! four-byte length. Lengths are checked before anything is allocated.

use std::io::{self, Read, Write};

/// The largest frame either side reads, in bytes.
pub const MAX_FRAME: usize = 64 * 1024;

/// The helper's hello: its nonce.
pub const HELLO: u8 = 1;
/// The plugin's challenge: its nonce and its proof.
pub const CHALLENGE: u8 = 2;
/// The helper's question: the prompt's kind, the prompt, the proof.
pub const ASK: u8 = 3;
/// The plugin's answer: a status byte and, for a secret, its text.
pub const ANSWER: u8 = 4;

/// The answer's status: write the text and a line break, exit 0.
pub const STATUS_TEXT: u8 = 0;
/// The answer's status: exit 0 and write nothing (a confirmation).
pub const STATUS_CONFIRM: u8 = 1;
/// The answer's status: write nothing and exit non-zero.
pub const STATUS_REFUSE: u8 = 2;

/// Writes one frame.
pub fn write_frame(out: &mut dyn Write, kind: u8, parts: &[&[u8]]) -> io::Result<()> {
    let mut body = vec![kind];
    for part in parts {
        let len = u32::try_from(part.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "part too long"))?;
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(part);
    }
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too long",
        ));
    }
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    out.write_all(&len.to_be_bytes())?;
    out.write_all(&body)?;
    out.flush()
}

/// Reads one frame: its kind and its parts.
pub fn read_frame(input: &mut dyn Read) -> io::Result<(u8, Vec<Vec<u8>>)> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
    let mut len = [0u8; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(bad("a frame of a wrong length"));
    }
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    let kind = body[0];
    let mut parts = Vec::new();
    let mut at = 1;
    while at < body.len() {
        let Some(head) = body.get(at..at + 4) else {
            return Err(bad("a part's length is cut"));
        };
        let part_len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        at += 4;
        let Some(part) = body.get(at..at.saturating_add(part_len)) else {
            return Err(bad("a part is longer than its frame"));
        };
        parts.push(part.to_vec());
        at += part_len;
    }
    Ok((kind, parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_bad_ones_are_refused() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, ASK, &[b"", b"ada's password: ", &[0xff, 0]]).unwrap();
        let (kind, parts) = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(kind, ASK);
        assert_eq!(
            parts,
            [b"".to_vec(), b"ada's password: ".to_vec(), vec![0xff, 0]]
        );
        for bad in [
            vec![0, 0, 0, 0],
            vec![0xff, 0xff, 0xff, 0xff],
            vec![0, 0, 0, 3, ASK, 0, 0],
            vec![0, 0, 0, 6, ASK, 0, 0, 0, 9, 1],
        ] {
            assert!(read_frame(&mut bad.as_slice()).is_err(), "{bad:?}");
        }
        assert!(write_frame(&mut Vec::new(), ASK, &[&vec![0u8; MAX_FRAME]]).is_err());
    }
}
