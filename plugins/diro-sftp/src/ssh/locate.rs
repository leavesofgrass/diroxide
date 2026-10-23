//! Finding ssh (R24 section 1.1; answer 85).
//!
//! The plugin never starts a bare name: Windows' process start looks in
//! the current folder first, where a planted `ssh.exe` would run. It
//! builds a list of absolute paths instead, in this order: the
//! `ssh_program` setting (the owner's choice, used alone when set); on
//! Windows `System32\OpenSSH` and `Program Files\OpenSSH`; then every
//! absolute PATH entry (an empty, `.` or relative entry is skipped); on
//! Unix `/usr/bin` last. Each is asked `ssh -V`, and the highest version
//! wins, the Windows build on a tie. Say Status names the choice.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use diroxide_tools::{ProcessSpec, Stream};

use super::version::SshVersion;
use crate::lines;

/// How long `ssh -V` may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Where a candidate came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The `ssh_program` setting.
    Setting,
    /// Windows' own copy, in `System32\OpenSSH`.
    System32,
    /// Microsoft's installer, in `Program Files\OpenSSH`.
    ProgramFiles,
    /// A folder on the PATH.
    Path,
    /// The system's own folder on Unix, `/usr/bin`.
    System,
}

impl Origin {
    /// Where, in words: "in System32", "from PATH".
    pub const fn words(self) -> &'static str {
        match self {
            Origin::Setting => "from settings",
            Origin::System32 => "in System32",
            Origin::ProgramFiles => "in Program Files",
            Origin::Path => "from PATH",
            Origin::System => "in /usr/bin",
        }
    }
}

/// The places to look, read from the environment or given by a test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPlaces {
    /// The PATH.
    pub path: Option<OsString>,
    /// Windows' folder (`%SystemRoot%`).
    pub system_root: Option<PathBuf>,
    /// `%ProgramFiles%`.
    pub program_files: Option<PathBuf>,
    /// Whether to look as on Windows.
    pub windows: bool,
}

impl SearchPlaces {
    /// The places of this computer.
    pub fn from_env() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            system_root: std::env::var_os("SystemRoot").map(PathBuf::from),
            program_files: std::env::var_os("ProgramFiles").map(PathBuf::from),
            windows: cfg!(windows),
        }
    }

    fn program(&self) -> &'static str {
        if self.windows { "ssh.exe" } else { "ssh" }
    }
}

/// Every absolute place ssh may be, in the order of R24 section 1.1, each
/// once; never a relative path, never the current folder.
pub fn candidates(places: &SearchPlaces) -> Vec<(PathBuf, Origin)> {
    let mut out: Vec<(PathBuf, Origin)> = Vec::new();
    let mut add = |path: PathBuf, origin: Origin| {
        if path.is_absolute() && !out.iter().any(|(p, _)| same_path(p, &path, places.windows)) {
            out.push((path, origin));
        }
    };
    let program = places.program();
    if places.windows {
        if let Some(root) = &places.system_root {
            add(
                root.join("System32").join("OpenSSH").join(program),
                Origin::System32,
            );
        }
        if let Some(files) = &places.program_files {
            add(files.join("OpenSSH").join(program), Origin::ProgramFiles);
        }
    }
    if let Some(path) = &places.path {
        for folder in std::env::split_paths(path) {
            // An empty entry means the current folder to some shells; a
            // relative one depends on it. Neither is ever used.
            if folder.as_os_str().is_empty() || !folder.is_absolute() {
                continue;
            }
            add(folder.join(program), Origin::Path);
        }
    }
    if !places.windows {
        add(PathBuf::from("/usr/bin").join(program), Origin::System);
    }
    out
}

fn same_path(a: &Path, b: &Path, windows: bool) -> bool {
    if windows {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// The ssh the plugin uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its absolute path.
    pub path: PathBuf,
    /// What `ssh -V` said.
    pub version: SshVersion,
    /// Where it came from.
    pub origin: Origin,
}

impl Found {
    /// Say Status's line: "ssh: OpenSSH for Windows 9.5 in System32".
    pub fn status(&self) -> String {
        lines::which_ssh(&self.version.words(), self.origin.words())
    }
}

/// Why no ssh could be used, said in words.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LocateError {
    /// No candidate answered `ssh -V` as OpenSSH.
    #[error("OpenSSH was not found.")]
    NotFound {
        /// Whether to give Windows' way to add it.
        windows: bool,
    },
    /// The `ssh_program` setting names something that cannot be used.
    #[error("The ssh program in settings cannot be used.")]
    Setting {
        /// Why, in words.
        why: String,
    },
}

impl LocateError {
    /// The lines for the owner, one fact a line, the cause first.
    pub fn lines(&self) -> Vec<String> {
        match self {
            LocateError::NotFound { windows } => lines::ssh_not_found(*windows),
            LocateError::Setting { why } => vec![self.to_string(), why.clone()],
        }
    }
}

/// Asks one program `-V` with no console window, its standard error read
/// with a time limit; `None` when it does not answer as OpenSSH.
pub fn probe(path: &Path) -> Option<SshVersion> {
    let mut child = ProcessSpec::new(path)
        .arg("-V")
        .stdin(Stream::Null)
        .stdout(Stream::Null)
        .stderr(Stream::Piped)
        .spawn()
        .ok()?;
    let mut stderr = child.take_stderr()?;
    let reader = thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.by_ref().take(4096).read_to_end(&mut text);
        text
    });
    let status = child.wait_timeout(PROBE_TIMEOUT).ok().flatten();
    if status.is_none() {
        let _ = child.kill_tree();
    }
    let text = reader.join().ok()?;
    SshVersion::parse(&String::from_utf8_lossy(&text))
}

/// Chooses the ssh to use: the setting alone when it is set, otherwise
/// the highest version among the candidates, the Windows build on a tie
/// (answer 85).
pub fn locate(
    setting: Option<&Path>,
    places: &SearchPlaces,
    probe: impl Fn(&Path) -> Option<SshVersion>,
) -> Result<Found, LocateError> {
    if let Some(path) = setting {
        if !path.is_absolute() {
            return Err(LocateError::Setting {
                why: lines::setting_not_absolute(),
            });
        }
        return probe(path)
            .map(|version| Found {
                path: path.to_path_buf(),
                version,
                origin: Origin::Setting,
            })
            .ok_or_else(|| LocateError::Setting {
                why: lines::setting_not_openssh(),
            });
    }
    let mut best: Option<Found> = None;
    for (path, origin) in candidates(places) {
        if !path.is_file() {
            continue;
        }
        let Some(version) = probe(&path) else {
            continue;
        };
        let better = best
            .as_ref()
            .is_none_or(|b| version.preference(&b.version).is_gt());
        if better {
            best = Some(Found {
                path,
                version,
                origin,
            });
        }
    }
    best.ok_or(LocateError::NotFound {
        windows: places.windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_places(path: &str) -> SearchPlaces {
        SearchPlaces {
            path: Some(OsString::from(path)),
            system_root: Some(PathBuf::from(r"C:\Windows")),
            program_files: Some(PathBuf::from(r"C:\Program Files")),
            windows: true,
        }
    }

    #[test]
    fn candidates_are_absolute_ordered_and_unique() {
        if !cfg!(windows) {
            return;
        }
        let places = windows_places(r".;relative\bin;;C:\Windows\System32\OpenSSH;D:\tools");
        let found: Vec<(String, Origin)> = candidates(&places)
            .into_iter()
            .map(|(p, o)| (p.display().to_string(), o))
            .collect();
        assert_eq!(
            found,
            [
                (
                    r"C:\Windows\System32\OpenSSH\ssh.exe".to_owned(),
                    Origin::System32
                ),
                (
                    r"C:\Program Files\OpenSSH\ssh.exe".to_owned(),
                    Origin::ProgramFiles
                ),
                (r"D:\tools\ssh.exe".to_owned(), Origin::Path),
            ]
        );
    }

    #[test]
    fn unix_candidates_end_with_usr_bin() {
        if cfg!(windows) {
            return;
        }
        let places = SearchPlaces {
            path: Some(OsString::from(".:bin:/opt/ssh/bin:/usr/bin")),
            windows: false,
            ..SearchPlaces::default()
        };
        let found: Vec<PathBuf> = candidates(&places).into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            found,
            [
                PathBuf::from("/opt/ssh/bin/ssh"),
                PathBuf::from("/usr/bin/ssh")
            ]
        );
    }

    #[test]
    fn a_relative_setting_is_refused_in_words() {
        let error = locate(Some(Path::new("ssh.exe")), &SearchPlaces::default(), |_| {
            None
        })
        .unwrap_err();
        assert_eq!(
            error.lines(),
            [
                "The ssh program in settings cannot be used.",
                "Its path must be a full path."
            ]
        );
        let not_found = LocateError::NotFound { windows: true };
        assert_eq!(
            not_found.lines(),
            [
                "OpenSSH was not found.",
                "Add it in Settings, Optional features."
            ]
        );
    }
}
