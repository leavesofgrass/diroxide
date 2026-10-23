//! The hosts of the owner's ssh configuration, for the Sites list (R24
//! section 1.3; answer 94).
//!
//! **ssh resolves, the plugin displays.** This module reads the owner's
//! own configuration file and the files its `Include` lines name, only to
//! list the names on `Host` lines; everything else is ssh's (`ssh -G`,
//! when a host is opened, [`super::gcmd`]). A name with `*`, `?` or `!`
//! is a pattern, not a host, and is not listed; the words after `Match`
//! are not hosts. A name that ssh could not be given safely (one that
//! starts with a dash, or holds a space or another odd character) is
//! left out and said. `Include` paths are expanded as ssh does: `~` is
//! the home folder, a relative path is from `~/.ssh`, and `*` and `?` in
//! the last part match file names in lexical order; includes nest at
//! most [`MAX_DEPTH`] deep. The system's own file is not listed.
//!
//! Nothing here runs a program, and nothing is written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::target::check_host;

/// How deep `Include` lines nest, as ssh allows.
pub const MAX_DEPTH: usize = 16;

/// The largest configuration file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// A host named on a `Host` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigHost {
    /// The name, as ssh is given it.
    pub name: String,
    /// The file that names it.
    pub file: PathBuf,
}

impl ConfigHost {
    /// Its row in the Sites list: "nas, from ssh config".
    pub fn row(&self) -> String {
        crate::lines::config_host_row(&self.name)
    }
}

/// What reading the configuration found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostList {
    /// The hosts, in the order the files name them, each once.
    pub hosts: Vec<ConfigHost>,
    /// Names left out because ssh could not be given them safely, with
    /// the reason in words.
    pub skipped: Vec<(String, String)>,
    /// Files that an `Include` named but that could not be read.
    pub unreadable: Vec<PathBuf>,
}

/// Where the owner's configuration is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPlaces {
    /// The file to read: the `ssh_config_file` setting, or `~/.ssh/config`.
    pub file: PathBuf,
    /// The home folder, for `~`.
    pub home: PathBuf,
    /// `~/.ssh`, where relative `Include` paths start.
    pub ssh_dir: PathBuf,
}

impl ConfigPlaces {
    /// The owner's own places, or the setting's file; `None` when there is
    /// no home folder.
    pub fn for_user(setting: Option<&Path>) -> Option<Self> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .filter(|h| h.is_absolute())?;
        let ssh_dir = home.join(".ssh");
        let file = setting.map_or_else(|| ssh_dir.join("config"), Path::to_path_buf);
        Some(Self {
            file,
            home,
            ssh_dir,
        })
    }
}

/// Splits a configuration line into its keyword and its arguments, as
/// ssh reads it: `Keyword args`, `Keyword=args` or `Keyword = args`,
/// arguments separated by spaces, double quotes around one with spaces.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_ascii_lowercase();
    let rest = line[end..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    let mut args = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '#' {
            break;
        }
        let mut arg = String::new();
        if c == '"' {
            chars.next();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                arg.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                arg.push(c);
                chars.next();
            }
        }
        args.push(arg);
    }
    Some((keyword, args))
}

fn is_pattern(name: &str) -> bool {
    name.contains(['*', '?', '!'])
}

/// Whether a file name matches a pattern of `*` and `?`.
fn wildcard(pattern: &[char], name: &[char]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            wildcard(&pattern[1..], name) || (!name.is_empty() && wildcard(pattern, &name[1..]))
        }
        (Some('?'), Some(_)) => wildcard(&pattern[1..], &name[1..]),
        (Some(p), Some(n)) if p == n => wildcard(&pattern[1..], &name[1..]),
        _ => false,
    }
}

/// The files an `Include` argument names, in lexical order.
fn expand_include(arg: &str, places: &ConfigPlaces) -> Vec<PathBuf> {
    if arg.contains('%') || arg.contains("${") {
        // Tokens and variables are ssh's to expand; the list does without.
        return Vec::new();
    }
    let path = if let Some(rest) = arg.strip_prefix("~/").or_else(|| arg.strip_prefix("~\\")) {
        places.home.join(rest)
    } else {
        let path = PathBuf::from(arg);
        if path.is_absolute() {
            path
        } else {
            places.ssh_dir.join(path)
        }
    };
    let Some(pattern) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    if !pattern.contains(['*', '?']) {
        return vec![path];
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let pattern: Vec<char> = pattern.chars().collect();
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter(|e| {
                    let name: Vec<char> = e.file_name().to_string_lossy().chars().collect();
                    wildcard(&pattern, &name)
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn read_limited(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn walk(
    path: &Path,
    places: &ConfigPlaces,
    depth: usize,
    seen: &mut HashSet<String>,
    out: &mut HostList,
) {
    let Some(text) = read_limited(path) else {
        if depth > 0 {
            out.unreadable.push(path.to_path_buf());
        }
        return;
    };
    for line in text.lines() {
        let Some((keyword, args)) = split_line(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => {
                for name in args {
                    if is_pattern(&name) || seen.contains(&name) {
                        continue;
                    }
                    match check_host(&name) {
                        Ok(_) => {
                            seen.insert(name.clone());
                            out.hosts.push(ConfigHost {
                                name,
                                file: path.to_path_buf(),
                            });
                        }
                        Err(why) => {
                            seen.insert(name.clone());
                            out.skipped.push((name, why.to_string()));
                        }
                    }
                }
            }
            "include" if depth < MAX_DEPTH => {
                for arg in args {
                    for file in expand_include(&arg, places) {
                        walk(&file, places, depth + 1, seen, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The hosts of the configuration, its `Include` files with it. A
/// missing file lists nothing.
pub fn config_hosts(places: &ConfigPlaces) -> HostList {
    let mut out = HostList::default();
    walk(&places.file, places, 0, &mut HashSet::new(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_as_ssh_reads_them() {
        assert_eq!(
            split_line("  Host build nas  # two"),
            Some(("host".into(), vec!["build".into(), "nas".into()]))
        );
        assert_eq!(
            split_line("HostName=build.example"),
            Some(("hostname".into(), vec!["build.example".into()]))
        );
        assert_eq!(
            split_line("Host = \"two words\" one"),
            Some(("host".into(), vec!["two words".into(), "one".into()]))
        );
        assert_eq!(split_line("# a comment"), None);
        assert_eq!(split_line(""), None);
    }

    #[test]
    fn wildcards_match_file_names() {
        let w = |p: &str, n: &str| {
            wildcard(
                &p.chars().collect::<Vec<_>>(),
                &n.chars().collect::<Vec<_>>(),
            )
        };
        assert!(w("*.conf", "a.conf"));
        assert!(w("*", "anything"));
        assert!(w("?.conf", "b.conf"));
        assert!(!w("*.conf", "a.conf.bak"));
        assert!(!w("?.conf", "ab.conf"));
    }
}
