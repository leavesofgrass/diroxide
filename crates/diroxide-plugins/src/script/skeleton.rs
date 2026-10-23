//! `diro plugin new`: a working skeleton, copied from a shipped example
//! with its id and name replaced, so the skeletons and the examples
//! cannot drift apart (R22 section 4).
//!
//! - `--language bash`: one script in the scripts folder, from the total
//!   size example: the header with a sentence for each key, the NUL read
//!   loop, a directive written with one `printf`, the exit codes, a "Try
//!   it" block and the rules.
//! - `--language python`: a folder, from the line counter (`--kind
//!   command`, the default), the changed-files lister (`--kind lister`) or
//!   the memory-files provider (`--kind provider`): `plugin.toml`,
//!   `main.py` with the helpers and every lifecycle method, a standard
//!   library `test_main.py`, and a `README.txt`.
//!
//! The name must be a plain id; an odd one is refused with its first bad
//! character spelled out. Nothing is ever overwritten. Files are UTF-8
//! without a byte order mark, with `\n` line ends on every system (a bash
//! script with `\r\n` does not run), and a bash script is made executable
//! on Linux and macOS.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::script::installed::{plain_name_problem, scripts_dir};

/// A skeleton's language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// One bash script command.
    Bash,
    /// A Python plugin folder.
    Python,
}

impl Language {
    /// The language for its word.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "bash" => Some(Language::Bash),
            "python" => Some(Language::Python),
            _ => None,
        }
    }
}

/// What a Python skeleton is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A command plugin (the line counter).
    #[default]
    Command,
    /// A lister of rows that are not files (changed files).
    Lister,
    /// A file-system provider (memory files).
    Provider,
}

impl Kind {
    /// The kind for its word.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "command" => Some(Kind::Command),
            "lister" => Some(Kind::Lister),
            "provider" => Some(Kind::Provider),
            _ => None,
        }
    }
}

/// One example the skeletons are copied from.
struct Example {
    id: &'static str,
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

const TOTAL_SIZE: Example = Example {
    id: "total-size",
    name: "Total size of selection",
    files: &[(
        "total-size.sh",
        include_str!("../../../../examples/plugins/scripts/total-size.sh"),
    )],
};

const LINE_COUNTER: Example = Example {
    id: "line-counter",
    name: "Line counter",
    files: &[
        (
            "plugin.toml",
            include_str!("../../../../examples/plugins/line-counter/plugin.toml"),
        ),
        (
            "main.py",
            include_str!("../../../../examples/plugins/line-counter/main.py"),
        ),
        (
            "test_main.py",
            include_str!("../../../../examples/plugins/line-counter/test_main.py"),
        ),
        (
            "README.txt",
            include_str!("../../../../examples/plugins/line-counter/README.txt"),
        ),
    ],
};

const CHANGED_FILES: Example = Example {
    id: "changed-files",
    name: "Changed files",
    files: &[
        (
            "plugin.toml",
            include_str!("../../../../examples/plugins/changed-files/plugin.toml"),
        ),
        (
            "main.py",
            include_str!("../../../../examples/plugins/changed-files/main.py"),
        ),
        (
            "test_main.py",
            include_str!("../../../../examples/plugins/changed-files/test_main.py"),
        ),
        (
            "README.txt",
            include_str!("../../../../examples/plugins/changed-files/README.txt"),
        ),
    ],
};

const MEMFS: Example = Example {
    id: "memfs",
    name: "Memory files (toy)",
    files: &[
        (
            "plugin.toml",
            include_str!("../../../../examples/plugins/memfs/plugin.toml"),
        ),
        (
            "main.py",
            include_str!("../../../../examples/plugins/memfs/main.py"),
        ),
        (
            "test_main.py",
            include_str!("../../../../examples/plugins/memfs/test_main.py"),
        ),
        (
            "README.txt",
            include_str!("../../../../examples/plugins/memfs/README.txt"),
        ),
    ],
};

/// Why a skeleton was not written, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkeletonError {
    /// The name is not a plain id.
    BadName {
        /// The name as typed.
        name: String,
        /// Why, with the first bad character spelled out.
        why: String,
    },
    /// A file or folder of that name is there already.
    Exists(String),
    /// Bash makes command scripts only.
    BashKind,
    /// A provider's name is also its scheme, which must start with a
    /// letter and has no `_`.
    BadScheme(String),
    /// Writing failed.
    Io(String),
}

impl SkeletonError {
    /// The sentence: "Could not make line-counter: it exists. Choose
    /// another name."
    pub fn sentence(&self) -> String {
        match self {
            SkeletonError::BadName { name, why } => format!(
                "Could not make {}: {why}. Use lowercase letters, digits, - and _.",
                diroxide_core::names::spell_out(name)
            ),
            SkeletonError::Exists(name) => {
                format!("Could not make {name}: it exists. Choose another name.")
            }
            SkeletonError::BashKind => {
                "Bash makes command scripts only. Use --language python for a lister or a provider."
                    .to_owned()
            }
            SkeletonError::BadScheme(name) => format!(
                "Could not make {name}: a provider's name is also its scheme. Start it with a letter and use - instead of _."
            ),
            SkeletonError::Io(cause) => format!("Could not make the plugin: {cause}."),
        }
    }
}

/// A skeleton, ready to write: its files, relative to where it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skeleton {
    /// The id.
    pub id: String,
    /// The name in words.
    pub name: String,
    /// The language.
    pub language: Language,
    /// The files: a path relative to the target, and the text.
    pub files: Vec<(PathBuf, String)>,
}

/// The name in words for an id: "line-counter" says "Line counter".
pub fn title_of(id: &str) -> String {
    let spaced = id.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Makes a skeleton named `name`.
pub fn generate(name: &str, language: Language, kind: Kind) -> Result<Skeleton, SkeletonError> {
    if let Some(why) = plain_name_problem(name) {
        return Err(SkeletonError::BadName {
            name: name.to_owned(),
            why,
        });
    }
    let example = match (language, kind) {
        (Language::Bash, Kind::Command) => &TOTAL_SIZE,
        (Language::Bash, _) => return Err(SkeletonError::BashKind),
        (Language::Python, Kind::Command) => &LINE_COUNTER,
        (Language::Python, Kind::Lister) => &CHANGED_FILES,
        (Language::Python, Kind::Provider) => &MEMFS,
    };
    if kind == Kind::Provider
        && (name.contains('_') || !name.starts_with(|c: char| c.is_ascii_lowercase()))
    {
        return Err(SkeletonError::BadScheme(name.to_owned()));
    }
    let title = title_of(name);
    let files = example
        .files
        .iter()
        .map(|(file, text)| {
            let text = text
                .replace("\r\n", "\n")
                .trim_start_matches('\u{FEFF}')
                .replace(example.name, &title)
                .replace(example.id, name);
            let file = if language == Language::Bash {
                PathBuf::from(format!("{name}.sh"))
            } else {
                PathBuf::from(name).join(file)
            };
            (file, text)
        })
        .collect();
    Ok(Skeleton {
        id: name.to_owned(),
        name: title,
        language,
        files,
    })
}

/// Where a skeleton goes by default: a script into the scripts folder, a
/// plugin folder into the plugins folder.
pub fn default_dir(plugins_dir: &Path, language: Language) -> PathBuf {
    match language {
        Language::Bash => scripts_dir(plugins_dir),
        Language::Python => plugins_dir.to_path_buf(),
    }
}

/// Writes a skeleton under `dir`, never over anything: the first file or
/// folder it would replace stops it before anything is written. Returns
/// what it wrote.
pub fn write(skeleton: &Skeleton, dir: &Path) -> Result<Vec<PathBuf>, SkeletonError> {
    let io = |e: io::Error| SkeletonError::Io(e.to_string());
    let top = skeleton
        .files
        .first()
        .and_then(|(p, _)| p.components().next())
        .map(|c| dir.join(c.as_os_str()))
        .ok_or_else(|| SkeletonError::Io("the skeleton is empty".to_owned()))?;
    if top.exists() || fs::symlink_metadata(&top).is_ok() {
        return Err(SkeletonError::Exists(skeleton.id.clone()));
    }
    fs::create_dir_all(dir).map_err(io)?;
    let mut written = Vec::new();
    for (relative, text) in &skeleton.files {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| io::Write::write_all(&mut file, text.as_bytes()))
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    SkeletonError::Exists(skeleton.id.clone())
                } else {
                    SkeletonError::Io(e.to_string())
                }
            })?;
        if skeleton.language == Language::Bash {
            make_executable(&path).map_err(io)?;
        }
        written.push(path);
    }
    Ok(written)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The three lines `diro plugin new` prints: what it wrote, how to check
/// it, how to try it once without enabling it.
pub fn written_lines(
    skeleton: &Skeleton,
    written: &[PathBuf],
    dir: &Path,
    in_plugins: bool,
) -> Vec<String> {
    let names: Vec<String> = written
        .iter()
        .filter_map(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    let first = match skeleton.language {
        Language::Bash => format!("Wrote {} in {}.", names.join(", "), dir.display()),
        Language::Python => format!(
            "Wrote {} with {} files: {}.",
            skeleton.id,
            names.len(),
            names.join(", ")
        ),
    };
    let target = if in_plugins {
        skeleton.id.clone()
    } else {
        match skeleton.language {
            Language::Bash => dir
                .join(format!("{}.sh", skeleton.id))
                .display()
                .to_string(),
            Language::Python => dir.join(&skeleton.id).display().to_string(),
        }
    };
    let mut lines = vec![first, format!("Check it: diro plugin check {target}")];
    if in_plugins {
        lines.push(format!(
            "Try it once: diro plugin run {} -- FILE",
            skeleton.id
        ));
    } else {
        lines.push(format!("Install it: diro plugin install {target}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Os, Severity};
    use crate::script::installed::{check, read_path};
    use crate::script::interpreter::Lookup;

    fn all() -> Vec<(Language, Kind, &'static str)> {
        vec![
            (Language::Bash, Kind::Command, "tidy-up"),
            (Language::Python, Kind::Command, "count-words"),
            (Language::Python, Kind::Lister, "my-lister"),
            (Language::Python, Kind::Provider, "my-store"),
        ]
    }

    #[test]
    fn every_skeleton_passes_check_and_keeps_its_line_ends() {
        let root = tempfile::tempdir().unwrap();
        let lookup = Lookup::system();
        for (language, kind, name) in all() {
            let skeleton = generate(name, language, kind).unwrap();
            let written = write(&skeleton, root.path()).unwrap();
            for path in &written {
                let bytes = fs::read(path).unwrap();
                assert!(!bytes.contains(&b'\r'), "{}", path.display());
                assert!(
                    !bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
                    "{}",
                    path.display()
                );
            }
            let target = match language {
                Language::Bash => root.path().join(format!("{name}.sh")),
                Language::Python => root.path().join(name),
            };
            let installed = read_path(&target, Os::current()).unwrap();
            assert_eq!(installed.id(), name);
            let problems = check(&installed, &lookup);
            let serious: Vec<String> = problems
                .iter()
                .filter(|p| p.severity == Severity::Error || p.field != "file")
                .map(ToString::to_string)
                .collect();
            assert!(serious.is_empty(), "{name}: {serious:?}");
            for (_, text) in &skeleton.files {
                assert!(!text.contains("line-counter") || name == "line-counter");
            }
        }
    }

    #[test]
    fn names_become_titles_and_the_example_is_reproduced() {
        assert_eq!(title_of("line-counter"), "Line counter");
        assert_eq!(title_of("total_size"), "Total size");
        let same = generate("line-counter", Language::Python, Kind::Command).unwrap();
        for ((path, text), (file, original)) in same.files.iter().zip(LINE_COUNTER.files) {
            assert_eq!(path, &PathBuf::from("line-counter").join(file));
            assert_eq!(text, &original.replace("\r\n", "\n"));
        }
        let bash = generate("total-size", Language::Bash, Kind::Command).unwrap();
        assert!(bash.files[0].1.contains("# diro: name = \"Total size\"\n"));
        assert!(bash.files[0].1.contains("diro plugin check total-size"));
    }

    #[test]
    fn odd_names_and_kinds_are_refused_with_words() {
        let error = generate("my tool", Language::Bash, Kind::Command).unwrap_err();
        assert_eq!(
            error.sentence(),
            "Could not make my tool: U+0020 is not allowed in a name. Use lowercase letters, digits, - and _."
        );
        let hidden = generate("a\u{202e}b", Language::Python, Kind::Command).unwrap_err();
        assert!(
            hidden
                .sentence()
                .starts_with("Could not make a[U+202E]b: U+202E is not allowed")
        );
        assert_eq!(
            generate("lister", Language::Bash, Kind::Lister).unwrap_err(),
            SkeletonError::BashKind
        );
        assert_eq!(
            generate("my_store", Language::Python, Kind::Provider).unwrap_err(),
            SkeletonError::BadScheme("my_store".into())
        );
    }

    #[test]
    fn nothing_is_ever_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let skeleton = generate("line-counter", Language::Python, Kind::Command).unwrap();
        write(&skeleton, root.path()).unwrap();
        let error = write(&skeleton, root.path()).unwrap_err();
        assert_eq!(
            error.sentence(),
            "Could not make line-counter: it exists. Choose another name."
        );
        let bash = generate("tidy", Language::Bash, Kind::Command).unwrap();
        fs::write(root.path().join("tidy.sh"), b"mine").unwrap();
        assert!(matches!(
            write(&bash, root.path()),
            Err(SkeletonError::Exists(_))
        ));
        assert_eq!(fs::read(root.path().join("tidy.sh")).unwrap(), b"mine");
    }

    #[test]
    fn the_printed_lines_say_what_to_do_next() {
        let skeleton = generate("line-counter", Language::Python, Kind::Command).unwrap();
        let written: Vec<PathBuf> = skeleton.files.iter().map(|(p, _)| p.clone()).collect();
        let lines = written_lines(&skeleton, &written, Path::new("plugins"), true);
        assert_eq!(
            lines,
            [
                "Wrote line-counter with 4 files: plugin.toml, main.py, test_main.py, README.txt.",
                "Check it: diro plugin check line-counter",
                "Try it once: diro plugin run line-counter -- FILE",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_bash_skeleton_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let skeleton = generate("tidy", Language::Bash, Kind::Command).unwrap();
        let written = write(&skeleton, root.path()).unwrap();
        let mode = fs::metadata(&written[0]).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }

    #[test]
    fn the_bash_skeleton_passes_bash_n() {
        let Ok(bash) = Lookup::system().bash() else {
            eprintln!("bash was not found; this test is skipped here");
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let skeleton = generate("tidy", Language::Bash, Kind::Command).unwrap();
        let written = write(&skeleton, root.path()).unwrap();
        let (program, mut args) = bash.command_for(&written[0]);
        args.insert(args.len() - 1, "-n".into());
        let status = std::process::Command::new(program)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn the_python_skeletons_pass_their_own_tests() {
        let Ok(python) = Lookup::system().python(&[]) else {
            eprintln!("Python 3 was not found; this test is skipped here");
            return;
        };
        let root = tempfile::tempdir().unwrap();
        for (language, kind, name) in all().into_iter().skip(1) {
            let skeleton = generate(name, language, kind).unwrap();
            write(&skeleton, root.path()).unwrap();
            let folder = root.path().join(name);
            let mut args = python.args.clone();
            args.retain(|a| a != "-P");
            let output = std::process::Command::new(&python.program)
                .args(args)
                .args(["-I", "-B", "-m", "unittest", "discover", "-s"])
                .arg(&folder)
                .arg("-t")
                .arg(&folder)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
