//! The 7-Zip fallback (R29 1.4 and 6.1, R30 commit 94): an image, or a
//! partition, that no built-in reader opens (UDF, HFS+, APFS, exFAT,
//! squashfs with xz or zstd, a compressed NTFS file, a container a crate
//! refuses) is read through the 7z program the host finds, never a shell
//! line.
//!
//! - **The listing** is `7z l -slt -spd -sccUTF-8 -ba -- IMAGE`: a block
//!   of `Key = value` lines an item ([`parse_slt`], fuzzed). Spike DS3
//!   fixed the forms: the same keys on 7-Zip 24.08 (Windows), 25.01
//!   (Debian) and 26.05; `\` between names on Windows and `/` on Linux;
//!   CR LF line ends on Windows; `-sccUTF-8` needed on Windows for names
//!   that are not ASCII; a disk with a partition table lists its
//!   partitions as items named `0.LABEL.fat`, and a container holding one
//!   file system lists that file system's files.
//! - **A member** comes back through `x -so` (written to nothing but the
//!   pipe) when it fits one answer of the host's (6 MiB); a larger one is
//!   unpacked into a folder of the plugin's own in the spool with `x -o`,
//!   only by 7-Zip 25.01 or later, which fixed two link path-traversal
//!   bugs (R7); an older one says "7-Zip 24.08 is too old: needs 25.01".
//!   Whatever it writes stays inside the spool budget.
//! - **A password** is never on a command line: spike DS4 found that
//!   with no `-p` switch every 7-Zip tried (24.08 and 26.05 on Windows,
//!   25.01 on Linux) asks for it and reads it from standard input, which
//!   the host's `tools/run` feeds; `-p` with nothing after it means an
//!   empty password. This plugin gives 7-Zip an empty standard input, so
//!   an encrypted image says "it is encrypted, and this plugin asks no
//!   password" instead of waiting; the archive provider's tool backend
//!   (commit 80) is where a password prompt belongs.

use std::collections::HashMap;

use diroxide_core::Timestamp;

use crate::image::fs::{Node, NodeKind};

/// One item of a `-slt` listing: its keys as 7-Zip printed them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    /// Every `Key = value` of the item's block.
    pub keys: HashMap<String, String>,
}

impl Item {
    /// A key's value, when the item has it and it is not empty.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// The item's path as 7-Zip printed it.
    pub fn path(&self) -> Option<&str> {
        self.get("Path")
    }

    /// Whether it is a folder (`Folder = +`, or a `D` attribute).
    pub fn is_folder(&self) -> bool {
        self.get("Folder") == Some("+")
            || self
                .get("Attributes")
                .is_some_and(|a| a.split_whitespace().next().is_some_and(|f| f.contains('D')))
    }

    /// Whether 7-Zip says its data is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.get("Encrypted") == Some("+")
    }
}

/// The items of a `7z l -slt -ba` listing. The header block a listing
/// without `-ba` starts with (the archive's own `Path` and `Type`, before
/// a line of ten dashes) is left out; a block is the lines between blank
/// lines; a line without ` = ` is ignored, so nothing 7-Zip adds stops
/// the parse.
pub fn parse_slt(text: &str) -> Vec<Item> {
    let body = match text.find("\n----------") {
        Some(at) => &text[at + 11..],
        None => text,
    };
    let mut items = Vec::new();
    let mut current = Item::default();
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            if current.path().is_some() {
                items.push(std::mem::take(&mut current));
            } else {
                current = Item::default();
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(" = ") {
            current.keys.insert(key.trim().to_owned(), value.to_owned());
        } else if let Some(key) = line.strip_suffix(" =") {
            current.keys.insert(key.trim().to_owned(), String::new());
        }
    }
    if current.path().is_some() {
        items.push(current);
    }
    items
}

/// A version of 7-Zip, from its banner: "7-Zip 24.08 (x64) : ...",
/// "7-Zip (a) 26.05 (x64) : ...", "p7zip Version 16.02 (locale=...".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// The major number.
    pub major: u32,
    /// The minor number.
    pub minor: u32,
}

impl Version {
    /// The first version whose extraction is trusted (R7: 25.00 fixed two
    /// link path-traversal bugs; R29 names 25.01).
    pub const EXTRACT: Version = Version {
        major: 25,
        minor: 1,
    };

    /// The version in its usual words: "24.08".
    pub fn words(self) -> String {
        format!("{}.{:02}", self.major, self.minor)
    }

    /// The version a banner names.
    pub fn from_banner(banner: &str) -> Option<Self> {
        let rest = banner.lines().map(str::trim).find_map(|line| {
            line.strip_prefix("7-Zip")
                .or_else(|| line.strip_prefix("p7zip Version"))
        })?;
        let number = rest
            .split_whitespace()
            .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
        let (major, minor) = number.split_once('.')?;
        let minor: String = minor.chars().take_while(char::is_ascii_digit).collect();
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }
}

/// A `-slt` time, `2026-11-06 12:48:43` with an optional fraction, read
/// as UTC (7-Zip prints the archive's own time, whose zone it does not
/// know).
pub fn slt_time(text: &str) -> Option<Timestamp> {
    let (date, time) = text.split_once(' ')?;
    let mut d = date.split('-');
    let year = d.next()?.parse().ok()?;
    let month = d.next()?.parse().ok()?;
    let day = d.next()?.parse().ok()?;
    let mut t = time.split(['.', ':']);
    let hour = t.next()?.parse().ok()?;
    let minute = t.next()?.parse().ok()?;
    let second = t.next()?.parse().ok()?;
    crate::image::fs::civil(year, month, day, hour, minute, second)
}

/// The items of a listing as a tree: each folder's children by their
/// stored path, folders a path implies made where 7-Zip lists none.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    /// Each folder's nodes, by its stored names joined with `/`.
    pub folders: HashMap<Vec<u8>, Vec<Node>>,
    /// Each file's member path as 7-Zip printed it, by its stored names
    /// joined with `/`.
    pub members: HashMap<Vec<u8>, String>,
}

impl Tree {
    /// Builds the tree from a listing, its paths split at `/` and `\`.
    /// An item whose path climbs out (`..`) or is empty is left out; the
    /// tree's rules then judge every name.
    pub fn from_items(items: &[Item]) -> Self {
        let mut tree = Tree::default();
        tree.folders.entry(Vec::new()).or_default();
        for item in items {
            let Some(path) = item.path() else {
                continue;
            };
            let parts: Vec<&str> = path
                .split(['/', '\\'])
                .filter(|p| !p.is_empty() && *p != ".")
                .collect();
            if parts.is_empty() || parts.contains(&"..") {
                continue;
            }
            for depth in 1..parts.len() {
                tree.folder(&parts[..depth]);
            }
            let parent = parts[..parts.len() - 1].join("/").into_bytes();
            let key = parts.join("/").into_bytes();
            let name = parts[parts.len() - 1].as_bytes().to_vec();
            let kind = if item.is_folder() {
                NodeKind::Folder
            } else if item.get("Symbolic Link").is_some() {
                NodeKind::Link
            } else {
                NodeKind::File
            };
            let siblings = tree.folders.entry(parent).or_default();
            if let Some(existing) = siblings.iter_mut().find(|n| n.name == name) {
                // A folder an earlier path implied: the item's own facts.
                existing.modified = item.get("Modified").and_then(slt_time);
                continue;
            }
            let mut node = Node::new(name, kind);
            node.size = item
                .get("Size")
                .and_then(|s| s.parse().ok())
                .filter(|_| kind == NodeKind::File);
            node.modified = item.get("Modified").and_then(slt_time);
            node.link = item.get("Symbolic Link").map(|t| t.as_bytes().to_vec());
            if item.is_encrypted() {
                node.note = Some("encrypted, 7-Zip asks its password".to_owned());
            }
            siblings.push(node);
            if kind == NodeKind::Folder {
                tree.folders.entry(key).or_default();
            } else {
                tree.members.insert(key, path.to_owned());
            }
        }
        tree
    }

    fn folder(&mut self, parts: &[&str]) {
        let key = parts.join("/").into_bytes();
        if self.folders.contains_key(&key) {
            return;
        }
        let parent = parts[..parts.len() - 1].join("/").into_bytes();
        let name = parts[parts.len() - 1].as_bytes().to_vec();
        let siblings = self.folders.entry(parent).or_default();
        if !siblings.iter().any(|n| n.name == name) {
            siblings.push(Node::new(name, NodeKind::Folder));
        }
        self.folders.insert(key, Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ISO block 7-Zip 25.01 printed in spike DS3 (Linux).
    const ISO: &str = "\n7-Zip 25.01 (x64) : Copyright (c) 1999-2025 Igor Pavlov : 2025-08-03\n\nListing archive: /pd/sample.iso\n\n--\nPath = /pd/sample.iso\nType = Iso\nPhysical Size = 385024\n\n----------\nPath = folder\nFolder = +\nSize = \nPacked Size = \nModified = 2026-11-06 12:48:43\nMode = \nSymbolic Link = \n\nPath = folder/sub/inner.txt\nFolder = -\nSize = 6\nPacked Size = 6\nModified = 2026-11-06 12:48:43\nMode = \nSymbolic Link = \n\nPath = hello.txt\nFolder = -\nSize = 13\nPacked Size = 13\nModified = 2026-11-06 12:48:43\nMode = \nSymbolic Link = \n\n";

    #[test]
    fn a_listing_parses_and_skips_the_archive_block() {
        let items = parse_slt(ISO);
        let paths: Vec<_> = items.iter().filter_map(Item::path).collect();
        assert_eq!(paths, ["folder", "folder/sub/inner.txt", "hello.txt"]);
        assert!(items[0].is_folder());
        assert_eq!(items[2].get("Size"), Some("13"));
        assert_eq!(items[2].get("Mode"), None);
        let windows = ISO
            .replace('\n', "\r\n")
            .replace("folder/sub/inner.txt", "folder\\sub\\inner.txt");
        let items = parse_slt(&windows);
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].path(), Some("folder\\sub\\inner.txt"));
    }

    #[test]
    fn a_tree_makes_the_folders_a_path_implies() {
        let tree = Tree::from_items(&parse_slt(ISO));
        let root: Vec<_> = tree.folders[&Vec::new()]
            .iter()
            .map(|n| String::from_utf8_lossy(&n.name).into_owned())
            .collect();
        assert_eq!(root, ["folder", "hello.txt"]);
        let sub = &tree.folders[&b"folder/sub".to_vec()];
        assert_eq!(sub[0].name, b"inner.txt");
        assert_eq!(sub[0].size, Some(6));
        assert_eq!(tree.members[&b"hello.txt".to_vec()], "hello.txt");
        let hostile = parse_slt("Path = ../../x\nSize = 1\n\nPath = \nSize = 2\n");
        let tree = Tree::from_items(&hostile);
        assert!(tree.members.is_empty());
    }

    #[test]
    fn banners_give_their_versions() {
        let cases = [
            (
                "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11",
                (24, 8),
            ),
            (
                "\n7-Zip (a) 26.05 (x64) : Copyright (c) 1999-2026 Igor Pavlov",
                (26, 5),
            ),
            ("7-Zip 25.01 (x64) : Copyright", (25, 1)),
            (
                "p7zip Version 16.02 (locale=C.UTF-8,Utf16=on,HugeFiles=on,64 bits)",
                (16, 2),
            ),
        ];
        for (banner, (major, minor)) in cases {
            assert_eq!(
                Version::from_banner(banner),
                Some(Version { major, minor }),
                "{banner}"
            );
        }
        assert!(Version::from_banner("nothing").is_none());
        assert!(
            Version {
                major: 24,
                minor: 8
            } < Version::EXTRACT
        );
        assert_eq!(
            Version {
                major: 24,
                minor: 8
            }
            .words(),
            "24.08"
        );
        assert!(slt_time("2026-01-01 00:00:00.0000000").is_some());
    }
}
