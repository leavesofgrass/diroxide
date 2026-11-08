//! The external tools registry: one table of programs diroxide can use
//! when they are present and never requires ([`BUILTIN`]), each found in
//! one order, and run with an argument list: no shell, no console window
//! on Windows, output captured, cancellable.
//!
//! **Where a tool is found** (the owner's answer 124, textweaver's
//! order; [`LOOKUP_ORDER`]): its `path` in `tools.toml`, then an
//! installed helper component, then PATH, then the folders its installers
//! use (Program Files for 7-Zip and Tesseract, Git for Windows, Gpg4win,
//! Homebrew on macOS). The report says where in words: "ImageMagick:
//! 7.1.2-35, on PATH", "7-Zip: 24.08, in Program Files".
//!
//! **What counts as the tool.** ImageMagick's version line must say
//! "ImageMagick" ([`ToolSpec::must_say`]), and on Windows only `magick`
//! is looked for, so System32's `convert.exe`, a disk tool, is never
//! taken for it. On Windows `PATHEXT` comes before the bare name
//! ([`lookup`]).
//!
//! **The version** is read from the version command's standard output,
//! else its standard error, the first non-empty line, cleaned to 80 cells,
//! never with a file argument ([`version`]).
//!
//! Every child diroxide starts, a plugin, a script command or a tool,
//! goes through the process helper ([`process`]): no shell, no console
//! window, and a tree that is killed as a whole. Every run under limits
//! goes through the one runner ([`run`]), which was the plugin host's:
//! the host, the manifest recipes, the registry and the version reader
//! all call it.

use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use diroxide_config::ToolsConfig;
use diroxide_core::Explain;
use diroxide_core::format::human_size;
use diroxide_vfs::{OpContext, RunLimits};
use thiserror::Error;

pub mod lookup;
pub mod package;
pub mod process;
pub mod report;
pub mod run;
mod runner;
pub mod table;
pub mod version;

pub use lookup::{Base, Bases, InstallFolder, SYSTEM_FOLDERS, file_names, which, which_all};
pub use package::{Packages, Platform, package_command_for};
pub use process::{ProcessError, ProcessHandle, ProcessSpec, Stream, Tie};
pub use report::{
    FoundBy, LOOKUP_ORDER, LookupStep, NextStep, ToolKind, ToolReport, ToolState, ToolsReport,
};
pub use run::{AtCap, Caps, Cut, Emit, Finished, Limited, RunFailure, Streaming, run_limited};
pub use table::{BUILTIN, CARTA_VERSION};
pub use version::{version_line, version_line_within, version_number};

/// One external tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolSpec {
    /// The id used in recipes, actions and `tools.toml`: `magick`.
    pub id: &'static str,
    /// The name said in Settings: "ImageMagick".
    pub name: &'static str,
    /// Program names to look for on this system, in order: `magick`
    /// alone on Windows.
    pub programs: &'static [&'static str],
    /// The arguments that make it print its version, and only those,
    /// never a file: `i` for 7-Zip, whose banner before the list of
    /// formats is its version; empty for a program with no version
    /// command (Explorer).
    pub version_args: &'static [&'static str],
    /// Words its version line must hold for a program to count as this
    /// tool: "ImageMagick", which a `convert` that is not ImageMagick's
    /// never says.
    pub must_say: Option<&'static str>,
    /// The folders its installers use on this system, looked in after
    /// PATH.
    pub install_folders: &'static [InstallFolder],
    /// What diroxide uses it for, in words.
    pub purpose: &'static str,
    /// Where to get it, in words: said when no package command is known.
    pub install_hint: &'static str,
    /// Its packages, for the command that installs it on this system
    /// ([`ToolRegistry::package_command`]).
    pub packages: Packages,
    /// A program, or built into diro (carta).
    pub kind: ToolKind,
}

/// The arguments every curl run starts with: `-q` first, so a planted
/// configuration file cannot switch off certificate checks (plan section
/// 5.14), then failure on HTTP errors, quiet progress, and redirects
/// followed only over HTTPS.
pub const CURL_SAFE_ARGS: &[&str] = &[
    "-q",
    "--fail",
    "--silent",
    "--show-error",
    "--location",
    "--proto",
    "=https",
    "--proto-redir",
    "=https",
];

/// Whether a tool can be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    /// Found, at this path.
    Found(PathBuf),
    /// Not found anywhere the lookup looks, or not at its `tools.toml`
    /// path.
    NotFound,
    /// Turned off in `tools.toml`.
    Disabled,
}

/// Where a lookup ended, with where a found program was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// Found at this path, by this step.
    Found {
        /// The program.
        path: PathBuf,
        /// The step that found it.
        by: FoundBy,
    },
    /// Built into diro: carta.
    BuiltIn,
    /// Not found anywhere the lookup looks.
    NotFound,
    /// `tools.toml` gives a path with no program at it.
    PathMissing,
    /// Turned off in `tools.toml`.
    TurnedOff,
}

/// What a tool printed and how it ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolOutput {
    /// The exit code, or `None` when it was stopped by a signal.
    pub code: Option<i32>,
    /// Standard output.
    pub stdout: Vec<u8>,
    /// Standard error.
    pub stderr: Vec<u8>,
}

impl ToolOutput {
    /// Whether it exited with code zero.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Why a tool could not be used.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ToolError {
    /// No tool has this id.
    #[error("no tool is called {0}")]
    Unknown(String),
    /// The tool is not installed.
    #[error("{0} is not installed")]
    NotFound(String),
    /// The tool is turned off.
    #[error("{0} is turned off in tools.toml")]
    Disabled(String),
    /// The tool is built into diro and runs in process, never as a
    /// program.
    #[error("{0} is built into diro and is not run as a program")]
    BuiltIn(String),
    /// The run was cancelled.
    #[error("it was cancelled")]
    Cancelled,
    /// The tool could not be started.
    #[error("{tool} could not be started: {message}")]
    Start {
        /// The tool's name.
        tool: String,
        /// The system's words.
        message: String,
    },
    /// The tool ran past its time limit and was stopped, with every child
    /// it started.
    #[error("{tool} took longer than {} and was stopped", diroxide_vfs::runner::seconds_words(.seconds))]
    TimedOut {
        /// The tool's name.
        tool: String,
        /// The limit, in whole seconds.
        seconds: u64,
    },
    /// The tool wrote more than its output cap and was stopped.
    #[error("{tool} wrote more than {limit} and was stopped")]
    TooMuchOutput {
        /// The tool's name.
        tool: String,
        /// The cap in words: "16.8 MB".
        limit: String,
    },
}

impl Explain for ToolError {
    fn next_step(&self) -> String {
        match self {
            ToolError::NotFound(_) => "Install it, or set its path in tools.toml.".to_owned(),
            ToolError::Disabled(_) => "Turn it on in tools.toml.".to_owned(),
            ToolError::Unknown(_)
            | ToolError::BuiltIn(_)
            | ToolError::Cancelled
            | ToolError::Start { .. }
            | ToolError::TimedOut { .. }
            | ToolError::TooMuchOutput { .. } => String::new(),
        }
    }
}

/// The tools, with the owner's overrides, the components that hold
/// programs, and where else to look.
#[derive(Clone, Debug)]
pub struct ToolRegistry {
    specs: Vec<ToolSpec>,
    overrides: ToolsConfig,
    path_var: Option<OsString>,
    bases: Bases,
    components: Vec<PathBuf>,
    /// Whether a program said its tool's [`ToolSpec::must_say`] words,
    /// by path, so a version command runs once per program.
    verdicts: Arc<Mutex<HashMap<PathBuf, bool>>>,
    /// The system, for package commands.
    platform: Platform,
    /// `/etc/os-release`'s text on Linux, read once.
    os_release: Option<String>,
    /// How long a version command may run: [`version::VERSION_TIMEOUT`],
    /// or longer for tests ([`ToolRegistry::with_version_timeout`]).
    version_timeout: Duration,
}

impl ToolRegistry {
    /// The built-in tools, looked for on the process's PATH and in the
    /// system's install folders.
    pub fn builtin() -> Self {
        Self {
            specs: BUILTIN.to_vec(),
            overrides: ToolsConfig::default(),
            path_var: env::var_os("PATH"),
            bases: Bases::from_environment(),
            components: Vec::new(),
            verdicts: Arc::default(),
            platform: Platform::current(),
            os_release: package::os_release(),
            version_timeout: version::VERSION_TIMEOUT,
        }
    }

    /// The same registry letting a version command run for `timeout`
    /// rather than two seconds: for tests whose fake tools are scripts,
    /// which a machine loaded with other builds can be slow to start. A
    /// passing run never waits it out, since the command's end is the
    /// event the read waits for.
    pub fn with_version_timeout(mut self, timeout: Duration) -> Self {
        self.version_timeout = timeout;
        self
    }

    /// How long a version command may run.
    pub fn version_timeout(&self) -> Duration {
        self.version_timeout
    }

    /// The same registry naming package commands for another system, for
    /// tests and for a report about another computer.
    pub fn with_platform(mut self, platform: Platform, os_release: Option<String>) -> Self {
        self.platform = platform;
        self.os_release = os_release;
        self
    }

    /// The command that installs a tool from this system's packages, for
    /// the owner to read or copy: "winget install JohnMacFarlane.Pandoc",
    /// "sudo apt install pandoc"; `None` when the system's manager has no
    /// package for it or is not known.
    pub fn package_command(&self, id: &str) -> Option<String> {
        let spec = self.spec(id)?;
        package_command_for(&spec.packages, self.platform, self.os_release.as_deref())
    }

    /// The next step for a tool that is not found: its package command,
    /// else its sentence.
    pub fn next_step(&self, id: &str) -> Option<NextStep> {
        let spec = self.spec(id)?;
        Some(match self.package_command(id) {
            Some(command) => NextStep::Command(command),
            None => NextStep::Words(spec.install_hint.to_owned()),
        })
    }

    /// The same registry with `tools.toml` overrides.
    pub fn with_overrides(mut self, overrides: ToolsConfig) -> Self {
        self.overrides = overrides;
        self
    }

    /// The same registry looking on another PATH and in no install
    /// folder, for tests: it finds only what the test puts there, its
    /// `tools.toml` paths and the components it is given.
    pub fn with_path(mut self, path_var: Option<OsString>) -> Self {
        self.path_var = path_var;
        self.bases = Bases::none();
        self
    }

    /// The same registry with these install folders' bases.
    pub fn with_bases(mut self, bases: Bases) -> Self {
        self.bases = bases;
        self
    }

    /// The same registry with the programs of the installed helper
    /// components: each path is a program inside a component's folder
    /// (its `exec`), matched to a tool by its file name.
    pub fn with_components(mut self, programs: Vec<PathBuf>) -> Self {
        self.components = programs;
        self
    }

    /// Every tool, in table order.
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// A tool by id.
    pub fn spec(&self, id: &str) -> Option<&ToolSpec> {
        self.specs.iter().find(|spec| spec.id == id)
    }

    /// Where a tool is, by the lookup's order ([`LOOKUP_ORDER`]), and
    /// which step found it.
    pub fn locate(&self, id: &str) -> Result<Location, ToolError> {
        let spec = self
            .spec(id)
            .ok_or_else(|| ToolError::Unknown(id.to_owned()))?;
        if spec.kind == ToolKind::BuiltIn {
            return Ok(Location::BuiltIn);
        }
        for step in LOOKUP_ORDER {
            if let Some(location) = self.look(spec, step) {
                return Ok(location);
            }
        }
        Ok(Location::NotFound)
    }

    /// One step of the lookup: `Some` when it settles the tool.
    fn look(&self, spec: &ToolSpec, step: LookupStep) -> Option<Location> {
        match step {
            LookupStep::ToolsToml => {
                let own = self.overrides.tools.get(spec.id)?;
                if own.disabled {
                    return Some(Location::TurnedOff);
                }
                let path = PathBuf::from(own.path.as_ref()?);
                // The owner chose this path: nothing else is looked in,
                // and its program is not second-guessed.
                Some(if path.is_file() {
                    Location::Found {
                        path,
                        by: FoundBy::ToolsToml,
                    }
                } else {
                    Location::PathMissing
                })
            }
            LookupStep::Component => self
                .components
                .iter()
                .find(|path| is_program_of(path, spec) && path.is_file() && self.says(spec, path))
                .map(|path| Location::Found {
                    path: path.clone(),
                    by: FoundBy::Component,
                }),
            LookupStep::Path => spec.programs.iter().find_map(|program| {
                which_all(program, self.path_var.as_deref())
                    .into_iter()
                    .find(|candidate| self.says(spec, candidate))
                    .map(|path| Location::Found {
                        path,
                        by: FoundBy::Path,
                    })
            }),
            LookupStep::InstallFolders => spec
                .install_folders
                .iter()
                .chain(SYSTEM_FOLDERS)
                .find_map(|folder| {
                    self.bases.folders(folder).into_iter().find_map(|dir| {
                        spec.programs.iter().find_map(|program| {
                            lookup::in_folder(&dir, &file_names(program))
                                .into_iter()
                                .find(|candidate| self.says(spec, candidate))
                                .map(|path| Location::Found {
                                    path,
                                    by: FoundBy::InstallFolder(folder.words.to_owned()),
                                })
                        })
                    })
                }),
        }
    }

    /// Whether a program says its tool's [`ToolSpec::must_say`] words in
    /// its version line; always for a tool with none. Each program's
    /// version command runs once per registry and its clones.
    fn says(&self, spec: &ToolSpec, path: &Path) -> bool {
        let Some(words) = spec.must_say else {
            return true;
        };
        if let Some(verdict) = self
            .verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
        {
            return *verdict;
        }
        let verdict = !spec.version_args.is_empty()
            && version_line_within(path, spec.version_args, self.version_timeout)
                .is_some_and(|line| line.contains(words));
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), verdict);
        verdict
    }

    /// Whether and where a tool is. A built-in tool is not a program, so
    /// it is an error here; [`ToolRegistry::has`] says it is there.
    pub fn find(&self, id: &str) -> Result<ToolStatus, ToolError> {
        match self.locate(id)? {
            Location::Found { path, .. } => Ok(ToolStatus::Found(path)),
            Location::BuiltIn => Err(ToolError::BuiltIn(self.name_of(id))),
            Location::NotFound | Location::PathMissing => Ok(ToolStatus::NotFound),
            Location::TurnedOff => Ok(ToolStatus::Disabled),
        }
    }

    /// Whether a tool can be used: found, or built in.
    pub fn has(&self, id: &str) -> bool {
        matches!(
            self.locate(id),
            Ok(Location::Found { .. } | Location::BuiltIn)
        )
    }

    /// The tool's name as said, or the id when the table does not know it.
    pub fn name_of(&self, id: &str) -> String {
        self.spec(id)
            .map_or_else(|| id.to_owned(), |spec| spec.name.to_owned())
    }

    /// Every tool's line for the Settings Tools page and `diro doctor`,
    /// in the table's order: each looked up, and each found program's
    /// version read, the programs side by side so the report takes about
    /// as long as the slowest one (two seconds at most).
    pub fn report(&self) -> ToolsReport {
        let tools = thread::scope(|scope| {
            let running: Vec<_> = self
                .specs
                .iter()
                .map(|spec| (spec, scope.spawn(move || self.report_one(spec))))
                .collect();
            running
                .into_iter()
                .map(|(spec, handle)| {
                    handle.join().unwrap_or_else(|_| ToolReport {
                        id: spec.id.to_owned(),
                        name: spec.name.to_owned(),
                        state: ToolState::NotFound { install: None },
                    })
                })
                .collect()
        });
        ToolsReport { tools }
    }

    /// One tool's line of the report.
    fn report_one(&self, spec: &ToolSpec) -> ToolReport {
        let state = match self.locate(spec.id) {
            Ok(Location::BuiltIn) => ToolState::Found {
                version: (spec.id == "carta").then(|| CARTA_VERSION.to_owned()),
                by: FoundBy::BuiltIn,
            },
            Ok(Location::Found { by, .. }) if spec.version_args.is_empty() => {
                ToolState::Found { version: None, by }
            }
            Ok(Location::Found { path, by }) => {
                match version_line_within(&path, spec.version_args, self.version_timeout)
                    .and_then(|l| version_number(&l))
                {
                    Some(version) => ToolState::Found {
                        version: Some(version),
                        by,
                    },
                    None => ToolState::VersionNotRead { by },
                }
            }
            Ok(Location::TurnedOff) => ToolState::TurnedOff,
            Ok(Location::PathMissing) => ToolState::PathMissing,
            Ok(Location::NotFound) | Err(_) => ToolState::NotFound {
                install: self.next_step(spec.id),
            },
        };
        ToolReport {
            id: spec.id.to_owned(),
            name: spec.name.to_owned(),
            state,
        }
    }

    /// The tool's state in words, its name first: "pandoc, found at
    /// /usr/bin/pandoc", or "FFmpeg, not found. Install FFmpeg from ...".
    pub fn describe(&self, id: &str) -> String {
        let Some(spec) = self.spec(id) else {
            return format!("{id}, unknown tool");
        };
        match self.locate(id) {
            Ok(Location::Found { path, .. }) => {
                format!("{}, found at {}", spec.name, path.display())
            }
            Ok(Location::BuiltIn) => format!("{}, built in", spec.name),
            Ok(Location::TurnedOff) => format!("{}, turned off", spec.name),
            Ok(Location::NotFound | Location::PathMissing) | Err(_) => {
                format!("{}, not found. {}", spec.name, spec.install_hint)
            }
        }
    }

    /// Runs a tool with an argument list through the one runner
    /// ([`run_limited`]): no shell, no console window, diroxide's
    /// environment, standard input closed, output captured, under the
    /// plugin host's default limits ([`RunLimits::DEFAULT`], a minute and
    /// 16 MiB), and stopped with its children when the context is
    /// cancelled.
    pub fn run(
        &self,
        id: &str,
        args: &[OsString],
        ctx: &OpContext,
    ) -> Result<ToolOutput, ToolError> {
        let spec = self
            .spec(id)
            .ok_or_else(|| ToolError::Unknown(id.to_owned()))?;
        let path = match self.find(id)? {
            ToolStatus::Found(path) => path,
            ToolStatus::NotFound => return Err(ToolError::NotFound(spec.name.to_owned())),
            ToolStatus::Disabled => return Err(ToolError::Disabled(spec.name.to_owned())),
        };
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let limits = RunLimits::DEFAULT;
        let process = ProcessSpec::new(path)
            .args(args.iter().cloned())
            .diroxide_environment();
        let limited = Limited::new(
            Some(limits.timeout),
            Caps::shared(limits.max_output_bytes as u64),
        );
        let tool = spec.name.to_owned();
        match run_limited(process, limited, &|| ctx.cancel.is_cancelled()) {
            Ok(done) if done.stopped_at_cap => Err(ToolError::TooMuchOutput {
                tool,
                limit: human_size(limits.max_output_bytes as u64),
            }),
            Ok(done) => Ok(ToolOutput {
                code: done.status,
                stdout: done.stdout,
                stderr: done.stderr,
            }),
            Err(RunFailure::Cancelled) => Err(ToolError::Cancelled),
            Err(RunFailure::TimedOut(limit)) => Err(ToolError::TimedOut {
                tool,
                seconds: limit.as_secs(),
            }),
            Err(RunFailure::Start(error)) => Err(ToolError::Start {
                tool,
                message: system_words(&error),
            }),
        }
    }
}

/// The system's own words in a process error, without the program's name
/// the error puts first.
pub(crate) fn system_words(error: &ProcessError) -> String {
    match error {
        ProcessError::Start { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// Whether a component's program is one of a tool's programs, by its file
/// name: on Windows without a startable extension and ignoring case.
fn is_program_of(path: &Path, spec: &ToolSpec) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    spec.programs.iter().any(|program| {
        file_names(program)
            .iter()
            .any(|candidate| same_name(candidate, name))
    })
}

fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_config::ToolOverride;

    fn here() -> OsString {
        OsString::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn which_looks_only_on_the_given_path() {
        let found = which("Cargo.toml", Some(&here())).unwrap();
        assert!(found.ends_with("Cargo.toml"));
        assert!(which("no-such-program", Some(&here())).is_none());
        assert!(which("Cargo.toml", None).is_none());
        assert!(which("Cargo.toml", Some(OsStr::new("relative-folder"))).is_none());
    }

    #[test]
    fn a_missing_tool_says_how_to_get_it() {
        let tools = ToolRegistry::builtin().with_path(None);
        assert_eq!(tools.find("pandoc"), Ok(ToolStatus::NotFound));
        assert_eq!(
            tools.describe("pandoc"),
            "pandoc, not found. Install pandoc from pandoc.org or your package manager."
        );
        let err = tools.run("pandoc", &[], &OpContext::new()).unwrap_err();
        assert_eq!(err.to_string(), "pandoc is not installed");
        assert_eq!(
            tools.find("nothing"),
            Err(ToolError::Unknown("nothing".into()))
        );
    }

    /// "Open in textweaver" (the quick view, W2e) finds `tw` through the
    /// one PATH rule, or says how to get it (R20 commit 8).
    #[test]
    fn textweaver_is_found_on_a_fake_path_or_said_how_to_install() {
        let spec = ToolRegistry::builtin().spec("tw").copied().unwrap();
        assert_eq!(spec.name, "textweaver");
        assert_eq!(spec.programs, ["tw"]);
        assert_eq!(spec.version_args, ["--version"]);
        let folder = tempfile::tempdir().unwrap();
        let program = if cfg!(windows) { "tw.exe" } else { "tw" };
        std::fs::write(folder.path().join(program), b"").unwrap();
        let tools = ToolRegistry::builtin().with_path(Some(folder.path().as_os_str().to_owned()));
        match tools.find("tw") {
            Ok(ToolStatus::Found(path)) => assert!(path.starts_with(folder.path()), "{path:?}"),
            other => panic!("tw on the fake path: {other:?}"),
        }
        assert!(
            tools.describe("tw").starts_with("textweaver, found at "),
            "{}",
            tools.describe("tw")
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let missing =
            ToolRegistry::builtin().with_path(Some(elsewhere.path().as_os_str().to_owned()));
        assert_eq!(missing.find("tw"), Ok(ToolStatus::NotFound));
        assert_eq!(
            missing.describe("tw"),
            "textweaver, not found. Install textweaver from its releases page or with cargo."
        );
        assert_eq!(
            missing
                .run("tw", &[], &OpContext::new())
                .unwrap_err()
                .to_string(),
            "textweaver is not installed"
        );
    }

    #[test]
    fn overrides_turn_tools_off() {
        let mut overrides = ToolsConfig::default();
        overrides.tools.insert(
            "ffmpeg".to_owned(),
            ToolOverride {
                path: None,
                disabled: true,
                needed: false,
            },
        );
        let tools = ToolRegistry::builtin().with_overrides(overrides);
        assert_eq!(tools.find("ffmpeg"), Ok(ToolStatus::Disabled));
        assert_eq!(tools.describe("ffmpeg"), "FFmpeg, turned off");
    }

    /// The report on an empty PATH (alpha 4's contract step, then the
    /// table's lane): every tool of the table, its line name first, the
    /// count first; only the built-in carta is found, and each other
    /// tool says how to get it.
    #[test]
    fn the_report_lists_every_tool_name_first() {
        let mut overrides = ToolsConfig::default();
        overrides.tools.insert(
            "ffmpeg".to_owned(),
            ToolOverride {
                path: None,
                disabled: true,
                needed: false,
            },
        );
        let report = ToolRegistry::builtin()
            .with_path(None)
            .with_platform(Platform::Other, None)
            .with_overrides(overrides)
            .report();
        assert_eq!(report.tools.len(), BUILTIN.len());
        let lines = report.lines();
        assert_eq!(lines[0], format!("Tools: 1 of {} found", BUILTIN.len()));
        assert_eq!(lines[1], "ImageMagick: not found");
        assert_eq!(
            lines[2],
            "Install ImageMagick from imagemagick.org or your package manager."
        );
        assert!(
            lines
                .iter()
                .any(|l| l == "FFmpeg: turned off in tools.toml")
        );
        assert!(lines.iter().any(|l| l == "carta: 0.0.13, built in"));
        let built_in: Vec<&str> = BUILTIN
            .iter()
            .filter(|spec| spec.kind == ToolKind::BuiltIn)
            .map(|spec| spec.id)
            .collect();
        assert_eq!(built_in, ["carta"]);
    }

    /// A tool not found says the command that installs it on this
    /// system (commit 19), "Install:" first; a tool with no package there
    /// says its sentence.
    #[test]
    fn a_missing_tool_names_its_package_command() {
        let lines_on = |platform, os_release: Option<&str>| {
            ToolRegistry::builtin()
                .with_path(None)
                .with_platform(platform, os_release.map(str::to_owned))
                .report()
                .lines()
        };
        let after = |lines: &[String], line: &str| {
            let at = lines.iter().position(|l| l == line).unwrap();
            lines[at + 1].clone()
        };
        let windows = lines_on(Platform::Windows, None);
        assert_eq!(
            after(&windows, "pandoc: not found"),
            "Install: winget install JohnMacFarlane.Pandoc"
        );
        assert_eq!(
            after(&windows, "xorriso: not found"),
            "Install xorriso from your package manager."
        );
        let debian = lines_on(Platform::Linux, Some("ID=debian\n"));
        assert_eq!(
            after(&debian, "ExifTool: not found"),
            "Install: sudo apt install libimage-exiftool-perl"
        );
        let mac = lines_on(Platform::Macos, None);
        assert_eq!(
            after(&mac, "7-Zip: not found"),
            "Install: brew install sevenzip"
        );
        for line in [&windows, &debian, &mac].into_iter().flatten() {
            if let Some(command) = line.strip_prefix("Install: ") {
                assert!(!command.is_empty());
                assert!(diroxide_core::format::within_cells(line, "Install:"));
            }
        }
    }

    /// carta's version in the table is the root manifest's pin.
    #[test]
    fn cartas_version_is_the_workspace_pin() {
        let manifest = include_str!("../../../Cargo.toml");
        let pin = manifest
            .lines()
            .find(|line| line.starts_with("carta = "))
            .expect("the root manifest pins carta");
        assert!(
            pin.contains(&format!("version = \"={CARTA_VERSION}\"")),
            "{pin}"
        );
        let tools = ToolRegistry::builtin().with_path(None);
        assert!(tools.has("carta"));
        assert_eq!(
            tools.find("carta"),
            Err(ToolError::BuiltIn("carta".to_owned()))
        );
        assert_eq!(tools.describe("carta"), "carta, built in");
    }
}
