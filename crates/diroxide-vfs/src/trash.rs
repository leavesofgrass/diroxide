//! The trash gate and the post-check (plan section 5.12; spikes four and
//! six). The `trash` crate's Ok never means "in the trash": on Windows it
//! removes a file for good, with no dialog and Ok, on a removable drive
//! and for any path of 260 UTF-16 units or more. So a gate decides before
//! every call, and a post-check confirms after it.
//!
//! The gate is pure, so its table is tested with fake drive kinds and the
//! lengths 259 and 260; no test ever touches the real trash.

use std::path::Path;

use diroxide_core::Message;
use diroxide_core::catalog;

use crate::volume::VolumeKind;

/// The longest path, in UTF-16 units, that the Windows Recycle Bin takes
/// (spike six: 259 reached the bin, 260 was removed for good).
pub const TRASH_MAX_PATH_UNITS: usize = 259;

/// Above this size an item the trash would copy to another drive asks
/// first (research/file-operations.md, R8: 64 MB): the copy takes as long
/// as copying the file, and fills the home drive.
pub const TRASH_COPY_ASK_BYTES: u64 = 64_000_000;

/// The rules of the platform's trash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrashRules {
    /// Only fixed local drives have a trash (Windows).
    pub fixed_drives_only: bool,
    /// The longest path the trash takes, in UTF-16 units, if it has a
    /// limit (Windows).
    pub max_path_units: Option<usize>,
}

impl TrashRules {
    /// Windows: the Recycle Bin on fixed local drives only, for paths of
    /// at most 259 units, never on a network path.
    pub const WINDOWS: TrashRules = TrashRules {
        fixed_drives_only: true,
        max_path_units: Some(TRASH_MAX_PATH_UNITS),
    };

    /// Linux and macOS: the system trash, wherever the platform offers
    /// one; network places are still refused.
    pub const OTHER: TrashRules = TrashRules {
        fixed_drives_only: false,
        max_path_units: None,
    };

    /// The rules of the platform this was built for.
    pub const fn native() -> Self {
        if cfg!(windows) {
            Self::WINDOWS
        } else {
            Self::OTHER
        }
    }
}

/// What the gate needs to know about one item: its volume's kind (asked
/// of the path's own volume root), its longest path (for a folder, the
/// longest path inside it), and whether it is on a network path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrashFacts {
    /// The kind of the volume the item is really on.
    pub volume: VolumeKind,
    /// The longest path in UTF-16 units: the item's own, or for a folder
    /// the longest path of anything inside it.
    pub longest_path_units: usize,
    /// Whether the path is a network path (a share, a mapped drive).
    pub network_path: bool,
    /// Whether the trash would take the item by copying it to a trash on
    /// another drive and deleting the original (Linux and the other
    /// freedesktop systems: a drive whose own trash cannot be made sends
    /// its items to the home trash; [`copies_across_drives`]). Never on
    /// Windows or macOS.
    pub copies_across_drives: bool,
}

/// The freedesktop rule (R8; the `trash` crate as it is pinned): an item
/// on a volume other than the home folder's, whose volume has no trash of
/// its own that can be used or made, is copied into the home trash. When
/// the home folder's volume is not known, the trash is taken not to copy,
/// so nothing asks without cause.
pub fn copies_across_drives(
    item_root: &Path,
    home_root: Option<&Path>,
    own_trash_usable: bool,
) -> bool {
    home_root.is_some_and(|home| home != item_root) && !own_trash_usable
}

/// The gate's decision for one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrashGate {
    /// The trash may take it.
    Allowed,
    /// The trash may take it, but only by copying it to another drive's
    /// trash and deleting the original ([`TrashFacts::copies_across_drives`]):
    /// slow and filling for a big item, so one over
    /// [`TRASH_COPY_ASK_BYTES`] asks first ([`TrashGate::copy_question`]).
    Copies,
    /// This drive has no trash (a removable drive on Windows, or a kind
    /// that is not known).
    NoTrashHere(VolumeKind),
    /// A network place: never the trash.
    Network,
    /// A path too long for the trash, with its length in UTF-16 units.
    TooLong {
        /// The longest path's length.
        units: usize,
    },
}

impl TrashGate {
    /// Whether the trash may be called.
    pub const fn allows(self) -> bool {
        matches!(self, TrashGate::Allowed | TrashGate::Copies)
    }

    /// Whether an item of `bytes` asks before the trash takes it: the trash
    /// would copy it to another drive and it is over
    /// [`TRASH_COPY_ASK_BYTES`].
    pub const fn asks_first(self, bytes: u64) -> bool {
        matches!(self, TrashGate::Copies) && bytes > TRASH_COPY_ASK_BYTES
    }

    /// The question before such a copy, starting on No: "Trashing copies
    /// 3.2 GB to another drive. Go on? y or n". `None` when it need not ask.
    pub fn copy_question(self, bytes: u64) -> Option<Message> {
        self.asks_first(bytes)
            .then(|| catalog::trash_copies_across(bytes))
    }

    /// Why the trash cannot take it, in lowercase words, for
    /// [`crate::VfsError::TrashRefused`]; empty when it can.
    pub fn why(self) -> String {
        match self {
            TrashGate::Allowed | TrashGate::Copies => String::new(),
            TrashGate::NoTrashHere(kind) => format!("this {} has no trash", kind.word()),
            TrashGate::Network => "a network drive has no trash".to_owned(),
            TrashGate::TooLong { units } => {
                format!("the path is {units} characters, too long for the trash")
            }
        }
    }

    /// The question that offers the listed permanent delete instead,
    /// starting on No: "This drive has no trash. Delete 3 items for good? y
    /// or n". `None` when the trash may take it.
    pub fn question(self, items: u64) -> Option<Message> {
        match self {
            TrashGate::Allowed | TrashGate::Copies => None,
            TrashGate::NoTrashHere(_) | TrashGate::Network => Some(catalog::no_trash_here(items)),
            TrashGate::TooLong { .. } => Some(catalog::too_long_for_trash(items)),
        }
    }
}

/// Decides, before the call, whether the trash may take an item.
pub fn trash_gate(facts: &TrashFacts, rules: TrashRules) -> TrashGate {
    if facts.network_path || facts.volume == VolumeKind::Network {
        return TrashGate::Network;
    }
    if rules.fixed_drives_only && facts.volume != VolumeKind::Fixed {
        return TrashGate::NoTrashHere(facts.volume);
    }
    if let Some(max) = rules.max_path_units
        && facts.longest_path_units > max
    {
        return TrashGate::TooLong {
            units: facts.longest_path_units,
        };
    }
    if facts.copies_across_drives {
        return TrashGate::Copies;
    }
    TrashGate::Allowed
}

/// What the post-check found after a trash call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrashCheck {
    /// The original is gone and the trash holds it (on Windows, its file
    /// id resolves inside the drive's `$RECYCLE.BIN`, or the bin's count
    /// rose).
    InTrash,
    /// The original is gone and the trash does not hold it: the loud
    /// failure, [`catalog::TRASH_NOT_CONFIRMED`].
    GoneNotInTrash,
    /// The original is still where it was: the trash did nothing.
    StillThere,
    /// The provider cannot check (a plugin's trash); reported as done with
    /// a warning, never as confirmed.
    NotChecked,
}

impl TrashCheck {
    /// Whether the item is known to be in the trash.
    pub const fn confirmed(self) -> bool {
        matches!(self, TrashCheck::InTrash)
    }

    /// The line for the report when the trash is not confirmed: the loud
    /// failure, or "Still there after the trash", or "Moved to the trash;
    /// not checked". `None` when confirmed.
    pub fn problem(self) -> Option<&'static str> {
        match self {
            TrashCheck::InTrash => None,
            TrashCheck::GoneNotInTrash => Some(catalog::TRASH_NOT_CONFIRMED),
            TrashCheck::StillThere => Some("Still there after the trash."),
            TrashCheck::NotChecked => Some("Moved to the trash; not checked."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn facts(volume: VolumeKind, units: usize) -> TrashFacts {
        TrashFacts {
            volume,
            longest_path_units: units,
            network_path: false,
            copies_across_drives: false,
        }
    }

    #[test]
    fn a_trash_that_copies_across_drives_asks_first_above_64_mb() {
        let copying = TrashFacts {
            copies_across_drives: true,
            ..facts(VolumeKind::Removable, 40)
        };
        let gate = trash_gate(&copying, TrashRules::OTHER);
        assert_eq!(gate, TrashGate::Copies);
        assert!(gate.allows() && gate.why().is_empty() && gate.question(1).is_none());
        assert!(!gate.asks_first(TRASH_COPY_ASK_BYTES));
        assert!(gate.asks_first(TRASH_COPY_ASK_BYTES + 1));
        assert!(gate.copy_question(1_000).is_none());
        let q = gate.copy_question(3_200_000_000).unwrap();
        assert_eq!(
            q.text,
            "Trashing copies 3.2 GB to another drive. Go on? y or n"
        );
        assert!(within_cells(
            &q.text,
            "Trashing copies 3.2 GB to another drive"
        ));
        assert!(!TrashGate::Allowed.asks_first(u64::MAX));
        let network = TrashFacts {
            network_path: true,
            ..copying
        };
        assert_eq!(trash_gate(&network, TrashRules::OTHER), TrashGate::Network);
        let windows_stick = trash_gate(&copying, TrashRules::WINDOWS);
        assert_eq!(windows_stick, TrashGate::NoTrashHere(VolumeKind::Removable));
    }

    #[test]
    fn the_freedesktop_rule_copies_only_off_the_home_volume_without_its_own_trash() {
        let home = Path::new("/");
        let usb = Path::new("/media/ada/USB");
        assert!(copies_across_drives(usb, Some(home), false));
        assert!(
            !copies_across_drives(usb, Some(home), true),
            "its own trash"
        );
        assert!(
            !copies_across_drives(home, Some(home), false),
            "the home volume"
        );
        assert!(
            !copies_across_drives(usb, None, false),
            "home unknown: no question"
        );
    }

    #[test]
    fn the_windows_gate_follows_spike_six() {
        let gate = |volume, units| trash_gate(&facts(volume, units), TrashRules::WINDOWS);
        assert_eq!(gate(VolumeKind::Fixed, 58), TrashGate::Allowed);
        assert_eq!(gate(VolumeKind::Fixed, 259), TrashGate::Allowed);
        assert_eq!(
            gate(VolumeKind::Fixed, 260),
            TrashGate::TooLong { units: 260 }
        );
        assert_eq!(
            gate(VolumeKind::Fixed, 1305),
            TrashGate::TooLong { units: 1305 }
        );
        assert_eq!(
            gate(VolumeKind::Removable, 30),
            TrashGate::NoTrashHere(VolumeKind::Removable)
        );
        assert_eq!(
            gate(VolumeKind::Unknown, 30),
            TrashGate::NoTrashHere(VolumeKind::Unknown)
        );
        assert_eq!(gate(VolumeKind::Network, 30), TrashGate::Network);
        let mapped = TrashFacts {
            network_path: true,
            ..facts(VolumeKind::Fixed, 30)
        };
        assert_eq!(trash_gate(&mapped, TrashRules::WINDOWS), TrashGate::Network);
    }

    #[test]
    fn other_platforms_have_no_length_or_drive_limit() {
        let gate = |volume, units| trash_gate(&facts(volume, units), TrashRules::OTHER);
        assert_eq!(gate(VolumeKind::Removable, 4000), TrashGate::Allowed);
        assert_eq!(gate(VolumeKind::Network, 10), TrashGate::Network);
    }

    #[test]
    fn a_refusal_offers_the_listed_permanent_delete() {
        let q = TrashGate::NoTrashHere(VolumeKind::Removable)
            .question(3)
            .unwrap();
        assert_eq!(
            q.text,
            "This drive has no trash. Delete 3 items for good? y or n"
        );
        assert!(TrashGate::Allowed.question(3).is_none());
        assert_eq!(
            TrashGate::NoTrashHere(VolumeKind::Removable).why(),
            "this removable drive has no trash"
        );
    }

    #[test]
    fn only_a_confirmed_check_counts_as_in_the_trash() {
        assert!(TrashCheck::InTrash.confirmed());
        assert_eq!(
            TrashCheck::GoneNotInTrash.problem(),
            Some("Deleted but not found in the trash.")
        );
        for check in [
            TrashCheck::GoneNotInTrash,
            TrashCheck::StillThere,
            TrashCheck::NotChecked,
        ] {
            assert!(!check.confirmed());
            assert!(within_cells(check.problem().unwrap(), "trash"));
        }
    }
}
