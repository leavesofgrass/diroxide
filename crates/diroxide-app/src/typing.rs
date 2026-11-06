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

use std::collections::HashMap;

use diroxide_config::FileTypesConfig;
use diroxide_core::{Clock, Entry, EntryKind, Timestamp};
use diroxide_filetype::{ActionTarget, HEAD_BYTES, Probe, Special, TypeRegistry, TypeStage};

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

// Typed actions (R30 commit 30; R28 4.3): each type lists its actions,
// and each action fills a slot: Enter, View, Preview, Convert, one of ten
// numbered user actions, or none (a named action, listed by Actions for
// NAME). An empty list falls back to the parent type's.

/// Which slot a type's action fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// Enter: the file's own action, before the system's program.
    Enter,
    /// View, before the built-in viewer.
    View,
    /// Preview.
    Preview,
    /// Convert.
    Convert,
    /// A numbered user action, 0 to 9, run by the commands `type.user0`
    /// to `type.user9`, which have no key until one is bound.
    User(u8),
    /// A named action, listed by Actions for NAME alone.
    Named,
}

impl Slot {
    /// A slot as `filetypes.toml` writes it: `enter`, `view`, `preview`,
    /// `convert`, `user0` to `user9`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        Some(match text.as_str() {
            "enter" => Slot::Enter,
            "view" => Slot::View,
            "preview" => Slot::Preview,
            "convert" => Slot::Convert,
            _ => {
                let digit = text.strip_prefix("user")?;
                let n: u8 = digit.parse().ok().filter(|n| *n <= 9)?;
                if digit.len() != 1 {
                    return None;
                }
                Slot::User(n)
            }
        })
    }

    /// The slot in words, said after an action's name: "Enter", "user
    /// action 3"; empty for a named action.
    pub fn words(self) -> String {
        match self {
            Slot::Enter => "Enter".to_owned(),
            Slot::View => "View".to_owned(),
            Slot::Preview => "Preview".to_owned(),
            Slot::Convert => "Convert".to_owned(),
            Slot::User(n) => format!("user action {n}"),
            Slot::Named => String::new(),
        }
    }
}

/// The slots of the owner's type actions, read from `filetypes.toml`.
/// The type model keeps each action's name and target; the slot it fills
/// is read here from the same file, by the type's id and the action's
/// name (the first of a name wins).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeSlots {
    slots: HashMap<String, Vec<(String, Slot)>>,
    problems: Vec<String>,
}

impl TypeSlots {
    /// The slots of the types in the owner's file; a slot that does not
    /// read leaves its action named, and says so.
    pub fn from_config(config: &FileTypesConfig) -> Self {
        let mut slots: HashMap<String, Vec<(String, Slot)>> = HashMap::new();
        let mut problems = Vec::new();
        for file_type in &config.types {
            for action in &file_type.actions {
                let slot = match action.slot.as_deref() {
                    None => Slot::Named,
                    Some(text) => Slot::parse(text).unwrap_or_else(|| {
                        problems.push(format!(
                            "File type {}: action {} has no slot \"{text}\"; it is listed by name.",
                            file_type.id, action.name
                        ));
                        Slot::Named
                    }),
                };
                slots
                    .entry(file_type.id.clone())
                    .or_default()
                    .push((action.name.clone(), slot));
            }
        }
        Self { slots, problems }
    }

    /// The slot of a type's action, by the type's id and the action's
    /// name; a named action where the file says none.
    pub fn slot(&self, type_id: &str, action: &str) -> Slot {
        self.slots
            .get(type_id)
            .and_then(|actions| actions.iter().find(|(name, _)| name == action))
            .map_or(Slot::Named, |(_, slot)| *slot)
    }

    /// The slots that did not read, each line meaning first.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

/// One action a file's type offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    /// Its name, as listed: "Make thumbnail".
    pub name: String,
    /// The slot it fills.
    pub slot: Slot,
    /// What it does: a command, a tool, a plugin's command.
    pub target: ActionTarget,
}

/// The actions a type offers (by [`diroxide_core::Entry::type_index`]):
/// its own, or where its list is empty its parent's, and so on up; in
/// the order the file lists them, the first of each slot the one that
/// runs. Empty when no type in its line has any.
pub fn offered(types: &TypeRegistry, slots: &TypeSlots, type_index: u16) -> Vec<Offered> {
    let mut at = types.by_index(type_index);
    for _ in 0..64 {
        let Some(file_type) = at else {
            break;
        };
        if !file_type.actions.is_empty() {
            return file_type
                .actions
                .iter()
                .map(|action| Offered {
                    name: action.name.clone(),
                    slot: slots.slot(&file_type.id, &action.name),
                    target: action.target.clone(),
                })
                .collect();
        }
        at = file_type.parent.as_deref().and_then(|p| types.get(p));
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use diroxide_core::{FixedClock, UtcOffset};

    use super::*;

    /// R30 commit 30: an action's slot comes from the owner's file; an
    /// empty list falls back to the parent type's; a slot that does not
    /// read is said and the action stays, by name.
    #[test]
    fn a_type_offers_its_actions_or_its_parents() {
        let config = FileTypesConfig::from_toml(
            r#"
[[type]]
id = "zip"
name = "zip archive"
short = "zip"
extensions = ["zip"]

[[type.actions]]
name = "List with 7-Zip"
slot = "enter"
tool = "7z"
args = ["l", "{file}"]

[[type.actions]]
name = "Test"
slot = "user3"
command = "archive.test"

[[type.actions]]
name = "Count"
slot = "user12"
command = "file.details"

[[type]]
id = "jar"
name = "Java archive"
short = "JAR"
parent = "zip"
extensions = ["jar"]
"#,
        )
        .unwrap();
        let types = TypeRegistry::builtin().with_config(&config);
        let slots = TypeSlots::from_config(&config);
        assert_eq!(
            slots.problems(),
            ["File type zip: action Count has no slot \"user12\"; it is listed by name."]
        );
        let jar = types.types().iter().position(|t| t.id == "jar").unwrap();
        let actions = offered(&types, &slots, u16::try_from(jar + 1).unwrap());
        let shown: Vec<(&str, Slot)> = actions.iter().map(|o| (o.name.as_str(), o.slot)).collect();
        assert_eq!(
            shown,
            [
                ("List with 7-Zip", Slot::Enter),
                ("Test", Slot::User(3)),
                ("Count", Slot::Named)
            ]
        );
        assert!(matches!(actions[0].target, ActionTarget::Tool { ref tool, .. } if tool == "7z"));
        assert_eq!(Slot::User(3).words(), "user action 3");
        assert_eq!(Slot::parse("USER0"), Some(Slot::User(0)));
        assert_eq!(Slot::parse("user10"), None);
        assert!(offered(&types, &slots, 0).is_empty());
    }

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
