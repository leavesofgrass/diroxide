//! The checksum engine (R28 3.2 "Checksums as computed metadata"; R29
//! 3.5; R30 commit 49; the owner's answers 45 and 121).
//!
//! - **Algorithms** ([`Algorithm`]): SHA-256 and SHA-512 (`sha2`),
//!   BLAKE3 (the native gate's allowed entry, answer 45), MD5 (labeled
//!   "old, not safe against tampering"), CRC-32 (for `.sfv` lists,
//!   "for damage, not tampering"). SHA-1 lines are read from old lists
//!   and said as not checked: no SHA-1 crate is in the workspace.
//! - **Computing** ([`compute`], [`compute_reader`]): the file read
//!   through its provider in 1 MiB steps, every algorithm asked for in
//!   one pass, the cancel token checked between steps, progress reported
//!   each step ("Checking report.pdf: 40 percent").
//! - **Saying it** ([`Digest`]): in groups of four with the ending,
//!   "SHA-256 of report.pdf, 9f3a 1c52, ending 4e7b"; the whole value as
//!   lines of eight groups, 39 cells each.
//! - **Comparing** ([`Digest::compare`]): a pasted value, spaces and case
//!   ignored, the verdict first: "Match: report.pdf, SHA-256", or "Does
//!   not match: setup.exe, SHA-256" and "The first difference is in group
//!   3".
//! - **Lists** ([`list`]): the GNU, BSD tagged, one-file and `.sfv` forms
//!   read and written; **verifying** ([`verify`]) a list with the verdict
//!   first.
//!
//! A match is never called safe, genuine or authored (R9's refusal
//! list): [`NOT_PROOF`] says what a match shows.

pub mod list;
pub mod verify;

use std::io::Read;

use diroxide_core::Place;
use diroxide_core::names::display_name;
use diroxide_vfs::{CancelToken, OpContext, ProgressUpdate, Provider, VfsError};

use crate::reader::MetaError;

/// The step of a read: 1 MiB.
pub const STEP: usize = 1024 * 1024;

/// What a match shows, and what it does not.
pub const NOT_PROOF: &str = "This shows the files are not damaged. It does not show who made them.";

/// A checksum algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Algorithm {
    /// SHA-256: what publishers post, and the Checksum command's default
    /// (R28 question 3.3).
    Sha256,
    /// SHA-512.
    Sha512,
    /// BLAKE3: fast, for columns and the duplicate finder.
    Blake3,
    /// MD5: old lists only.
    Md5,
    /// CRC-32 (IEEE), the `.sfv` lists' sum.
    Crc32,
    /// SHA-1: read from old lists, not computed here.
    Sha1,
}

impl Algorithm {
    /// Every algorithm, the computed ones first.
    pub const ALL: [Algorithm; 6] = [
        Algorithm::Sha256,
        Algorithm::Sha512,
        Algorithm::Blake3,
        Algorithm::Md5,
        Algorithm::Crc32,
        Algorithm::Sha1,
    ];

    /// The name as said: "SHA-256", "BLAKE3", "CRC-32".
    pub const fn name(self) -> &'static str {
        match self {
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha512 => "SHA-512",
            Algorithm::Blake3 => "BLAKE3",
            Algorithm::Md5 => "MD5",
            Algorithm::Crc32 => "CRC-32",
            Algorithm::Sha1 => "SHA-1",
        }
    }

    /// The tag of a BSD line, as GNU's `--tag` writes it: "SHA256".
    pub const fn tag(self) -> &'static str {
        match self {
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
            Algorithm::Blake3 => "BLAKE3",
            Algorithm::Md5 => "MD5",
            Algorithm::Crc32 => "CRC32",
            Algorithm::Sha1 => "SHA1",
        }
    }

    /// The extension of a one-file list: "sha256", "md5", "sfv".
    pub const fn extension(self) -> &'static str {
        match self {
            Algorithm::Sha256 => "sha256",
            Algorithm::Sha512 => "sha512",
            Algorithm::Blake3 => "blake3",
            Algorithm::Md5 => "md5",
            Algorithm::Crc32 => "sfv",
            Algorithm::Sha1 => "sha1",
        }
    }

    /// The name of a list of many files: "SHA256SUMS", "B3SUMS",
    /// "checksums.sfv".
    pub const fn list_name(self) -> &'static str {
        match self {
            Algorithm::Sha256 => "SHA256SUMS",
            Algorithm::Sha512 => "SHA512SUMS",
            Algorithm::Blake3 => "B3SUMS",
            Algorithm::Md5 => "MD5SUMS",
            Algorithm::Crc32 => "checksums.sfv",
            Algorithm::Sha1 => "SHA1SUMS",
        }
    }

    /// The number of hexadecimal digits of its value.
    pub const fn hex_len(self) -> usize {
        match self {
            Algorithm::Sha256 | Algorithm::Blake3 => 64,
            Algorithm::Sha512 => 128,
            Algorithm::Md5 => 32,
            Algorithm::Crc32 => 8,
            Algorithm::Sha1 => 40,
        }
    }

    /// Whether this program computes it.
    pub const fn computed(self) -> bool {
        !matches!(self, Algorithm::Sha1)
    }

    /// The caution said beside it, or `None`: "old, not safe against
    /// tampering" for MD5 and SHA-1, "for damage, not tampering" for
    /// CRC-32.
    pub const fn caution(self) -> Option<&'static str> {
        match self {
            Algorithm::Md5 | Algorithm::Sha1 => Some("old, not safe against tampering"),
            Algorithm::Crc32 => Some("for damage, not tampering"),
            _ => None,
        }
    }

    /// The algorithm a tag names, any case, with or without the dash:
    /// "SHA256", "sha-256", "BLAKE3", "B3".
    pub fn from_tag(tag: &str) -> Option<Self> {
        let tag: String = tag
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_uppercase();
        match tag.as_str() {
            "SHA256" => Some(Algorithm::Sha256),
            "SHA512" => Some(Algorithm::Sha512),
            "BLAKE3" | "B3" => Some(Algorithm::Blake3),
            "MD5" => Some(Algorithm::Md5),
            "CRC32" | "CRC32B" | "SFV" => Some(Algorithm::Crc32),
            "SHA1" => Some(Algorithm::Sha1),
            _ => None,
        }
    }

    /// The algorithm a list's file name says: `SHA256SUMS`,
    /// `SHA256SUMS.txt`, `report.pdf.sha256`, `B3SUMS`, `files.md5`,
    /// `checksums.sfv`.
    pub fn from_list_name(name: &str) -> Option<Self> {
        let upper = name.to_ascii_uppercase();
        let upper = upper.strip_suffix(".TXT").unwrap_or(&upper);
        if let Some(stem) = upper.strip_suffix("SUMS") {
            let stem = stem.rsplit(['/', '\\', '.']).next().unwrap_or(stem);
            return Self::from_tag(stem);
        }
        let ext = upper.rsplit('.').next()?;
        Self::from_tag(ext)
    }

    /// The algorithms whose values have `digits` hexadecimal digits:
    /// 64 is SHA-256 or BLAKE3.
    pub fn for_length(digits: usize) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|a| a.hex_len() == digits)
            .collect()
    }
}

/// One computed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digest {
    /// Its algorithm.
    pub algorithm: Algorithm,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

impl Digest {
    /// The value in lowercase hexadecimal.
    pub fn hex(&self) -> String {
        self.bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The value in groups of four digits.
    pub fn groups(&self) -> Vec<String> {
        group_hex(&self.hex())
    }

    /// The value as said, short: the name, the first two groups and the
    /// last: "SHA-256: 9f3a 1c52, ending 4e7b".
    pub fn short(&self) -> String {
        format!("{}: {}", self.algorithm.name(), self.short_value())
    }

    /// "9f3a 1c52, ending 4e7b"; a CRC-32 is two groups whole.
    pub fn short_value(&self) -> String {
        let groups = self.groups();
        if groups.len() <= 3 {
            return groups.join(" ");
        }
        format!(
            "{} {}, ending {}",
            groups[0],
            groups[1],
            groups[groups.len() - 1]
        )
    }

    /// The value as said of a file: "SHA-256 of report.pdf, 9f3a 1c52,
    /// ending 4e7b".
    pub fn spoken(&self, file: &str) -> String {
        format!(
            "{} of {}, {}",
            self.algorithm.name(),
            display_name(file),
            self.short_value()
        )
    }

    /// The whole value as lines of eight groups, 39 cells each: a
    /// SHA-256 is two lines, a SHA-512 four.
    pub fn lines(&self) -> Vec<String> {
        self.groups().chunks(8).map(|c| c.join(" ")).collect()
    }

    /// Compares a pasted value, spaces, dashes and case ignored.
    pub fn compare(&self, pasted: &str) -> Comparison {
        let pasted = clean_hex(pasted);
        if pasted.is_empty() || !pasted.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Comparison::NotHex;
        }
        let ours = self.hex();
        if pasted.len() != ours.len() {
            return Comparison::WrongLength {
                got: pasted.len(),
                needs: ours.len(),
            };
        }
        match ours.bytes().zip(pasted.bytes()).position(|(a, b)| a != b) {
            None => Comparison::Match,
            Some(at) => Comparison::Differs { group: at / 4 + 1 },
        }
    }
}

/// A pasted value's digits: spaces, dashes and colons out, lowercase.
fn clean_hex(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | ':'))
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Hexadecimal text in groups of four.
fn group_hex(hex: &str) -> Vec<String> {
    hex.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect()
}

/// What comparing a pasted value found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// Every digit is the same.
    Match,
    /// They differ, first in this group of four (from 1).
    Differs {
        /// The group, counted from 1.
        group: usize,
    },
    /// The pasted value is the wrong length for the algorithm.
    WrongLength {
        /// The digits pasted.
        got: usize,
        /// The digits the algorithm has.
        needs: usize,
    },
    /// The pasted value is not hexadecimal.
    NotHex,
}

impl Comparison {
    /// The verdict, first: "Match: report.pdf, SHA-256", "Does not
    /// match: setup.exe, SHA-256", "Not a SHA-256 value: 63 digits, needs
    /// 64", "Not a checksum: use 0 to 9 and a to f".
    pub fn verdict(&self, file: &str, algorithm: Algorithm) -> String {
        let file = display_name(file);
        match self {
            Comparison::Match => format!("Match: {file}, {}", algorithm.name()),
            Comparison::Differs { .. } => {
                format!("Does not match: {file}, {}", algorithm.name())
            }
            Comparison::WrongLength { got, needs } => format!(
                "Not a {} value: {got} digits, needs {needs}",
                algorithm.name()
            ),
            Comparison::NotHex => "Not a checksum: use 0 to 9 and a to f".to_owned(),
        }
    }

    /// The line after a difference: "The first difference is in group
    /// 3", which helps a person who typed one digit wrong.
    pub fn detail(&self) -> Option<String> {
        match self {
            Comparison::Differs { group } => {
                Some(format!("The first difference is in group {group}"))
            }
            _ => None,
        }
    }
}

/// Which of several values a pasted value matches: "Matches BLAKE3, not
/// SHA-256" when 64 digits matched the second of the two, for a value
/// whose algorithm nothing named.
pub fn which_matches(digests: &[Digest], pasted: &str) -> Option<String> {
    let matched = digests
        .iter()
        .position(|d| d.compare(pasted) == Comparison::Match)?;
    let others: Vec<&str> = digests
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != matched)
        .map(|(_, d)| d.algorithm.name())
        .collect();
    let name = digests[matched].algorithm.name();
    Some(if others.is_empty() {
        format!("Matches {name}")
    } else {
        format!("Matches {name}, not {}", others.join(" or "))
    })
}

/// The progress words of one file: "Checking report.pdf: 40 percent".
pub fn progress_words(file: &str, done: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!(
            "Checking {}: {} percent",
            display_name(file),
            (done.saturating_mul(100) / total).min(100)
        ),
        _ => format!("Checking {}", display_name(file)),
    }
}

/// The hasher of one algorithm.
enum Hasher {
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
    Md5(md5::Md5),
    Crc32(crc32fast::Hasher),
}

impl Hasher {
    fn new(algorithm: Algorithm) -> Option<Self> {
        use sha2::Digest as _;
        Some(match algorithm {
            Algorithm::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            Algorithm::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
            Algorithm::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
            Algorithm::Md5 => Hasher::Md5(md5::Md5::new()),
            Algorithm::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
            Algorithm::Sha1 => return None,
        })
    }

    fn update(&mut self, bytes: &[u8]) {
        use sha2::Digest as _;
        match self {
            Hasher::Sha256(h) => h.update(bytes),
            Hasher::Sha512(h) => h.update(bytes),
            Hasher::Blake3(h) => {
                h.update(bytes);
            }
            Hasher::Md5(h) => h.update(bytes),
            Hasher::Crc32(h) => h.update(bytes),
        }
    }

    fn finish(self) -> Digest {
        use sha2::Digest as _;
        match self {
            Hasher::Sha256(h) => Digest {
                algorithm: Algorithm::Sha256,
                bytes: h.finalize().to_vec(),
            },
            Hasher::Sha512(h) => Digest {
                algorithm: Algorithm::Sha512,
                bytes: h.finalize().to_vec(),
            },
            Hasher::Blake3(h) => Digest {
                algorithm: Algorithm::Blake3,
                bytes: h.finalize().as_bytes().to_vec(),
            },
            Hasher::Md5(h) => Digest {
                algorithm: Algorithm::Md5,
                bytes: h.finalize().to_vec(),
            },
            Hasher::Crc32(h) => Digest {
                algorithm: Algorithm::Crc32,
                bytes: h.finalize().to_be_bytes().to_vec(),
            },
        }
    }
}

/// Computes every algorithm of `algorithms` over one reader in one pass,
/// in [`STEP`] reads, the token checked between steps and `on_step`
/// told the bytes done after each. SHA-1, which is not computed, is
/// refused in words.
pub fn compute_reader(
    reader: &mut dyn Read,
    algorithms: &[Algorithm],
    cancel: &CancelToken,
    on_step: &mut dyn FnMut(u64),
) -> Result<Vec<Digest>, MetaError> {
    let mut hashers = algorithms
        .iter()
        .map(|a| {
            Hasher::new(*a)
                .ok_or_else(|| MetaError::unreadable(format!("{} is not computed here", a.name())))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut buffer = vec![0u8; STEP];
    let mut done = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(MetaError::Place(VfsError::Cancelled));
        }
        let mut filled = 0;
        while filled < STEP {
            match reader.read(&mut buffer[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    if cancel.is_cancelled() {
                        return Err(MetaError::Place(VfsError::Cancelled));
                    }
                }
                Err(e) => return Err(MetaError::Place(e.into())),
            }
        }
        if filled == 0 {
            break;
        }
        for hasher in &mut hashers {
            hasher.update(&buffer[..filled]);
        }
        done += filled as u64;
        on_step(done);
        if filled < STEP {
            break;
        }
    }
    Ok(hashers.into_iter().map(Hasher::finish).collect())
}

/// Computes `algorithms` over the file at `place`, read through its
/// provider in 1 MiB steps; progress goes to the context's sink as bytes
/// done of the file's size, with the file's name as the current item.
pub fn compute(
    provider: &dyn Provider,
    place: &Place,
    algorithms: &[Algorithm],
    ctx: &OpContext,
) -> Result<Vec<Digest>, MetaError> {
    ctx.check()?;
    let total = provider.stat(place, ctx).ok().and_then(|entry| entry.size);
    let name = place.name().map(str::to_owned);
    let mut reader = provider.read(place, ctx)?;
    compute_reader(reader.as_mut(), algorithms, &ctx.cancel, &mut |done| {
        ctx.progress.report(ProgressUpdate {
            bytes_done: done,
            bytes_total: total,
            current: name.clone(),
            ..ProgressUpdate::default()
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(algorithm: Algorithm, bytes: &[u8]) -> Digest {
        compute_reader(
            &mut std::io::Cursor::new(bytes.to_vec()),
            &[algorithm],
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap()
        .remove(0)
    }

    #[test]
    fn the_known_values() {
        // The values of "abc" every implementation agrees on.
        assert_eq!(
            digest(Algorithm::Sha256, b"abc").hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(Algorithm::Md5, b"abc").hex(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            digest(Algorithm::Blake3, b"abc").hex(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        assert!(
            digest(Algorithm::Sha512, b"abc")
                .hex()
                .starts_with("ddaf35a193617aba")
        );
        // Spike M5: cksum -a crc32b gave 2673897196 for "alpha\n".
        assert_eq!(digest(Algorithm::Crc32, b"alpha\n").hex(), "9f606eec");
        assert_eq!(u32::from_str_radix("9f606eec", 16).unwrap(), 2_673_897_196);
    }

    #[test]
    fn one_pass_for_several_and_progress_by_the_mebibyte() {
        let bytes = vec![7u8; STEP * 2 + 10];
        let mut steps = Vec::new();
        let got = compute_reader(
            &mut std::io::Cursor::new(bytes),
            &[Algorithm::Sha256, Algorithm::Blake3],
            &CancelToken::new(),
            &mut |done| steps.push(done),
        )
        .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(steps, [STEP as u64, 2 * STEP as u64, 2 * STEP as u64 + 10]);
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            compute_reader(
                &mut std::io::Cursor::new(vec![1u8; 4]),
                &[Algorithm::Md5],
                &cancel,
                &mut |_| {}
            ),
            Err(MetaError::Place(VfsError::Cancelled))
        );
        assert_eq!(
            compute_reader(
                &mut std::io::Cursor::new(vec![1u8; 4]),
                &[Algorithm::Sha1],
                &CancelToken::new(),
                &mut |_| {}
            ),
            Err(MetaError::unreadable("SHA-1 is not computed here"))
        );
    }

    #[test]
    fn said_in_groups_of_four_with_the_ending() {
        let sha = digest(Algorithm::Sha256, b"abc");
        assert_eq!(sha.short(), "SHA-256: ba78 16bf, ending 15ad");
        assert_eq!(
            sha.spoken("report.pdf"),
            "SHA-256 of report.pdf, ba78 16bf, ending 15ad"
        );
        let lines = sha.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "ba78 16bf 8f01 cfea 4141 40de 5dae 2223");
        for line in &lines {
            assert_eq!(line.chars().count(), 39);
        }
        assert_eq!(digest(Algorithm::Sha512, b"abc").lines().len(), 4);
        assert_eq!(
            digest(Algorithm::Crc32, b"alpha\n").short(),
            "CRC-32: 9f60 6eec"
        );
        assert!(sha.short().chars().count() <= 40);
        assert!(sha.spoken("report.pdf").chars().count() <= 48);
    }

    #[test]
    fn a_pasted_value_gets_the_verdict_first() {
        let sha = digest(Algorithm::Sha256, b"abc");
        let pasted = "BA7816BF 8F01CFEA 414140DE 5DAE2223 B00361A3 96177A9C B410FF61 F20015AD";
        assert_eq!(sha.compare(pasted), Comparison::Match);
        assert_eq!(
            sha.compare(pasted).verdict("report.pdf", Algorithm::Sha256),
            "Match: report.pdf, SHA-256"
        );
        let wrong = pasted.replace("414140DE", "414141DE");
        let verdict = sha.compare(&wrong);
        assert_eq!(verdict, Comparison::Differs { group: 6 });
        assert_eq!(
            verdict.verdict("setup.exe", Algorithm::Sha256),
            "Does not match: setup.exe, SHA-256"
        );
        assert_eq!(
            verdict.detail().as_deref(),
            Some("The first difference is in group 6")
        );
        assert_eq!(
            sha.compare(&pasted[..pasted.len() - 1])
                .verdict("a", Algorithm::Sha256),
            "Not a SHA-256 value: 63 digits, needs 64"
        );
        assert_eq!(
            sha.compare("hello").verdict("a", Algorithm::Sha256),
            "Not a checksum: use 0 to 9 and a to f"
        );
        for line in [
            "Match: report.pdf, SHA-256",
            "Does not match: setup.exe, SHA-256",
            "The first difference is in group 3",
            "Not a SHA-256 value: 63 digits, needs 64",
            "Not a checksum: use 0 to 9 and a to f",
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
        let blake = digest(Algorithm::Blake3, b"abc");
        assert_eq!(
            which_matches(&[sha.clone(), blake.clone()], &blake.hex()).as_deref(),
            Some("Matches BLAKE3, not SHA-256")
        );
        assert_eq!(which_matches(&[sha, blake], "00"), None);
    }

    #[test]
    fn names_tags_and_lengths() {
        assert_eq!(Algorithm::from_tag("sha-256"), Some(Algorithm::Sha256));
        assert_eq!(Algorithm::from_tag("B3"), Some(Algorithm::Blake3));
        assert_eq!(Algorithm::from_tag("CRC"), None);
        for (name, algorithm) in [
            ("SHA256SUMS", Algorithm::Sha256),
            ("SHA512SUMS.txt", Algorithm::Sha512),
            ("MD5SUMS", Algorithm::Md5),
            ("B3SUMS", Algorithm::Blake3),
            ("report.pdf.sha256", Algorithm::Sha256),
            ("report.pdf.blake3", Algorithm::Blake3),
            ("files.md5", Algorithm::Md5),
            ("checksums.sfv", Algorithm::Crc32),
        ] {
            assert_eq!(Algorithm::from_list_name(name), Some(algorithm), "{name}");
        }
        assert_eq!(Algorithm::from_list_name("notes.txt"), None);
        assert_eq!(
            Algorithm::for_length(64),
            [Algorithm::Sha256, Algorithm::Blake3]
        );
        assert_eq!(
            Algorithm::Md5.caution(),
            Some("old, not safe against tampering")
        );
        assert_eq!(
            Algorithm::Crc32.caution(),
            Some("for damage, not tampering")
        );
        assert_eq!(
            progress_words("report.pdf", 40, Some(100)),
            "Checking report.pdf: 40 percent"
        );
        assert_eq!(progress_words("report.pdf", 4, None), "Checking report.pdf");
        assert_eq!(NOT_PROOF.split(". ").count(), 2);
    }
}
