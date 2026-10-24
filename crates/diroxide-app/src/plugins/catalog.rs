//! The plugins and scripts found on disk, as the app shows them: each one's
//! state (enabled, changed since, not enabled, or with errors), the rows
//! of its consent question, and its commands with what each needs.
//!
//! The runtime makes a [`Catalog`] with [`scan`] on a worker (it reads the
//! plugin folders, the scripts folder and the consent record, and hashes
//! the files a consent covers), and the app keeps it to fill the Plugins
//! menu, the palette, the banks and Manage plugins. Only an enabled item
//! contributes commands; one that is not enabled appears only in Manage
//! plugins (R3's rule).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use diroxide_core::{CommandId, EntryKind, Place, PlaceKind};
use diroxide_plugins::manifest::{
    CommandSpec, ListerSpec, Manifest, Os, Places, Selection, SettingSpec, Severity,
};
use diroxide_plugins::script::command::ScriptCommand;
use diroxide_plugins::script::consent::{
    ConsentState, ConsentStore, Fingerprint, Hasher, Kind as ConsentKind, Subject,
    plugin_fingerprint, script_fingerprint, trust_problem,
};
use diroxide_plugins::script::context::{Refusal, RunContext, effective};
use diroxide_plugins::script::installed::{self, Installed};
use diroxide_plugins::script::interpreter::Lookup;
use diroxide_plugins::script::runner::RunEnv;
use diroxide_plugins::script::words;

/// Where an item's state stands now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemState {
    /// Never enabled, or disabled: it contributes nothing.
    NotEnabled,
    /// Enabled, and unchanged since.
    Enabled,
    /// Enabled, but its manifest or its program changed since: it asks
    /// again before it runs.
    Changed,
    /// Its manifest or header has errors: the first, in words.
    HasErrors(String),
}

impl ItemState {
    /// The state in words, for a Manage plugins row: "not enabled".
    pub fn words(&self) -> &'static str {
        match self {
            ItemState::NotEnabled => "not enabled",
            ItemState::Enabled => "enabled",
            ItemState::Changed => "changed, asks again",
            ItemState::HasErrors(_) => "has errors",
        }
    }
}

/// What an item is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A plugin folder, with its version.
    Plugin {
        /// The manifest's version.
        version: String,
        /// Whether it runs a program of its own (a `[run]` table); a
        /// manifest-only plugin runs its commands' programs.
        process: bool,
    },
    /// A one-file script.
    Script,
}

impl ItemKind {
    /// "plugin 0.1.0" or "script".
    pub fn words(&self) -> String {
        match self {
            ItemKind::Plugin { version, .. } => format!("plugin {version}"),
            ItemKind::Script => "script".to_owned(),
        }
    }
}

/// How a command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Runs {
    /// Answered by the plugin's own program (`command/run`).
    Process,
    /// Run by the script runner: a script file, or a manifest-only
    /// command with an argument list.
    Script(Box<ScriptCommand>),
}

/// One command a plugin or a script adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandEntry {
    /// The full id: `line-counter.count`, `script.total-size`.
    pub id: CommandId,
    /// The plugin-local id (`count`), or the script's id.
    pub local: String,
    /// Its name in menus and the palette, meaning first.
    pub name: String,
    /// One sentence for F1 help.
    pub help: String,
    /// The plugin's or the script's name, last in every line it causes.
    pub source: String,
    /// The item's id.
    pub owner: String,
    /// A proposed key, bound only when free.
    pub key: Option<String>,
    /// How many entries it needs.
    pub selection: Selection,
    /// Which places it works on.
    pub places: Places,
    /// Tools it needs that were not found.
    pub missing: Vec<String>,
    /// Whether it is made for another system.
    pub other_system: bool,
    /// How it runs.
    pub runs: Runs,
}

impl CommandEntry {
    /// Why it cannot run with this context, in words, or `None` when it
    /// can: the systems, the tools, the places, the selection's count, a
    /// row that is not a file. A script's own rules come from the runner's
    /// check, so the menu and the run agree.
    pub fn refusal(&self, context: &RunContext) -> Option<String> {
        if self.other_system {
            return Some(words::other_system(&self.name));
        }
        if let Some(tool) = self.missing.first() {
            return Some(words::needs_tool(tool));
        }
        let refusal = match &self.runs {
            Runs::Script(command) => effective(command, context).err(),
            Runs::Process => process_refusal(self.selection, self.places, context),
        };
        refusal.map(|r| r.sentence(&self.name))
    }

    /// Whether the command is a script's or a manifest-only command's.
    pub fn is_script(&self) -> bool {
        matches!(self.runs, Runs::Script(_))
    }
}

/// The count and places check for a command its plugin's program answers,
/// with the runner's words.
fn process_refusal(selection: Selection, places: Places, context: &RunContext) -> Option<Refusal> {
    if places == Places::Local
        && [&context.active, &context.other]
            .iter()
            .any(|p| p.kind() != PlaceKind::Local)
    {
        return Some(Refusal::NotLocal);
    }
    if selection == Selection::None {
        return None;
    }
    let entries = if context.marked.is_empty() {
        context.cursor.iter().cloned().collect::<Vec<_>>()
    } else {
        context.marked.clone()
    };
    if entries.iter().any(|e| e.kind == EntryKind::Item) {
        return Some(Refusal::NotAFile);
    }
    let have = entries.len();
    let fits = match selection {
        Selection::None => true,
        Selection::One => have == 1,
        Selection::Two => have == 2,
        Selection::Any => have >= 1,
        Selection::Many => have >= 2,
    };
    (!fits).then_some(Refusal::Count {
        needs: selection,
        have,
    })
}

/// One plugin or script found.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Its id: the manifest's, or the script's file stem.
    pub id: String,
    /// Its name in words.
    pub name: String,
    /// What it is.
    pub kind: ItemKind,
    /// Its folder, or the script file.
    pub path: PathBuf,
    /// Whether it came from the example plugins component.
    pub example: bool,
    /// Its state now.
    pub state: ItemState,
    /// One sentence of what it does, for F1 help.
    pub description: String,
    /// The consent question for a first enable, one row a line, the last
    /// "Enable it? y or n".
    pub enable_rows: Vec<String>,
    /// The question after a change: "Line counter changed since you
    /// enabled it." ... "Enable it again? y or n".
    pub changed_rows: Vec<String>,
    /// Its commands.
    pub commands: Vec<CommandEntry>,
    /// Its settings, as the manifest declares them.
    pub settings: Vec<SettingSpec>,
    /// Its listers, as the manifest declares them.
    pub listers: Vec<ListerSpec>,
    /// Its manifest, for a plugin whose manifest reads.
    pub manifest: Option<Manifest>,
    /// What the consent covers now: the hashes of the manifest and the
    /// program. `None` when they could not be read.
    pub fingerprint: Option<Fingerprint>,
    /// The owner's values for its settings, from its state folder.
    pub values: serde_json::Map<String, serde_json::Value>,
}

impl Item {
    /// Its row in Manage plugins, meaning first: "Total size of selection:
    /// not enabled, script".
    pub fn row(&self) -> String {
        format!(
            "{}: {}, {}",
            self.name,
            self.state.words(),
            self.kind.words()
        )
    }

    /// Whether it contributes its commands now.
    pub fn contributes(&self) -> bool {
        self.state == ItemState::Enabled
    }

    /// The consent record's kind.
    pub fn consent_kind(&self) -> ConsentKind {
        match self.kind {
            ItemKind::Plugin { .. } => ConsentKind::Plugin,
            ItemKind::Script => ConsentKind::Script,
        }
    }

    /// Whether its program is a process the host starts.
    pub fn is_process(&self) -> bool {
        matches!(self.kind, ItemKind::Plugin { process: true, .. })
    }
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    /// Whether plugins are off for this session (`--no-plugins`): then
    /// nothing contributes and nothing starts.
    pub off: bool,
    /// The items, by id.
    pub items: Vec<Item>,
    /// Problems found while scanning, each a sentence: a command id taken
    /// by diroxide, an id twice.
    pub problems: Vec<String>,
}

impl Catalog {
    /// An item by id.
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The commands of the items that contribute, in item order.
    pub fn commands(&self) -> impl Iterator<Item = &CommandEntry> {
        self.items
            .iter()
            .filter(|i| !self.off && i.contributes())
            .flat_map(|i| i.commands.iter())
    }

    /// A contributing command by its full id.
    pub fn command(&self, id: &CommandId) -> Option<&CommandEntry> {
        self.commands().find(|c| &c.id == id)
    }

    /// A command by its full id, whatever its item's state (the bank or
    /// the menu that names a command of a plugin that is not enabled).
    pub fn any_command(&self, id: &CommandId) -> Option<(&Item, &CommandEntry)> {
        self.items
            .iter()
            .find_map(|i| i.commands.iter().find(|c| &c.id == id).map(|c| (i, c)))
    }
}

/// Where plugins are looked for: the owner's plugins folder first, then
/// the example plugins component's folder, then the plugin components
/// (one plugin folder each, such as `diro-sftp`), whose items lose to the
/// owner's own of the same id.
#[derive(Clone, Debug)]
pub struct Roots {
    /// The owner's plugins folder (its `scripts` folder holds scripts).
    pub plugins: PathBuf,
    /// The examples' `plugins` folder inside the components folder.
    pub examples: Option<PathBuf>,
    /// The components folder, whose plugin components are plugin folders
    /// of their own: the SFTP places plugin unpacks to
    /// `components/diro-sftp` (lane A3).
    pub components: Option<PathBuf>,
    /// The plugins' state folder, where the consent record is.
    pub state: PathBuf,
}

/// The examples' folder inside the components folder.
pub fn examples_dir(components_dir: &Path) -> PathBuf {
    components_dir.join("diro-examples").join("plugins")
}

/// Finds every plugin and script under the roots and reads its state,
/// with `hash` for the consent's fingerprints, `lookup` for the
/// interpreters and `env` for the tools. Ids diroxide's own commands use
/// are refused. Disk I/O: run it on a worker.
pub fn scan(roots: &Roots, hash: Hasher, lookup: &Lookup, env: &RunEnv) -> Catalog {
    let builtin: BTreeSet<&str> = crate::commands::CATALOG
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let store = ConsentStore::open(&roots.state);
    let mut catalog = Catalog::default();
    let mut found: Vec<(Installed, bool)> = installed::scan(&roots.plugins, lookup.os)
        .into_iter()
        .map(|i| (i, false))
        .collect();
    if let Some(examples) = &roots.examples {
        for item in installed::scan(examples, lookup.os) {
            if found.iter().any(|(f, _)| f.id() == item.id()) {
                continue;
            }
            found.push((item, true));
        }
    }
    // A plugin component is its own plugin folder: a folder there without
    // a manifest (the examples pack) is no plugin and is passed over.
    if let Some(components) = &roots.components {
        for item in installed::scan(components, lookup.os) {
            if matches!(item, Installed::Script(_))
                || found.iter().any(|(f, _)| f.id() == item.id())
            {
                continue;
            }
            found.push((item, false));
        }
    }
    for (installed, example) in found {
        let item = read_item(&installed, example, &store, hash, lookup, env);
        let mut item = item;
        item.commands.retain(|c| {
            if builtin.contains(c.id.as_str()) {
                catalog.problems.push(format!(
                    "{} was left out: diroxide's own command has the id {}.",
                    c.name, c.id
                ));
                false
            } else {
                true
            }
        });
        catalog.items.push(item);
    }
    catalog.items.sort_by(|a, b| a.id.cmp(&b.id));
    catalog
}

/// Reads one plugin or script's state, consent rows and commands.
pub fn read_item(
    installed: &Installed,
    example: bool,
    store: &ConsentStore,
    hash: Hasher,
    lookup: &Lookup,
    env: &RunEnv,
) -> Item {
    let errors: Vec<String> = installed::check(installed, lookup)
        .into_iter()
        .filter(|p| p.severity == Severity::Error)
        .map(|p| p.to_string())
        .collect();
    let (kind, path, manifest) = match installed {
        Installed::Plugin(plugin) => (
            ItemKind::Plugin {
                version: plugin
                    .report
                    .manifest
                    .as_ref()
                    .map(|m| m.version.clone())
                    .unwrap_or_default(),
                process: plugin
                    .report
                    .manifest
                    .as_ref()
                    .is_some_and(Manifest::is_process),
            },
            plugin.folder.clone(),
            plugin.report.manifest.clone(),
        ),
        Installed::Script(script) => (ItemKind::Script, script.path.clone(), None),
    };
    let (fingerprint, subject) = fingerprint_and_subject(installed, hash, lookup);
    let mut state = match &fingerprint {
        Some(now) => match store.state(installed.id(), now) {
            ConsentState::NotEnabled => ItemState::NotEnabled,
            ConsentState::Enabled => ItemState::Enabled,
            ConsentState::Changed => ItemState::Changed,
        },
        None => ItemState::HasErrors(format!("{} could not be read", installed.name())),
    };
    if let Some(first) = errors.first() {
        state = ItemState::HasErrors(first.clone());
    }
    if let Installed::Script(script) = installed
        && let Some(why) = trust_problem(&script.path)
    {
        state = ItemState::HasErrors(why);
    }
    let commands = match installed {
        Installed::Plugin(plugin) => match &plugin.report.manifest {
            Some(manifest) => plugin_commands(manifest, &plugin.folder, lookup.os, env),
            None => Vec::new(),
        },
        Installed::Script(_) => installed
            .script_command()
            .map(|command| vec![script_entry(command, installed.id(), lookup.os, env)])
            .unwrap_or_default(),
    };
    let (enable_rows, changed_rows) = match &subject {
        Some(subject) => (subject.enable_question(), subject.changed_question()),
        None => (Vec::new(), Vec::new()),
    };
    let description = match (installed, &manifest) {
        (_, Some(m)) => m.description.clone(),
        (Installed::Script(s), _) => s.header.header.description.clone(),
        _ => String::new(),
    };
    Item {
        id: installed.id().to_owned(),
        name: installed.name(),
        kind,
        path,
        example,
        state,
        description,
        enable_rows,
        changed_rows,
        commands,
        settings: manifest
            .as_ref()
            .map(|m| m.settings.clone())
            .unwrap_or_default(),
        listers: manifest
            .as_ref()
            .map(|m| m.listers.clone())
            .unwrap_or_default(),
        manifest,
        fingerprint,
        values: serde_json::Map::new(),
    }
}

/// The fingerprint a consent covers now, and the question's subject, as
/// `diro plugin enable` makes them.
pub fn fingerprint_and_subject(
    installed: &Installed,
    hash: Hasher,
    lookup: &Lookup,
) -> (Option<Fingerprint>, Option<Subject>) {
    match installed {
        Installed::Plugin(plugin) => {
            let Some(manifest) = plugin.report.manifest.as_ref() else {
                return (None, None);
            };
            (
                plugin_fingerprint(manifest, &plugin.folder, hash).ok(),
                Some(Subject::plugin(manifest, lookup.os)),
            )
        }
        Installed::Script(script) => {
            let header = &script.header.header;
            let file = script
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (runs, policy) = match lookup.choose(&script.path, &header.recipe.interpreter) {
                Ok(interpreter) => (
                    interpreter.runs_line(&script.path),
                    interpreter.policy_override,
                ),
                Err(_) => (format!("It runs: {file}"), false),
            };
            (
                script_fingerprint(&script.path, hash).ok(),
                Some(Subject::script(
                    &installed.name(),
                    runs,
                    policy,
                    header.recipe.unattended,
                )),
            )
        }
    }
}

fn missing_tools(needs: &[String], env: &RunEnv) -> Vec<String> {
    needs
        .iter()
        .filter(|tool| env.find_tool(tool).is_none())
        .cloned()
        .collect()
}

fn plugin_commands(manifest: &Manifest, folder: &Path, os: Os, env: &RunEnv) -> Vec<CommandEntry> {
    manifest
        .commands
        .iter()
        .enumerate()
        .filter_map(|(index, spec)| plugin_command(manifest, index, spec, folder, os, env))
        .collect()
}

fn plugin_command(
    manifest: &Manifest,
    index: usize,
    spec: &CommandSpec,
    folder: &Path,
    os: Os,
    env: &RunEnv,
) -> Option<CommandEntry> {
    let id = CommandId::new(spec.full_id(&manifest.id)).ok()?;
    let runs = match ScriptCommand::from_manifest(manifest, index, spec, folder) {
        Some((command, _)) => Runs::Script(Box::new(command)),
        None => Runs::Process,
    };
    Some(CommandEntry {
        id,
        local: spec.id.clone(),
        name: spec.name.clone(),
        help: spec.description.clone(),
        source: manifest.name.clone(),
        owner: manifest.id.clone(),
        key: spec.key.clone(),
        selection: spec.selection,
        places: spec.places,
        missing: missing_tools(&spec.needs, env),
        other_system: !(spec.os.is_empty() || spec.os.contains(&os)),
        runs,
    })
}

fn script_entry(command: ScriptCommand, owner: &str, os: Os, env: &RunEnv) -> CommandEntry {
    let id = CommandId::new(command.id.clone())
        .unwrap_or_else(|_| CommandId::from_static("script.unnamed"));
    CommandEntry {
        id,
        local: owner.to_owned(),
        name: command.title.clone(),
        help: command.description.clone(),
        source: command.source.clone(),
        owner: owner.to_owned(),
        key: None,
        selection: command.selection,
        places: command.places,
        missing: missing_tools(&command.needs, env),
        other_system: !command.exists_on(os),
        runs: Runs::Script(Box::new(command)),
    }
}

/// The context of a run from the panels: the active and the other
/// panel's places, their cursors, and the marked entries, each with its
/// kind (an item row is not a file).
pub fn context_of(
    active: &Place,
    other: &Place,
    cursor: Option<(Place, EntryKind)>,
    other_cursor: Option<(Place, EntryKind)>,
    marked: Vec<(Place, EntryKind)>,
) -> RunContext {
    use diroxide_plugins::script::context::Picked;
    let mut context = RunContext::new(active.clone(), other.clone());
    context.cursor = cursor.map(|(p, k)| Picked::new(p, k));
    context.other_cursor = other_cursor.map(|(p, k)| Picked::new(p, k));
    context.marked = marked.into_iter().map(|(p, k)| Picked::new(p, k)).collect();
    context
}
