//! Finding a program on disk: the one PATH rule, and the folders the
//! programs' own installers use (alpha.7, R28 1.3; the owner's answer
//! 124). The registry looks in `tools.toml`, then the installed
//! components, then PATH, then these folders; this module holds the last
//! two steps.
//!
//! - **PATH.** Each folder in order, never a relative one, so the current
//!   folder is never searched. On Windows a name without an extension
//!   tries the extensions of `PATHEXT` first, then the bare name last: a
//!   bare `whisper` beside `whisper.exe` is often a script Windows cannot
//!   start. Only the extensions a program can be started with count
//!   (`.com`, `.exe`, `.bat`, `.cmd`); a `.js` or `.vbs` earlier on PATH
//!   would be found and then fail to start.
//! - **Install folders.** 7-Zip's and Tesseract's installers do not put
//!   themselves on PATH, Git for Windows keeps its `gpg` off it, and a
//!   window started on macOS does not get the shell's PATH, so Homebrew's
//!   folders are looked in there. Each folder has words for the line that
//!   says where a tool was found: "in Program Files", "in Git for
//!   Windows", "in Homebrew".

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The extensions a program can be started with on Windows, in no order:
/// `PATHEXT` gives the order.
const STARTABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// The extensions tried when `PATHEXT` is not set.
const DEFAULT_PATHEXT: &str = ".EXE;.COM;.CMD;.BAT";

/// The file names to try for a program in one folder, in order: on Windows
/// a name without an extension gets each startable extension `PATHEXT`
/// names, in its order, then the bare name last; elsewhere, and for a name
/// with an extension, the name alone.
pub fn file_names(program: &str) -> Vec<String> {
    file_names_with(program, env::var("PATHEXT").ok().as_deref(), cfg!(windows))
}

/// [`file_names`] with the `PATHEXT` text and the system given, for tests.
pub fn file_names_with(program: &str, pathext: Option<&str>, windows: bool) -> Vec<String> {
    if !windows || Path::new(program).extension().is_some() {
        return vec![program.to_owned()];
    }
    let mut names: Vec<String> = pathext
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(DEFAULT_PATHEXT)
        .split(';')
        .map(str::trim)
        .filter(|ext| STARTABLE.contains(&ext.to_ascii_lowercase().as_str()))
        .map(|ext| format!("{program}{ext}"))
        .collect();
    names.dedup();
    names.push(program.to_owned());
    names
}

/// The one PATH lookup rule: the first match of [`which_all`].
pub fn which(program: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    which_all(program, path_var).into_iter().next()
}

/// Every match of a program on `path_var`, in the order the rule tries
/// them: each absolute folder in order (never the current folder), and in
/// each folder the names of [`file_names`].
pub fn which_all(program: &str, path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let Some(path_var) = path_var else {
        return Vec::new();
    };
    let names = file_names(program);
    let mut found = Vec::new();
    for folder in env::split_paths(path_var) {
        if !folder.is_absolute() {
            continue;
        }
        found.extend(in_folder(&folder, &names));
    }
    found
}

/// The names of `names` that are files in `folder`, in order.
pub(crate) fn in_folder(folder: &Path, names: &[String]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| folder.join(name))
        .filter(|candidate| candidate.is_file())
        .collect()
}

/// Where an install folder sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    /// `%ProgramFiles%`: `C:\Program Files`.
    ProgramFiles,
    /// `%ProgramFiles(x86)%`: `C:\Program Files (x86)`.
    ProgramFilesX86,
    /// The user's own programs folder, `%LOCALAPPDATA%\Programs`, where a
    /// per-user installer puts a program.
    LocalPrograms,
    /// Homebrew's program folders on macOS: `/opt/homebrew/bin` on Apple
    /// silicon, `/usr/local/bin` on Intel.
    Homebrew,
}

/// A folder a tool's installer uses, under a [`Base`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstallFolder {
    /// Where it sits.
    pub base: Base,
    /// The folder under the base, plain names joined by `/`; empty for the
    /// base itself.
    pub sub: &'static str,
    /// Its name in the line that says where a tool was found: "Program
    /// Files" makes "in Program Files".
    pub words: &'static str,
}

impl InstallFolder {
    /// A folder under a base.
    pub const fn new(base: Base, sub: &'static str, words: &'static str) -> Self {
        Self { base, sub, words }
    }
}

/// The folders every program tool is also looked for in, after its own:
/// Homebrew's on macOS, nothing elsewhere.
pub const SYSTEM_FOLDERS: &[InstallFolder] = if cfg!(target_os = "macos") {
    &[InstallFolder::new(Base::Homebrew, "", "Homebrew")]
} else {
    &[]
};

/// The real folders behind each [`Base`], read from the system once; a
/// test gives its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bases {
    /// `%ProgramFiles%`, when the system names an absolute one.
    pub program_files: Option<PathBuf>,
    /// `%ProgramFiles(x86)%`, likewise.
    pub program_files_x86: Option<PathBuf>,
    /// `%LOCALAPPDATA%\Programs`, likewise.
    pub local_programs: Option<PathBuf>,
    /// Homebrew's program folders, in order.
    pub homebrew: Vec<PathBuf>,
}

impl Bases {
    /// No folders at all: a registry that looks only where a test says.
    pub fn none() -> Self {
        Self::default()
    }

    /// The system's folders: on Windows from `ProgramFiles` (or
    /// `ProgramW6432`), `ProgramFiles(x86)` and `LOCALAPPDATA`, each only
    /// when it is an absolute path; on macOS Homebrew's two folders;
    /// nothing elsewhere.
    pub fn from_environment() -> Self {
        let absolute = |name: &str| {
            env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        if cfg!(windows) {
            let program_files = absolute("ProgramFiles").or_else(|| absolute("ProgramW6432"));
            let x86 =
                absolute("ProgramFiles(x86)").filter(|x86| Some(x86) != program_files.as_ref());
            Self {
                program_files,
                program_files_x86: x86,
                local_programs: absolute("LOCALAPPDATA").map(|local| local.join("Programs")),
                homebrew: Vec::new(),
            }
        } else if cfg!(target_os = "macos") {
            Self {
                homebrew: vec![
                    PathBuf::from("/opt/homebrew/bin"),
                    PathBuf::from("/usr/local/bin"),
                ],
                ..Self::default()
            }
        } else {
            Self::default()
        }
    }

    /// The real folders an install folder names, in order; none when the
    /// system has no such base.
    pub fn folders(&self, folder: &InstallFolder) -> Vec<PathBuf> {
        let bases: Vec<&PathBuf> = match folder.base {
            Base::ProgramFiles => self.program_files.iter().collect(),
            Base::ProgramFilesX86 => self.program_files_x86.iter().collect(),
            Base::LocalPrograms => self.local_programs.iter().collect(),
            Base::Homebrew => self.homebrew.iter().collect(),
        };
        bases
            .into_iter()
            .map(|base| {
                folder
                    .sub
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .fold(base.clone(), |path, part| path.join(part))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn pathext_comes_first_and_the_bare_name_last_on_windows() {
        assert_eq!(
            file_names_with("magick", Some(".COM;.EXE;.BAT;.CMD;.VBS;.JS"), true),
            [
                "magick.COM",
                "magick.EXE",
                "magick.BAT",
                "magick.CMD",
                "magick"
            ]
        );
        assert_eq!(
            file_names_with("7z", None, true),
            ["7z.EXE", "7z.COM", "7z.CMD", "7z.BAT", "7z"]
        );
        assert_eq!(file_names_with("7z.exe", Some(".EXE"), true), ["7z.exe"]);
        assert_eq!(file_names_with("magick", Some(".EXE"), false), ["magick"]);
    }

    #[test]
    fn which_skips_relative_folders_and_finds_every_match_in_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(first.path().join(name), b"").unwrap();
        std::fs::write(second.path().join(name), b"").unwrap();
        let path: OsString =
            env::join_paths([Path::new("relative-folder"), first.path(), second.path()]).unwrap();
        let all = which_all("tool", Some(&path));
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(all[0].starts_with(first.path()));
        assert!(all[1].starts_with(second.path()));
        assert_eq!(which("tool", Some(&path)), Some(all[0].clone()));
        assert!(which("tool", None).is_none());
    }

    #[test]
    fn install_folders_join_their_base_and_say_their_words() {
        let bases = Bases {
            program_files: Some(PathBuf::from("/pf")),
            program_files_x86: None,
            local_programs: Some(PathBuf::from("/local/Programs")),
            homebrew: vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ],
        };
        let git = InstallFolder::new(Base::ProgramFiles, "Git/cmd", "Git for Windows");
        assert_eq!(
            bases.folders(&git),
            [Path::new("/pf").join("Git").join("cmd")]
        );
        let gpg4win = InstallFolder::new(Base::ProgramFilesX86, "GnuPG/bin", "Gpg4win");
        assert!(bases.folders(&gpg4win).is_empty());
        let brew = InstallFolder::new(Base::Homebrew, "", "Homebrew");
        assert_eq!(bases.folders(&brew).len(), 2);
        assert!(Bases::none().folders(&git).is_empty());
    }
}
