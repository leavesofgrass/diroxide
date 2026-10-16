//! Volumes: what kind of drive a place is on, and how much room it has.
//! The trash gate asks the volume kind of the path's own volume root, not
//! of its drive letter alone, so a folder mounted from another drive is
//! judged by the drive it is really on (spike six).

use diroxide_core::Place;

/// What kind of drive a volume is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VolumeKind {
    /// A fixed local disk.
    Fixed,
    /// A removable drive: a USB stick, a card.
    Removable,
    /// A network drive or share.
    Network,
    /// Not known: treated as having no trash.
    #[default]
    Unknown,
}

impl VolumeKind {
    /// The kind in words, for the drive list and messages: "disk",
    /// "removable drive", "network drive", "drive".
    pub const fn word(self) -> &'static str {
        match self {
            VolumeKind::Fixed => "disk",
            VolumeKind::Removable => "removable drive",
            VolumeKind::Network => "network drive",
            VolumeKind::Unknown => "drive",
        }
    }
}

/// The volume a place is on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VolumeInfo {
    /// The volume's root: `D:\`, a mount point, a share, `/`.
    pub root: Place,
    /// What kind of drive it is.
    pub kind: VolumeKind,
    /// The volume's label, when it has one.
    pub label: Option<String>,
    /// The file system's name, such as "NTFS" or "exFAT", when known.
    pub file_system: Option<String>,
}

impl VolumeInfo {
    /// A volume of unknown kind at a root.
    pub fn unknown(root: Place) -> Self {
        Self {
            root,
            kind: VolumeKind::Unknown,
            label: None,
            file_system: None,
        }
    }

    /// The same volume with a kind.
    pub fn with_kind(mut self, kind: VolumeKind) -> Self {
        self.kind = kind;
        self
    }

    /// The top of a place: the volume root a provider with no better
    /// answer reports (a drive, a share, `/`, a remote root, the folder
    /// that holds an archive's top).
    pub fn top_of(place: &Place) -> Place {
        let mut top = place.clone();
        while let Some(parent) = top.parent() {
            top = parent;
        }
        top
    }
}

/// Free space and size limits at a place (research/file-operations.md
/// C7). `None` means not known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpaceInfo {
    /// The bytes the user may still write (quotas counted).
    pub free_to_user: Option<u64>,
    /// The volume's size in bytes.
    pub total: Option<u64>,
    /// The largest file the file system takes (4 GiB less one byte on
    /// FAT32).
    pub max_file_size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;

    #[test]
    fn the_top_of_a_place_is_its_drive() {
        let place = Place::parse_with("D:\\docs\\a.zip!x/y", PathStyle::Windows).unwrap();
        assert_eq!(VolumeInfo::top_of(&place).to_string(), "D:\\");
        assert_eq!(VolumeKind::Removable.word(), "removable drive");
        assert_eq!(
            VolumeInfo::unknown(VolumeInfo::top_of(&place))
                .with_kind(VolumeKind::Fixed)
                .kind,
            VolumeKind::Fixed
        );
    }
}
