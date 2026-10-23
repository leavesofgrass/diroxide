//! The receipt an unpacked archive leaves in its place.
//!
//! When a component's action is `unpack`, its archive is unpacked into
//! the component's folder and a receipt, `<archive>.unpacked`, takes the
//! archive's place. Its first line is `sha256 <the archive's pin>`; each
//! line after it is one file the archive wrote: its SHA-256, its size and
//! its path, plain names joined by `/`. So the component counts as
//! installed without the archive, Verify hashes each unpacked file, and
//! Remove takes away exactly what the archive put there and nothing the
//! owner added. textweaver's receipt kept the paths only; the per-file
//! hash is diroxide's.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::ComponentError;
use crate::pin::{FilePin, is_plain_path, is_sha256};

/// The receipt's suffix: `<archive>.unpacked`.
pub const SUFFIX: &str = ".unpacked";

/// One file an archive wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptFile {
    /// Its path under the component's folder: plain names joined by `/`.
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
}

/// What one archive wrote, by the archive's pin.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    /// The archive's SHA-256: the receipt stands for that archive only.
    pub archive_sha256: String,
    /// Each file written, in the order written.
    pub files: Vec<ReceiptFile>,
}

/// Where the receipt for `pin` lives in `dir`.
pub fn path_for(dir: &Path, pin: &FilePin) -> PathBuf {
    dir.join(format!("{}{SUFFIX}", pin.name))
}

/// A receipt's file as a path under `dir`, when every part is plain.
pub fn listed(dir: &Path, rel: &str) -> Option<PathBuf> {
    is_plain_path(rel).then(|| rel.split('/').fold(dir.to_path_buf(), |p, s| p.join(s)))
}

impl Receipt {
    /// The receipt's text.
    pub fn to_text(&self) -> String {
        let mut text = format!("sha256 {}\n", self.archive_sha256);
        for file in &self.files {
            text.push_str(&format!("{} {} {}\n", file.sha256, file.size, file.path));
        }
        text
    }

    /// Reads a receipt's text; `None` when it is not one. A line that is
    /// not a plain path with a size and a hash is left out, so a damaged
    /// receipt can never name a file outside the folder.
    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let archive_sha256 = lines.next()?.strip_prefix("sha256 ")?.trim().to_owned();
        if !is_sha256(&archive_sha256) {
            return None;
        }
        let files = lines
            .filter_map(|line| {
                let mut parts = line.splitn(3, ' ');
                let sha256 = parts.next()?.to_owned();
                let size = parts.next()?.parse().ok()?;
                let path = parts.next()?.to_owned();
                (is_sha256(&sha256) && is_plain_path(&path)).then_some(ReceiptFile {
                    path,
                    size,
                    sha256,
                })
            })
            .collect();
        Some(Self {
            archive_sha256,
            files,
        })
    }

    /// The receipt for `pin` in `dir`, when there is one.
    pub fn read(dir: &Path, pin: &FilePin) -> Option<Self> {
        let path = path_for(dir, pin);
        let meta = fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > 16 << 20 {
            return None;
        }
        Self::parse(&fs::read_to_string(path).ok()?)
    }

    /// The receipt for `pin` in `dir` when it stands for `pin`'s archive.
    pub fn matching(dir: &Path, pin: &FilePin) -> Option<Self> {
        Self::read(dir, pin).filter(|r| r.archive_sha256 == pin.sha256)
    }

    /// Writes the receipt for `pin` in `dir`.
    pub fn write(&self, dir: &Path, pin: &FilePin) -> Result<(), ComponentError> {
        let path = path_for(dir, pin);
        fs::write(&path, self.to_text()).map_err(|e| ComponentError::io(&path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_receipt_reads_back_and_refuses_paths_that_leave() {
        let hash = "a".repeat(64);
        let receipt = Receipt {
            archive_sha256: hash.clone(),
            files: vec![ReceiptFile {
                path: "plugins/a.toml".into(),
                size: 3,
                sha256: "b".repeat(64),
            }],
        };
        assert_eq!(Receipt::parse(&receipt.to_text()), Some(receipt));
        let hostile = format!(
            "sha256 {hash}\n{b} 1 ../outside\n{b} 1 /abs\n{b} 1 C:/x\n{b} x ok\n{b} 2 fine.txt\n",
            b = "b".repeat(64)
        );
        let read = Receipt::parse(&hostile).unwrap();
        assert_eq!(read.files.len(), 1);
        assert_eq!(read.files[0].path, "fine.txt");
        assert_eq!(Receipt::parse("sha256 00\n"), None);
        assert_eq!(Receipt::parse("nothing"), None);
        let dir = Path::new("base");
        assert_eq!(listed(dir, "a/b"), Some(dir.join("a").join("b")));
        assert_eq!(listed(dir, "a/../b"), None);
    }
}
