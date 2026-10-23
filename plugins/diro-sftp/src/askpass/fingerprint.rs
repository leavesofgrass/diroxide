//! A host key's fingerprint as the owner hears and reads it (R24 section
//! 1.6; answer 84): the digest ssh printed, in hexadecimal, in groups of
//! four, four groups a row, each row saying which part it is. Hexadecimal
//! has no look-alike letters (no 0 and O, no 1, l and I), and a row of 32
//! cells fits the 40-cell display. No spoken form (answer 84).
//!
//! `SHA256:` fingerprints are base64 without padding; a `MD5:` one (the
//! owner's `FingerprintHash md5`) is hexadecimal pairs already.

use std::fmt;

/// The digest's algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Algorithm {
    /// SHA-256, ssh's default: 32 bytes.
    Sha256,
    /// MD5, by the owner's choice: 16 bytes.
    Md5,
}

/// A fingerprint ssh printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// The algorithm.
    pub algorithm: Algorithm,
    /// The digest's bytes.
    pub digest: Vec<u8>,
    /// The text exactly as ssh printed it, for comparing with an
    /// administrator's.
    pub original: String,
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.original)
    }
}

fn base64_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes base64 with or without padding; `None` for anything else.
fn base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        buffer = (buffer << 6) | base64_value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

impl Fingerprint {
    /// Reads `SHA256:<base64>` or `MD5:<hex pairs>`, with or without a
    /// final period; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_end_matches('.');
        if let Some(rest) = text.strip_prefix("SHA256:") {
            let digest = base64(rest).filter(|d| d.len() == 32)?;
            return Some(Self {
                algorithm: Algorithm::Sha256,
                digest,
                original: text.to_owned(),
            });
        }
        if let Some(rest) = text.strip_prefix("MD5:") {
            let digest = rest
                .split(':')
                .map(|pair| {
                    (pair.len() == 2)
                        .then(|| u8::from_str_radix(pair, 16).ok())
                        .flatten()
                })
                .collect::<Option<Vec<u8>>>()
                .filter(|d| d.len() == 16)?;
            return Some(Self {
                algorithm: Algorithm::Md5,
                digest,
                original: text.to_owned(),
            });
        }
        None
    }

    /// The digest in hexadecimal groups of four, four groups a row, each
    /// row naming its part: "Part 1 of 4: 2134 141b 8436 8a94".
    pub fn hex_rows(&self) -> Vec<String> {
        let hex: String = self.digest.iter().map(|b| format!("{b:02x}")).collect();
        let groups: Vec<&str> = hex
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect();
        let rows: Vec<String> = groups.chunks(4).map(|row| row.join(" ")).collect();
        let total = rows.len();
        rows.into_iter()
            .enumerate()
            .map(|(i, row)| format!("Part {} of {total}: {row}", i + 1))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_reads_into_four_rows_of_four_groups() {
        // The digest of the dialog fixture (fixtures/sftp/dialogs).
        let f = Fingerprint::parse("SHA256:ITQUG4Q2ipSm2NXBIVWHWY00pQAnxvt9xPmEMC5l+x0").unwrap();
        assert_eq!(f.algorithm, Algorithm::Sha256);
        assert_eq!(
            f.hex_rows(),
            [
                "Part 1 of 4: 2134 141b 8436 8a94",
                "Part 2 of 4: a6d8 d5c1 2155 8759",
                "Part 3 of 4: 8d34 a500 27c6 fb7d",
                "Part 4 of 4: c4f9 8430 2e65 fb1d",
            ]
        );
        assert!(f.hex_rows().iter().all(|r| r.chars().count() <= 40));
        // The period ssh for Windows ends the line with is not part of it.
        let windows =
            Fingerprint::parse("SHA256:HcIVc9zwwjD8lnpxADs5mQMGnCOBDUkP9dC/28u+FDA.").unwrap();
        assert_eq!(
            windows.original,
            "SHA256:HcIVc9zwwjD8lnpxADs5mQMGnCOBDUkP9dC/28u+FDA"
        );
        assert_eq!(windows.digest.len(), 32);
    }

    #[test]
    fn md5_reads_into_two_rows() {
        let f = Fingerprint::parse("MD5:3b:9e:01:aa:5c:7d:10:ff:00:42:9a:bc:de:f0:12:34").unwrap();
        assert_eq!(
            f.hex_rows(),
            [
                "Part 1 of 2: 3b9e 01aa 5c7d 10ff",
                "Part 2 of 2: 0042 9abc def0 1234"
            ]
        );
    }

    #[test]
    fn anything_else_is_not_a_fingerprint() {
        for bad in [
            "",
            "SHA256:",
            "SHA256:short",
            "SHA256:ITQUG4Q2ipSm2NXBIVWHWY00pQAnxvt9xPmEMC5l+x0AAAA",
            "SHA256:ITQUG4Q2ipSm2NXBIVWHWY00pQAnxvt9xPmEMC5l*x0",
            "MD5:3b:9e",
            "MD5:3b:9e:01:aa:5c:7d:10:ff:00:42:9a:bc:de:f0:12:zz",
            "SHA1:abc",
        ] {
            assert!(Fingerprint::parse(bad).is_none(), "{bad}");
        }
    }
}
