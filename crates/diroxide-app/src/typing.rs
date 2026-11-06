//! File types in the app (alpha 4, lane TA, R28 4.3): what the background
//! check reads and answers.
//!
//! A row shows the type its name gives at once, and "type pending" only
//! where the name says nothing. A check off the input thread then reads
//! the first [`HEAD_BYTES`] of each file of a folder on this computer, the
//! rows nearest the focus first, and types it by its contents
//! ([`diroxide_filetype::TypeRegistry::check`]): four readers on a local
//! disk, one on a share. A scroll that moves the focus a page away asks
//! again from there, and the readers of the old order stop. Each file
//! keeps its type as a two-byte index into the registry
//! ([`diroxide_core::Entry::type_index`]), and the panel one bit for each
//! file it checked; a refresh keeps the types of the files that did not
//! change. A file whose name and contents disagree keeps the line "Name
//! says PNG; contents say JPEG" for the details and for Open; it is never
//! said by itself.
//!
//! Archives, plugins' places, remote sites and the folders of the setting
//! `check.skip` are typed by name alone: their files are never opened for
//! their types.

use diroxide_core::{Clock, Entry, EntryKind, Timestamp};
use diroxide_filetype::{HEAD_BYTES, Probe, Special, TypeRegistry, TypeStage};

/// Readers of first bytes at once on a disk of this computer.
pub const LOCAL_READERS: usize = 4;

/// Readers on a share, where every read crosses the network.
pub const SHARE_READERS: usize = 1;

/// One file whose first bytes the check reads: its place in the panel's
/// listing and the facts a type's condition may test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeItem {
    /// Its index in the listing ([`crate::Panel::all_entries`]).
    pub index: u32,
    /// Its name, also the check that the answer is still for it.
    pub name: String,
    /// Its kind.
    pub kind: EntryKind,
    /// Its size.
    pub size: Option<u64>,
    /// When it changed.
    pub modified: Option<Timestamp>,
    /// Whether it is hidden.
    pub hidden: bool,
}

impl TypeItem {
    /// The item for the entry at `index` of a listing.
    pub fn of(index: usize, entry: &Entry) -> Self {
        Self {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            name: entry.name.clone(),
            kind: entry.kind,
            size: entry.size,
            modified: entry.modified,
            hidden: entry.hidden,
        }
    }

    /// The entry the type's tests look at.
    pub fn entry(&self) -> Entry {
        let mut entry = Entry::new(self.name.clone(), self.kind);
        entry.size = self.size;
        entry.modified = self.modified;
        entry.hidden = self.hidden;
        entry
    }
}

/// What the check found for one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAnswer {
    /// Its index in the listing it was asked for.
    pub index: u32,
    /// Its name.
    pub name: String,
    /// Its type as [`diroxide_core::Entry::type_index`] holds it, and the
    /// stage that found it; or what its row says when no type takes it.
    pub found: Result<(u16, TypeStage), Special>,
}

/// Types one file by its first bytes (`None` when they could not be read:
/// it is then typed by its name alone).
pub fn check_item(
    types: &TypeRegistry,
    item: &TypeItem,
    head: Option<&[u8]>,
    clock: &dyn Clock,
) -> TypeAnswer {
    let entry = item.entry();
    let head = head.map(|h| &h[..h.len().min(HEAD_BYTES)]);
    let probe = Probe::new(&entry, head).skipping(head.is_none());
    let found = types
        .check(&probe, clock)
        .map(|found| (TypeRegistry::index_of(&found), found.stage));
    TypeAnswer {
        index: item.index,
        name: item.name.clone(),
        found,
    }
}

#[cfg(test)]
mod tests {
    use diroxide_core::{FixedClock, UtcOffset};

    use super::*;

    fn clock() -> FixedClock {
        let now = Timestamp::from_civil(2026, 11, 6, 9, 0, 0, UtcOffset::UTC).unwrap();
        FixedClock::new(now, UtcOffset::UTC)
    }

    #[test]
    fn a_file_is_typed_by_its_first_bytes_and_else_by_its_name() {
        let types = TypeRegistry::builtin();
        let item = TypeItem::of(3, &Entry::new("photo.png", EntryKind::File).with_size(40));
        let jpeg = check_item(
            &types,
            &item,
            Some(b"\xFF\xD8\xFF\xE0\0\x10JFIF\0"),
            &clock(),
        );
        let (index, stage) = jpeg.found.unwrap();
        assert_eq!(types.by_index(index).unwrap().id, "jpeg");
        assert_eq!(stage, TypeStage::Signature);
        assert_eq!((jpeg.index, jpeg.name.as_str()), (3, "photo.png"));
        let unread = check_item(&types, &item, None, &clock());
        let (index, stage) = unread.found.unwrap();
        assert_eq!(types.by_index(index).unwrap().id, "png");
        assert_eq!(stage, TypeStage::Name);
        let nameless = TypeItem::of(0, &Entry::new("data", EntryKind::File).with_size(9));
        assert_eq!(
            check_item(&types, &nameless, Some(b"plain text"), &clock()).found,
            Err(Special::Unknown)
        );
        assert_eq!(
            check_item(&types, &nameless, None, &clock()).found,
            Err(Special::Unknown),
            "a file that could not be read is never left pending"
        );
    }
}
