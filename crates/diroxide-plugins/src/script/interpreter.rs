//! Which program runs a script (R3 section 5.5; R22 sections 5.4 and
//! 10, spike S6).
//!
//! - **By extension:** `.sh` and `.bash` with bash, `.py` with Python 3,
//!   `.ps1` with PowerShell, `.cmd` and `.bat` on Windows as themselves
//!   (the standard library hands them to `cmd.exe` with its own quoting).
//!   On Linux and macOS another file runs directly when it is executable,
//!   so the kernel reads its shebang. The header's `interpreter` key
//!   replaces the choice.
//! - **Python** gets `-X utf8 -u`, and `-P` from 3.11 on, so a script
//!   named `json.py` in the shared scripts folder cannot shadow the
//!   standard library for its neighbors. On Windows the `py -3` launcher
//!   comes first; anything under a `WindowsApps` folder is refused without
//!   being run (an app-execution alias, often the Store stub that opens
//!   the Store), and so is a program that answers `--version` with the
//!   stub's exit code 9009.
//! - **bash on Windows** is Git for Windows' bash, found beside `git`
//!   or under Program Files. The `bash.exe` in the system folder is WSL's
//!   launcher, which runs Linux with `/mnt/c` paths and would break every
//!   path the host passes; it is refused with words. Paths reach Git's
//!   bash in the forward-slash form (`D:/photos/a.jpg`), which both
//!   Windows and the MSYS runtime read.
//! - **PowerShell** is `pwsh` (7) first, then Windows PowerShell 5.1,
//!   with `-NoProfile -NonInteractive`, and on Windows a per-process
//!   execution-policy override (`-ExecutionPolicy Bypass`) for a script
//!   the owner enabled, which the consent text says (answer 92). Nothing
//!   about the system's policy changes.
//!
//! Spike S6 (Friday, October 23, 2026, on the owner's Windows 11): `bash`
//! on PATH was WSL's "Microsoft Bash Launcher" in System32, ahead of Git's
//! bash; `python3` existed only as a zero-byte alias in WindowsApps;
//! `python` found a real Python 3.13 first; `py -3` worked; `pwsh` was not
//! installed; Windows PowerShell 5.1 ran a local `.ps1` both under the
//! machine's policy and with the override; Git's bash read a NUL
//! selection file through a redirect with both `D:/` and `D:\` paths, and
//! its `du` took both forms.

use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use diroxide_tools::process::{ProcessSpec, Stream};

use crate::manifest::Os;

/// The exit code of Windows' "command not found", which the Store stub
/// answers with.
pub const STUB_EXIT_CODE: i32 = 9009;

/// The oldest Python that gets `-P`.
pub const SAFE_PATH_PYTHON: (u32, u32) = (3, 11);

/// What kind of script a file is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScriptKind {
    /// `.sh` or `.bash`.
    Bash,
    /// `.py`.
    Python,
    /// `.ps1`.
    PowerShell,
    /// `.cmd` or `.bat`, Windows only.
    Batch,
    /// Anything else: run directly when executable (Linux and macOS).
    Direct,
}

impl ScriptKind {
    /// The kind of the script at `path`, by its extension.
    pub fn of(path: &Path) -> Self {
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "sh" | "bash" => ScriptKind::Bash,
            "py" => ScriptKind::Python,
            "ps1" => ScriptKind::PowerShell,
            "cmd" | "bat" => ScriptKind::Batch,
            _ => ScriptKind::Direct,
        }
    }
}

/// Why no program could run a script, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// No Python 3 was found.
    NoPython,
    /// Only the Store stub was found.
    StoreStubOnly,
    /// No bash was found.
    NoBash,
    /// Only WSL's launcher was found.
    WslOnly,
    /// No PowerShell was found.
    NoPowerShell,
    /// The script is for another system (a batch file off Windows).
    OtherSystem,
    /// Nothing says how to run the file.
    NoInterpreter,
    /// The header's interpreter was not found.
    NotFound(String),
}

impl Missing {
    /// The sentence, meaning first.
    pub fn sentence(&self) -> String {
        match self {
            Missing::NoPython => {
                "Python 3 was not found. Install Python 3, or name one in the interpreter key."
                    .to_owned()
            }
            Missing::StoreStubOnly => {
                "Python is not installed; the Store stub was found. Install Python, or use a bash script command."
                    .to_owned()
            }
            Missing::NoBash => {
                if cfg!(windows) {
                    "bash was not found. Install Git for Windows, which brings it.".to_owned()
                } else {
                    "bash was not found. Install bash.".to_owned()
                }
            }
            Missing::WslOnly => {
                "Only WSL's bash was found; it breaks paths. Install Git for Windows.".to_owned()
            }
            Missing::NoPowerShell => {
                "PowerShell was not found. Install PowerShell 7.".to_owned()
            }
            Missing::OtherSystem => "A .cmd or .bat script runs on Windows only.".to_owned(),
            Missing::NoInterpreter => {
                "No program runs this file. Name it .sh, .py or .ps1, or add an interpreter key."
                    .to_owned()
            }
            Missing::NotFound(program) => {
                format!("{program} was not found. Install it, or fix the interpreter key.")
            }
        }
    }
}

/// The program that runs a script, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interpreter {
    /// The program, a full path; for a batch file or a direct run, the
    /// script itself.
    pub program: PathBuf,
    /// Its arguments before the script's path.
    pub args: Vec<OsString>,
    /// What it runs.
    pub kind: ScriptKind,
    /// Whether places reach it in the forward-slash form (Git's bash on
    /// Windows).
    pub forward_slashes: bool,
    /// Whether PowerShell's execution policy is overridden for this
    /// process, which the consent text says.
    pub policy_override: bool,
    /// The program as the consent text names it: "bash", "py -3",
    /// "python3", "powershell".
    pub words: String,
    /// Whether the script's own path is added after the arguments (false
    /// when the program is the script).
    pub takes_script: bool,
}

impl Interpreter {
    /// The program and its whole argument list for a script.
    pub fn command_for(&self, script: &Path) -> (PathBuf, Vec<OsString>) {
        let mut args = self.args.clone();
        if self.takes_script {
            args.push(self.path_text(script));
        }
        (self.program.clone(), args)
    }

    /// A path as this program should see it: forward slashes for Git's
    /// bash on Windows, else as it is.
    pub fn path_text(&self, path: &Path) -> OsString {
        if self.forward_slashes {
            OsString::from(forward_slashes(&path.to_string_lossy()))
        } else {
            path.as_os_str().to_owned()
        }
    }

    /// The consent text's "It runs:" line for a script file:
    /// "It runs: bash total-size.sh".
    pub fn runs_line(&self, script: &Path) -> String {
        let file = script
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.takes_script {
            format!("It runs: {} {file}", self.words)
        } else {
            format!("It runs: {file}")
        }
    }
}

/// Text with every backslash made a forward slash: `D:\a b\c` becomes
/// `D:/a b/c`, which Windows and the MSYS runtime both read.
pub fn forward_slashes(text: &str) -> String {
    text.replace('\\', "/")
}

/// What a probe of a program found: its exit code and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    /// The exit code; `None` when it did not end or could not start.
    pub code: Option<i32>,
    /// Standard output and standard error together.
    pub text: String,
}

/// Runs `PROGRAM ARGS --version` and reports what it said. Tests answer
/// from a table; the system probe starts the program with no console
/// window and stops it after five seconds.
pub trait Probe: Send + Sync {
    /// Asks a program for its version.
    fn version(&self, program: &Path, args: &[&str]) -> Probed;
}

/// The real probe, through the process helper.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn version(&self, program: &Path, args: &[&str]) -> Probed {
        let spec = ProcessSpec::new(program)
            .args(args.iter().copied())
            .arg("--version")
            .diroxide_environment()
            .stdin(Stream::Null)
            .stdout(Stream::Piped)
            .stderr(Stream::Piped);
        let Ok(mut child) = spec.spawn() else {
            return Probed {
                code: None,
                text: String::new(),
            };
        };
        let mut text = String::new();
        let out = child.take_stdout();
        let err = child.take_stderr();
        let reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut out) = out {
                let _ = out.by_ref().take(64 * 1024).read_to_end(&mut buffer);
            }
            if let Some(mut err) = err {
                let _ = err.by_ref().take(64 * 1024).read_to_end(&mut buffer);
            }
            buffer
        });
        let code = match child.wait_timeout(Duration::from_secs(5)) {
            Ok(Some(status)) => status.code(),
            _ => {
                let _ = child.kill_tree();
                None
            }
        };
        if let Ok(bytes) = reader.join() {
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        Probed { code, text }
    }
}

/// The versions already asked for, by program and arguments.
type VersionCache = HashMap<(PathBuf, Vec<String>), Probed>;

/// Where the chooser looks: the system, PATH and the folders it trusts.
/// Tests build one with fake folders and a fake probe.
#[derive(Clone)]
pub struct Lookup {
    /// The system whose rules apply.
    pub os: Os,
    /// The PATH to search.
    pub path: Option<OsString>,
    /// Program Files folders, where Git for Windows installs.
    pub program_files: Vec<PathBuf>,
    /// Runs `--version`.
    pub probe: Arc<dyn Probe>,
    cache: Arc<Mutex<VersionCache>>,
}

impl std::fmt::Debug for Lookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lookup")
            .field("os", &self.os)
            .field("path", &self.path)
            .field("program_files", &self.program_files)
            .finish_non_exhaustive()
    }
}

impl Lookup {
    /// This system's PATH and Program Files, with the real probe.
    pub fn system() -> Self {
        let program_files = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();
        Self::new(
            Os::current(),
            env::var_os("PATH"),
            program_files,
            Arc::new(SystemProbe),
        )
    }

    /// A lookup over these folders.
    pub fn new(
        os: Os,
        path: Option<OsString>,
        program_files: Vec<PathBuf>,
        probe: Arc<dyn Probe>,
    ) -> Self {
        Self {
            os,
            path,
            program_files,
            probe,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Asks a program for its version once; later asks read the answer.
    fn version(&self, program: &Path, args: &[&str]) -> Probed {
        let key = (
            program.to_path_buf(),
            args.iter().map(|a| (*a).to_owned()).collect(),
        );
        if let Some(found) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return found.clone();
        }
        let probed = self.probe.version(program, args);
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, probed.clone());
        probed
    }

    /// Every file named `program` on PATH, in order, by the one PATH rule
    /// (absolute folders only, never the current folder; on Windows the
    /// usual program extensions are tried).
    pub fn which_all(&self, program: &str) -> Vec<PathBuf> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        let names: Vec<String> =
            if self.os == Os::Windows && Path::new(program).extension().is_none() {
                [".exe", ".com", ".cmd", ".bat"]
                    .iter()
                    .map(|ext| format!("{program}{ext}"))
                    .chain(std::iter::once(program.to_owned()))
                    .collect()
            } else {
                vec![program.to_owned()]
            };
        let mut found = Vec::new();
        for folder in env::split_paths(path) {
            if !folder.is_absolute() {
                continue;
            }
            for name in &names {
                let candidate = folder.join(name);
                if candidate.is_file() && !found.contains(&candidate) {
                    found.push(candidate);
                    break;
                }
            }
        }
        found
    }

    /// Chooses the program for a script, by the header's interpreter when
    /// it names one, else by the file's extension.
    pub fn choose(&self, script: &Path, interpreter: &[String]) -> Result<Interpreter, Missing> {
        if let Some((program, args)) = interpreter.split_first() {
            return self.by_header(program, args);
        }
        match ScriptKind::of(script) {
            ScriptKind::Python => self.python(&[]),
            ScriptKind::Bash => self.bash(),
            ScriptKind::PowerShell => self.powershell(),
            ScriptKind::Batch if self.os == Os::Windows => Ok(Interpreter {
                program: script.to_path_buf(),
                args: Vec::new(),
                kind: ScriptKind::Batch,
                forward_slashes: false,
                policy_override: false,
                words: "cmd".to_owned(),
                takes_script: false,
            }),
            ScriptKind::Batch => Err(Missing::OtherSystem),
            ScriptKind::Direct if self.os != Os::Windows && is_executable(script) => {
                Ok(Interpreter {
                    program: script.to_path_buf(),
                    args: Vec::new(),
                    kind: ScriptKind::Direct,
                    forward_slashes: false,
                    policy_override: false,
                    words: String::new(),
                    takes_script: false,
                })
            }
            ScriptKind::Direct => Err(Missing::NoInterpreter),
        }
    }

    /// The header's `interpreter`: the first word found by the PATH rule
    /// (refusing stubs and WSL's bash as the defaults do), then its
    /// arguments; Python still gets its flags.
    fn by_header(&self, program: &str, args: &[String]) -> Result<Interpreter, Missing> {
        let stem = Path::new(program)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if stem == "py" || stem.starts_with("python") {
            let extra: Vec<&str> = args.iter().map(String::as_str).collect();
            if stem == "py" {
                return self.python_launcher(&extra).ok_or(Missing::NoPython);
            }
            return self.python_named(program, &extra);
        }
        if stem == "bash" && self.os == Os::Windows {
            let mut chosen = self.bash()?;
            chosen.args.extend(args.iter().map(OsString::from));
            return Ok(chosen);
        }
        let as_path = Path::new(program);
        let found = if as_path.is_absolute() {
            as_path.is_file().then(|| as_path.to_path_buf())
        } else {
            self.which_all(program)
                .into_iter()
                .find(|p| !is_app_alias(p) && !is_wsl_launcher(p))
        };
        let program_path = found.ok_or_else(|| Missing::NotFound(program.to_owned()))?;
        Ok(Interpreter {
            program: program_path,
            args: args.iter().map(OsString::from).collect(),
            kind: ScriptKind::Direct,
            forward_slashes: false,
            policy_override: false,
            words: std::iter::once(stem)
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" "),
            takes_script: true,
        })
    }

    /// Python 3: on Windows the `py -3` launcher first; then `python3`
    /// and `python` on PATH, never an app alias, never the stub.
    pub fn python(&self, extra: &[&str]) -> Result<Interpreter, Missing> {
        if self.os == Os::Windows
            && let Some(found) = self.python_launcher(extra)
        {
            return Ok(found);
        }
        let mut stub_seen = false;
        for name in ["python3", "python"] {
            match self.python_named(name, extra) {
                Ok(found) => return Ok(found),
                Err(Missing::StoreStubOnly) => stub_seen = true,
                Err(_) => {}
            }
        }
        Err(if stub_seen {
            Missing::StoreStubOnly
        } else {
            Missing::NoPython
        })
    }

    fn python_launcher(&self, extra: &[&str]) -> Option<Interpreter> {
        let launcher = self
            .which_all("py")
            .into_iter()
            .find(|p| !is_app_alias(p))?;
        let probed = self.version(&launcher, &["-3"]);
        let version = python_version(&probed)?;
        let mut args: Vec<OsString> = vec!["-3".into()];
        args.extend(extra.iter().map(OsString::from));
        args.extend(python_flags(version));
        Some(Interpreter {
            program: launcher,
            args,
            kind: ScriptKind::Python,
            forward_slashes: false,
            policy_override: false,
            words: "py -3".to_owned(),
            takes_script: true,
        })
    }

    fn python_named(&self, name: &str, extra: &[&str]) -> Result<Interpreter, Missing> {
        let mut stub_seen = false;
        for candidate in self.which_all(name) {
            if is_app_alias(&candidate) {
                stub_seen = true;
                continue;
            }
            let probed = self.version(&candidate, &[]);
            if probed.code == Some(STUB_EXIT_CODE) {
                stub_seen = true;
                continue;
            }
            let Some(version) = python_version(&probed) else {
                continue;
            };
            let mut args: Vec<OsString> = extra.iter().map(OsString::from).collect();
            args.extend(python_flags(version));
            return Ok(Interpreter {
                program: candidate,
                args,
                kind: ScriptKind::Python,
                forward_slashes: false,
                policy_override: false,
                words: name.to_owned(),
                takes_script: true,
            });
        }
        Err(if stub_seen {
            Missing::StoreStubOnly
        } else {
            Missing::NoPython
        })
    }

    /// bash: on Windows Git for Windows' bash, never WSL's launcher;
    /// elsewhere the first on PATH.
    pub fn bash(&self) -> Result<Interpreter, Missing> {
        let windows = self.os == Os::Windows;
        let chosen = |program: PathBuf| Interpreter {
            program,
            args: vec!["--noprofile".into(), "--norc".into()],
            kind: ScriptKind::Bash,
            forward_slashes: windows,
            policy_override: false,
            words: "bash".to_owned(),
            takes_script: true,
        };
        if !windows {
            return self
                .which_all("bash")
                .into_iter()
                .next()
                .map(chosen)
                .ok_or(Missing::NoBash);
        }
        for root in self.git_roots() {
            for inner in ["bin", "usr/bin"] {
                let candidate = root.join(inner).join("bash.exe");
                if candidate.is_file() {
                    return Ok(chosen(candidate));
                }
            }
        }
        let mut wsl_seen = false;
        for candidate in self.which_all("bash") {
            if is_wsl_launcher(&candidate) || is_app_alias(&candidate) {
                wsl_seen = true;
                continue;
            }
            return Ok(chosen(candidate));
        }
        Err(if wsl_seen {
            Missing::WslOnly
        } else {
            Missing::NoBash
        })
    }

    /// Git for Windows' install folders: beside each `git` on PATH
    /// (`cmd\git.exe` or `mingw64\bin\git.exe`), then under Program Files.
    fn git_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for git in self.which_all("git") {
            let Some(folder) = git.parent() else {
                continue;
            };
            let name = folder
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let root = match name.as_str() {
                "cmd" => folder.parent(),
                "bin" => folder
                    .parent()
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| n.eq_ignore_ascii_case("mingw64"))
                    })
                    .and_then(Path::parent),
                _ => None,
            };
            if let Some(root) = root
                && !roots.iter().any(|r: &PathBuf| r == root)
            {
                roots.push(root.to_path_buf());
            }
        }
        for folder in &self.program_files {
            let root = folder.join("Git");
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
    }

    /// PowerShell: `pwsh` first, then Windows PowerShell; on Windows with
    /// the per-process policy override.
    pub fn powershell(&self) -> Result<Interpreter, Missing> {
        let windows = self.os == Os::Windows;
        for name in ["pwsh", "powershell"] {
            let Some(program) = self.which_all(name).into_iter().find(|p| !is_app_alias(p)) else {
                continue;
            };
            let mut args: Vec<OsString> = vec!["-NoProfile".into(), "-NonInteractive".into()];
            if windows {
                args.push("-ExecutionPolicy".into());
                args.push("Bypass".into());
            }
            args.push("-File".into());
            return Ok(Interpreter {
                program,
                args,
                kind: ScriptKind::PowerShell,
                forward_slashes: false,
                policy_override: windows,
                words: name.to_owned(),
                takes_script: true,
            });
        }
        Err(Missing::NoPowerShell)
    }
}

/// Python's flags for a version: `-X utf8 -u`, and `-P` from 3.11.
fn python_flags(version: (u32, u32)) -> Vec<OsString> {
    let mut flags: Vec<OsString> = vec!["-X".into(), "utf8".into(), "-u".into()];
    if version >= SAFE_PATH_PYTHON {
        flags.push("-P".into());
    }
    flags
}

/// The major and minor version in a probe's words, "Python 3.13.15";
/// `None` for anything but Python 3 that ended well.
pub fn python_version(probed: &Probed) -> Option<(u32, u32)> {
    if probed.code != Some(0) {
        return None;
    }
    let rest = probed.text.trim().strip_prefix("Python ")?;
    let mut parts = rest.split(['.', ' ', '\r', '\n']);
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    (major == 3).then_some((major, minor))
}

/// Whether a path lies under a `WindowsApps` folder: an app-execution
/// alias, which is never run to find out what it is.
pub fn is_app_alias(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .eq_ignore_ascii_case(OsStr::new("WindowsApps"))
    })
}

/// Whether a path is a `bash` in the Windows system folder: WSL's
/// launcher.
pub fn is_wsl_launcher(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    stem == "bash" && (folder == "system32" || folder == "syswow64" || folder == "sysnative")
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Canned answers to `--version`, by the program's file name.
    struct FakeProbe(Vec<(&'static str, Probed)>);

    impl Probe for FakeProbe {
        fn version(&self, program: &Path, _args: &[&str]) -> Probed {
            let name = program
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parent = program
                .parent()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.0
                .iter()
                .find(|(key, _)| *key == name || *key == format!("{parent}/{name}"))
                .map(|(_, p)| p.clone())
                .unwrap_or(Probed {
                    code: None,
                    text: String::new(),
                })
        }
    }

    fn says(code: i32, text: &str) -> Probed {
        Probed {
            code: Some(code),
            text: text.to_owned(),
        }
    }

    /// A fake system: folders under a temporary folder, each holding empty
    /// files with the names given, and a PATH over the named folders.
    struct Fake {
        root: tempfile::TempDir,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                root: tempfile::tempdir().unwrap(),
            }
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.root.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"").unwrap();
            path
        }

        fn folder(&self, relative: &str) -> PathBuf {
            self.root.path().join(relative)
        }

        fn lookup(&self, os: Os, path: &[&str], probe: FakeProbe) -> Lookup {
            let folders: Vec<PathBuf> = path.iter().map(|f| self.folder(f)).collect();
            Lookup::new(
                os,
                Some(env::join_paths(folders).unwrap()),
                vec![self.folder("ProgramFiles")],
                Arc::new(probe),
            )
        }
    }

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(ScriptKind::of(Path::new("a.sh")), ScriptKind::Bash);
        assert_eq!(ScriptKind::of(Path::new("a.PY")), ScriptKind::Python);
        assert_eq!(ScriptKind::of(Path::new("a.ps1")), ScriptKind::PowerShell);
        assert_eq!(ScriptKind::of(Path::new("a.cmd")), ScriptKind::Batch);
        assert_eq!(ScriptKind::of(Path::new("a")), ScriptKind::Direct);
    }

    #[test]
    fn windows_prefers_the_py_launcher_with_the_python_flags() {
        let fake = Fake::new();
        fake.file("Launcher/py.exe");
        fake.file("WindowsApps/python3.exe");
        let lookup = fake.lookup(
            Os::Windows,
            &["WindowsApps", "Launcher"],
            FakeProbe(vec![("py.exe", says(0, "Python 3.13.15\r\n"))]),
        );
        let chosen = lookup.choose(Path::new("count.py"), &[]).unwrap();
        assert!(chosen.program.ends_with("py.exe"));
        assert_eq!(chosen.args, ["-3", "-X", "utf8", "-u", "-P"]);
        assert_eq!(chosen.words, "py -3");
        assert_eq!(
            chosen.runs_line(Path::new("D:/x/count.py")),
            "It runs: py -3 count.py"
        );
    }

    #[test]
    fn the_store_stub_is_refused_with_words() {
        let fake = Fake::new();
        fake.file("WindowsApps/python3.exe");
        fake.file("WindowsApps/python.exe");
        fake.file("Stub/python.exe");
        let lookup = fake.lookup(
            Os::Windows,
            &["WindowsApps", "Stub"],
            FakeProbe(vec![("Stub/python.exe", says(STUB_EXIT_CODE, ""))]),
        );
        let missing = lookup.choose(Path::new("count.py"), &[]).unwrap_err();
        assert_eq!(missing, Missing::StoreStubOnly);
        assert_eq!(
            missing.sentence(),
            "Python is not installed; the Store stub was found. Install Python, or use a bash script command."
        );
    }

    #[test]
    fn a_real_python_on_path_is_found_past_the_alias() {
        let fake = Fake::new();
        fake.file("WindowsApps/python3.exe");
        fake.file("Python313/python.exe");
        let lookup = fake.lookup(
            Os::Windows,
            &["WindowsApps", "Python313"],
            FakeProbe(vec![("python.exe", says(0, "Python 3.10.4"))]),
        );
        let chosen = lookup.python(&[]).unwrap();
        assert!(chosen.program.ends_with("python.exe"));
        assert!(!is_app_alias(&chosen.program));
        // 3.10 gets no -P.
        assert_eq!(chosen.args, ["-X", "utf8", "-u"]);
    }

    #[test]
    fn linux_and_macos_use_python3_on_path() {
        let fake = Fake::new();
        fake.file("bin/python3");
        let lookup = fake.lookup(
            Os::Linux,
            &["bin"],
            FakeProbe(vec![("python3", says(0, "Python 3.12.1"))]),
        );
        let chosen = lookup.choose(Path::new("x.py"), &[]).unwrap();
        assert_eq!(chosen.words, "python3");
        assert_eq!(chosen.args.last().unwrap(), "-P");
        let none = fake.lookup(Os::Linux, &["empty"], FakeProbe(vec![]));
        assert_eq!(none.python(&[]).unwrap_err(), Missing::NoPython);
    }

    #[test]
    fn windows_bash_is_gits_never_wsls() {
        let fake = Fake::new();
        fake.file("System32/bash.exe");
        fake.file("Git/cmd/git.exe");
        fake.file("Git/bin/bash.exe");
        let lookup = fake.lookup(Os::Windows, &["System32", "Git/cmd"], FakeProbe(vec![]));
        let chosen = lookup.choose(Path::new("total-size.sh"), &[]).unwrap();
        assert!(
            chosen.program.ends_with("bin/bash.exe") || chosen.program.ends_with("bin\\bash.exe")
        );
        assert!(!is_wsl_launcher(&chosen.program));
        assert!(chosen.forward_slashes);
        let (_, args) = chosen.command_for(Path::new(r"D:\scripts\total-size.sh"));
        assert_eq!(args.last().unwrap(), "D:/scripts/total-size.sh");
        assert_eq!(
            chosen.runs_line(Path::new("total-size.sh")),
            "It runs: bash total-size.sh"
        );
    }

    #[test]
    fn git_under_program_files_and_mingw64_is_found() {
        let fake = Fake::new();
        fake.file("ProgramFiles/Git/bin/bash.exe");
        let lookup = fake.lookup(Os::Windows, &["nothing"], FakeProbe(vec![]));
        assert!(lookup.bash().is_ok());
        let other = Fake::new();
        other.file("G/mingw64/bin/git.exe");
        other.file("G/usr/bin/bash.exe");
        let lookup = other.lookup(Os::Windows, &["G/mingw64/bin"], FakeProbe(vec![]));
        assert!(
            lookup
                .bash()
                .unwrap()
                .program
                .parent()
                .unwrap()
                .ends_with("usr/bin")
        );
    }

    #[test]
    fn wsl_alone_is_refused_with_words() {
        let fake = Fake::new();
        fake.file("System32/bash.exe");
        let lookup = fake.lookup(Os::Windows, &["System32"], FakeProbe(vec![]));
        let missing = lookup.choose(Path::new("a.sh"), &[]).unwrap_err();
        assert_eq!(missing, Missing::WslOnly);
        assert!(missing.sentence().starts_with("Only WSL's bash was found"));
        let none = fake.lookup(Os::Windows, &["nothing"], FakeProbe(vec![]));
        assert_eq!(none.bash().unwrap_err(), Missing::NoBash);
    }

    #[test]
    fn powershell_on_windows_carries_the_per_process_override() {
        let fake = Fake::new();
        fake.file("PS/powershell.exe");
        let lookup = fake.lookup(Os::Windows, &["PS"], FakeProbe(vec![]));
        let chosen = lookup.choose(Path::new("tidy.ps1"), &[]).unwrap();
        assert!(chosen.policy_override);
        assert_eq!(
            chosen.args,
            [
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File"
            ]
        );
        fake.file("PS7/pwsh.exe");
        let both = fake.lookup(Os::Windows, &["PS", "PS7"], FakeProbe(vec![]));
        assert_eq!(both.powershell().unwrap().words, "pwsh");
        let linux = fake.lookup(Os::Linux, &["nothing"], FakeProbe(vec![]));
        assert_eq!(linux.powershell().unwrap_err(), Missing::NoPowerShell);
    }

    #[test]
    fn batch_files_run_as_themselves_on_windows_only() {
        let fake = Fake::new();
        let windows = fake.lookup(Os::Windows, &[], FakeProbe(vec![]));
        let chosen = windows.choose(Path::new("backup.cmd"), &[]).unwrap();
        assert!(!chosen.takes_script);
        let linux = fake.lookup(Os::Linux, &[], FakeProbe(vec![]));
        assert_eq!(
            linux.choose(Path::new("backup.cmd"), &[]).unwrap_err(),
            Missing::OtherSystem
        );
        assert_eq!(
            linux.choose(Path::new("notes.txt"), &[]).unwrap_err(),
            Missing::NoInterpreter
        );
    }

    #[test]
    fn the_header_interpreter_replaces_the_guess() {
        let fake = Fake::new();
        fake.file("bin/lua");
        fake.file("bin/python3");
        let lookup = fake.lookup(
            Os::Linux,
            &["bin"],
            FakeProbe(vec![("python3", says(0, "Python 3.11.0"))]),
        );
        let lua = lookup.choose(Path::new("x.sh"), &["lua".into()]).unwrap();
        assert!(lua.program.ends_with("lua"));
        assert_eq!(lua.runs_line(Path::new("x.sh")), "It runs: lua x.sh");
        let python = lookup
            .choose(Path::new("x"), &["python3".into(), "-I".into()])
            .unwrap();
        assert_eq!(python.args, ["-I", "-X", "utf8", "-u", "-P"]);
        assert_eq!(
            lookup.choose(Path::new("x"), &["ruby".into()]).unwrap_err(),
            Missing::NotFound("ruby".into())
        );
    }

    #[test]
    fn versions_are_read_once() {
        struct Counting(Mutex<usize>);
        impl Probe for Counting {
            fn version(&self, _: &Path, _: &[&str]) -> Probed {
                *self.0.lock().unwrap() += 1;
                says(0, "Python 3.12.0")
            }
        }
        let fake = Fake::new();
        fake.file("bin/python3");
        let probe = Arc::new(Counting(Mutex::new(0)));
        let lookup = Lookup::new(
            Os::Linux,
            Some(env::join_paths([fake.folder("bin")]).unwrap()),
            Vec::new(),
            probe.clone(),
        );
        lookup.python(&[]).unwrap();
        lookup.python(&[]).unwrap();
        assert_eq!(*probe.0.lock().unwrap(), 1);
    }

    #[test]
    fn python_versions_read_from_words() {
        assert_eq!(python_version(&says(0, "Python 3.13.15\n")), Some((3, 13)));
        assert_eq!(python_version(&says(0, "Python 2.7.18")), None);
        assert_eq!(python_version(&says(9009, "Python 3.13.15")), None);
        assert_eq!(python_version(&says(0, "something else")), None);
    }

    #[test]
    fn forward_slashes_keep_the_drive() {
        assert_eq!(forward_slashes(r"D:\a b\c.txt"), "D:/a b/c.txt");
    }

    #[test]
    #[ignore = "starts the system's interpreters; run by hand for spike S6"]
    fn spike_s6_the_system_lookup() {
        let lookup = Lookup::system();
        for (what, chosen) in [
            ("python", lookup.python(&[])),
            ("bash", lookup.bash()),
            ("powershell", lookup.powershell()),
        ] {
            match chosen {
                Ok(i) => println!("{what}: {} {:?}", i.program.display(), i.args),
                Err(m) => println!("{what}: {}", m.sentence()),
            }
        }
    }
}
