//! File types: detection and the registry of types with their actions.
//!
//! The type model of alpha.7 ([`tree`]) tests a file in three stages, in
//! one order: a signature in its first bytes, then an extended condition,
//! then its name; among the types hit, a child beats its parent and
//! otherwise the higher priority wins (the corrected plan 5.6). A row
//! shows the name's type at once and "type pending" only where the name
//! says nothing, until a background check reads the first bytes. Types
//! and their groups come built in and from `filetypes.toml`; each type has
//! actions (open, view, preview, convert, and the user's own), each a
//! command, a tool or a plugin call, never a shell string.
//!
//! Phase 0's flat registry ([`TypeRegistry::detect`]: the extension, then
//! the first bytes) stays until the type model's lane (alpha 4, commit
//! 24) moves it onto the tree.

pub mod tree;

pub use tree::{
    Condition, Found, Probe, Signature, SignatureError, Special, TreeError, TypeStage, TypeTree,
};

use diroxide_config::{FileTypeConfig, FileTypesConfig, GroupConfig};
use diroxide_core::filter::GROUPS;
use diroxide_core::{CommandId, Entry, EntryKind};
use diroxide_tools::{ToolRegistry, ToolStatus};

/// How many bytes from the start of a file detection reads.
pub const HEAD_BYTES: usize = 64;

/// Bytes at an offset that mark a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Magic {
    /// Where the bytes start.
    pub offset: usize,
    /// The bytes.
    pub bytes: Vec<u8>,
}

impl Magic {
    /// Whether the head of a file matches.
    pub fn matches(&self, head: &[u8]) -> bool {
        head.get(self.offset..self.offset + self.bytes.len()) == Some(self.bytes.as_slice())
    }
}

/// What an action does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionTarget {
    /// Run a command.
    Command(CommandId),
    /// Run a registered tool with arguments (placeholders such as `{file}`).
    Tool {
        /// The tool's id.
        tool: String,
        /// The arguments.
        args: Vec<String>,
    },
    /// Call a plugin's command.
    Plugin {
        /// The plugin's name.
        plugin: String,
        /// The plugin's command id.
        command: String,
    },
}

/// An action of a file type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAction {
    /// The name, as listed: "Open", "Make thumbnail".
    pub name: String,
    /// What it does.
    pub target: ActionTarget,
}

/// A file type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileType {
    /// A stable id: `pdf`.
    pub id: String,
    /// The name said in details: "PDF document".
    pub name: String,
    /// The short name for rows: "PDF".
    pub short: String,
    /// The type this one refines.
    pub parent: Option<String>,
    /// Extensions, lowercase, without the dot.
    pub extensions: Vec<String>,
    /// Magic byte signatures.
    pub magic: Vec<Magic>,
    /// Higher is checked first.
    pub priority: i32,
    /// The actions; the first is the default.
    pub actions: Vec<TypeAction>,
    /// The media type, such as `application/pdf`, for plugins, previews
    /// and the details view (research/metadata.md item 5).
    pub mime: Option<String>,
    /// The group it belongs to, by id: `images` (alpha.7).
    pub group: Option<String>,
    /// The type model's signatures, `??` allowed (alpha.7); the built-in
    /// table's `magic` as signatures, and the config's `magic`.
    pub signatures: Vec<Signature>,
    /// Its extended condition (alpha.7).
    pub when: Option<Condition>,
    /// Name patterns, globs (alpha.7).
    pub patterns: Vec<String>,
    /// Whether it belongs to the retro pack (alpha.7).
    pub retro: bool,
}

/// A group of file types: what Select by group, `group:`, actions,
/// recipes and theme words name (alpha.7, R28 4.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeGroup {
    /// Its id, as typed in `group:`: `images`.
    pub id: String,
    /// Its name as said and listed: "Images".
    pub name: String,
    /// What it holds, in one sentence.
    pub about: String,
    /// Name patterns that put a file in it whatever its type.
    pub patterns: Vec<String>,
}

impl TypeGroup {
    fn from_config(config: &GroupConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            about: config.about.clone(),
            patterns: config.patterns.clone(),
        }
    }

    /// The seven built-in groups of core's table, the fallback when no
    /// `filetypes.toml` names any.
    pub fn builtin() -> Vec<Self> {
        GROUPS
            .iter()
            .map(|g| {
                let mut name = g.name.to_owned();
                if let Some(first) = name.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                Self {
                    id: g.name.to_owned(),
                    name,
                    about: g.description.to_owned(),
                    patterns: Vec::new(),
                }
            })
            .collect()
    }
}

/// The media types of the built-in types, by id.
const BUILTIN_MIME: [(&str, &str); 7] = [
    ("text", "text/plain"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("zip", "application/zip"),
    ("7z", "application/x-7z-compressed"),
];

impl FileType {
    fn new(
        id: &str,
        name: &str,
        short: &str,
        extensions: &[&str],
        magic: &[(usize, &[u8])],
    ) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            short: short.to_owned(),
            parent: None,
            extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
            magic: magic
                .iter()
                .map(|(offset, bytes)| Magic {
                    offset: *offset,
                    bytes: bytes.to_vec(),
                })
                .collect(),
            priority: 0,
            actions: Vec::new(),
            mime: BUILTIN_MIME
                .iter()
                .find(|(type_id, _)| *type_id == id)
                .map(|(_, mime)| (*mime).to_owned()),
            group: extensions
                .first()
                .and_then(|ext| diroxide_core::filter::group_of_extension(ext))
                .map(|g| g.name.to_owned()),
            // The table's tar magic sits past the type model's 64 bytes;
            // the type model's lane moves it into a container test.
            signatures: magic
                .iter()
                .filter(|(offset, bytes)| offset + bytes.len() <= HEAD_BYTES)
                .map(|(offset, bytes)| Signature::fixed(*offset, bytes))
                .collect(),
            when: None,
            patterns: Vec::new(),
            retro: false,
        }
    }

    fn from_config(config: &FileTypeConfig) -> Self {
        let actions = config
            .actions
            .iter()
            .filter_map(|action| {
                let target = if let Some(command) = &action.command {
                    ActionTarget::Command(CommandId::new(command.clone()).ok()?)
                } else if let Some(tool) = &action.tool {
                    ActionTarget::Tool {
                        tool: tool.clone(),
                        args: action.args.clone(),
                    }
                } else {
                    ActionTarget::Plugin {
                        plugin: action.plugin.clone()?,
                        command: action.args.first().cloned().unwrap_or_default(),
                    }
                };
                Some(TypeAction {
                    name: action.name.clone(),
                    target,
                })
            })
            .collect();
        // Naive: a signature or a condition that does not read is left
        // out; the type model's lane says why, in words, when the file
        // loads.
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            short: config.short.clone(),
            parent: config.parent.clone(),
            extensions: config.extensions.iter().map(|e| e.to_lowercase()).collect(),
            magic: Vec::new(),
            priority: config.priority,
            actions,
            mime: config.mime.clone(),
            group: config.group.clone(),
            signatures: config
                .magic
                .iter()
                .filter_map(|m| Signature::parse(m.at, &m.hex).ok())
                .collect(),
            when: config
                .when
                .as_deref()
                .and_then(|w| Condition::parse(w).ok()),
            patterns: config.patterns.clone(),
            retro: config.retro,
        }
    }
}

/// How a type was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// By the name's extension.
    Extension,
    /// By magic bytes.
    Magic,
}

/// A detected type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    /// The type's id.
    pub type_id: String,
    /// How it was found.
    pub stage: Stage,
}

/// The file types, user types first, and their groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRegistry {
    types: Vec<FileType>,
    groups: Vec<TypeGroup>,
}

impl TypeRegistry {
    /// The built-in types.
    pub fn builtin() -> Self {
        let types = vec![
            FileType::new(
                "text",
                "text file",
                "text",
                &["txt", "log", "md", "csv"],
                &[],
            ),
            FileType::new("pdf", "PDF document", "PDF", &["pdf"], &[(0, b"%PDF")]),
            FileType::new("png", "PNG image", "PNG", &["png"], &[(0, b"\x89PNG")]),
            FileType::new(
                "jpeg",
                "JPEG image",
                "JPEG",
                &["jpg", "jpeg"],
                &[(0, b"\xFF\xD8\xFF")],
            ),
            FileType::new("gif", "GIF image", "GIF", &["gif"], &[(0, b"GIF8")]),
            FileType::new("zip", "zip archive", "zip", &["zip"], &[(0, b"PK\x03\x04")]),
            FileType::new(
                "7z",
                "7z archive",
                "7z",
                &["7z"],
                &[(0, b"7z\xBC\xAF\x27\x1C")],
            ),
            FileType::new(
                "gzip",
                "gzip file",
                "gzip",
                &["gz", "tgz"],
                &[(0, b"\x1F\x8B")],
            ),
            FileType::new(
                "xz",
                "xz file",
                "xz",
                &["xz", "txz"],
                &[(0, b"\xFD7zXZ\x00")],
            ),
            FileType::new("tar", "tar archive", "tar", &["tar"], &[(257, b"ustar")]),
            FileType::new("mp3", "MP3 audio", "MP3", &["mp3"], &[(0, b"ID3")]),
            FileType::new("html", "web page", "HTML", &["html", "htm"], &[]),
        ];
        Self {
            types,
            groups: TypeGroup::builtin(),
        }
    }

    /// The same registry with the user's types added before the built-in
    /// ones; a user type with a built-in id replaces it. The file's
    /// groups replace the built-in groups of the same id and add the
    /// rest.
    pub fn with_config(mut self, config: &FileTypesConfig) -> Self {
        let mut types: Vec<FileType> = config.types.iter().map(FileType::from_config).collect();
        self.types
            .retain(|builtin| !types.iter().any(|own| own.id == builtin.id));
        types.append(&mut self.types);
        types.sort_by_key(|t| std::cmp::Reverse(t.priority));
        self.types = types;
        for own in &config.groups {
            let own = TypeGroup::from_config(own);
            match self.groups.iter_mut().find(|g| g.id == own.id) {
                Some(builtin) => *builtin = own,
                None => self.groups.push(own),
            }
        }
        self
    }

    /// The groups, the built-in seven first, in the Groups list's order.
    pub fn groups(&self) -> &[TypeGroup] {
        &self.groups
    }

    /// The group a type belongs to, by the type's id.
    pub fn group_of(&self, type_id: &str) -> Option<&TypeGroup> {
        let id = self.get(type_id)?.group.as_deref()?;
        self.groups.iter().find(|g| g.id == id)
    }

    /// The types as the type model's tree (alpha.7), or why they do not
    /// make one.
    pub fn tree(&self) -> Result<TypeTree, TreeError> {
        TypeTree::new(self.types.clone())
    }

    /// Every type, in checking order.
    pub fn types(&self) -> &[FileType] {
        &self.types
    }

    /// A type by id.
    pub fn get(&self, id: &str) -> Option<&FileType> {
        self.types.iter().find(|t| t.id == id)
    }

    /// Detects a type from a name and the first bytes of the file (empty
    /// when not read yet): the extension first, then magic.
    pub fn detect(&self, name: &str, head: &[u8]) -> Option<Detection> {
        let lower = name.to_lowercase();
        if let Some((_, ext)) = lower.rsplit_once('.')
            && let Some(found) = self
                .types
                .iter()
                .find(|t| t.extensions.iter().any(|e| e == ext))
        {
            return Some(Detection {
                type_id: found.id.clone(),
                stage: Stage::Extension,
            });
        }
        self.types
            .iter()
            .find(|t| t.magic.iter().any(|m| m.matches(head)))
            .map(|found| Detection {
                type_id: found.id.clone(),
                stage: Stage::Magic,
            })
    }

    /// The short type name for an entry's row: the detected type, "folder"
    /// for folders, or `None` when not yet checked.
    pub fn short_name(&self, entry: &Entry) -> Option<String> {
        match entry.kind {
            EntryKind::Folder => Some("folder".to_owned()),
            _ => self
                .detect(&entry.name, &[])
                .and_then(|d| self.get(&d.type_id))
                .map(|t| t.short.clone()),
        }
    }

    /// Whether an action can run here: tool actions need their tool.
    pub fn action_available(action: &TypeAction, tools: &ToolRegistry) -> bool {
        match &action.target {
            ActionTarget::Tool { tool, .. } => matches!(tools.find(tool), Ok(ToolStatus::Found(_))),
            ActionTarget::Command(_) | ActionTarget::Plugin { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_types_carry_their_media_types() {
        let types = TypeRegistry::builtin();
        assert_eq!(
            types.get("pdf").and_then(|t| t.mime.as_deref()),
            Some("application/pdf")
        );
    }

    #[test]
    fn extension_first_then_magic() {
        let types = TypeRegistry::builtin();
        let by_name = types.detect("Report.PDF", &[]).unwrap();
        assert_eq!(
            (by_name.type_id.as_str(), by_name.stage),
            ("pdf", Stage::Extension)
        );
        let by_magic = types.detect("noextension", b"\x89PNG\r\n").unwrap();
        assert_eq!(
            (by_magic.type_id.as_str(), by_magic.stage),
            ("png", Stage::Magic)
        );
        assert!(types.detect("mystery", b"nothing").is_none());
        let mut tar_head = vec![0u8; 262];
        tar_head[257..].copy_from_slice(b"ustar");
        assert_eq!(types.detect("x", &tar_head).unwrap().type_id, "tar");
    }

    #[test]
    fn user_types_come_first_and_replace() {
        let config = FileTypesConfig {
            types: vec![FileTypeConfig {
                id: "text".into(),
                name: "notes".into(),
                short: "notes".into(),
                extensions: vec!["TXT".into()],
                ..FileTypeConfig::default()
            }],
            ..FileTypesConfig::default()
        };
        let types = TypeRegistry::builtin().with_config(&config);
        assert_eq!(types.get("text").unwrap().short, "notes");
        assert_eq!(types.types().iter().filter(|t| t.id == "text").count(), 1);
        let entry = Entry::new("a.txt", EntryKind::File);
        assert_eq!(types.short_name(&entry).as_deref(), Some("notes"));
        assert_eq!(
            types
                .short_name(&Entry::new("x", EntryKind::Folder))
                .as_deref(),
            Some("folder")
        );
    }

    #[test]
    fn tool_actions_need_their_tool() {
        let tools = ToolRegistry::builtin().with_path(None);
        let action = TypeAction {
            name: "Make thumbnail".into(),
            target: ActionTarget::Tool {
                tool: "magick".into(),
                args: vec!["{file}".into()],
            },
        };
        assert!(!TypeRegistry::action_available(&action, &tools));
    }
}
