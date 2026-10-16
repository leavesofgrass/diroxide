//! The SFTP provider plugin: places such as `sftp://user@host:22/path`,
//! with key and agent authentication, known hosts checked, one connection
//! per host kept open, listings and transfers streamed. A password is asked
//! through the frontend's prompt, never taken from a command line.
//!
//! This crate is a plugin in a workspace of its own (`plugins/diro-sftp`),
//! outside the main workspace, because russh's cryptography carries C and
//! assembly and the `diro` binary links none (plan section 5.8, item 3).
//! It reaches the app through the plugin protocol, as a separate process.
//!
//! Phase 0: the provider's shape, host settings and the known-hosts answer;
//! every provider method says "not built yet". Wave 3 (W3c) owns this
//! crate and brings in `russh` and `russh-sftp`, tested against the
//! container's OpenSSH server.

use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{AttributeChange, Capabilities, OpContext, Provider, VfsError, WriteMode};

/// The default SFTP port.
pub const DEFAULT_PORT: u16 = 22;

/// How to prove who the user is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// Ask the SSH agent.
    Agent,
    /// Use a private key file.
    Key(PathBuf),
    /// Ask for a password through the frontend's prompt.
    Password,
}

/// Where to connect and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostSettings {
    /// The host name or address.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The user name; `None` uses the local user's name.
    pub user: Option<String>,
    /// The ways to authenticate, tried in order.
    pub auth: Vec<AuthMethod>,
}

impl HostSettings {
    /// The settings a remote place implies: its host, port and user, with
    /// the agent first and a password last.
    pub fn for_place(place: &Place) -> Option<Self> {
        let remote = place.remote().filter(|r| r.scheme() == "sftp")?;
        Some(Self {
            host: remote.host().to_owned(),
            port: remote.port().unwrap_or(DEFAULT_PORT),
            user: remote.user().map(str::to_owned),
            auth: vec![AuthMethod::Agent, AuthMethod::Password],
        })
    }
}

/// A password, kept out of logs and debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a password.
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The password, for the SSH library only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

/// Asks the user for a password, through a frontend's accessible prompt.
pub trait PasswordPrompt: Send + Sync {
    /// Asks for the password of `user` at `host`; `None` when cancelled.
    fn ask(&self, host: &str, user: &str) -> Option<Secret>;
}

/// What the known-hosts file says about a host's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostKeyStatus {
    /// The key is the one recorded.
    Known,
    /// The host is not recorded; ask before trusting it.
    Unknown,
    /// The host is recorded with another key: refuse, and say so.
    Changed,
}

impl HostKeyStatus {
    /// The status in words, meaning first.
    pub const fn describe(self) -> &'static str {
        match self {
            HostKeyStatus::Known => "Known host",
            HostKeyStatus::Unknown => "Unknown host: check its key before trusting it",
            HostKeyStatus::Changed => "Host key changed: refusing to connect",
        }
    }
}

/// The SFTP provider.
#[derive(Debug, Default)]
pub struct SftpProvider {
    _private: (),
}

impl SftpProvider {
    /// The provider, with no connections yet.
    pub fn new() -> Self {
        Self::default()
    }

    fn not_built(&self) -> VfsError {
        VfsError::unsupported(self.scheme(), "connect yet")
    }
}

impl Provider for SftpProvider {
    fn scheme(&self) -> &str {
        "sftp"
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        Capabilities::read_only()
    }

    fn list(&self, _place: &Place, _ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        Err(self.not_built())
    }

    fn stat(&self, _place: &Place, _ctx: &OpContext) -> Result<Entry, VfsError> {
        Err(self.not_built())
    }

    fn read(&self, _place: &Place, _ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Err(self.not_built())
    }

    fn write(
        &self,
        _place: &Place,
        _mode: WriteMode,
        _ctx: &OpContext,
    ) -> Result<Box<dyn Write + Send>, VfsError> {
        Err(self.not_built())
    }

    fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built())
    }

    fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built())
    }

    fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built())
    }

    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<(), VfsError> {
        Err(self.not_built())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_settings_come_from_the_place() {
        let place = Place::parse("sftp://ada@example.org:2222/home/ada").unwrap();
        let settings = HostSettings::for_place(&place).unwrap();
        assert_eq!(settings.host, "example.org");
        assert_eq!(settings.port, 2222);
        assert_eq!(settings.user.as_deref(), Some("ada"));
        let default_port = Place::parse("sftp://example.org").unwrap();
        assert_eq!(
            HostSettings::for_place(&default_port).unwrap().port,
            DEFAULT_PORT
        );
        assert!(HostSettings::for_place(&Place::parse("s3://bucket/x").unwrap()).is_none());
    }

    #[test]
    fn secrets_stay_out_of_debug_output() {
        let secret = Secret::new("hunter2".to_owned());
        assert_eq!(format!("{secret:?}"), "Secret(hidden)");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn the_provider_says_it_is_not_built() {
        let place = Place::parse("sftp://example.org/x").unwrap();
        let err = SftpProvider::new()
            .list(&place, &OpContext::new())
            .unwrap_err();
        assert_eq!(err.to_string(), "the sftp provider cannot connect yet");
    }
}
