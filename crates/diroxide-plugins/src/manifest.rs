//! `plugin.toml`, version 1: what a plugin is, what it provides, and what
//! it may do (R3's review, section 4, with R22's listers, columns and place
//! descriptions).
//!
//! ```toml
//! [plugin]
//! id = "line-counter"
//! name = "Line counter"
//! version = "0.1.0"
//! protocol = 1
//!
//! [run]
//! command = ["python3", "main.py"]
//!
//! [run.windows]
//! command = ["py", "-3", "main.py"]
//!
//! [permissions]
//! fs = "read"
//!
//! [[command]]
//! id = "count_lines"
//! name = "Count lines in files"
//! ```
//!
//! [`Manifest::check`] reads a manifest field by field and returns every
//! problem it finds, each naming its field and the fix: errors refuse the
//! manifest; warnings (unknown keys, which are kept and ignored, and the
//! accessibility lint) do not. [`Manifest::from_toml`] is the strict form
//! the host uses.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use diroxide_core::names::has_hidden;
use diroxide_core::pattern::Glob;
use diroxide_core::{CommandId, Key, NameCase};
use toml::{Table, Value};

use crate::error::PluginError;
use crate::protocol::{MAX_LINE_BYTES, MAX_LINE_BYTES_LIMIT, PROTOCOL};

/// The longest name, label or header, in characters: one Braille line.
pub const MAX_NAME_CHARS: usize = 40;

/// The longest declared statement (`permissions.declared`), in characters.
pub const MAX_DECLARED_CHARS: usize = 80;

/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 200;

/// The value of `permissions.declared.programs` that allows an interpreter
/// or a shell among the tools, and makes the consent text say so.
pub const ANY_PROGRAM: &str = "any program";

/// Tool names that run code of any kind: refused in `permissions.tools`
/// and `permissions.launch` unless the plugin declares [`ANY_PROGRAM`]
/// (R22, section 7.1). `python` covers `python3` and the like.
pub const INTERPRETERS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "cmd",
    "powershell",
    "pwsh",
    "python",
    "py",
    "perl",
    "ruby",
    "node",
    "deno",
    "bun",
    "lua",
    "env",
    "xargs",
    "find",
    "wsl",
];

/// Words a description should not start with: the meaning comes first.
const FILLER: &[&str] = &[
    "this plugin",
    "this command",
    "a plugin",
    "plugin that",
    "allows you to",
    "lets you",
    "used to",
    "this is",
];

/// How serious a problem is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The manifest is refused.
    Error,
    /// The manifest is used; the problem is reported by `diro plugin
    /// check`.
    Warning,
}

/// One problem in a manifest: the field, what is wrong, and the fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// Error or warning.
    pub severity: Severity,
    /// The field, such as `plugin.id` or `command[2].key`.
    pub field: String,
    /// What is wrong, meaning first, without a final period.
    pub message: String,
    /// How to fix it, one sentence.
    pub fix: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
        };
        write!(
            f,
            "{severity}, {}: {}. {}",
            self.field, self.message, self.fix
        )
    }
}

/// What [`Manifest::check`] found: the manifest, when it has no errors, and
/// every problem.
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestReport {
    /// The manifest, when there is no error.
    pub manifest: Option<Manifest>,
    /// Every problem, errors first, in the order found.
    pub problems: Vec<Problem>,
}

impl ManifestReport {
    /// The errors.
    pub fn errors(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
    }

    /// The warnings.
    pub fn warnings(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity == Severity::Warning)
    }

    /// The report as lines: "Pass." when there is nothing to say, else one
    /// line per problem.
    pub fn text(&self) -> String {
        if self.problems.is_empty() {
            return "Pass.\n".to_owned();
        }
        let mut text = String::new();
        for problem in &self.problems {
            text.push_str(&problem.to_string());
            text.push('\n');
        }
        text
    }
}

/// A limit in time: a duration, or none.
pub type Limit = Option<Duration>;

/// When the host starts the plugin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Start {
    /// The first time something it contributes is used.
    #[default]
    OnDemand,
    /// With the app.
    Startup,
}

/// How to run a process plugin, for one system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The program and its arguments, never a shell line.
    pub command: Vec<String>,
    /// The working folder, relative to the plugin's folder.
    pub cwd: String,
    /// Extra environment variables.
    pub env: BTreeMap<String, String>,
    /// When it starts.
    pub start: Start,
    /// How many host requests may be in flight at once, 1 to 16.
    pub concurrency: u32,
    /// How long it may stay idle before the host stops it; `None` never.
    pub idle_timeout: Limit,
    /// How long `initialize` may take.
    pub startup_timeout: Duration,
    /// How long `shutdown` may take before the host goes on.
    pub shutdown_grace: Duration,
    /// The longest line it may send or receive, in bytes.
    pub max_message_bytes: usize,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            command: Vec::new(),
            cwd: ".".to_owned(),
            env: BTreeMap::new(),
            start: Start::OnDemand,
            concurrency: 1,
            idle_timeout: Some(Duration::from_secs(300)),
            startup_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(3),
            max_message_bytes: MAX_LINE_BYTES,
        }
    }
}

/// The inactivity limits of each kind of request (R3, section 6.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// Provider calls: 30 seconds.
    pub request: Limit,
    /// Previews: 5 seconds.
    pub preview: Limit,
    /// `before_*` hooks: 2 seconds.
    pub hook: Limit,
    /// Commands: none, they are jobs the owner can cancel.
    pub command: Limit,
    /// Conversions: none.
    pub convert: Limit,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            request: Some(Duration::from_secs(30)),
            preview: Some(Duration::from_secs(5)),
            hook: Some(Duration::from_secs(2)),
            command: None,
            convert: None,
        }
    }
}

/// A level of access through the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    /// None.
    #[default]
    None,
    /// Reading.
    Read,
    /// Reading and writing.
    Write,
}

impl Access {
    /// The word in the manifest.
    pub const fn word(self) -> &'static str {
        match self {
            Access::None => "none",
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// What the plugin states about itself, shown at enable and never checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declared {
    /// What it does on the network.
    pub network: Option<String>,
    /// Which files it reads and writes on its own.
    pub files: Option<String>,
    /// Which programs it starts on its own.
    pub programs: Option<String>,
}

/// What the host lets the plugin do (enforced) and what it states.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    /// Files through the host's `fs/*` services.
    pub fs: Access,
    /// The panels through `panel/*`.
    pub panel: Access,
    /// Tools `tools/run` may run, by registry name.
    pub tools: Vec<String>,
    /// Command id patterns `command/execute` may run; `*` matches a word.
    pub commands: Vec<String>,
    /// Programs `tools/launch` may start detached.
    pub launch: Vec<String>,
    /// What the plugin states, unchecked.
    pub declared: Declared,
}

impl Permissions {
    /// Whether the plugin declared that it may run any program.
    pub fn any_program(&self) -> bool {
        self.declared.programs.as_deref() == Some(ANY_PROGRAM)
    }

    /// Whether `command/execute` may run a command id.
    pub fn allows_command(&self, id: &str) -> bool {
        self.commands
            .iter()
            .any(|pattern| command_matches(pattern, id))
    }
}

/// Whether a command pattern matches an id: words by dots, `*` matching one
/// word, and a final `*` matching the rest.
pub fn command_matches(pattern: &str, id: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('.').collect();
    let id: Vec<&str> = id.split('.').collect();
    for (i, word) in pattern.iter().enumerate() {
        if *word == "*" && i == pattern.len() - 1 {
            return id.len() > i;
        }
        match id.get(i) {
            Some(part) if *word == "*" || word == part => continue,
            _ => return false,
        }
    }
    pattern.len() == id.len()
}

/// How many entries a command needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    /// None.
    None,
    /// Exactly one.
    One,
    /// Exactly two.
    Two,
    /// Any, the cursor counting when nothing is marked.
    #[default]
    Any,
    /// Two or more.
    Many,
}

/// Which places a command works on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Places {
    /// Any provider's.
    #[default]
    Any,
    /// Local files only.
    Local,
}

/// An operating system a command exists on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Os {
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// macOS.
    Macos,
}

impl Os {
    /// The system this build runs on.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }
}

/// A command a plugin adds.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandSpec {
    /// The plugin-local id, such as `count_lines`; the full id is the
    /// plugin's id, a dot, and this.
    pub id: String,
    /// Its name in menus and the palette, 40 characters at most.
    pub name: String,
    /// One sentence for F1 help.
    pub description: String,
    /// A default key, such as `ctrl+alt+l`.
    pub key: Option<String>,
    /// The menu path; `Plugins` by default.
    pub menu: String,
    /// How many entries it needs.
    pub selection: Selection,
    /// Which places it works on.
    pub places: Places,
    /// Tools it needs; it is shown disabled with the reason when one is
    /// missing.
    pub needs: Vec<String>,
    /// The systems it exists on; empty means every system.
    pub os: Vec<Os>,
    /// For a manifest-only command: the program and its arguments, with
    /// placeholders.
    pub argv: Vec<String>,
    /// For a manifest-only command: how it runs (the script command modes).
    pub mode: Option<String>,
    /// Further keys of the script-command family, kept for the script
    /// runner: `prompt`, `confirm`, `unattended` and the rest.
    pub extra: Table,
}

impl CommandSpec {
    /// The full command id: `line-counter.count_lines`.
    pub fn full_id(&self, plugin: &str) -> String {
        format!("{plugin}.{}", self.id)
    }
}

/// A file system a plugin provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSpec {
    /// The scheme, such as `s3`.
    pub scheme: String,
    /// Its name in words.
    pub name: String,
    /// The path separator; `/` by default.
    pub separator: String,
    /// The capabilities that are booleans, by name, absent meaning false:
    /// `case_sensitive`, `writable`, `trash`, `links`, `attributes`,
    /// `watch`, `server_side_copy`, `atomic_rename`.
    pub flags: BTreeSet<String>,
    /// The longest name it takes, if it says.
    pub max_name_length: Option<u32>,
    /// Names it refuses.
    pub reserved_names: Vec<String>,
    /// The provider methods it implements, without the `provider/` prefix;
    /// `list` and `stat` are always there.
    pub methods: BTreeSet<String>,
}

/// A previewer.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewerSpec {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// What it previews: `mime`, `extensions`, `types`, `schemes` and
    /// `max_size`, as written.
    pub matches: Table,
    /// `low`, `normal` or `high`.
    pub priority: String,
    /// For a manifest-only previewer: the program and its arguments.
    pub argv: Vec<String>,
}

/// A converter.
#[derive(Clone, Debug, PartialEq)]
pub struct ConverterSpec {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// What it converts from, as written.
    pub from: Table,
    /// The outputs it makes, as written.
    pub to: Vec<Table>,
    /// Tools it needs.
    pub needs: Vec<String>,
    /// For a manifest-only converter: the program and its arguments.
    pub argv: Vec<String>,
}

/// The events a plugin can hook.
pub const HOOK_EVENTS: &[&str] = &[
    "before_copy",
    "before_move",
    "before_delete",
    "before_rename",
    "before_mkdir",
    "after_copy",
    "after_move",
    "after_delete",
    "after_rename",
    "after_mkdir",
    "enter_folder",
    "select",
    "folder_changed",
];

/// What a `before_*` hook's silence means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnTimeout {
    /// Go on, with a warning (the default).
    #[default]
    Continue,
    /// Cancel the job.
    Cancel,
}

/// A hook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookSpec {
    /// The event, such as `before_delete`.
    pub event: String,
    /// The schemes it is woken for; empty means all.
    pub schemes: Vec<String>,
    /// A glob on the place, with `/` separators.
    pub glob: Option<String>,
    /// `file` or `folder`, when only one kind wakes it.
    pub kind: Option<String>,
    /// What silence means, for `before_*` events.
    pub on_timeout: OnTimeout,
    /// Its own timeout, when it differs from `[timeouts] hook`.
    pub timeout: Option<Limit>,
}

impl HookSpec {
    /// Whether the event is a `before_*` one, which waits for an answer.
    pub fn is_before(&self) -> bool {
        self.event.starts_with("before_")
    }
}

/// A command bound to a file type's action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiletypeAction {
    /// The file type's id.
    pub file_type: String,
    /// `open`, `view`, `preview`, `convert`, or a user slot.
    pub action: String,
    /// The plugin-local command id.
    pub command: String,
}

/// The kinds of a setting.
pub const SETTING_TYPES: &[&str] = &["bool", "int", "string", "choice", "path", "argv"];

/// A setting the owner can change.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    /// Its key, in `snake_case`.
    pub key: String,
    /// `bool`, `int`, `string`, `choice`, `path` or `argv`.
    pub kind: String,
    /// Its default.
    pub default: Value,
    /// One sentence for F1 help.
    pub description: String,
    /// The choices, for `choice`.
    pub choices: Vec<String>,
}

/// The kinds of a column's values.
pub const COLUMN_KINDS: &[&str] = &["text", "number", "time", "size"];

/// A column of a lister.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListerColumn {
    /// Its id, the key of the rows' `extra` values.
    pub id: String,
    /// Its header in words.
    pub header: String,
    /// `text`, `number`, `time` or `size`.
    pub kind: String,
    /// Whether it sorts.
    pub sorts: bool,
}

/// An action on a lister's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListerAction {
    /// The key.
    pub key: String,
    /// The plugin-local command it runs.
    pub command: String,
    /// How many rows it needs.
    pub selection: Selection,
}

/// A list of rows that are not files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListerSpec {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its columns.
    pub columns: Vec<ListerColumn>,
    /// The plugin-local command Enter runs.
    pub activate: Option<String>,
    /// More actions by key.
    pub actions: Vec<ListerAction>,
}

/// A column the plugin adds to file rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnSpec {
    /// Its id.
    pub id: String,
    /// Its header in words.
    pub header: String,
    /// `text`, `number`, `time` or `size`.
    pub kind: String,
    /// The schemes or globs it applies to; empty means all.
    pub applies_to: Vec<String>,
    /// How many places it takes in a batch, at most 500.
    pub batch: u32,
}

/// Which places a plugin describes for the title line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceInfoSpec {
    /// A scheme it describes.
    pub scheme: Option<String>,
    /// A file or folder found by walking up, such as `.git`.
    pub marker: Option<String>,
}

/// A plugin's manifest, version 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// The id: lowercase letters, digits, `-` and `_`; the folder name and
    /// the prefix of every command id.
    pub id: String,
    /// The spoken name, 40 characters at most.
    pub name: String,
    /// Its version, semver.
    pub version: String,
    /// One sentence about it.
    pub description: String,
    /// Who wrote it.
    pub authors: Vec<String>,
    /// Its license.
    pub license: Option<String>,
    /// Its home page.
    pub homepage: Option<String>,
    /// The protocol's major version.
    pub protocol: u32,
    /// The oldest diroxide it runs with.
    pub min_host: Option<String>,
    /// How to run it on this system; `None` for a manifest-only plugin.
    pub run: Option<Run>,
    /// Its timeouts.
    pub timeouts: Timeouts,
    /// Its permissions.
    pub permissions: Permissions,
    /// Its commands.
    pub commands: Vec<CommandSpec>,
    /// Its providers.
    pub providers: Vec<ProviderSpec>,
    /// Its previewers.
    pub previewers: Vec<PreviewerSpec>,
    /// Its converters.
    pub converters: Vec<ConverterSpec>,
    /// Its hooks.
    pub hooks: Vec<HookSpec>,
    /// Its file-type actions.
    pub filetype_actions: Vec<FiletypeAction>,
    /// Its settings.
    pub settings: Vec<SettingSpec>,
    /// Its listers.
    pub listers: Vec<ListerSpec>,
    /// Its columns.
    pub columns: Vec<ColumnSpec>,
    /// The places it describes.
    pub place_info: Vec<PlaceInfoSpec>,
}

impl Manifest {
    /// The manifest's file name in a plugin's folder.
    pub const FILE: &'static str = "plugin.toml";

    /// Reads a manifest for this system and refuses it on any error; the
    /// message names the first error's field and fix.
    pub fn from_toml(text: &str) -> Result<Self, PluginError> {
        let report = Self::check(text);
        match report.manifest {
            Some(manifest) => Ok(manifest),
            None => Err(PluginError::Manifest {
                plugin: String::new(),
                message: report
                    .errors()
                    .next()
                    .map_or_else(|| "it could not be read".to_owned(), ToString::to_string),
            }),
        }
    }

    /// Reads a manifest for this system and reports every problem.
    pub fn check(text: &str) -> ManifestReport {
        Self::check_for(text, Os::current())
    }

    /// Reads a manifest as it would be on another system (`[run.windows]`
    /// and the like apply there).
    pub fn check_for(text: &str, os: Os) -> ManifestReport {
        let mut reader = Reader::default();
        let manifest = match text.parse::<Table>() {
            Ok(table) => read_manifest(&table, os, &mut reader),
            Err(e) => {
                reader.error(
                    "plugin.toml",
                    &format!("not valid TOML: {}", one_line(e.message())),
                    "Fix the TOML syntax at the place it names.",
                );
                None
            }
        };
        let mut problems = reader.problems;
        problems.sort_by_key(|p| p.severity);
        let failed = problems.iter().any(|p| p.severity == Severity::Error);
        ManifestReport {
            manifest: if failed { None } else { manifest },
            problems,
        }
    }

    /// A command by its plugin-local id.
    pub fn command(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Whether the plugin runs a process (it has a `[run]` table).
    pub fn is_process(&self) -> bool {
        self.run.is_some()
    }

    /// The sentence when this diroxide is older than the manifest's
    /// `min_host`: "Line counter needs diroxide 0.1.0-alpha.3 or later."
    pub fn needs_newer_host(&self, host_version: &str) -> Option<String> {
        let wanted = self.min_host.as_deref()?;
        match (Semver::parse(host_version), Semver::parse(wanted)) {
            (Some(host), Some(min)) if host < min => {
                Some(format!("{} needs diroxide {wanted} or later.", self.name))
            }
            _ => None,
        }
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collects problems while a manifest is read.
#[derive(Default)]
struct Reader {
    problems: Vec<Problem>,
}

impl Reader {
    fn error(&mut self, field: &str, message: &str, fix: &str) {
        self.problems.push(Problem {
            severity: Severity::Error,
            field: field.to_owned(),
            message: message.to_owned(),
            fix: fix.to_owned(),
        });
    }

    fn warn(&mut self, field: &str, message: &str, fix: &str) {
        self.problems.push(Problem {
            severity: Severity::Warning,
            field: field.to_owned(),
            message: message.to_owned(),
            fix: fix.to_owned(),
        });
    }
}

/// Reads one table's keys, remembering which were known.
struct Fields<'a> {
    table: &'a Table,
    path: String,
    known: &'static [&'static str],
}

impl<'a> Fields<'a> {
    fn new(table: &'a Table, path: impl Into<String>, known: &'static [&'static str]) -> Self {
        Self {
            table,
            path: path.into(),
            known,
        }
    }

    fn field(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.table.get(key)
    }

    /// Warns about keys this version does not know: kept and ignored.
    fn finish(&self, r: &mut Reader) {
        for key in self.table.keys() {
            if !self.known.contains(&key.as_str()) {
                r.warn(
                    &self.field(key),
                    "unknown key, kept and ignored",
                    "Check its spelling, or remove it.",
                );
            }
        }
    }

    fn string(&self, key: &str, r: &mut Reader) -> Option<String> {
        match self.get(key)? {
            Value::String(s) => Some(s.clone()),
            other => {
                r.error(
                    &self.field(key),
                    &format!("is {}, not text", type_words(other)),
                    "Put the value in quotes.",
                );
                None
            }
        }
    }

    fn required_string(&self, key: &str, r: &mut Reader, fix: &str) -> Option<String> {
        if self.get(key).is_none() {
            r.error(&self.field(key), "is missing", fix);
            return None;
        }
        self.string(key, r)
    }

    fn strings(&self, key: &str, r: &mut Reader) -> Vec<String> {
        match self.get(key) {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let mut out = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    match item {
                        Value::String(s) => out.push(s.clone()),
                        other => r.error(
                            &format!("{}[{i}]", self.field(key)),
                            &format!("is {}, not text", type_words(other)),
                            "Put each item in quotes.",
                        ),
                    }
                }
                out
            }
            Some(Value::String(s)) => {
                r.error(
                    &self.field(key),
                    "is text, not a list",
                    &format!("Write it as a list: [\"{}\"].", s.replace('"', "'")),
                );
                Vec::new()
            }
            Some(other) => {
                r.error(
                    &self.field(key),
                    &format!("is {}, not a list", type_words(other)),
                    "Write it as a list of quoted items.",
                );
                Vec::new()
            }
        }
    }

    fn boolean(&self, key: &str, r: &mut Reader) -> Option<bool> {
        match self.get(key)? {
            Value::Boolean(b) => Some(*b),
            other => {
                r.error(
                    &self.field(key),
                    &format!("is {}, not true or false", type_words(other)),
                    "Write true or false, without quotes.",
                );
                None
            }
        }
    }

    fn integer(&self, key: &str, low: i64, high: i64, r: &mut Reader) -> Option<i64> {
        match self.get(key)? {
            Value::Integer(n) if (low..=high).contains(n) => Some(*n),
            Value::Integer(n) => {
                r.error(
                    &self.field(key),
                    &format!("{n} is out of range"),
                    &format!("Use a whole number from {low} to {high}."),
                );
                None
            }
            other => {
                r.error(
                    &self.field(key),
                    &format!("is {}, not a whole number", type_words(other)),
                    "Write a whole number, without quotes.",
                );
                None
            }
        }
    }

    fn word(&self, key: &str, allowed: &[&str], r: &mut Reader) -> Option<String> {
        let value = self.string(key, r)?;
        if allowed.contains(&value.as_str()) {
            Some(value)
        } else {
            r.error(
                &self.field(key),
                &format!("\"{value}\" is not a known value"),
                &format!("Use one of: {}.", allowed.join(", ")),
            );
            None
        }
    }

    fn duration(&self, key: &str, r: &mut Reader) -> Option<Limit> {
        let text = self.string(key, r)?;
        match parse_duration(&text) {
            Ok(limit) => Some(limit),
            Err(why) => {
                r.error(
                    &self.field(key),
                    &format!("\"{text}\" {why}"),
                    "Write a duration such as \"500ms\", \"30s\" or \"5m\", or \"none\".",
                );
                None
            }
        }
    }

    fn table(&self, key: &str, r: &mut Reader) -> Option<&'a Table> {
        match self.get(key)? {
            Value::Table(t) => Some(t),
            other => {
                r.error(
                    &self.field(key),
                    &format!("is {}, not a table", type_words(other)),
                    &format!("Write it as a [{}] table.", self.field(key)),
                );
                None
            }
        }
    }

    fn tables(&self, key: &str, r: &mut Reader) -> Vec<&'a Table> {
        match self.get(key) {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let mut out = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    match item {
                        Value::Table(t) => out.push(t),
                        other => r.error(
                            &format!("{}[{i}]", self.field(key)),
                            &format!("is {}, not a table", type_words(other)),
                            &format!("Write each one as a [[{}]] table.", self.field(key)),
                        ),
                    }
                }
                out
            }
            Some(other) => {
                r.error(
                    &self.field(key),
                    &format!("is {}, not a list of tables", type_words(other)),
                    &format!("Write each one as a [[{}]] table.", self.field(key)),
                );
                Vec::new()
            }
        }
    }
}

fn type_words(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "text",
        Value::Integer(_) => "a number",
        Value::Float(_) => "a decimal number",
        Value::Boolean(_) => "true or false",
        Value::Datetime(_) => "a date",
        Value::Array(_) => "a list",
        Value::Table(_) => "a table",
    }
}

/// Reads a duration: a whole number and `ms`, `s`, `m` or `h`, or `none`
/// (also `never`) for no limit. Zero is refused.
pub fn parse_duration(text: &str) -> Result<Limit, &'static str> {
    let text = text.trim();
    if text == "none" || text == "never" {
        return Ok(None);
    }
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let number: u64 = number.parse().map_err(|_| "is not a duration")?;
    let millis = match unit {
        "ms" => Some(number),
        "s" => number.checked_mul(1000),
        "m" => number.checked_mul(60_000),
        "h" => number.checked_mul(3_600_000),
        _ => return Err("is not a duration"),
    }
    .ok_or("is too long")?;
    if millis == 0 {
        return Err("is zero");
    }
    if millis > 7 * 24 * 3_600_000 {
        return Err("is longer than a week");
    }
    Ok(Some(Duration::from_millis(millis)))
}

/// Writes a duration the way [`parse_duration`] reads it.
pub fn format_duration(limit: Limit) -> String {
    match limit {
        None => "none".to_owned(),
        Some(d) => {
            let ms = d.as_millis();
            if ms % 3_600_000 == 0 {
                format!("{}h", ms / 3_600_000)
            } else if ms % 60_000 == 0 {
                format!("{}m", ms / 60_000)
            } else if ms % 1000 == 0 {
                format!("{}s", ms / 1000)
            } else {
                format!("{ms}ms")
            }
        }
    }
}

/// Whether text is a plain id: lowercase letters, digits, `-` and `_`,
/// starting and ending with a letter or digit, 2 to 40 characters.
pub fn is_plain_id(id: &str) -> bool {
    (2..=40).contains(&id.len()) && !id.contains('.') && CommandId::is_valid(id)
}

/// Whether text is a plain name for people: not empty, at most 40
/// characters, no hidden characters.
fn name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("is empty".to_owned());
    }
    if has_hidden(name) {
        return Some("holds a hidden or control character".to_owned());
    }
    if name.trim() != name {
        return Some("starts or ends with a space".to_owned());
    }
    let count = name.chars().count();
    (count > MAX_NAME_CHARS)
        .then(|| format!("is {count} characters, over the 40 of one Braille line"))
}

fn check_name(r: &mut Reader, field: &str, name: &str) {
    if let Some(why) = name_problem(name) {
        r.error(
            field,
            &why,
            "Use plain words, 40 characters at most, meaning first.",
        );
    }
}

fn check_description(r: &mut Reader, field: &str, text: &str) {
    if has_hidden(text) {
        r.error(
            field,
            "holds a hidden or control character",
            "Use plain words.",
        );
        return;
    }
    let count = text.chars().count();
    if count > MAX_DESCRIPTION_CHARS {
        r.warn(
            field,
            &format!("is {count} characters, over {MAX_DESCRIPTION_CHARS}"),
            "Keep it to one sentence.",
        );
    }
    let lower = text.to_lowercase();
    if let Some(filler) = FILLER.iter().find(|f| lower.starts_with(*f)) {
        r.warn(
            field,
            &format!("starts with filler (\"{filler}\")"),
            "Say what it does first, within 40 characters.",
        );
    }
}

fn check_tool_name(r: &mut Reader, field: &str, name: &str) -> bool {
    let plain = !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !plain {
        r.error(
            field,
            &format!("\"{name}\" is not a tool name"),
            "Name the tool, not its path: letters, digits, - _ and . only.",
        );
    }
    plain
}

/// Whether a tool name is an interpreter or a shell.
pub fn is_interpreter(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    INTERPRETERS.iter().any(|known| {
        stem == *known
            || (*known == "python" && stem.starts_with("python"))
            || (*known == "perl" && stem.starts_with("perl"))
            || (*known == "ruby" && stem.starts_with("ruby"))
            || (*known == "lua" && stem.starts_with("lua"))
    })
}

const PLUGIN_KEYS: &[&str] = &[
    "id",
    "name",
    "version",
    "description",
    "authors",
    "license",
    "homepage",
    "protocol",
    "min_host",
];
const RUN_KEYS: &[&str] = &[
    "command",
    "cwd",
    "env",
    "start",
    "concurrency",
    "idle_timeout",
    "startup_timeout",
    "shutdown_grace",
    "max_message_bytes",
    "windows",
    "linux",
    "macos",
];
const RUN_OS_KEYS: &[&str] = &[
    "command",
    "cwd",
    "env",
    "start",
    "concurrency",
    "idle_timeout",
    "startup_timeout",
    "shutdown_grace",
    "max_message_bytes",
];
const TIMEOUT_KEYS: &[&str] = &["request", "preview", "hook", "command", "convert"];
const PERMISSION_KEYS: &[&str] = &["fs", "panel", "tools", "commands", "launch", "declared"];
const DECLARED_KEYS: &[&str] = &["network", "files", "programs"];
const COMMAND_KEYS: &[&str] = &[
    "id",
    "name",
    "description",
    "key",
    "menu",
    "selection",
    "places",
    "needs",
    "os",
    "argv",
    "mode",
    // The script-command family (R22 section 2.6), read by the script
    // runner.
    "cwd",
    "timeout",
    "confirm",
    "prompt",
    "unattended",
    "dry_run",
    "cursor_fallback",
    "exit_words",
    "list",
    "max_rows",
    "per",
    "stdin",
    "selection_file",
    "interpreter",
    "stdout_directives",
];
const PROVIDER_KEYS: &[&str] = &[
    "scheme",
    "name",
    "separator",
    "case_sensitive",
    "max_name_length",
    "reserved_names",
    "writable",
    "trash",
    "links",
    "attributes",
    "watch",
    "server_side_copy",
    "atomic_rename",
    "methods",
    "lease",
];
const PROVIDER_FLAGS: &[&str] = &[
    "case_sensitive",
    "writable",
    "trash",
    "links",
    "attributes",
    "watch",
    "server_side_copy",
    "atomic_rename",
];
/// The provider methods a `[[provider]]` table can list.
pub const PROVIDER_METHODS: &[&str] = &[
    "connect",
    "list",
    "stat",
    "read",
    "write_begin",
    "write_chunk",
    "write_end",
    "create_dir",
    "rename",
    "remove",
    "trash",
    "set_attributes",
    "copy_hint",
    "copy",
    "watch",
    "unwatch",
    "volumes",
    "places",
];
const PREVIEWER_KEYS: &[&str] = &["id", "name", "match", "priority", "argv", "needs"];
const MATCH_KEYS: &[&str] = &["mime", "extensions", "types", "schemes", "max_size"];
const CONVERTER_KEYS: &[&str] = &["id", "name", "from", "to", "options", "needs", "argv"];
const FROM_KEYS: &[&str] = &["mime", "extensions", "types"];
const TO_KEYS: &[&str] = &["id", "name", "extension", "mime"];
const HOOK_KEYS: &[&str] = &["event", "schemes", "glob", "kind", "on_timeout", "timeout"];
const ACTION_KEYS: &[&str] = &["type", "action", "command"];
const SETTING_KEYS: &[&str] = &["key", "type", "default", "description", "choices"];
const LISTER_KEYS: &[&str] = &["id", "name", "columns", "activate", "action"];
const LISTER_COLUMN_KEYS: &[&str] = &["id", "header", "kind", "sorts"];
const LISTER_ACTION_KEYS: &[&str] = &["key", "command", "selection"];
const COLUMN_KEYS: &[&str] = &["id", "header", "kind", "applies_to", "batch"];
const PLACE_INFO_KEYS: &[&str] = &["scheme", "marker"];
const TOP_KEYS: &[&str] = &[
    "plugin",
    "run",
    "timeouts",
    "permissions",
    "command",
    "provider",
    "previewer",
    "converter",
    "hook",
    "filetype_action",
    "setting",
    "lister",
    "column",
    "place_info",
];

fn read_manifest(table: &Table, os: Os, r: &mut Reader) -> Option<Manifest> {
    let top = Fields::new(table, "", TOP_KEYS);
    let Some(plugin) = top.table("plugin", r) else {
        if table.get("plugin").is_none() {
            let phase0 = table.contains_key("program") || table.contains_key("name");
            r.error(
                "plugin",
                "the [plugin] table is missing",
                if phase0 {
                    "This is the old form: put id, name, version and protocol under [plugin], and the program under [run] command."
                } else {
                    "Add a [plugin] table with id, name, version and protocol."
                },
            );
        }
        return None;
    };
    top.finish(r);
    let p = Fields::new(plugin, "plugin", PLUGIN_KEYS);
    let id = p.required_string("id", r, "Add id = \"my-plugin\", the folder's name.");
    if let Some(id) = &id
        && !is_plain_id(id)
    {
        r.error(
            "plugin.id",
            &format!("\"{id}\" is not a plain id"),
            "Use lowercase letters, digits, - and _, 2 to 40 characters.",
        );
    }
    let name = p.required_string("name", r, "Add name, the plugin's name in words.");
    if let Some(name) = &name {
        check_name(r, "plugin.name", name);
    }
    let version = p.required_string("version", r, "Add version = \"0.1.0\".");
    if let Some(v) = &version
        && Semver::parse(v).is_none()
    {
        r.error(
            "plugin.version",
            &format!("\"{v}\" is not a version"),
            "Write a semantic version such as \"0.1.0\".",
        );
    }
    let description = p.string("description", r).unwrap_or_default();
    check_description(r, "plugin.description", &description);
    let authors = p.strings("authors", r);
    let license = p.string("license", r);
    let homepage = p.string("homepage", r);
    let protocol = match p.get("protocol") {
        None => {
            r.error(
                "plugin.protocol",
                "is missing",
                &format!("Add protocol = {PROTOCOL}."),
            );
            None
        }
        Some(_) => p.integer("protocol", 1, i64::from(u32::MAX), r),
    };
    if let Some(n) = protocol
        && n != i64::from(PROTOCOL)
    {
        r.error(
            "plugin.protocol",
            &format!("the plugin speaks protocol {n}; diroxide speaks {PROTOCOL}"),
            "Use a version of the plugin made for this diroxide.",
        );
    }
    let min_host = p.string("min_host", r);
    if let Some(v) = &min_host
        && Semver::parse(v).is_none()
    {
        r.error(
            "plugin.min_host",
            &format!("\"{v}\" is not a version"),
            "Write a semantic version such as \"0.1.0-alpha.2\".",
        );
    }
    p.finish(r);

    let run = top.table("run", r).map(|t| read_run(t, os, r));
    let timeouts = top
        .table("timeouts", r)
        .map(|t| read_timeouts(t, r))
        .unwrap_or_default();
    let permissions = top
        .table("permissions", r)
        .map(|t| read_permissions(t, r))
        .unwrap_or_default();

    let commands: Vec<CommandSpec> = top
        .tables("command", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_command(t, &format!("command[{i}]"), run.is_some(), r))
        .collect();
    let mut seen = BTreeSet::new();
    let mut keys = BTreeMap::new();
    for (i, c) in commands.iter().enumerate() {
        if !seen.insert(c.id.clone()) {
            r.error(
                &format!("command[{i}].id"),
                &format!("\"{}\" is used twice", c.id),
                "Give each command its own id.",
            );
        }
        if let Some(key) = &c.key
            && let Some(first) = keys.insert(key.clone(), i)
        {
            r.warn(
                &format!("command[{i}].key"),
                &format!("{key} is also command[{first}]'s key"),
                "Give each command its own key, or none.",
            );
        }
    }
    let command_ids: BTreeSet<String> = commands.iter().map(|c| c.id.clone()).collect();

    let providers = top
        .tables("provider", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_provider(t, &format!("provider[{i}]"), r))
        .collect::<Vec<_>>();
    let previewers = top
        .tables("previewer", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_previewer(t, &format!("previewer[{i}]"), run.is_some(), r))
        .collect::<Vec<_>>();
    let converters = top
        .tables("converter", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_converter(t, &format!("converter[{i}]"), run.is_some(), r))
        .collect::<Vec<_>>();
    let hooks = top
        .tables("hook", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_hook(t, &format!("hook[{i}]"), r))
        .collect::<Vec<_>>();
    let filetype_actions = top
        .tables("filetype_action", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| {
            read_filetype_action(t, &format!("filetype_action[{i}]"), &command_ids, r)
        })
        .collect::<Vec<_>>();
    let settings = top
        .tables("setting", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_setting(t, &format!("setting[{i}]"), r))
        .collect::<Vec<_>>();
    let listers = top
        .tables("lister", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_lister(t, &format!("lister[{i}]"), &command_ids, r))
        .collect::<Vec<_>>();
    let columns = top
        .tables("column", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_column(t, &format!("column[{i}]"), r))
        .collect::<Vec<_>>();
    let place_info = top
        .tables("place_info", r)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| read_place_info(t, &format!("place_info[{i}]"), r))
        .collect::<Vec<_>>();

    if run.is_none() {
        let needs_process = [
            (!providers.is_empty(), "provider"),
            (!hooks.is_empty(), "hook"),
            (!listers.is_empty(), "lister"),
            (!columns.is_empty(), "column"),
            (!place_info.is_empty(), "place_info"),
        ];
        for (present, table) in needs_process {
            if present {
                r.error(
                    table,
                    &format!("a [[{table}]] needs a program, and there is no [run]"),
                    "Add a [run] table with the plugin's command.",
                );
            }
        }
    }

    Some(Manifest {
        id: id?,
        name: name?,
        version: version?,
        description,
        authors,
        license,
        homepage,
        protocol: u32::try_from(protocol?).ok()?,
        min_host,
        run: run.flatten(),
        timeouts,
        permissions,
        commands,
        providers,
        previewers,
        converters,
        hooks,
        filetype_actions,
        settings,
        listers,
        columns,
        place_info,
    })
}

fn read_run(table: &Table, os: Os, r: &mut Reader) -> Option<Run> {
    let base = Fields::new(table, "run", RUN_KEYS);
    base.finish(r);
    let mut run = Run::default();
    let mut ok = apply_run(&base, &mut run, r);
    for (key, system) in [
        ("windows", Os::Windows),
        ("linux", Os::Linux),
        ("macos", Os::Macos),
    ] {
        if let Some(own) = base.table(key, r) {
            let fields = Fields::new(own, format!("run.{key}"), RUN_OS_KEYS);
            fields.finish(r);
            let mut scratch = run.clone();
            // Every system's table is checked; only this system's applies.
            let applied = apply_run(&fields, &mut scratch, r);
            if system == os {
                ok &= applied;
                run = scratch;
            }
        }
    }
    if run.command.is_empty() {
        if base.get("command").is_none() {
            r.error(
                "run.command",
                "is missing",
                "Add command = [\"python3\", \"main.py\"], the program and its arguments.",
            );
        }
        return None;
    }
    ok.then_some(run)
}

/// Applies one `[run]` table's keys; false when one was wrong.
fn apply_run(f: &Fields<'_>, run: &mut Run, r: &mut Reader) -> bool {
    let before = r.problems.len();
    match f.get("command") {
        None => {}
        Some(Value::String(line)) => r.error(
            &f.field("command"),
            "is one line of text, which would need a shell",
            &format!(
                "Write it as a list: [{}].",
                line.split_whitespace()
                    .map(|w| format!("\"{w}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Some(_) => {
            let command = f.strings("command", r);
            if command.is_empty() || command[0].trim().is_empty() {
                r.error(
                    &f.field("command"),
                    "is empty",
                    "Name the program first, then its arguments.",
                );
            } else {
                run.command = command;
            }
        }
    }
    if let Some(cwd) = f.string("cwd", r) {
        let escapes = cwd.split(['/', '\\']).any(|part| part == "..")
            || cwd.starts_with(['/', '\\'])
            || cwd.contains(':');
        if escapes {
            r.error(
                &f.field("cwd"),
                &format!("\"{cwd}\" leaves the plugin's folder"),
                "Use a folder inside the plugin's folder, such as \".\".",
            );
        } else {
            run.cwd = cwd;
        }
    }
    if let Some(env) = f.table("env", r) {
        for (name, value) in env {
            match value {
                Value::String(v)
                    if !name.is_empty()
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
                {
                    if name.starts_with("DIRO_") {
                        r.error(
                            &format!("{}.{name}", f.field("env")),
                            "DIRO_ variables are the host's",
                            "Use another name.",
                        );
                    } else {
                        run.env.insert(name.clone(), v.clone());
                    }
                }
                _ => r.error(
                    &format!("{}.{name}", f.field("env")),
                    "is not a plain variable with a text value",
                    "Use letters, digits and _ in the name, and a quoted value.",
                ),
            }
        }
    }
    if let Some(start) = f.word("start", &["on_demand", "startup"], r) {
        run.start = if start == "startup" {
            Start::Startup
        } else {
            Start::OnDemand
        };
    }
    if let Some(n) = f.integer("concurrency", 1, 16, r) {
        run.concurrency = u32::try_from(n).unwrap_or(1);
    }
    if let Some(limit) = f.duration("idle_timeout", r) {
        run.idle_timeout = limit;
    }
    for (key, slot) in [
        ("startup_timeout", &mut run.startup_timeout),
        ("shutdown_grace", &mut run.shutdown_grace),
    ] {
        match f.duration(key, r) {
            Some(Some(d)) => *slot = d,
            Some(None) => r.error(
                &f.field(key),
                "cannot be none",
                "Write a duration such as \"10s\".",
            ),
            None => {}
        }
    }
    let low = 64 * 1024;
    let high = MAX_LINE_BYTES_LIMIT as i64;
    if let Some(n) = f.integer("max_message_bytes", low, high, r) {
        run.max_message_bytes = usize::try_from(n).unwrap_or(MAX_LINE_BYTES);
    }
    r.problems.len() == before
}

fn read_timeouts(table: &Table, r: &mut Reader) -> Timeouts {
    let f = Fields::new(table, "timeouts", TIMEOUT_KEYS);
    f.finish(r);
    let mut t = Timeouts::default();
    for (key, slot) in [
        ("request", &mut t.request),
        ("preview", &mut t.preview),
        ("hook", &mut t.hook),
        ("command", &mut t.command),
        ("convert", &mut t.convert),
    ] {
        if let Some(limit) = f.duration(key, r) {
            *slot = limit;
        }
    }
    t
}

fn read_access(f: &Fields<'_>, key: &str, r: &mut Reader) -> Access {
    match f.word(key, &["none", "read", "write"], r).as_deref() {
        Some("read") => Access::Read,
        Some("write") => Access::Write,
        _ => Access::None,
    }
}

fn read_permissions(table: &Table, r: &mut Reader) -> Permissions {
    let f = Fields::new(table, "permissions", PERMISSION_KEYS);
    f.finish(r);
    let mut p = Permissions {
        fs: read_access(&f, "fs", r),
        panel: read_access(&f, "panel", r),
        ..Permissions::default()
    };
    if let Some(declared) = f.table("declared", r) {
        let d = Fields::new(declared, "permissions.declared", DECLARED_KEYS);
        d.finish(r);
        let read = |key: &str, r: &mut Reader| {
            let text = d.string(key, r)?;
            let count = text.chars().count();
            if count > MAX_DECLARED_CHARS || has_hidden(&text) || text.trim().is_empty() {
                r.error(
                    &d.field(key),
                    &format!("is not a short plain statement ({count} characters)"),
                    "Say it in plain words, 80 characters at most.",
                );
                None
            } else {
                Some(text)
            }
        };
        p.declared = Declared {
            network: read("network", r),
            files: read("files", r),
            programs: read("programs", r),
        };
    }
    for key in ["tools", "launch"] {
        let names = f.strings(key, r);
        let mut kept = Vec::new();
        for (i, name) in names.into_iter().enumerate() {
            let field = format!("permissions.{key}[{i}]");
            if !check_tool_name(r, &field, &name) {
                continue;
            }
            if is_interpreter(&name) && !p.any_program() {
                r.error(
                    &field,
                    &format!("{name} is an interpreter or a shell, which can run anything"),
                    "Remove it, or set permissions.declared.programs = \"any program\".",
                );
                continue;
            }
            kept.push(name);
        }
        if key == "tools" {
            p.tools = kept;
        } else {
            p.launch = kept;
        }
    }
    for (i, pattern) in f.strings("commands", r).into_iter().enumerate() {
        let probe = pattern.replace('*', "x");
        if CommandId::is_valid(&probe) && !pattern.starts_with('*') {
            p.commands.push(pattern);
        } else {
            r.error(
                &format!("permissions.commands[{i}]"),
                &format!("\"{pattern}\" is not a command id pattern"),
                "Write an id such as \"panel.refresh\", with * for one word, as in \"panel.*\".",
            );
        }
    }
    p
}

fn read_selection(f: &Fields<'_>, key: &str, r: &mut Reader) -> Selection {
    match f
        .word(key, &["none", "one", "two", "any", "many"], r)
        .as_deref()
    {
        Some("none") => Selection::None,
        Some("one") => Selection::One,
        Some("two") => Selection::Two,
        Some("many") => Selection::Many,
        _ => Selection::Any,
    }
}

fn check_key(r: &mut Reader, field: &str, key: &str) -> bool {
    match Key::parse(key) {
        Ok(_) => true,
        Err(diroxide_core::CoreError::KeySequence(_)) => {
            r.error(
                field,
                &format!("\"{key}\" is two keys in a row"),
                "Use one key with its modifiers, such as \"ctrl+alt+l\".",
            );
            false
        }
        Err(_) => {
            r.error(
                field,
                &format!("\"{key}\" is not a key"),
                "Write a key such as \"ctrl+alt+l\" or \"F5\".",
            );
            false
        }
    }
}

fn read_local_id(f: &Fields<'_>, r: &mut Reader, what: &str) -> Option<String> {
    let id = f.required_string("id", r, &format!("Add id, the {what}'s id."))?;
    if CommandId::is_valid(&id) && !id.contains('.') && id.len() <= 40 {
        Some(id)
    } else {
        r.error(
            &f.field("id"),
            &format!("\"{id}\" is not a plain id"),
            "Use lowercase letters, digits, - and _, without dots.",
        );
        None
    }
}

fn read_name(f: &Fields<'_>, key: &str, r: &mut Reader) -> Option<String> {
    let name = f.required_string(key, r, "Add it, in plain words, meaning first.")?;
    check_name(r, &f.field(key), &name);
    Some(name)
}

fn read_command(table: &Table, path: &str, process: bool, r: &mut Reader) -> Option<CommandSpec> {
    let f = Fields::new(table, path, COMMAND_KEYS);
    f.finish(r);
    let id = read_local_id(&f, r, "command");
    let name = read_name(&f, "name", r);
    let description = f.string("description", r).unwrap_or_default();
    check_description(r, &f.field("description"), &description);
    let key = f.string("key", r);
    if let Some(k) = &key {
        check_key(r, &f.field("key"), k);
    }
    let menu = f.string("menu", r).unwrap_or_else(|| "Plugins".to_owned());
    let selection = read_selection(&f, "selection", r);
    let places = match f.word("places", &["any", "local"], r).as_deref() {
        Some("local") => Places::Local,
        _ => Places::Any,
    };
    let needs = f.strings("needs", r);
    for (i, tool) in needs.iter().enumerate() {
        check_tool_name(r, &format!("{}[{i}]", f.field("needs")), tool);
    }
    let mut os = Vec::new();
    for (i, word) in f.strings("os", r).into_iter().enumerate() {
        match word.as_str() {
            "windows" => os.push(Os::Windows),
            "linux" => os.push(Os::Linux),
            "macos" => os.push(Os::Macos),
            _ => r.error(
                &format!("{}[{i}]", f.field("os")),
                &format!("\"{word}\" is not a system"),
                "Use windows, linux or macos.",
            ),
        }
    }
    let argv = f.strings("argv", r);
    if !process && argv.is_empty() {
        r.error(
            &f.field("argv"),
            "is missing, and the plugin has no [run] program",
            "Add argv, the program and its arguments, or a [run] table.",
        );
    }
    let mode = f.string("mode", r);
    let extra: Table = table
        .iter()
        .filter(|(k, _)| {
            ![
                "id",
                "name",
                "description",
                "key",
                "menu",
                "selection",
                "places",
                "needs",
                "os",
                "argv",
                "mode",
            ]
            .contains(&k.as_str())
                && COMMAND_KEYS.contains(&k.as_str())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Some(CommandSpec {
        id: id?,
        name: name?,
        description,
        key,
        menu,
        selection,
        places,
        needs,
        os,
        argv,
        mode,
        extra,
    })
}

fn is_scheme(s: &str) -> bool {
    s.len() >= 2
        && s.len() <= 32
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-.".contains(c))
}

fn read_provider(table: &Table, path: &str, r: &mut Reader) -> Option<ProviderSpec> {
    let f = Fields::new(table, path, PROVIDER_KEYS);
    f.finish(r);
    let scheme = f.required_string("scheme", r, "Add scheme, such as \"s3\".");
    if let Some(s) = &scheme {
        if !is_scheme(s) {
            r.error(
                &f.field("scheme"),
                &format!("\"{s}\" is not a scheme"),
                "Use lowercase letters, digits, + - and ., starting with a letter.",
            );
        } else if ["file", "local", "archive"].contains(&s.as_str()) {
            r.error(
                &f.field("scheme"),
                &format!("\"{s}\" belongs to diroxide"),
                "Choose another scheme.",
            );
        }
    }
    let name = read_name(&f, "name", r);
    let separator = f.string("separator", r).unwrap_or_else(|| "/".to_owned());
    if separator.chars().count() != 1 {
        r.error(
            &f.field("separator"),
            "is not one character",
            "Use one character, such as \"/\".",
        );
    }
    let mut flags = BTreeSet::new();
    for flag in PROVIDER_FLAGS {
        if f.boolean(flag, r) == Some(true) {
            flags.insert((*flag).to_owned());
        }
    }
    let max_name_length = f
        .integer("max_name_length", 1, 65_535, r)
        .and_then(|n| u32::try_from(n).ok());
    let reserved_names = f.strings("reserved_names", r);
    let mut methods: BTreeSet<String> = ["list", "stat"].map(str::to_owned).into();
    for (i, method) in f.strings("methods", r).into_iter().enumerate() {
        if PROVIDER_METHODS.contains(&method.as_str()) {
            methods.insert(method);
        } else {
            r.error(
                &format!("{}[{i}]", f.field("methods")),
                &format!("\"{method}\" is not a provider method"),
                &format!(
                    "Use the names without provider/: {}.",
                    PROVIDER_METHODS.join(", ")
                ),
            );
        }
    }
    let writes = ["write_begin", "write_chunk", "write_end"];
    let some_writes = writes.iter().filter(|m| methods.contains(**m)).count();
    if some_writes != 0 && some_writes != writes.len() {
        r.error(
            &f.field("methods"),
            "lists part of the write session",
            "List write_begin, write_chunk and write_end together.",
        );
    }
    f.word("lease", &["listers"], r);
    Some(ProviderSpec {
        scheme: scheme?,
        name: name?,
        separator,
        flags,
        max_name_length,
        reserved_names,
        methods,
    })
}

fn read_previewer(
    table: &Table,
    path: &str,
    process: bool,
    r: &mut Reader,
) -> Option<PreviewerSpec> {
    let f = Fields::new(table, path, PREVIEWER_KEYS);
    f.finish(r);
    let id = read_local_id(&f, r, "previewer");
    let name = read_name(&f, "name", r);
    let matches = f.table("match", r).cloned().unwrap_or_default();
    Fields::new(&matches, f.field("match"), MATCH_KEYS).finish(r);
    if matches.is_empty() {
        r.error(
            &f.field("match"),
            "is missing or empty",
            "Add a [previewer.match] table with mime, extensions or types.",
        );
    }
    let priority = f
        .word("priority", &["low", "normal", "high"], r)
        .unwrap_or_else(|| "normal".to_owned());
    let argv = f.strings("argv", r);
    if !process && argv.is_empty() {
        r.error(
            &f.field("argv"),
            "is missing, and the plugin has no [run] program",
            "Add argv, the program and its arguments, or a [run] table.",
        );
    }
    Some(PreviewerSpec {
        id: id?,
        name: name?,
        matches,
        priority,
        argv,
    })
}

fn read_converter(
    table: &Table,
    path: &str,
    process: bool,
    r: &mut Reader,
) -> Option<ConverterSpec> {
    let f = Fields::new(table, path, CONVERTER_KEYS);
    f.finish(r);
    let id = read_local_id(&f, r, "converter");
    let name = read_name(&f, "name", r);
    let from = f.table("from", r).cloned().unwrap_or_default();
    Fields::new(&from, f.field("from"), FROM_KEYS).finish(r);
    if from.is_empty() {
        r.error(
            &f.field("from"),
            "is missing or empty",
            "Add a from table with mime, extensions or types.",
        );
    }
    let to: Vec<Table> = f.tables("to", r).into_iter().cloned().collect();
    if to.is_empty() {
        r.error(
            &f.field("to"),
            "is missing",
            "Add at least one output with id, name and extension.",
        );
    }
    for (i, output) in to.iter().enumerate() {
        let o = Fields::new(output, format!("{}[{i}]", f.field("to")), TO_KEYS);
        o.finish(r);
        o.required_string("id", r, "Add id, the output's id.");
        if let Some(n) = o.required_string("name", r, "Add name, the output's name in words.") {
            check_name(r, &o.field("name"), &n);
        }
        o.required_string("extension", r, "Add extension, such as \"docx\".");
    }
    let needs = f.strings("needs", r);
    let argv = f.strings("argv", r);
    if !process && argv.is_empty() {
        r.error(
            &f.field("argv"),
            "is missing, and the plugin has no [run] program",
            "Add argv, the program and its arguments, or a [run] table.",
        );
    }
    Some(ConverterSpec {
        id: id?,
        name: name?,
        from,
        to,
        needs,
        argv,
    })
}

fn read_hook(table: &Table, path: &str, r: &mut Reader) -> Option<HookSpec> {
    let f = Fields::new(table, path, HOOK_KEYS);
    f.finish(r);
    let event = if f.get("event").is_none() {
        r.error(
            &f.field("event"),
            "is missing",
            "Add event, such as \"before_delete\".",
        );
        None
    } else {
        f.word("event", HOOK_EVENTS, r)
    };
    let schemes = f.strings("schemes", r);
    let glob = f.string("glob", r);
    if let Some(g) = &glob
        && Glob::for_path(g, NameCase::Ignore).is_err()
    {
        r.error(
            &f.field("glob"),
            &format!("\"{g}\" is not a glob"),
            "Write a glob such as \"**/*.pdf\", with / between folders.",
        );
    }
    let kind = f.word("kind", &["file", "folder"], r);
    let on_timeout = match f.word("on_timeout", &["continue", "cancel"], r).as_deref() {
        Some("cancel") => OnTimeout::Cancel,
        _ => OnTimeout::Continue,
    };
    let timeout = f.duration("timeout", r);
    let event = event?;
    if !event.starts_with("before_") && f.get("on_timeout").is_some() {
        r.error(
            &f.field("on_timeout"),
            &format!("{event} is never waited for, so it cannot time out"),
            "Remove on_timeout, or hook a before_ event.",
        );
    }
    Some(HookSpec {
        event,
        schemes,
        glob,
        kind,
        on_timeout,
        timeout,
    })
}

fn read_filetype_action(
    table: &Table,
    path: &str,
    commands: &BTreeSet<String>,
    r: &mut Reader,
) -> Option<FiletypeAction> {
    let f = Fields::new(table, path, ACTION_KEYS);
    f.finish(r);
    let file_type = f.required_string("type", r, "Add type, a file type's id.");
    let action = f.required_string("action", r, "Add action, such as \"open\" or \"view\".");
    let command = f.required_string(
        "command",
        r,
        "Add command, one of this plugin's command ids.",
    );
    if let Some(c) = &command
        && !commands.contains(c)
    {
        r.error(
            &f.field("command"),
            &format!("\"{c}\" is not one of this plugin's commands"),
            "Use the id of a [[command]] in this manifest.",
        );
    }
    Some(FiletypeAction {
        file_type: file_type?,
        action: action?,
        command: command?,
    })
}

fn read_setting(table: &Table, path: &str, r: &mut Reader) -> Option<SettingSpec> {
    let f = Fields::new(table, path, SETTING_KEYS);
    f.finish(r);
    let key = f.required_string("key", r, "Add key, the setting's name in snake_case.");
    if let Some(k) = &key {
        let snake = !k.is_empty()
            && k.starts_with(|c: char| c.is_ascii_lowercase())
            && k.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !snake {
            r.error(
                &f.field("key"),
                &format!("\"{k}\" is not snake_case"),
                "Use lowercase letters, digits and _, starting with a letter.",
            );
        }
    }
    let kind = if f.get("type").is_none() {
        r.error(
            &f.field("type"),
            "is missing",
            &format!("Add type, one of: {}.", SETTING_TYPES.join(", ")),
        );
        None
    } else {
        f.word("type", SETTING_TYPES, r)
    };
    let description = f.string("description", r).unwrap_or_default();
    check_description(r, &f.field("description"), &description);
    let choices = f.strings("choices", r);
    let default = f.get("default").cloned();
    let kind = kind?;
    let default = match default {
        None => {
            r.error(
                &f.field("default"),
                "is missing",
                "Add the value the setting has until the owner changes it.",
            );
            return None;
        }
        Some(d) => d,
    };
    let fits = match (kind.as_str(), &default) {
        ("bool", Value::Boolean(_)) | ("int", Value::Integer(_)) => true,
        ("string" | "path", Value::String(_)) => true,
        ("choice", Value::String(s)) => {
            if choices.is_empty() {
                r.error(
                    &f.field("choices"),
                    "is missing for a choice",
                    "List the choices.",
                );
            } else if !choices.contains(s) {
                r.error(
                    &f.field("default"),
                    &format!("\"{s}\" is not one of the choices"),
                    "Make the default one of the choices.",
                );
            }
            true
        }
        ("argv", Value::Array(items)) => items.iter().all(Value::is_str),
        _ => false,
    };
    if !fits {
        r.error(
            &f.field("default"),
            &format!(
                "is {}, which a {kind} setting cannot hold",
                type_words(&default)
            ),
            "Give a default of the setting's type.",
        );
    }
    Some(SettingSpec {
        key: key?,
        kind,
        default,
        description,
        choices,
    })
}

fn read_lister(
    table: &Table,
    path: &str,
    commands: &BTreeSet<String>,
    r: &mut Reader,
) -> Option<ListerSpec> {
    let f = Fields::new(table, path, LISTER_KEYS);
    f.finish(r);
    let id = read_local_id(&f, r, "lister");
    let name = read_name(&f, "name", r);
    let mut columns = Vec::new();
    for (i, t) in f.tables("columns", r).into_iter().enumerate() {
        let c = Fields::new(
            t,
            format!("{}[{i}]", f.field("columns")),
            LISTER_COLUMN_KEYS,
        );
        c.finish(r);
        let id = read_local_id(&c, r, "column");
        let header = read_name(&c, "header", r);
        let kind = c
            .word("kind", COLUMN_KINDS, r)
            .unwrap_or_else(|| "text".to_owned());
        let sorts = c.boolean("sorts", r).unwrap_or(false);
        if let (Some(id), Some(header)) = (id, header) {
            columns.push(ListerColumn {
                id,
                header,
                kind,
                sorts,
            });
        }
    }
    let known_command = |field: &str, c: &str, r: &mut Reader| {
        if !commands.contains(c) {
            r.error(
                field,
                &format!("\"{c}\" is not one of this plugin's commands"),
                "Use the id of a [[command]] in this manifest.",
            );
        }
    };
    let activate = f.string("activate", r);
    if let Some(c) = &activate {
        known_command(&f.field("activate"), c, r);
    }
    let mut actions = Vec::new();
    for (i, t) in f.tables("action", r).into_iter().enumerate() {
        let a = Fields::new(t, format!("{}[{i}]", f.field("action")), LISTER_ACTION_KEYS);
        a.finish(r);
        let key = a.required_string("key", r, "Add key, such as \"ctrl+d\".");
        if let Some(k) = &key {
            check_key(r, &a.field("key"), k);
        }
        let command = a.required_string("command", r, "Add command, one of this plugin's ids.");
        if let Some(c) = &command {
            known_command(&a.field("command"), c, r);
        }
        let selection = read_selection(&a, "selection", r);
        if let (Some(key), Some(command)) = (key, command) {
            actions.push(ListerAction {
                key,
                command,
                selection,
            });
        }
    }
    Some(ListerSpec {
        id: id?,
        name: name?,
        columns,
        activate,
        actions,
    })
}

fn read_column(table: &Table, path: &str, r: &mut Reader) -> Option<ColumnSpec> {
    let f = Fields::new(table, path, COLUMN_KEYS);
    f.finish(r);
    let id = read_local_id(&f, r, "column");
    let header = read_name(&f, "header", r);
    let kind = f
        .word("kind", COLUMN_KINDS, r)
        .unwrap_or_else(|| "text".to_owned());
    let applies_to = f.strings("applies_to", r);
    let batch = f
        .integer("batch", 1, 500, r)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(500);
    Some(ColumnSpec {
        id: id?,
        header: header?,
        kind,
        applies_to,
        batch,
    })
}

fn read_place_info(table: &Table, path: &str, r: &mut Reader) -> Option<PlaceInfoSpec> {
    let f = Fields::new(table, path, PLACE_INFO_KEYS);
    f.finish(r);
    let scheme = f.string("scheme", r);
    let marker = f.string("marker", r);
    if let Some(m) = &marker
        && (m.is_empty() || m.contains(['/', '\\']) || m == "." || m == "..")
    {
        r.error(
            &f.field("marker"),
            &format!("\"{m}\" is not one file or folder name"),
            "Name one file or folder, such as \".git\".",
        );
    }
    if scheme.is_none() && marker.is_none() {
        r.error(
            path,
            "names no scheme and no marker",
            "Add scheme, or marker such as \".git\".",
        );
        return None;
    }
    Some(PlaceInfoSpec { scheme, marker })
}

/// A semantic version, compared by its numbers and then its pre-release
/// words (a release is newer than any of its pre-releases).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Semver {
    numbers: [u64; 3],
    pre: Vec<String>,
}

impl Semver {
    /// Reads `MAJOR.MINOR.PATCH`, with an optional `-pre` and `+build`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.split_once('+').map_or(text, |(v, _)| v);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = [0u64; 3];
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        for (slot, part) in numbers.iter_mut().zip(parts) {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => {
                let words: Vec<String> = pre.split('.').map(str::to_owned).collect();
                let plain = |w: &String| {
                    !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                };
                if !words.iter().all(plain) {
                    return None;
                }
                words
            }
        };
        Some(Self { numbers, pre })
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Semver {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.numbers.cmp(&other.numbers).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use proptest::prelude::*;

    use super::*;

    /// R3's sample manifest (section 4.1), with its `min_host` moved to
    /// alpha 2's version.
    const SAMPLE: &str = r#"
[plugin]
id = "line-counter"
name = "Line counter"
version = "0.1.0"
description = "Counts the lines of the selected text files."
authors = ["Ada Example"]
license = "MIT"
protocol = 1
min_host = "0.1.0-alpha.2"

[run]
command = ["python3", "main.py"]
cwd = "."
start = "on_demand"
concurrency = 1
idle_timeout = "5m"
startup_timeout = "10s"
shutdown_grace = "3s"
max_message_bytes = 8388608

[run.windows]
command = ["py", "-3", "main.py"]

[timeouts]
request = "30s"
preview = "5s"
hook = "2s"
command = "none"
convert = "none"

[permissions]
fs = "read"
tools = ["magick"]
panel = "write"
commands = ["panel.*"]

[permissions.declared]
network = "none"
files = "only the files I am given"
programs = "none"

[[command]]
id = "count_lines"
name = "Count lines in files"
description = "Says how many lines the selected files hold."
key = "ctrl+alt+l"
menu = "Plugins"
selection = "any"
places = "local"

[[setting]]
key = "skip_blank"
type = "bool"
default = false
description = "Do not count blank lines."
"#;

    #[test]
    fn r3s_sample_reads_on_every_system() {
        for os in [Os::Windows, Os::Linux, Os::Macos] {
            let report = Manifest::check_for(SAMPLE, os);
            assert!(report.problems.is_empty(), "{}", report.text());
            let m = report.manifest.unwrap();
            assert_eq!(m.id, "line-counter");
            assert_eq!(m.commands[0].full_id(&m.id), "line-counter.count_lines");
            let run = m.run.unwrap();
            let expected: &[&str] = if os == Os::Windows {
                &["py", "-3", "main.py"]
            } else {
                &["python3", "main.py"]
            };
            assert_eq!(run.command, expected);
            assert_eq!(run.idle_timeout, Some(Duration::from_secs(300)));
            assert_eq!(m.permissions.fs, Access::Read);
            assert!(m.permissions.allows_command("panel.refresh"));
            assert!(!m.permissions.allows_command("file.delete"));
        }
    }

    #[test]
    fn the_host_version_check_says_what_it_needs() {
        let m = Manifest::check_for(SAMPLE, Os::Linux).manifest.unwrap();
        assert_eq!(
            m.needs_newer_host("0.1.0-alpha.1").as_deref(),
            Some("Line counter needs diroxide 0.1.0-alpha.2 or later.")
        );
        assert_eq!(m.needs_newer_host("0.1.0-alpha.2"), None);
        assert_eq!(m.needs_newer_host("0.1.0"), None);
        assert_eq!(m.needs_newer_host("0.2.0-alpha.1"), None);
    }

    #[test]
    fn a_41_cell_name_fails_the_40_cell_lint() {
        let forty = "N".repeat(40);
        let ok = SAMPLE.replace("name = \"Line counter\"", &format!("name = \"{forty}\""));
        assert!(Manifest::check(&ok).manifest.is_some());
        let over = SAMPLE.replace("name = \"Line counter\"", &format!("name = \"{forty}x\""));
        let report = Manifest::check(&over);
        assert!(report.manifest.is_none());
        assert_eq!(
            report.text(),
            "Error, plugin.name: is 41 characters, over the 40 of one Braille line. Use plain words, 40 characters at most, meaning first.\n"
        );
    }

    #[test]
    fn command_patterns_match_by_words() {
        assert!(command_matches("panel.*", "panel.refresh"));
        assert!(command_matches("panel.*", "panel.sort.name"));
        assert!(!command_matches("panel.*", "panel"));
        assert!(command_matches("panel.refresh", "panel.refresh"));
        assert!(!command_matches("panel.refresh", "panel.refresh.now"));
        assert!(command_matches("*.refresh", "panel.refresh"));
        assert!(!command_matches("file.*", "panel.refresh"));
    }

    #[test]
    fn interpreters_are_known_by_stem() {
        for name in ["bash", "python3", "python3.12", "pwsh.exe", "PERL", "find"] {
            assert!(is_interpreter(name), "{name}");
        }
        for name in ["git", "magick", "pandoc", "es", "plocate"] {
            assert!(!is_interpreter(name), "{name}");
        }
    }

    #[test]
    fn versions_compare_with_their_pre_releases() {
        let v = |s| Semver::parse(s).unwrap();
        assert!(v("0.1.0-alpha.2") < v("0.1.0-alpha.10"));
        assert!(v("0.1.0-alpha.9") < v("0.1.0"));
        assert!(v("0.1.0-alpha") < v("0.1.0-alpha.1"));
        assert!(v("1.0.0+build.5") == v("1.0.0"));
        for bad in [
            "1",
            "1.0",
            "1.0.0.0",
            "a.b.c",
            "1.0.0-",
            "1.0.0-a..b",
            "01.x.0",
        ] {
            assert!(Semver::parse(bad).is_none(), "{bad}");
        }
    }

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plugins/manifests")
    }

    /// Every fixture's report, against its golden `.expected` file. Run with
    /// `DIROXIDE_BLESS=1` to write the golden files after a deliberate
    /// change, and read every one before committing.
    #[test]
    fn golden_manifests() {
        let bless = std::env::var_os("DIROXIDE_BLESS").is_some();
        let mut count = 0;
        let mut mismatches = Vec::new();
        for group in ["good", "bad"] {
            let folder = fixtures().join(group);
            let mut paths: Vec<PathBuf> = fs::read_dir(&folder)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            paths.sort();
            for path in paths {
                count += 1;
                let text = fs::read_to_string(&path).unwrap();
                let report = Manifest::check_for(&text, Os::Linux);
                assert_eq!(
                    report.manifest.is_some(),
                    group == "good",
                    "{}: {}",
                    path.display(),
                    report.text()
                );
                let golden = path.with_extension("expected");
                if bless {
                    fs::write(&golden, report.text()).unwrap();
                    continue;
                }
                let expected = fs::read_to_string(&golden).unwrap_or_default();
                if expected.replace("\r\n", "\n") != report.text() {
                    mismatches.push(format!("{}:\n{}", path.display(), report.text()));
                }
            }
        }
        assert!(count >= 40, "only {count} fixtures");
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn every_problem_names_its_field_first_and_a_fix() {
        for group in ["good", "bad"] {
            for entry in fs::read_dir(fixtures().join(group)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_none_or(|e| e != "toml") {
                    continue;
                }
                let report = Manifest::check(&fs::read_to_string(&path).unwrap());
                for problem in &report.problems {
                    let line = problem.to_string();
                    assert!(
                        line.starts_with("Error, ") || line.starts_with("Warning, "),
                        "{line}"
                    );
                    assert!(!problem.field.is_empty(), "{line}");
                    assert!(problem.fix.ends_with('.'), "{line}");
                    assert!(!problem.message.ends_with('.'), "{line}");
                }
            }
        }
    }

    proptest! {
        #[test]
        fn durations_round_trip(ms in 1u64..(7 * 24 * 3_600_000)) {
            let limit = Some(Duration::from_millis(ms));
            prop_assert_eq!(parse_duration(&format_duration(limit)), Ok(limit));
        }

        #[test]
        fn duration_text_never_panics(text in "\\PC{0,12}") {
            let _ = parse_duration(&text);
        }

        #[test]
        fn plain_ids_hold_only_plain_characters(id in "[a-z0-9_.A-Z -]{0,45}") {
            if is_plain_id(&id) {
                prop_assert!(id.len() >= 2 && id.len() <= 40);
                prop_assert!(id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'));
            }
        }

        #[test]
        fn names_over_forty_are_refused(name in "[A-Za-z ]{41,60}") {
            prop_assert!(name_problem(&name).is_some());
        }
    }
}
