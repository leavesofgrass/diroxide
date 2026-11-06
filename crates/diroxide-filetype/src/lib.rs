//! File types: detection and the registry of types with their actions.
//!
//! The type model of alpha.7 ([`tree`]) tests a file in three stages, in
//! one order: a signature in its first bytes, then an extended condition,
//! then its name; among the types hit, a child beats its parent and
//! otherwise the higher priority wins (the corrected plan 5.6). A row
//! shows the name's type at once ([`TypeRegistry::by_name`]) and "type
//! pending" only where the name says nothing, until a background check
//! reads the first bytes ([`TypeRegistry::check`]). Types and their groups
//! come built in and from `filetypes.toml`; each type has actions (open,
//! view, preview, convert, and the user's own), each a command, a tool or
//! a plugin call, never a shell string.
//!
//! The shipped table, `data/filetypes.toml`, is embedded: about 180 types
//! with their media types, and the groups (the seven of core's table and
//! the screenshots group, R29 5.6). Core's extension table stays the
//! fallback for a file no type takes and for a table that would not read.
//!
//! The flat call of phase 0, [`TypeRegistry::detect`], runs on the tree
//! too: the name and whatever first bytes the caller has.

pub mod tree;

pub use tree::{
    Condition, Found, MIN_FIXED_BYTES, NamePattern, Probe, Signature, SignatureError, Special,
    TreeError, TypeStage, TypeTree, skips_content,
};

use std::sync::OnceLock;

use diroxide_config::{FileTypeConfig, FileTypesConfig, GroupConfig};
use diroxide_core::filter::{GROUPS, Unknown, group_of_extension};
use diroxide_core::{Clock, CommandId, Entry, EntryKind, SystemClock, UtcOffset};
use diroxide_tools::{ToolRegistry, ToolStatus};

/// How many bytes from the start of a file detection reads.
pub const HEAD_BYTES: usize = 64;

/// The shipped table: the types diroxide knows and their groups, in
/// `filetypes.toml`'s own shape.
pub const SHIPPED_TABLE: &str = include_str!("../data/filetypes.toml");

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
    /// Magic bytes of phase 0's flat registry: the fixed signatures, kept
    /// for callers that read them. Detection reads [`FileType::signatures`].
    pub magic: Vec<Magic>,
    /// Among the types that match, the higher number wins; a child always
    /// beats its parent, whatever the numbers.
    pub priority: i32,
    /// The actions; the first is the default.
    pub actions: Vec<TypeAction>,
    /// The media type, such as `application/pdf`, for plugins, previews
    /// and the details view (research/metadata.md item 5).
    pub mime: Option<String>,
    /// The group it belongs to, by id: `images` (alpha.7).
    pub group: Option<String>,
    /// The type model's signatures, `??` allowed (alpha.7).
    pub signatures: Vec<Signature>,
    /// Its extended condition (alpha.7).
    pub when: Option<Condition>,
    /// Name patterns (alpha.7): globs, comma lists of globs, or `re:` and
    /// a regular expression ([`NamePattern`]).
    pub patterns: Vec<String>,
    /// Whether it belongs to the retro pack (alpha.7).
    pub retro: bool,
    /// Whether it comes from the owner's own `filetypes.toml` rather than
    /// the shipped table: Settings says "yours" beside it.
    pub yours: bool,
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
    /// Name patterns that put a file in it whatever its type
    /// ([`NamePattern`]): the screenshots group's names on each system.
    pub patterns: Vec<String>,
    /// Whether it comes from the owner's own `filetypes.toml`.
    pub yours: bool,
}

impl TypeGroup {
    fn from_config(config: &GroupConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            about: config.about.clone(),
            patterns: config.patterns.clone(),
            yours: true,
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
                    yours: false,
                }
            })
            .collect()
    }
}

/// The media types of the fallback types, by id.
const FALLBACK_MIME: [(&str, &str); 7] = [
    ("text", "text/plain"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("zip", "application/zip"),
    ("7z", "application/x-7z-compressed"),
];

impl FileType {
    /// A type of the fallback table, with fixed signatures.
    pub(crate) fn new(
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
            mime: FALLBACK_MIME
                .iter()
                .find(|(type_id, _)| *type_id == id)
                .map(|(_, mime)| (*mime).to_owned()),
            group: extensions
                .first()
                .and_then(|ext| group_of_extension(ext))
                .map(|g| g.name.to_owned()),
            signatures: magic
                .iter()
                .filter(|(offset, bytes)| offset + bytes.len() <= HEAD_BYTES)
                .map(|(offset, bytes)| Signature::fixed(*offset, bytes))
                .collect(),
            when: None,
            patterns: Vec::new(),
            retro: false,
            yours: false,
        }
    }

    /// A type read from a `[[type]]` table, and what in it could not be
    /// used, each said meaning first: a signature, a condition, a name
    /// pattern or an action that does not read is left out, and the rest
    /// of the type is kept.
    pub fn from_config(config: &FileTypeConfig) -> (Self, Vec<String>) {
        let id = &config.id;
        let mut problems = Vec::new();
        let actions = config
            .actions
            .iter()
            .filter_map(|action| {
                let target = if let Some(command) = &action.command {
                    match CommandId::new(command.clone()) {
                        Ok(command) => ActionTarget::Command(command),
                        Err(_) => {
                            problems.push(format!(
                                "File type {id}: action {} left out: \"{command}\" is not a command id.",
                                action.name
                            ));
                            return None;
                        }
                    }
                } else if let Some(tool) = &action.tool {
                    ActionTarget::Tool {
                        tool: tool.clone(),
                        args: action.args.clone(),
                    }
                } else if let Some(plugin) = &action.plugin {
                    ActionTarget::Plugin {
                        plugin: plugin.clone(),
                        command: action.args.first().cloned().unwrap_or_default(),
                    }
                } else {
                    problems.push(format!(
                        "File type {id}: action {} left out: it names no command, tool or plugin.",
                        action.name
                    ));
                    return None;
                };
                Some(TypeAction {
                    name: action.name.clone(),
                    target,
                })
            })
            .collect();
        let signatures = config
            .magic
            .iter()
            .filter_map(|m| match Signature::parse(m.at, &m.hex) {
                Ok(signature) => Some(signature),
                Err(error) => {
                    problems.push(format!("File type {id}: signature left out: {error}."));
                    None
                }
            })
            .collect::<Vec<_>>();
        let when = config
            .when
            .as_deref()
            .and_then(|text| match Condition::parse(text) {
                Ok(condition) => Some(condition),
                Err(error) => {
                    problems.push(format!(
                        "File type {id}: condition left out: {}",
                        error.line()
                    ));
                    None
                }
            });
        let patterns = config
            .patterns
            .iter()
            .filter(|p| match NamePattern::parse(p) {
                Ok(_) => true,
                Err(error) => {
                    problems.push(format!("File type {id}: pattern left out: {error}."));
                    false
                }
            })
            .cloned()
            .collect();
        let magic = signatures
            .iter()
            .filter(|s| s.bytes.iter().all(Option::is_some))
            .map(|s| Magic {
                offset: s.at,
                bytes: s.bytes.iter().flatten().copied().collect(),
            })
            .collect();
        let file_type = Self {
            id: config.id.clone(),
            name: config.name.clone(),
            short: config.short.clone(),
            parent: config.parent.clone(),
            extensions: config
                .extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_lowercase())
                .collect(),
            magic,
            priority: config.priority,
            actions,
            mime: config.mime.clone(),
            group: config.group.clone(),
            signatures,
            when,
            patterns,
            retro: config.retro,
            yours: false,
        };
        (file_type, problems)
    }
}

/// How a type was found, in phase 0's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// By the name: its extension or a name pattern.
    Extension,
    /// By the contents: a signature or a condition.
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

/// The file types, in checking order, their tree and their groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRegistry {
    groups: Vec<TypeGroup>,
    group_patterns: Vec<Vec<NamePattern>>,
    tree: TypeTree,
    problems: Vec<String>,
}

impl TypeRegistry {
    /// The built-in types and groups: the shipped table
    /// ([`SHIPPED_TABLE`]), read once. Should it ever not read whole, the
    /// fallback ([`TypeRegistry::fallback`]) stands in, and
    /// [`TypeRegistry::problems`] says why.
    pub fn builtin() -> Self {
        static SHIPPED: OnceLock<TypeRegistry> = OnceLock::new();
        SHIPPED.get_or_init(Self::shipped).clone()
    }

    /// The shipped table, read.
    fn shipped() -> Self {
        let config = match FileTypesConfig::from_toml(SHIPPED_TABLE) {
            Ok(config) => config,
            Err(error) => {
                let mut fallback = Self::fallback();
                fallback
                    .problems
                    .push(format!("Shipped file types not read: {error}"));
                return fallback;
            }
        };
        let mut problems = Vec::new();
        let types = config
            .types
            .iter()
            .map(|c| {
                let (own, found) = FileType::from_config(c);
                problems.extend(found);
                own
            })
            .collect();
        let groups = config
            .groups
            .iter()
            .map(|g| TypeGroup {
                yours: false,
                ..TypeGroup::from_config(g)
            })
            .collect();
        Self::from_parts(types, groups, problems)
    }

    /// Phase 0's twelve types over core's seven groups: what is left when
    /// the shipped table cannot be read.
    pub fn fallback() -> Self {
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
            // tar's `ustar` sits at 257, past the first 64 bytes: the name
            // says tar, and "Open as archive" reads the bytes itself.
            FileType::new("tar", "tar archive", "tar", &["tar"], &[]),
            FileType::new("mp3", "MP3 audio", "MP3", &["mp3"], &[(0, b"ID3")]),
            FileType::new("html", "web page", "HTML", &["html", "htm"], &[]),
        ];
        Self::from_parts(types, TypeGroup::builtin(), Vec::new())
    }

    /// A registry of these types, in this order (sorted, stably, by
    /// priority, higher first), with these groups. A type the tree cannot
    /// take (its parent is no type, or it is its own ancestor) is left out
    /// and said in [`TypeRegistry::problems`].
    fn from_parts(mut types: Vec<FileType>, groups: Vec<TypeGroup>, problems: Vec<String>) -> Self {
        types.sort_by_key(|t| std::cmp::Reverse(t.priority));
        let mut problems = problems;
        let tree = loop {
            match TypeTree::new(types.clone()) {
                Ok(tree) => break tree,
                Err(error) => {
                    let id = match &error {
                        TreeError::UnknownParent { type_id, .. } | TreeError::Cycle { type_id } => {
                            type_id.clone()
                        }
                    };
                    problems.push(format!("File type {id} left out: {error}"));
                    types.retain(|t| t.id != id);
                }
            }
        };
        let group_patterns = groups
            .iter()
            .map(|g| {
                g.patterns
                    .iter()
                    .filter_map(|p| match NamePattern::parse(p) {
                        Ok(pattern) => Some(pattern),
                        Err(error) => {
                            problems
                                .push(format!("File group {}: pattern left out: {error}.", g.id));
                            None
                        }
                    })
                    .collect()
            })
            .collect();
        Self {
            groups,
            group_patterns,
            tree,
            problems,
        }
    }

    /// The same registry with the user's types added before the built-in
    /// ones; a user type with a built-in id replaces it. The file's
    /// groups replace the built-in groups of the same id and add the
    /// rest. What could not be used is said in
    /// [`TypeRegistry::problems`].
    pub fn with_config(self, config: &FileTypesConfig) -> Self {
        let mut problems = self.problems;
        let mut types: Vec<FileType> = config
            .types
            .iter()
            .map(|c| {
                let (mut own, found) = FileType::from_config(c);
                own.yours = true;
                problems.extend(found);
                own
            })
            .collect();
        let mut builtin = self.tree.types().to_vec();
        builtin.retain(|b| !types.iter().any(|own| own.id == b.id));
        types.append(&mut builtin);
        let mut groups = self.groups;
        for own in &config.groups {
            let own = TypeGroup::from_config(own);
            match groups.iter_mut().find(|g| g.id == own.id) {
                Some(builtin) => *builtin = own,
                None => groups.push(own),
            }
        }
        Self::from_parts(types, groups, problems)
    }

    /// What could not be used from the types' files, each line meaning
    /// first: "File type ebook left out: Type ebook names the parent
    /// books, which no type has."
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The groups, the built-in ones first, in the Groups list's order.
    pub fn groups(&self) -> &[TypeGroup] {
        &self.groups
    }

    /// The group a type belongs to, by the type's id.
    pub fn group_of(&self, type_id: &str) -> Option<&TypeGroup> {
        let id = self.get(type_id)?.group.as_deref()?;
        self.group(id)
    }

    /// A group by id.
    pub fn group(&self, id: &str) -> Option<&TypeGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The groups an entry is in, its type's group first: the group of
    /// the type its [`Entry::type_index`] names, or else of the type its
    /// name gives, or else core's extension table's (the fallback for a
    /// file no type takes); then every group whose name patterns take
    /// its name (a screenshot is in Images and in Screenshots). Folders
    /// and plugin items are in none. [`Unknown::TypeNotChecked`] while
    /// the name says nothing and the type is not checked yet.
    pub fn groups_of(&self, entry: &Entry, clock: &dyn Clock) -> Result<Vec<&TypeGroup>, Unknown> {
        if entry.is_folder_like() || matches!(entry.kind, EntryKind::Folder | EntryKind::Item) {
            return Ok(Vec::new());
        }
        let mut pending = false;
        let file_type = match self.by_index(entry.type_index) {
            Some(file_type) => Some(file_type),
            None => match self.by_name(entry, clock) {
                Ok(found) => self.tree.types().get(found.index),
                Err(special) => {
                    pending = special == Special::Pending && entry.type_name.is_none();
                    None
                }
            },
        };
        let mut groups: Vec<&TypeGroup> = Vec::new();
        let by_type = file_type
            .and_then(|t| t.group.as_deref())
            .and_then(|id| self.group(id));
        let by_extension = || {
            entry
                .extension()
                .and_then(group_of_extension)
                .and_then(|g| self.group(g.name))
        };
        if let Some(group) = by_type.or_else(|| file_type.is_none().then(by_extension).flatten()) {
            groups.push(group);
        }
        for (group, patterns) in self.groups.iter().zip(&self.group_patterns) {
            if patterns.iter().any(|p| p.is_match(&entry.name))
                && !groups.iter().any(|g| g.id == group.id)
            {
                groups.push(group);
            }
        }
        if groups.is_empty() && pending {
            return Err(Unknown::TypeNotChecked);
        }
        Ok(groups)
    }

    /// The one group `group:` and `Lookup::group` answer for an entry:
    /// the first of [`TypeRegistry::groups_of`].
    pub fn group_for(
        &self,
        entry: &Entry,
        clock: &dyn Clock,
    ) -> Result<Option<&TypeGroup>, Unknown> {
        self.groups_of(entry, clock)
            .map(|groups| groups.first().copied())
    }

    /// The groups' ids, in the Groups list's order, the owner's own among
    /// them: the values `group:` completes to.
    pub fn group_names(&self) -> Vec<String> {
        self.groups.iter().map(|g| g.id.clone()).collect()
    }

    /// The Groups list's summary: "Groups: 8 built in, 2 yours".
    pub fn groups_line(&self) -> String {
        let yours = self.groups.iter().filter(|g| g.yours).count();
        let built_in = self.groups.len() - yours;
        if yours == 0 {
            format!("Groups: {built_in} built in")
        } else {
            format!("Groups: {built_in} built in, {yours} yours")
        }
    }

    /// The line when an entry's name and its contents disagree: "Name
    /// says PNG; contents say JPEG". `None` when they agree, when the
    /// contents only refine the name (a zip that is a Word document) or
    /// the name refines the contents (a camera's raw file that is TIFF
    /// inside), and when the name gave nothing.
    pub fn disagreement(&self, by_name: &Found, checked: &Found) -> Option<String> {
        let related = by_name.index == checked.index
            || self.tree.is_ancestor(by_name.index, checked.index)
            || self.tree.is_ancestor(checked.index, by_name.index);
        if related || checked.stage == TypeStage::Name {
            return None;
        }
        let name = self.tree.types().get(by_name.index)?;
        let contents = self.tree.types().get(checked.index)?;
        Some(format!(
            "Name says {}; contents say {}",
            name.short, contents.short
        ))
    }

    /// The types as the type model's tree (alpha.7).
    pub fn tree(&self) -> Result<TypeTree, TreeError> {
        Ok(self.tree.clone())
    }

    /// Every type, in checking order.
    pub fn types(&self) -> &[FileType] {
        self.tree.types()
    }

    /// A type by id.
    pub fn get(&self, id: &str) -> Option<&FileType> {
        self.tree.types().iter().find(|t| t.id == id)
    }

    /// The entry's type with whatever its probe holds: the background
    /// check's call, with the first bytes read (at most [`HEAD_BYTES`]
    /// are looked at) or `None` while they are not.
    pub fn check(&self, probe: &Probe<'_>, clock: &dyn Clock) -> Result<Found, Special> {
        self.tree.detect(probe, clock)
    }

    /// The entry's type by its name and kind alone, what a row shows at
    /// once: [`Special::Pending`] where the name says nothing and the
    /// contents are still to be read.
    pub fn by_name(&self, entry: &Entry, clock: &dyn Clock) -> Result<Found, Special> {
        self.check(&Probe::new(entry, None), clock)
    }

    /// The type an [`Entry::type_index`] names: its place in this
    /// registry plus one, 0 for none.
    pub fn by_index(&self, index: u16) -> Option<&FileType> {
        usize::from(index)
            .checked_sub(1)
            .and_then(|i| self.tree.types().get(i))
    }

    /// The [`Entry::type_index`] for a type found here: its place plus
    /// one, or 0 past what two bytes hold.
    pub fn index_of(found: &Found) -> u16 {
        u16::try_from(found.index + 1).unwrap_or(0)
    }

    /// Detects a type from a name and the first bytes of the file (empty
    /// when not read yet), on the tree: the bytes' stages first, then the
    /// name.
    pub fn detect(&self, name: &str, head: &[u8]) -> Option<Detection> {
        let entry = Entry::new(name, EntryKind::File);
        let probe = Probe {
            entry: &entry,
            head: (!head.is_empty()).then_some(head),
            skip_content: false,
        };
        let found = self
            .tree
            .detect(&probe, &SystemClock::new(UtcOffset::UTC))
            .ok()?;
        Some(Detection {
            stage: match found.stage {
                TypeStage::Name => Stage::Extension,
                TypeStage::Signature | TypeStage::Condition => Stage::Magic,
            },
            type_id: found.type_id,
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
    use diroxide_core::{FixedClock, Timestamp};

    use super::*;

    fn clock() -> FixedClock {
        let now = Timestamp::from_civil(2026, 11, 6, 4, 0, 0, UtcOffset::UTC).unwrap();
        FixedClock::new(now, UtcOffset::UTC)
    }

    #[test]
    fn built_in_types_carry_their_media_types() {
        let types = TypeRegistry::builtin();
        assert_eq!(
            types.get("pdf").and_then(|t| t.mime.as_deref()),
            Some("application/pdf")
        );
    }

    /// The flat call on the tree: the contents decide where they speak,
    /// the name where they do not.
    #[test]
    fn detect_runs_on_the_tree() {
        let types = TypeRegistry::builtin();
        let by_name = types.detect("Report.PDF", &[]).unwrap();
        assert_eq!(
            (by_name.type_id.as_str(), by_name.stage),
            ("pdf", Stage::Extension)
        );
        let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00";
        let by_magic = types.detect("noextension", png).unwrap();
        assert_eq!(
            (by_magic.type_id.as_str(), by_magic.stage),
            ("png", Stage::Magic)
        );
        let both = types.detect("photo.png", png).unwrap();
        assert_eq!(both.stage, Stage::Magic, "the signature is the hit");
        let cut = types.detect("photo.png", b"\x89PNG").unwrap();
        assert_eq!(cut.stage, Stage::Extension, "a head too short to tell");
        let liar = types.detect("photo.png", b"\xFF\xD8\xFF\xE0").unwrap();
        assert_eq!(liar.type_id, "jpeg", "the bytes beat the name");
        assert!(types.detect("mystery", b"nothing").is_none());
        assert_eq!(types.detect("backup.tar", &[]).unwrap().type_id, "tar");
        let mut tar_head = vec![0u8; 262];
        tar_head[257..].copy_from_slice(b"ustar");
        assert!(
            types.detect("x", &tar_head).is_none(),
            "only the first 64 bytes are read"
        );
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
        assert!(types.get("text").unwrap().yours);
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

    /// What a user's file gets wrong is said, meaning first, and the rest
    /// is used.
    #[test]
    fn a_user_type_that_does_not_read_is_said() {
        let config = FileTypesConfig::from_toml(
            r#"
[[type]]
id = "orphan"
name = "orphan"
short = "orphan"
parent = "nothing"
extensions = ["orph"]

[[type]]
id = "half"
name = "half right"
short = "half"
extensions = ["half"]
magic = [{ at = 0, hex = "ZZ" }, { at = 0, hex = "48 41 4C 46" }]
when = "size >"
patterns = ["[a"]

[[type.actions]]
name = "Nothing"
"#,
        )
        .unwrap();
        let types = TypeRegistry::builtin().with_config(&config);
        assert!(types.get("orphan").is_none());
        let half = types.get("half").unwrap();
        assert_eq!(half.signatures.len(), 1);
        assert!(half.when.is_none());
        assert!(half.patterns.is_empty());
        assert!(half.actions.is_empty());
        let problems = types.problems();
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert_eq!(
            problems[0],
            "File type half: action Nothing left out: it names no command, tool or plugin."
        );
        for line in problems {
            assert!(line.starts_with("File type "), "{line}");
        }
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("File type orphan left out"))
        );
    }

    /// The background check's calls: the name's type at once, the
    /// contents' type after, and the two-byte index both ways.
    #[test]
    fn by_name_check_and_the_index() {
        let types = TypeRegistry::builtin();
        let clock = clock();
        let report = Entry::new("report.pdf", EntryKind::File).with_size(900);
        let found = types.by_name(&report, &clock).unwrap();
        assert_eq!(
            (found.type_id.as_str(), found.stage),
            ("pdf", TypeStage::Name)
        );
        let index = TypeRegistry::index_of(&found);
        assert_eq!(types.by_index(index).unwrap().id, "pdf");
        assert!(types.by_index(0).is_none());
        let mystery = Entry::new("mystery", EntryKind::File).with_size(900);
        assert_eq!(types.by_name(&mystery, &clock), Err(Special::Pending));
        let head: &[u8] = b"%PDF-1.7\n";
        let checked = types.check(&Probe::new(&mystery, Some(head)), &clock);
        assert_eq!(checked.unwrap().stage, TypeStage::Signature);
        let skipped = Probe::new(&mystery, Some(head)).skipping(true);
        assert_eq!(types.check(&skipped, &clock), Err(Special::Unknown));
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
