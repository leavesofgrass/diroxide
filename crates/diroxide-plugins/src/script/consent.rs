//! Consent: nothing runs until the owner enables it, and the record of
//! that yes covers the code as well as the manifest (R22 section 7.1;
//! answers 91 and 92).
//!
//! - **What is hashed.** For a plugin, its `plugin.toml` and the program
//!   file its command names (the first argument of `[run] command`, for
//!   this system, that is a file inside the plugin's folder, such as
//!   `main.py`); for a script, the whole file. A change to either asks
//!   again: "Line counter changed since you enabled it." The hash function
//!   is the caller's (SHA-256, from the components crate), so this crate
//!   carries no hashing code of its own.
//! - **Where.** `consent.toml` in the plugins' state folder, written
//!   whole through a temporary file and a rename. Installing a component
//!   never writes it; `--no-plugins` never reads it.
//! - **The question** says, one row a line on 40 cells, what the plugin
//!   runs, what diroxide checks (the manifest's permissions), what the
//!   plugin states and diroxide cannot check, and plainly "While it runs
//!   it can do what you can."; it ends "Enable it? y or n", starting on
//!   No. A PowerShell script says that its policy is overridden for it; an
//!   unattended command says "It can run without asking."
//! - **Folder trust** (Linux and macOS): a script whose file or folder
//!   others can change is refused, as Midnight Commander refuses such a
//!   menu file.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{Access, Manifest, Os};

/// A hash function: bytes in, lowercase hex out (SHA-256 in diroxide).
pub type Hasher = fn(&[u8]) -> String;

/// The consent file's name in the plugins' state folder.
pub const CONSENT_FILE: &str = "consent.toml";

/// What a plugin or script is: what the consent record covers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// The manifest's hash; none for a script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    /// The program file's hash: the script itself, or the file a plugin's
    /// command names; none for a manifest-only plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_sha256: Option<String>,
}

/// The program file a plugin's `[run] command` names on this system: the
/// first argument that is a file inside the plugin's folder.
pub fn program_file(manifest: &Manifest, folder: &Path) -> Option<PathBuf> {
    let run = manifest.run.as_ref()?;
    run.command.iter().find_map(|arg| {
        let relative = arg.strip_prefix("./").unwrap_or(arg);
        if relative.is_empty() || Path::new(relative).is_absolute() || relative.contains("..") {
            return None;
        }
        let candidate = folder.join(&run.cwd).join(relative);
        candidate.is_file().then_some(candidate)
    })
}

/// A plugin's fingerprint: its manifest's bytes and its program file.
pub fn plugin_fingerprint(
    manifest: &Manifest,
    folder: &Path,
    hash: Hasher,
) -> io::Result<Fingerprint> {
    let text = fs::read(folder.join(Manifest::FILE))?;
    let program = match program_file(manifest, folder) {
        Some(path) => Some(hash(&fs::read(path)?)),
        None => None,
    };
    Ok(Fingerprint {
        manifest_sha256: Some(hash(&text)),
        program_sha256: program,
    })
}

/// A script's fingerprint: the whole file.
pub fn script_fingerprint(path: &Path, hash: Hasher) -> io::Result<Fingerprint> {
    Ok(Fingerprint {
        manifest_sha256: None,
        program_sha256: Some(hash(&fs::read(path)?)),
    })
}

/// What the consent record says about a plugin or script now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentState {
    /// Never enabled, or disabled.
    NotEnabled,
    /// Enabled, and unchanged since.
    Enabled,
    /// Enabled, but its manifest or its program changed since: it asks
    /// again before it runs.
    Changed,
}

/// Whether a record is a plugin's or a script's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A plugin folder with a manifest.
    Plugin,
    /// A one-file script.
    Script,
}

/// One yes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// A plugin's or a script's.
    pub kind: Kind,
    /// What was enabled.
    #[serde(flatten)]
    pub fingerprint: Fingerprint,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    enabled: BTreeMap<String, Record>,
}

/// The consent records, in `consent.toml`.
#[derive(Debug)]
pub struct ConsentStore {
    path: PathBuf,
    file: File,
}

impl ConsentStore {
    /// The records in `folder` (the plugins' state folder); none when the
    /// file is missing. A file that cannot be read counts as no consent at
    /// all, so nothing runs on a damaged record.
    pub fn open(folder: &Path) -> Self {
        let path = folder.join(CONSENT_FILE);
        let file = fs::read_to_string(&path)
            .ok()
            .and_then(|text| toml::from_str::<File>(&text).ok())
            .unwrap_or_default();
        Self { path, file }
    }

    /// Where the records are kept.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether an id is enabled with this fingerprint.
    pub fn state(&self, id: &str, now: &Fingerprint) -> ConsentState {
        match self.file.enabled.get(id) {
            None => ConsentState::NotEnabled,
            Some(record) if record.fingerprint == *now => ConsentState::Enabled,
            Some(_) => ConsentState::Changed,
        }
    }

    /// The ids enabled, whatever their state.
    pub fn enabled_ids(&self) -> Vec<String> {
        self.file.enabled.keys().cloned().collect()
    }

    /// Records a yes, and writes the file.
    pub fn enable(&mut self, id: &str, kind: Kind, fingerprint: Fingerprint) -> io::Result<()> {
        self.file
            .enabled
            .insert(id.to_owned(), Record { kind, fingerprint });
        self.save()
    }

    /// Forgets a yes, and writes the file. False when it was not enabled.
    pub fn disable(&mut self, id: &str) -> io::Result<bool> {
        let was = self.file.enabled.remove(id).is_some();
        if was {
            self.save()?;
        }
        Ok(was)
    }

    fn save(&self) -> io::Result<()> {
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        let text = toml::to_string(&self.file).map_err(io::Error::other)?;
        let header = "# Plugins and scripts the owner enabled, each with the hashes of\n# what was enabled. diroxide writes this file; a change to a plugin\n# or a script makes it ask again.\n\n";
        let temporary = self.path.with_extension("toml.new");
        fs::write(&temporary, format!("{header}{text}"))?;
        fs::rename(&temporary, &self.path)
    }
}

/// What the consent question is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    /// The plugin's or the script's name.
    pub name: String,
    /// A plugin's version; none for a script.
    pub version: Option<String>,
    /// "It runs: py -3 main.py".
    pub runs: String,
    /// What diroxide enforces, each "it may ...".
    pub checks: Vec<String>,
    /// What the plugin states and diroxide cannot check.
    pub states: Vec<String>,
    /// Whether it may run any program through diroxide.
    pub any_program: bool,
    /// Whether PowerShell's policy is overridden for it.
    pub policy_override: bool,
    /// Whether it can run unattended.
    pub unattended: bool,
}

impl Subject {
    /// The subject of a plugin's manifest on a system: its program, its
    /// permissions in words, and what it states.
    pub fn plugin(manifest: &Manifest, os: Os) -> Self {
        let runs = match &manifest.run {
            Some(run) => format!("It runs: {}", run.command.join(" ")),
            None => {
                let mut programs: Vec<String> = Vec::new();
                let argvs = manifest
                    .commands
                    .iter()
                    .filter(|c| c.os.is_empty() || c.os.contains(&os))
                    .map(|c| &c.argv)
                    .chain(manifest.previewers.iter().map(|p| &p.argv))
                    .chain(manifest.converters.iter().map(|c| &c.argv));
                for argv in argvs {
                    if let Some(first) = argv.first()
                        && !first.starts_with('{')
                        && !programs.contains(first)
                    {
                        programs.push(first.clone());
                    }
                }
                if programs.is_empty() {
                    "It runs: the program in its settings.".to_owned()
                } else {
                    format!("It runs: {}", programs.join(", "))
                }
            }
        };
        let p = &manifest.permissions;
        let mut checks = Vec::new();
        match p.fs {
            Access::None => {}
            Access::Read => checks.push("it may read files.".to_owned()),
            _ => checks.push("it may read and change files.".to_owned()),
        }
        match p.panel {
            Access::None => {}
            Access::Read => checks.push("it may read the panels.".to_owned()),
            _ => checks.push("it may change the panels.".to_owned()),
        }
        if !p.tools.is_empty() {
            checks.push(format!("it may run: {}.", p.tools.join(", ")));
        }
        if !p.launch.is_empty() {
            checks.push(format!("it may start: {}.", p.launch.join(", ")));
        }
        if !p.commands.is_empty() {
            checks.push("it may run diroxide commands.".to_owned());
        }
        let mut states = Vec::new();
        for text in [&p.declared.network, &p.declared.files, &p.declared.programs]
            .into_iter()
            .flatten()
        {
            let clean = crate::services::clean_text(text, 80);
            if !clean.is_empty() {
                let end = if clean.ends_with(['.', '!', '?']) {
                    ""
                } else {
                    "."
                };
                states.push(format!("{clean}{end}"));
            }
        }
        let unattended = manifest.commands.iter().any(|c| {
            c.extra
                .get("unattended")
                .and_then(toml::Value::as_str)
                .is_some_and(|v| v == "ok")
        });
        Self {
            name: manifest.name.clone(),
            version: Some(manifest.version.clone()),
            runs,
            checks,
            states,
            any_program: p.any_program(),
            policy_override: false,
            unattended,
        }
    }

    /// The subject of a script: its interpreter's line, and whether the
    /// PowerShell policy is overridden for it.
    pub fn script(name: &str, runs: String, policy_override: bool, unattended: bool) -> Self {
        Self {
            name: name.to_owned(),
            version: None,
            runs,
            checks: Vec::new(),
            states: Vec::new(),
            any_program: false,
            policy_override,
            unattended,
        }
    }

    /// The rows of the question, without its last line: what it is, what
    /// it runs, what is checked, what is stated, and what it can do.
    pub fn rows(&self, first: &str) -> Vec<String> {
        let mut rows = vec![first.to_owned(), self.runs.clone()];
        if self.policy_override {
            rows.push("It overrides PowerShell's script policy.".to_owned());
        }
        match self.checks.as_slice() {
            [] => {}
            [one] => rows.push(format!("diroxide checks: {one}")),
            many => {
                rows.push("diroxide checks:".to_owned());
                rows.extend(many.iter().cloned());
            }
        }
        if !self.states.is_empty() {
            rows.push("The plugin states, unchecked:".to_owned());
            rows.extend(self.states.iter().cloned());
        }
        if self.any_program {
            rows.push("This plugin can run any program.".to_owned());
        }
        if self.unattended {
            rows.push("It can run without asking.".to_owned());
        }
        rows.push("While it runs it can do what you can.".to_owned());
        rows
    }

    /// The first enable: "Enable Line counter, version 0.1.0?" ... "Enable
    /// it? y or n".
    pub fn enable_question(&self) -> Vec<String> {
        let first = match &self.version {
            Some(version) => format!("Enable {}, version {version}?", self.name),
            None => format!("Enable {}?", self.name),
        };
        let mut rows = self.rows(&first);
        rows.push("Enable it? y or n".to_owned());
        rows
    }

    /// After a change: "Line counter changed since you enabled it." ...
    /// "Enable it again? y or n".
    pub fn changed_question(&self) -> Vec<String> {
        let mut rows = self.rows(&format!("{} changed since you enabled it.", self.name));
        rows.push("Enable it again? y or n".to_owned());
        rows
    }

    /// Running once without enabling (`diro plugin run`): "Line counter is
    /// not enabled." ... "Run it once? y or n".
    pub fn run_once_question(&self) -> Vec<String> {
        let mut rows = self.rows(&format!("{} is not enabled.", self.name));
        rows.push("Run it once? y or n".to_owned());
        rows
    }
}

/// "Enabled: Line counter."
pub fn enabled_line(name: &str) -> String {
    format!("Enabled: {name}.")
}

/// "Not enabled."
pub fn not_enabled_line() -> String {
    "Not enabled.".to_owned()
}

/// "Disabled: Line counter."
pub fn disabled_line(name: &str) -> String {
    format!("Disabled: {name}.")
}

/// Why a script's file or folder cannot be trusted: others can change it
/// (Linux and macOS). `None` when only the owner can.
#[cfg(unix)]
pub fn trust_problem(script: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let writable_by_others =
        |path: &Path| fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o022 != 0);
    if writable_by_others(script) {
        return Some("others can change its file".to_owned());
    }
    match script.parent() {
        Some(folder) if writable_by_others(folder) => {
            Some("others can change its folder".to_owned())
        }
        _ => None,
    }
}

/// Why a script's file or folder cannot be trusted. Windows' access lists
/// are not read yet, so nothing is refused there.
#[cfg(not(unix))]
pub fn trust_problem(_script: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    fn fake_hash(bytes: &[u8]) -> String {
        format!(
            "{:016x}",
            bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
            })
        )
    }

    const LINE_COUNTER: &str = r#"
[plugin]
id = "line-counter"
name = "Line counter"
version = "0.1.0"
protocol = 1

[run]
command = ["python3", "main.py"]

[run.windows]
command = ["py", "-3", "main.py"]

[permissions]
fs = "read"

[permissions.declared]
network = "no network"
files = "only the files it is given"
programs = "no other programs"

[[command]]
id = "count_lines"
name = "Count lines"
"#;

    #[test]
    fn the_consent_text_of_section_nine() {
        let manifest = Manifest::check_for(LINE_COUNTER, Os::Linux)
            .manifest
            .unwrap();
        let rows = Subject::plugin(&manifest, Os::Linux).enable_question();
        assert_eq!(
            rows,
            [
                "Enable Line counter, version 0.1.0?",
                "It runs: python3 main.py",
                "diroxide checks: it may read files.",
                "The plugin states, unchecked:",
                "no network.",
                "only the files it is given.",
                "no other programs.",
                "While it runs it can do what you can.",
                "Enable it? y or n",
            ]
        );
        for row in &rows {
            assert!(row.chars().count() <= BRAILLE_CELLS, "{row}");
        }
        let windows = Manifest::check_for(LINE_COUNTER, Os::Windows)
            .manifest
            .unwrap();
        assert_eq!(
            Subject::plugin(&windows, Os::Windows).enable_question()[1],
            "It runs: py -3 main.py"
        );
        let changed = Subject::plugin(&manifest, Os::Linux).changed_question();
        assert_eq!(changed[0], "Line counter changed since you enabled it.");
        assert!(diroxide_core::format::within_cells(
            &changed[0],
            "Line counter changed"
        ));
        assert_eq!(changed.last().unwrap(), "Enable it again? y or n");
        let once = Subject::plugin(&manifest, Os::Linux).run_once_question();
        assert_eq!(once[0], "Line counter is not enabled.");
        assert_eq!(once.last().unwrap(), "Run it once? y or n");
    }

    #[test]
    fn a_scripts_question_names_its_interpreter_and_the_policy() {
        let subject = Subject::script(
            "Tidy downloads",
            "It runs: powershell tidy.ps1".to_owned(),
            true,
            true,
        );
        assert_eq!(
            subject.enable_question(),
            [
                "Enable Tidy downloads?",
                "It runs: powershell tidy.ps1",
                "It overrides PowerShell's script policy.",
                "It can run without asking.",
                "While it runs it can do what you can.",
                "Enable it? y or n",
            ]
        );
        assert_eq!(enabled_line("Tidy downloads"), "Enabled: Tidy downloads.");
        assert_eq!(not_enabled_line(), "Not enabled.");
    }

    #[test]
    fn several_checks_get_a_row_each() {
        let text = LINE_COUNTER.replace(
            "fs = \"read\"",
            "fs = \"write\"\npanel = \"read\"\ntools = [\"git\"]",
        );
        let manifest = Manifest::check_for(&text, Os::Linux).manifest.unwrap();
        let rows = Subject::plugin(&manifest, Os::Linux).enable_question();
        assert_eq!(
            &rows[2..6],
            [
                "diroxide checks:",
                "it may read and change files.",
                "it may read the panels.",
                "it may run: git.",
            ]
        );
    }

    #[test]
    fn a_change_to_the_program_asks_again() {
        let folder = tempfile::tempdir().unwrap();
        let plugin = folder.path().join("line-counter");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("plugin.toml"), LINE_COUNTER).unwrap();
        fs::write(plugin.join("main.py"), "print('hi')\n").unwrap();
        let manifest = Manifest::check_for(LINE_COUNTER, Os::Linux)
            .manifest
            .unwrap();
        assert_eq!(
            program_file(&manifest, &plugin),
            Some(plugin.join("main.py"))
        );
        let before = plugin_fingerprint(&manifest, &plugin, fake_hash).unwrap();
        assert!(before.program_sha256.is_some());
        let state_folder = folder.path().join("state");
        let mut store = ConsentStore::open(&state_folder);
        assert_eq!(
            store.state("line-counter", &before),
            ConsentState::NotEnabled
        );
        store
            .enable("line-counter", Kind::Plugin, before.clone())
            .unwrap();
        let reopened = ConsentStore::open(&state_folder);
        assert_eq!(
            reopened.state("line-counter", &before),
            ConsentState::Enabled
        );
        assert_eq!(reopened.enabled_ids(), ["line-counter"]);
        // A blank line added to main.py, as in the owner's check.
        fs::write(plugin.join("main.py"), "print('hi')\n\n").unwrap();
        let after = plugin_fingerprint(&manifest, &plugin, fake_hash).unwrap();
        assert_eq!(after.manifest_sha256, before.manifest_sha256);
        assert_eq!(
            reopened.state("line-counter", &after),
            ConsentState::Changed
        );
        let mut store = reopened;
        assert!(store.disable("line-counter").unwrap());
        assert!(!store.disable("line-counter").unwrap());
        assert_eq!(
            ConsentStore::open(&state_folder).state("line-counter", &before),
            ConsentState::NotEnabled
        );
    }

    #[test]
    fn a_damaged_record_enables_nothing() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join(CONSENT_FILE), "not = [toml").unwrap();
        let store = ConsentStore::open(folder.path());
        assert!(store.enabled_ids().is_empty());
    }

    #[test]
    fn a_script_is_hashed_whole() {
        let folder = tempfile::tempdir().unwrap();
        let script = folder.path().join("total-size.sh");
        fs::write(&script, "echo one\n").unwrap();
        let one = script_fingerprint(&script, fake_hash).unwrap();
        fs::write(&script, "echo two\n").unwrap();
        let two = script_fingerprint(&script, fake_hash).unwrap();
        assert_ne!(one, two);
        assert!(one.manifest_sha256.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_script_others_can_change_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let scripts = folder.path().join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::set_permissions(&scripts, fs::Permissions::from_mode(0o755)).unwrap();
        let script = scripts.join("a.sh");
        fs::write(&script, "exit 0\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(trust_problem(&script), None);
        fs::set_permissions(&script, fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            trust_problem(&script).as_deref(),
            Some("others can change its file")
        );
        fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&scripts, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            trust_problem(&script).as_deref(),
            Some("others can change its folder")
        );
        fs::set_permissions(&scripts, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
