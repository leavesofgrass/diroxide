//! What ssh will do for one host: `ssh -G`, run only when the owner opens
//! or connects to that host, never for every row of a list (R24 section
//! 1.3; answer 94).
//!
//! `-G` prints the configuration after evaluating `Host` and `Match`
//! blocks, tokens and `Include` files, and exits; a hand parser would get
//! those wrong. A `Match exec` line or a `ProxyCommand` is the owner's own
//! program, the owner's trust; that is why it runs only on the owner's
//! action. The plugin reads the few answers it decides on (R24 section
//! 1.4) and says the rest in words on the site's Details line.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use diroxide_tools::{ProcessSpec, Stream};

use super::target::Target;
use crate::lines;

/// How long `ssh -G` may take.
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

/// The most standard output `-G` may print, in bytes.
pub const MAX_OUTPUT_BYTES: u64 = 256 * 1024;

/// The configuration ssh will use for one host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    values: BTreeMap<String, Vec<String>>,
}

/// What `StrictHostKeyChecking` comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostKeyChecking {
    /// `yes`: unknown keys refused, never asked.
    Yes,
    /// `ask`: unknown keys asked, the default.
    Ask,
    /// `accept-new`: unknown keys added without a question; changed keys
    /// refused.
    AcceptNew,
    /// `no` or `off`: unknown keys added and changed keys allowed. The
    /// plugin never leaves this in force.
    No,
}

impl Resolved {
    /// Reads `-G`'s output: one `keyword value` a line, keywords in small
    /// letters; a keyword may come more than once (`identityfile`).
    pub fn parse(output: &str) -> Self {
        let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in output.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            values
                .entry(key.to_ascii_lowercase())
                .or_default()
                .push(value.trim().to_owned());
        }
        Self { values }
    }

    /// The first value of a keyword.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .and_then(|v| v.first())
            .map(String::as_str)
    }

    /// Every value of a keyword.
    pub fn all(&self, key: &str) -> &[String] {
        self.values.get(key).map_or(&[], Vec::as_slice)
    }

    fn present(&self, key: &str) -> Option<&str> {
        self.get(key)
            .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("none"))
    }

    /// The real host name.
    pub fn hostname(&self) -> Option<&str> {
        self.present("hostname")
    }

    /// The user ssh signs in as.
    pub fn user(&self) -> Option<&str> {
        self.present("user")
    }

    /// The port.
    pub fn port(&self) -> Option<u16> {
        self.get("port").and_then(|p| p.parse().ok())
    }

    /// The host key policy.
    pub fn host_key_checking(&self) -> HostKeyChecking {
        match self
            .get("stricthostkeychecking")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("yes" | "true") => HostKeyChecking::Yes,
            Some("accept-new") => HostKeyChecking::AcceptNew,
            Some("no" | "off" | "false") => HostKeyChecking::No,
            _ => HostKeyChecking::Ask,
        }
    }

    /// The connect timeout in seconds, when one is set.
    pub fn connect_timeout(&self) -> Option<u32> {
        self.present("connecttimeout").and_then(|v| v.parse().ok())
    }

    /// The keepalive interval in seconds; 0 when off.
    pub fn server_alive_interval(&self) -> u32 {
        self.get("serveraliveinterval")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Whether the owner's configuration shares connections.
    pub fn control_master(&self) -> bool {
        !matches!(
            self.get("controlmaster")
                .map(str::to_ascii_lowercase)
                .as_deref(),
            None | Some("no" | "false")
        )
    }

    /// The jump host, when one is set.
    pub fn proxy_jump(&self) -> Option<&str> {
        self.present("proxyjump")
    }

    /// The proxy command, when one is set.
    pub fn proxy_command(&self) -> Option<&str> {
        self.present("proxycommand")
    }

    /// The key files the configuration names, in order.
    pub fn identity_files(&self) -> &[String] {
        self.all("identityfile")
    }

    /// The Details line's words, meaning first: "alice at build.example,
    /// port 22, jump host bastion".
    pub fn details(&self) -> String {
        let mut words = format!(
            "{} at {}, port {}",
            self.user().unwrap_or("you"),
            self.hostname().unwrap_or("this host"),
            self.port().unwrap_or(22)
        );
        if let Some(jump) = self.proxy_jump() {
            words.push_str(&format!(", jump host {jump}"));
        } else if self.proxy_command().is_some() {
            words.push_str(", through a proxy command");
        }
        words
    }
}

/// Why `-G` gave no answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// ssh could not be started.
    #[error("ssh could not be started.")]
    Start,
    /// ssh did not answer in time.
    #[error("ssh did not answer in time.")]
    TimedOut,
    /// ssh refused, with its first line, cleaned.
    #[error("ssh refused the configuration: {0}")]
    Refused(String),
}

/// The arguments of `-G` for a target: options, then `--`, then the host.
pub fn arguments(target: &Target, config_file: Option<&Path>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(file) = config_file {
        args.push("-F".to_owned());
        args.push(file.display().to_string());
    }
    args.push("-G".to_owned());
    if let Some(user) = target.user() {
        args.push("-l".to_owned());
        args.push(user.to_owned());
    }
    if let Some(port) = target.port() {
        args.push("-p".to_owned());
        args.push(port.to_string());
    }
    args.push("--".to_owned());
    args.push(target.host().to_owned());
    args
}

/// Runs `ssh -G` for a target with no console window and a time limit.
pub fn resolve(
    ssh: &Path,
    target: &Target,
    config_file: Option<&PathBuf>,
) -> Result<Resolved, ResolveError> {
    let mut child = ProcessSpec::new(ssh)
        .args(arguments(target, config_file.map(PathBuf::as_path)))
        .stdin(Stream::Null)
        .stdout(Stream::Piped)
        .stderr(Stream::Piped)
        .spawn()
        .map_err(|_| ResolveError::Start)?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.by_ref().take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let out = read(
        child
            .take_stdout()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .take_stderr()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let status = child
        .wait_timeout(RESOLVE_TIMEOUT)
        .map_err(|_| ResolveError::Start)?;
    let Some(status) = status else {
        let _ = child.kill_tree();
        return Err(ResolveError::TimedOut);
    };
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if !status.success() {
        let text = String::from_utf8_lossy(&err);
        let first = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        return Err(ResolveError::Refused(lines::cleaned(first, 200)));
    }
    Ok(Resolved::parse(&String::from_utf8_lossy(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: &str = include_str!("../../../../fixtures/sftp/ssh-g/windows-9.5p2-plain.txt");
    const JUMP: &str = include_str!("../../../../fixtures/sftp/ssh-g/jump-host.txt");

    #[test]
    fn the_windows_answer_reads() {
        let r = Resolved::parse(WINDOWS);
        assert_eq!(r.hostname(), Some("build.example"));
        assert_eq!(r.user(), Some("ada"));
        assert_eq!(r.port(), Some(2222));
        assert_eq!(r.host_key_checking(), HostKeyChecking::Ask);
        assert_eq!(r.connect_timeout(), None);
        assert_eq!(r.server_alive_interval(), 0);
        assert!(!r.control_master());
        assert_eq!(r.identity_files().len(), 7);
        assert_eq!(r.proxy_jump(), None);
        assert_eq!(r.details(), "ada at build.example, port 2222");
    }

    #[test]
    fn a_jump_host_and_the_owners_choices_read() {
        let r = Resolved::parse(JUMP);
        assert_eq!(r.hostname(), Some("10.0.0.7"));
        assert_eq!(r.host_key_checking(), HostKeyChecking::No);
        assert_eq!(r.connect_timeout(), Some(5));
        assert_eq!(r.server_alive_interval(), 30);
        assert!(r.control_master());
        assert_eq!(r.identity_files(), ["~/.ssh/id_ed25519_lab"]);
        assert_eq!(
            r.details(),
            "ada at 10.0.0.7, port 22, jump host bastion.example"
        );
    }

    #[test]
    fn arguments_end_with_the_host_after_two_dashes() {
        let target = Target::new("build.example", Some("ada"), Some(2222)).unwrap();
        assert_eq!(
            arguments(&target, Some(Path::new("D:/tests/config"))),
            [
                "-F",
                "D:/tests/config",
                "-G",
                "-l",
                "ada",
                "-p",
                "2222",
                "--",
                "build.example"
            ]
        );
        assert_eq!(
            arguments(&Target::new("nas", None, None).unwrap(), None),
            ["-G", "--", "nas"]
        );
    }
}
