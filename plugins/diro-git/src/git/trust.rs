//! The trust scan (R29 2.5; answer 123; spike GS4): before any automatic
//! `git status`, the repository's own configuration is read with `git
//! config --list -z --show-scope --includes`, which runs nothing, and
//! searched for a setting that makes git run a program. A repository that
//! arrived in a download, on a stick or in an archive carries its
//! `.git/config`, and plain `git status` runs its `core.fsmonitor` and its
//! filter drivers; this scan finds them first.
//!
//! Only the repository's own scopes count (`local` and `worktree`, with
//! their included files): the owner's global and system configuration are
//! the owner's own. A hit is said "Not trusted: its config runs a
//! program" and asked about once; the answer is kept by the repository's
//! path and the [`Scan::digest`] of what was scanned, so a changed
//! configuration asks again. An include of another file is a hit by
//! itself, since what it names may change without the configuration
//! changing.

use sha2::{Digest, Sha256};

/// The `git config` arguments the scan runs with (no `-c` of its own: the
/// command-line scope is not scanned).
pub const SCAN_ARGS: [&str; 5] = ["config", "--list", "-z", "--show-scope", "--includes"];

/// One setting that runs a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The key as git lists it: `core.fsmonitor`, `filter.lfs.clean`.
    pub key: String,
    /// Its value, cleaned of control characters.
    pub value: String,
}

impl Hit {
    /// The hit in words, the key first: "core.fsmonitor runs ./watch.sh".
    pub fn words(&self) -> String {
        let value: String = self.value.chars().filter(|c| !c.is_control()).collect();
        if value.is_empty() {
            format!("{} is set", self.key)
        } else {
            format!("{} runs {value}", self.key)
        }
    }
}

/// What the scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// The settings that run a program, in git's order.
    pub hits: Vec<Hit>,
    /// The SHA-256 of the repository's own settings as listed, in hex:
    /// what a trust answer is kept by.
    pub digest: String,
    /// Whether the configuration includes another file.
    pub includes: bool,
}

impl Scan {
    /// Whether nothing in the configuration runs a program.
    pub fn is_clean(&self) -> bool {
        self.hits.is_empty()
    }
}

fn is_false(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "no" | "off" | "0" | ""
    )
}

fn is_boolean(value: &str) -> bool {
    is_false(value)
        || matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "on" | "1"
        )
}

/// A credential helper that is a shell command (`!`) or a path, rather
/// than the name of an installed `git-credential-NAME`.
fn helper_runs_a_path(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('!') || value.contains('/') || value.contains('\\') || value.contains(':')
}

/// A hooks folder outside the repository: absolute, or climbing out with
/// `..`, or from the home folder with `~`.
fn hooks_outside(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('/')
        || value.starts_with('\\')
        || value.starts_with('~')
        || value.get(1..2) == Some(":")
        || value.split(['/', '\\']).any(|part| part == "..")
}

/// Whether a key (git lists section and name lowercased, subsections as
/// written) with its value makes git run a program.
pub fn runs_a_program(key: &str, value: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let mut parts = lower.splitn(2, '.');
    let section = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    let name = rest.rsplit('.').next().unwrap_or("");
    let with_sub = rest.contains('.');
    match (section, name, with_sub) {
        ("core", "fsmonitor", false) => !is_boolean(value),
        ("core", "hookspath", false) => hooks_outside(value),
        (
            "core",
            "sshcommand" | "pager" | "editor" | "askpass" | "gitproxy" | "alternaterefscommand",
            false,
        ) => true,
        ("sequence", "editor", false) => true,
        ("diff", "external", false) => true,
        ("diff", "textconv" | "command", true) => true,
        ("difftool" | "mergetool", "cmd", true) => true,
        ("merge", "driver", true) => true,
        ("filter", "clean" | "smudge" | "process", true) => true,
        ("alias", _, _) => value.trim_start().starts_with('!'),
        ("credential", "helper", _) => helper_runs_a_path(value),
        ("gpg", "program", _) => true,
        ("gpg", "defaultkeycommand", true) => true,
        ("remote", "uploadpack" | "receivepack", true) => true,
        ("submodule", "update", true) => value.trim_start().starts_with('!'),
        ("trailer", "command" | "cmd", true) => true,
        ("include", "path", false) => true,
        ("includeif", "path", true) => true,
        _ => false,
    }
}

/// Reads `git config --list -z --show-scope --includes`: records of a
/// scope, a NUL, a key, then a line feed and the value (no value for a
/// bare key), each ended by NUL.
pub fn scan(listing: &[u8]) -> Scan {
    let mut digest = Sha256::new();
    let mut out = Scan::default();
    let mut fields = listing.split(|b| *b == 0);
    while let Some(scope) = fields.next() {
        if scope.is_empty() {
            continue;
        }
        let Some(entry) = fields.next() else {
            break;
        };
        if scope != b"local" && scope != b"worktree" {
            continue;
        }
        digest.update(scope);
        digest.update([0]);
        digest.update(entry);
        digest.update([0]);
        let entry = String::from_utf8_lossy(entry);
        let (key, value) = entry.split_once('\n').unwrap_or((&entry, ""));
        let lower = key.to_ascii_lowercase();
        if lower == "include.path" || (lower.starts_with("includeif.") && lower.ends_with(".path"))
        {
            out.includes = true;
        }
        if runs_a_program(key, value) {
            out.hits.push(Hit {
                key: key.to_owned(),
                value: value.to_owned(),
            });
        }
    }
    out.digest = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(records: &[(&str, &str, Option<&str>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (scope, key, value) in records {
            out.extend_from_slice(scope.as_bytes());
            out.push(0);
            out.extend_from_slice(key.as_bytes());
            if let Some(value) = value {
                out.push(b'\n');
                out.extend_from_slice(value.as_bytes());
            }
            out.push(0);
        }
        out
    }

    #[test]
    fn an_ordinary_clone_is_clean() {
        let s = scan(&listing(&[
            ("global", "user.name", Some("Ada Example")),
            ("global", "core.pager", Some("less")),
            ("local", "core.repositoryformatversion", Some("0")),
            ("local", "core.filemode", Some("false")),
            ("local", "core.bare", Some("false")),
            ("local", "core.fsmonitor", Some("false")),
            (
                "local",
                "remote.origin.url",
                Some("https://example.invalid/r.git"),
            ),
            (
                "local",
                "remote.origin.fetch",
                Some("+refs/heads/*:refs/remotes/origin/*"),
            ),
            ("local", "branch.main.remote", Some("origin")),
            ("local", "alias.st", Some("status")),
            ("local", "credential.helper", Some("manager")),
            ("local", "core.hookspath", Some(".githooks")),
            ("command", "core.fsmonitor", Some("false")),
        ]));
        assert!(s.is_clean(), "{:?}", s.hits);
        assert!(!s.includes);
        assert_eq!(s.digest.len(), 64);
    }

    #[test]
    fn each_setting_that_runs_a_program_is_a_hit() {
        let hostile = [
            ("core.fsmonitor", "./watch.sh"),
            ("core.sshcommand", "ssh -o ProxyCommand=evil"),
            ("core.pager", "evil"),
            ("core.editor", "evil"),
            ("core.askpass", "evil"),
            ("core.gitproxy", "evil"),
            ("core.hookspath", "/tmp/hooks"),
            ("core.hookspath", "../outside"),
            ("core.hookspath", "C:/hooks"),
            ("sequence.editor", "evil"),
            ("diff.external", "evil"),
            ("diff.pdf.textconv", "evil"),
            ("difftool.x.cmd", "evil"),
            ("mergetool.x.cmd", "evil"),
            ("merge.ours.driver", "evil"),
            ("filter.Lfs.clean", "evil %f"),
            ("filter.x.smudge", "evil"),
            ("filter.x.process", "evil"),
            ("alias.st", "!sh -c evil"),
            ("credential.helper", "!evil"),
            ("credential.helper", "/tmp/evil"),
            ("credential.https://example.invalid.helper", "!evil"),
            ("gpg.program", "evil"),
            ("gpg.ssh.program", "evil"),
            ("gpg.ssh.defaultkeycommand", "evil"),
            ("remote.origin.uploadpack", "evil"),
            ("remote.origin.receivepack", "evil"),
            ("submodule.x.update", "!evil"),
            ("trailer.sign.command", "evil"),
            ("include.path", "extra.cfg"),
            ("includeif.onbranch:main.path", "extra.cfg"),
        ];
        for (key, value) in hostile {
            let s = scan(&listing(&[("local", key, Some(value))]));
            assert_eq!(s.hits.len(), 1, "{key} = {value}");
            let s = scan(&listing(&[("worktree", key, Some(value))]));
            assert_eq!(s.hits.len(), 1, "worktree {key} = {value}");
            // The owner's own configuration is never a hit.
            let s = scan(&listing(&[("global", key, Some(value))]));
            assert!(s.is_clean(), "global {key}");
        }
        let s = scan(&listing(&[("local", "include.path", Some("x"))]));
        assert!(s.includes);
    }

    #[test]
    fn a_changed_configuration_has_another_digest() {
        let a = scan(&listing(&[("local", "core.fsmonitor", Some("./a.sh"))]));
        let b = scan(&listing(&[("local", "core.fsmonitor", Some("./b.sh"))]));
        let a2 = scan(&listing(&[
            ("global", "user.name", Some("Ada Example")),
            ("local", "core.fsmonitor", Some("./a.sh")),
        ]));
        assert_ne!(a.digest, b.digest);
        assert_eq!(a.digest, a2.digest);
    }

    #[test]
    fn the_hit_reads_key_first_and_a_bare_key_reads_too() {
        let s = scan(&listing(&[(
            "local",
            "core.fsmonitor",
            Some("./watch\u{1b}[2J.sh"),
        )]));
        assert_eq!(s.hits[0].words(), "core.fsmonitor runs ./watch[2J.sh");
        let s = scan(&listing(&[("local", "core.pager", None)]));
        assert_eq!(s.hits[0].words(), "core.pager is set");
        // A record cut short ends the scan without a panic.
        let s = scan(b"local\0core.pager\nevil\0local");
        assert_eq!(s.hits.len(), 1);
    }
}
