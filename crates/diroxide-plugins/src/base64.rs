//! Base64 in the standard alphabet with padding (RFC 4648, section 4):
//! file contents and raw names on the wire (`data`, `name_raw`).

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The text of some bytes.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((v >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    }))
}

/// The bytes of a text; `None` when it is not base64 (wrong length, a
/// character outside the alphabet, padding in the middle).
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let quads = bytes.len() / 4;
    for (index, quad) in bytes.chunks(4).enumerate() {
        let padding = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != quads) {
            return None;
        }
        let mut v = 0u32;
        for (i, &c) in quad.iter().enumerate() {
            let digit = if i >= 4 - padding { 0 } else { value(c)? };
            v = (v << 6) | digit;
        }
        let decoded = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&decoded[..3 - padding]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_rfc_examples() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn bad_text_is_refused() {
        for bad in ["Zg=", "Zg==Zg==", "Z===", "Zm9v!", "Zm 9", "Zm9v\n"] {
            assert!(decode(bad).is_none(), "{bad}");
        }
    }

    proptest! {
        #[test]
        fn any_bytes_round_trip(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
            prop_assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
        }
    }
}
