//! The arguments ssh is started with (R24 section 1.4; answers 85, 94).
//!
//! Always an array, never a string and never a shell; options first,
//! then `--`, then the host, then the subsystem. The fixed options keep a
//! file transfer from doing anything else: no forwarding of any kind, no
//! terminal, no remote or local command, errors only in the log. The
//! conditional ones are decided from what `ssh -G` said, so the owner's
//! own choices stand unless they would let a changed host key through:
//!
//! - `StrictHostKeyChecking=ask` only when the owner's value is `no` or
//!   `off`, which accept changed keys; `yes`, `ask` and `accept-new` stay.
//! - `ConnectTimeout` and `ServerAliveInterval` only when the owner set
//!   none.
//! - On Windows always `ControlMaster=no` and `ControlPath=none`: the
//!   Windows build has no connection sharing and fails on a user's line.
//!   On Unix the owner's multiplexing is left alone (answer 94).
//! - `BatchMode=yes` for a background reconnect, and for an ssh too old
//!   to ask through the helper (below 8.4: keys and the agent only,
//!   answer 85), so nothing ever waits on a hidden console (spike SF1).
//!
//! Known hosts are never named here: writing them is ssh's own work.

use std::ffi::OsString;
use std::path::PathBuf;

use super::gcmd::{HostKeyChecking, Resolved};
use super::target::Target;
use super::version::SshVersion;
use crate::settings::Settings;

/// What the far end runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remote {
    /// The `sftp` subsystem (`-s ... sftp`), the usual case.
    Subsystem,
    /// An SFTP server program run as the remote command, for a server
    /// that lists no subsystem; an absolute path of plain characters.
    Program(String),
}

/// Why an SFTP server program was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The SFTP server program must be a full path of plain characters.")]
pub struct ProgramError;

impl Remote {
    /// A server program, checked: an absolute POSIX path of letters,
    /// digits, `/`, `.`, `-` and `_`.
    pub fn program(path: &str) -> Result<Self, ProgramError> {
        let plain = path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_'));
        if path.starts_with('/') && plain && !path.contains("..") {
            Ok(Remote::Program(path.to_owned()))
        } else {
            Err(ProgramError)
        }
    }
}

/// Everything the arguments depend on.
#[derive(Clone, Debug)]
pub struct SessionOptions<'a> {
    /// Where to connect.
    pub target: &'a Target,
    /// What `ssh -G` said for it.
    pub resolved: &'a Resolved,
    /// The ssh in use.
    pub version: &'a SshVersion,
    /// The owner's settings.
    pub settings: &'a Settings,
    /// Whether the owner is looking: false for a background reconnect,
    /// which must never open a dialog.
    pub interactive: bool,
    /// Whether to build the Windows arguments.
    pub windows: bool,
    /// A key file the site names (`-i`, with `IdentitiesOnly=yes`).
    pub key_file: Option<PathBuf>,
    /// Whether the site hands an unlocked key to the agent.
    pub add_keys_to_agent: bool,
    /// Where ssh's debug log goes, when the owner turned it on.
    pub debug_log: Option<PathBuf>,
    /// What the far end runs.
    pub remote: Remote,
}

fn o(args: &mut Vec<OsString>, option: impl Into<String>) {
    args.push("-o".into());
    args.push(option.into().into());
}

impl SessionOptions<'_> {
    /// Whether ssh may ask the owner anything: the owner is looking and
    /// this ssh can ask through the helper.
    pub fn may_ask(&self) -> bool {
        self.interactive && self.version.askpass_force()
    }

    /// The arguments, in order, ending `-- HOST sftp`.
    pub fn arguments(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = Vec::new();
        if let Some(file) = &self.settings.ssh_config_file {
            args.push("-F".into());
            args.push(file.into());
        }
        if self.remote == Remote::Subsystem {
            args.push("-s".into());
        }
        for flag in ["-T", "-a", "-x"] {
            args.push(flag.into());
        }
        for fixed in [
            "ClearAllForwardings=yes",
            "ForwardAgent=no",
            "ForwardX11=no",
            "PermitLocalCommand=no",
            "RequestTTY=no",
            "RemoteCommand=none",
        ] {
            o(&mut args, fixed);
        }
        match &self.debug_log {
            Some(file) => {
                o(&mut args, "LogLevel=DEBUG1");
                args.push("-E".into());
                args.push(file.into());
            }
            None => o(&mut args, "LogLevel=ERROR"),
        }
        o(
            &mut args,
            if self.may_ask() {
                "BatchMode=no"
            } else {
                "BatchMode=yes"
            },
        );
        if self.resolved.host_key_checking() == HostKeyChecking::No {
            o(&mut args, "StrictHostKeyChecking=ask");
        }
        if self.resolved.connect_timeout().is_none() {
            o(
                &mut args,
                format!("ConnectTimeout={}", self.settings.connect_timeout),
            );
        }
        if self.resolved.server_alive_interval() == 0 {
            o(
                &mut args,
                format!("ServerAliveInterval={}", self.settings.keepalive_seconds),
            );
            o(&mut args, "ServerAliveCountMax=3");
        }
        if let Some(key) = &self.key_file {
            args.push("-i".into());
            args.push(key.into());
            o(&mut args, "IdentitiesOnly=yes");
        }
        if self.add_keys_to_agent {
            o(&mut args, "AddKeysToAgent=yes");
        }
        if self.windows {
            o(&mut args, "ControlMaster=no");
            o(&mut args, "ControlPath=none");
        }
        if let Some(port) = self.target.port() {
            args.push("-p".into());
            args.push(port.to_string().into());
        }
        if let Some(user) = self.target.user() {
            args.push("-l".into());
            args.push(user.into());
        }
        args.push("--".into());
        args.push(self.target.host().into());
        match &self.remote {
            Remote::Subsystem => args.push("sftp".into()),
            Remote::Program(path) => args.push(path.into()),
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS_G: &str = include_str!("../../../../fixtures/sftp/ssh-g/windows-9.5p2-plain.txt");
    const JUMP_G: &str = include_str!("../../../../fixtures/sftp/ssh-g/jump-host.txt");

    fn text(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn version(text: &str) -> SshVersion {
        SshVersion::parse(text).unwrap_or(SshVersion {
            flavor: super::super::version::Flavor::Portable,
            major: 0,
            minor: 0,
            patch: 0,
        })
    }

    #[test]
    fn the_windows_arguments_in_full() {
        let target = Target::new("build.example", Some("ada"), Some(2222)).unwrap();
        let resolved = Resolved::parse(WINDOWS_G);
        let version = version("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2");
        let settings = Settings::default();
        let options = SessionOptions {
            target: &target,
            resolved: &resolved,
            version: &version,
            settings: &settings,
            interactive: true,
            windows: true,
            key_file: None,
            add_keys_to_agent: false,
            debug_log: None,
            remote: Remote::Subsystem,
        };
        assert_eq!(
            text(&options.arguments()).join(" "),
            "-s -T -a -x -o ClearAllForwardings=yes -o ForwardAgent=no -o ForwardX11=no \
             -o PermitLocalCommand=no -o RequestTTY=no -o RemoteCommand=none -o LogLevel=ERROR \
             -o BatchMode=no -o ConnectTimeout=20 -o ServerAliveInterval=15 \
             -o ServerAliveCountMax=3 -o ControlMaster=no -o ControlPath=none -p 2222 -l ada \
             -- build.example sftp"
        );
    }

    #[test]
    fn the_owners_choices_stand_unless_they_let_a_changed_key_through() {
        let target = Target::new("lab", None, None).unwrap();
        let resolved = Resolved::parse(JUMP_G);
        let version = version("OpenSSH_9.6p1, OpenSSL 3.0.13");
        let settings = Settings {
            ssh_config_file: Some(PathBuf::from("/tests/ssh_config")),
            ..Settings::default()
        };
        let options = SessionOptions {
            target: &target,
            resolved: &resolved,
            version: &version,
            settings: &settings,
            interactive: false,
            windows: false,
            key_file: Some(PathBuf::from("/keys/lab")),
            add_keys_to_agent: true,
            debug_log: Some(PathBuf::from("/state/ssh-debug.log")),
            remote: Remote::program("/usr/lib/openssh/sftp-server").unwrap(),
        };
        let args = text(&options.arguments());
        let joined = args.join(" ");
        assert!(
            joined.starts_with("-F /tests/ssh_config -T -a -x"),
            "{joined}"
        );
        assert!(joined.contains("-o LogLevel=DEBUG1 -E /state/ssh-debug.log"));
        // A background reconnect never asks.
        assert!(joined.contains("-o BatchMode=yes"));
        // "no" would accept a changed key: overridden.
        assert!(joined.contains("-o StrictHostKeyChecking=ask"));
        // The owner's timeout, keepalive and multiplexing stand on Unix.
        assert!(!joined.contains("ConnectTimeout"));
        assert!(!joined.contains("ServerAlive"));
        assert!(!joined.contains("ControlMaster"));
        assert!(joined.contains("-i /keys/lab -o IdentitiesOnly=yes -o AddKeysToAgent=yes"));
        assert!(!args.contains(&"-s".to_owned()));
        assert_eq!(
            &args[args.len() - 3..],
            ["--", "lab", "/usr/lib/openssh/sftp-server"]
        );
    }

    /// Answer 85: below 8.4 ssh cannot ask through the helper, so it never
    /// asks at all; nothing waits on a hidden console.
    #[test]
    fn an_old_ssh_never_asks() {
        let target = Target::new("build.example", None, None).unwrap();
        let resolved = Resolved::parse(WINDOWS_G);
        let version = version("OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2");
        let settings = Settings::default();
        let options = SessionOptions {
            target: &target,
            resolved: &resolved,
            version: &version,
            settings: &settings,
            interactive: true,
            windows: true,
            key_file: None,
            add_keys_to_agent: false,
            debug_log: None,
            remote: Remote::Subsystem,
        };
        assert!(!options.may_ask());
        assert!(text(&options.arguments()).contains(&"BatchMode=yes".to_owned()));
    }

    #[test]
    fn server_programs_are_plain_absolute_paths() {
        for bad in [
            "sftp-server",
            "/usr/lib/sftp-server; rm -rf /",
            "/usr/../bin/sh",
            "/usr/lib/sftp server",
            "-oProxyCommand=x",
            "",
        ] {
            assert_eq!(Remote::program(bad), Err(ProgramError), "{bad}");
        }
        assert!(Remote::program("/usr/libexec/sftp-server").is_ok());
    }
}
