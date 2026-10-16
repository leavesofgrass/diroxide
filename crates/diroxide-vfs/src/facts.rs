//! Facts about an entry that several parts of the program read the same
//! way: whether it is online only (a cloud placeholder whose contents a
//! read would download), whether it is sparse, and its state in words for
//! a row, a plan line or `diro ls`.
//!
//! On Windows both facts are attribute bits, so they also read in the
//! attribute words ("online only", "sparse"). macOS keeps the dataless
//! flag outside the mode bits, so the local provider records it in
//! [`Entry::extra`] under [`ONLINE_ONLY_KEY`]; [`is_online_only`] reads
//! either form.
//!
//! The guard that matters comes with the content readers (alpha.2 and
//! later): anything that reads file contents asks before it touches an
//! online-only file. A provider cannot ask on its own, so the callers
//! check with [`is_online_only`] first (research/unorthodox.md, R13).

use diroxide_core::{Attributes, Entry, EntryKind};

/// The [`Entry::extra`] key the local provider sets on a file whose
/// contents are online only, where no attribute bit says it.
pub const ONLINE_ONLY_KEY: &str = "online-only";

/// The words for an online-only file: "online only".
pub const ONLINE_ONLY_WORDS: &str = "online only";

/// The Windows bits of a file whose contents are not on this computer:
/// offline, recall on open, recall on data access.
const WINDOWS_ONLINE_ONLY: u32 = 0x1000 | 0x4_0000 | 0x40_0000;

/// The Windows sparse bit.
const WINDOWS_SPARSE: u32 = 0x200;

/// The Windows read-only bit.
const WINDOWS_READ_ONLY: u32 = 0x1;

/// Whether reading the entry's contents would download them first.
pub fn is_online_only(entry: &Entry) -> bool {
    match entry.attributes {
        Some(Attributes::Windows(flags)) if flags & WINDOWS_ONLINE_ONLY != 0 => true,
        _ => entry.extra.contains_key(ONLINE_ONLY_KEY),
    }
}

/// Whether a file is sparse: the Windows sparse bit, or on Unix fewer
/// bytes on disk than its length (blocks of 512 bytes). A file system
/// that compresses can make a dense file read as sparse; the plan only
/// counts it, so a false count costs a word, never data.
pub fn is_sparse(entry: &Entry) -> bool {
    if entry.kind != EntryKind::File && entry.kind != EntryKind::Archive {
        return false;
    }
    match (entry.attributes, entry.size, entry.blocks) {
        (Some(Attributes::Windows(flags)), _, _) => flags & WINDOWS_SPARSE != 0,
        (_, Some(size), Some(blocks)) => size > 0 && blocks.saturating_mul(512) < size,
        _ => false,
    }
}

/// Whether nobody may write it: the Windows read-only bit, or no write bit
/// at all in a Unix mode.
pub fn is_read_only(entry: &Entry) -> bool {
    match entry.attributes {
        Some(Attributes::Windows(flags)) => flags & WINDOWS_READ_ONLY != 0,
        Some(Attributes::Unix(mode)) => mode & 0o222 == 0,
        None => false,
    }
}

/// Whether a link points at nothing (its target was looked at and is
/// missing).
pub fn is_broken_link(entry: &Entry) -> bool {
    entry.kind == EntryKind::Link
        && entry
            .link
            .as_ref()
            .is_some_and(|link| link.points_to.is_none())
}

/// The entry's state in words, the most important first, for a row's
/// details, a plan line or `diro ls`: "name needs care", "name not
/// Unicode", "broken link", "online only", "sparse", "hidden",
/// "read-only". Empty for an ordinary entry.
pub fn state_words(entry: &Entry) -> Vec<&'static str> {
    let mut words = Vec::new();
    if entry.needs_care {
        words.push("name needs care");
    }
    if entry.name_not_unicode {
        words.push("name not Unicode");
    }
    if is_broken_link(entry) {
        words.push("broken link");
    }
    if is_online_only(entry) {
        words.push(ONLINE_ONLY_WORDS);
    }
    if is_sparse(entry) {
        words.push("sparse");
    }
    if entry.hidden {
        words.push("hidden");
    }
    if is_read_only(entry) {
        words.push("read-only");
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::Link;

    #[test]
    fn online_only_reads_either_form() {
        let mut placeholder = Entry::new("report.docx", EntryKind::File);
        placeholder.attributes = Some(Attributes::Windows(0x20 | 0x40_0000));
        assert!(is_online_only(&placeholder));
        let mut local = Entry::new("report.docx", EntryKind::File);
        local.attributes = Some(Attributes::Windows(0x20));
        assert!(!is_online_only(&local));
        let dataless = Entry::new("report.docx", EntryKind::File)
            .with_extra(ONLINE_ONLY_KEY, ONLINE_ONLY_WORDS);
        assert!(is_online_only(&dataless));
    }

    #[test]
    fn sparse_files_are_known_by_bit_or_blocks() {
        let mut image = Entry::new("disk.img", EntryKind::File).with_size(40 << 30);
        image.blocks = Some(8);
        image.attributes = Some(Attributes::Unix(0o644));
        assert!(is_sparse(&image));
        let mut dense = Entry::new("notes.txt", EntryKind::File).with_size(100);
        dense.blocks = Some(8);
        assert!(!is_sparse(&dense));
        let mut windows = Entry::new("disk.vhd", EntryKind::File).with_size(10);
        windows.attributes = Some(Attributes::Windows(0x200));
        assert!(is_sparse(&windows));
        let mut folder = Entry::new("dir", EntryKind::Folder).with_size(4096);
        folder.blocks = Some(0);
        assert!(!is_sparse(&folder));
    }

    #[test]
    fn state_words_put_the_name_first() {
        let mut entry = Entry::new("notes.", EntryKind::Link);
        entry.needs_care = true;
        entry.hidden = true;
        entry.link = Some(Link {
            target: "gone".into(),
            points_to: None,
        });
        entry.attributes = Some(Attributes::Unix(0o444));
        assert_eq!(
            state_words(&entry),
            vec!["name needs care", "broken link", "hidden", "read-only"]
        );
        assert!(state_words(&Entry::new("a.txt", EntryKind::File)).is_empty());
    }
}
