//! One pinned file: its name, size and SHA-256, the computer it is for,
//! and where it may be fetched. The pins are the contract, not the
//! addresses: a file is the same file whether it came from the public
//! release, the owner's own source, or a zip on a memory stick.
//!
//! Carried from textweaver's `pin.rs` (the owner's own code, relicensed
//! under answer 3), with SHA-256 only and a target per file.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

/// The target that matches every computer: a data file, a script.
pub const ANY_TARGET: &str = "any";

/// The target a macOS universal build carries; it matches both Apple
/// architectures.
pub const UNIVERSAL_APPLE: &str = "universal-apple-darwin";

/// The longest plain name, in bytes.
pub const MAX_NAME_BYTES: usize = 120;

/// One file of a component, pinned by size and SHA-256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePin {
    /// Its name in the component's folder: a plain name
    /// ([`is_plain_name`]).
    pub name: String,
    /// Its size in bytes, above zero.
    pub size: u64,
    /// Its SHA-256, 64 lowercase hex digits.
    pub sha256: String,
    /// The computer it is for: a Rust target triple, [`ANY_TARGET`], or
    /// [`UNIVERSAL_APPLE`].
    pub target: String,
    /// Its public address, an absolute `https` address. `None`: the
    /// address is derived from the project's repository, the release tag
    /// `<id>-<version>` and the name, so the list never names an account.
    pub url: Option<String>,
    /// For a plugin or a helper: the program's path inside the
    /// component's folder, plain parts joined by `/`. Only this file gets
    /// the executable bit, on Unix.
    pub exec: Option<String>,
    /// Leading folder levels an unpack drops from each member's path.
    pub strip: u32,
}

impl FilePin {
    /// A file for any computer, with no address of its own, no program
    /// and nothing stripped: what tests and data packs use.
    pub fn new(name: impl Into<String>, size: u64, sha256: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            size,
            sha256: sha256.into(),
            target: ANY_TARGET.to_owned(),
            url: None,
            exec: None,
            strip: 0,
        }
    }

    /// The pin of some bytes, named `name`.
    pub fn of_bytes(name: impl Into<String>, bytes: &[u8]) -> Self {
        Self::new(name, bytes.len() as u64, sha256_hex(bytes))
    }

    /// Whether this file is for the computer `target` names.
    pub fn applies_to(&self, target: &str) -> bool {
        self.target == ANY_TARGET
            || self.target == target
            || (self.target == UNIVERSAL_APPLE && target.ends_with("-apple-darwin"))
    }

    /// Whether `bytes` are this file.
    pub fn matches_bytes(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.size && sha256_hex(bytes) == self.sha256
    }

    /// Whether the file at `path` is this file: a regular file (a link is
    /// not followed) of the pinned size, then of the pinned hash.
    pub fn matches_file(&self, path: &Path) -> bool {
        self.size_matches(path) && hash_file(path).is_ok_and(|hash| hash == self.sha256)
    }

    /// Whether the file at `path` is a regular file of the pinned size:
    /// the quick check.
    pub fn size_matches(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == self.size)
    }

    /// Whether the hash is 64 lowercase hex digits.
    pub fn hash_is_well_formed(&self) -> bool {
        is_sha256(&self.sha256)
    }
}

/// Whether `text` is a SHA-256 written as 64 lowercase hex digits.
pub fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The SHA-256 of the file at `path`, lowercase hex, read in pieces.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    hash_reader(&mut file)
}

/// The SHA-256 of everything `reader` gives, lowercase hex.
pub fn hash_reader(reader: &mut dyn Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        text.push(char::from(DIGITS[usize::from(b >> 4)]));
        text.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    text
}

/// Whether `name` is a plain file or folder name: letters, digits, `-`,
/// `_` and `.` only; not starting with `.` or `-`; not ending with `.`;
/// no `..`; at most [`MAX_NAME_BYTES`] bytes; not a Windows device name
/// (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`, with
/// any extension). Every name from a list or an archive passes this before
/// anything is written under it.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !name.starts_with(['.', '-'])
        && !name.ends_with('.')
        && !name.contains("..")
        && !is_device_name(name)
}

/// Whether `name` (or its part before the first dot) is a Windows device
/// name, whatever its case.
pub fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit()
        && stem.as_bytes()[3] != b'0')
}

/// Whether `path` is plain names joined by single `/`: what a folder and
/// an `exec` path are written as.
pub fn is_plain_path(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(is_plain_name)
}

/// The target triple of this build, for choosing a component's files:
/// `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`,
/// `x86_64-unknown-linux-gnu`. A computer the list has no name for gets
/// `unknown`, which only [`ANY_TARGET`] files match.
pub const fn host_target() -> &'static str {
    if cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc"
    )) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(
        target_os = "windows",
        target_arch = "aarch64",
        target_env = "msvc"
    )) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        "x86_64-pc-windows-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "musl"
    )) {
        "x86_64-unknown-linux-musl"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "musl"
    )) {
        "aarch64-unknown-linux-musl"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_sha256_in_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"abc").unwrap();
        let pin = FilePin::of_bytes("a.bin", b"abc");
        assert!(pin.hash_is_well_formed());
        assert!(pin.matches_file(&path));
        assert!(pin.matches_bytes(b"abc"));
        assert!(!pin.matches_bytes(b"abd"));
        std::fs::write(&path, b"abd").unwrap();
        assert!(pin.size_matches(&path));
        assert!(!pin.matches_file(&path));
        assert!(!pin.matches_file(&dir.path().join("missing")));
        assert!(!is_sha256(&"A".repeat(64)));
        assert!(!is_sha256("00"));
    }

    #[test]
    fn plain_names_only() {
        for good in [
            "diro-examples-0.1.0.zip",
            "plugin.toml",
            "a",
            "base.en",
            "COM10",
            "console.txt",
        ] {
            assert!(is_plain_name(good), "{good}");
        }
        for bad in [
            "",
            ".hidden",
            "-x",
            "a b",
            "a:b",
            "a/b",
            "a\\b",
            "x.",
            "..",
            "a..b",
            "con.txt",
            "COM1",
            "lpt9.log",
            "nul",
            "a$b",
            "a`b",
            "na\u{f03a}me",
            "caf\u{e9}",
            "a\u{7}b",
        ] {
            assert!(!is_plain_name(bad), "{bad:?}");
        }
        assert!(is_device_name("Aux.tar.gz"));
        assert!(!is_device_name("COM0"));
        assert!(is_plain_name(&"a".repeat(MAX_NAME_BYTES)));
        assert!(!is_plain_name(&"a".repeat(MAX_NAME_BYTES + 1)));
        assert!(is_plain_path("plugins/line-counter/main.py"));
        for bad in ["", "/x", "x/", "a//b", "a/../b", "C:/x"] {
            assert!(!is_plain_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_file_is_for_its_own_computer() {
        let mut pin = FilePin::new("a", 1, "0".repeat(64));
        assert!(pin.applies_to("x86_64-pc-windows-msvc"));
        pin.target = "x86_64-unknown-linux-gnu".into();
        assert!(pin.applies_to("x86_64-unknown-linux-gnu"));
        assert!(!pin.applies_to("aarch64-unknown-linux-gnu"));
        pin.target = UNIVERSAL_APPLE.into();
        assert!(pin.applies_to("aarch64-apple-darwin"));
        assert!(pin.applies_to("x86_64-apple-darwin"));
        assert!(!pin.applies_to("x86_64-pc-windows-msvc"));
        assert_ne!(host_target(), "");
    }
}
