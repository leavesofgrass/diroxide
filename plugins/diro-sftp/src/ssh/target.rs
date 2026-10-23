//! Where to connect: a host, a user and a port, checked before any of it
//! reaches ssh (R24 section 5.4).
//!
//! A host or user string comes from a typed or pasted address, a site, or
//! the owner's configuration, and none of it is trusted. The old trick is
//! a host that starts with a dash, which a program reads as an option
//! (`-oProxyCommand=...`, Git's 2017 flaw). So a host holds only letters,
//! digits, `.`, `-` and `_`, or an IPv6 address in brackets; it never
//! starts with a dash; and it goes after `--` all the same. A user holds
//! only letters, digits, `.`, `-` and `_`. Each refusal is a sentence.

use std::fmt;

use diroxide_core::Place;

/// The longest host name DNS allows.
pub const MAX_HOST_CHARS: usize = 253;

/// The longest user name taken.
pub const MAX_USER_CHARS: usize = 64;

/// Why a host, user or port was refused, as the owner hears it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    /// No host.
    #[error("The address has no host name.")]
    NoHost,
    /// A host that starts with a dash would be read as an option.
    #[error("The host name starts with a dash.")]
    HostDash,
    /// A host with a character ssh must not get.
    #[error("The host name holds a character ssh cannot take.")]
    HostCharacter,
    /// A host longer than DNS allows.
    #[error("The host name is too long.")]
    HostLong,
    /// A user that starts with a dash.
    #[error("The user name starts with a dash.")]
    UserDash,
    /// A user with a character ssh must not get.
    #[error("The user name holds a character ssh cannot take.")]
    UserCharacter,
    /// Port 0.
    #[error("The port must be a number from 1 to 65535.")]
    Port,
    /// Not an sftp place.
    #[error("This is not an sftp place.")]
    NotSftp,
}

/// A host, a user and a port that ssh may be given.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    host: String,
    user: Option<String>,
    port: Option<u16>,
}

fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
}

/// Checks a host as an address or a configuration alias gives it; the
/// answer is the host as ssh is given it (an IPv6 address without its
/// brackets).
pub fn check_host(host: &str) -> Result<String, TargetError> {
    if host.is_empty() {
        return Err(TargetError::NoHost);
    }
    if host.chars().count() > MAX_HOST_CHARS {
        return Err(TargetError::HostLong);
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        let ipv6 = !inner.is_empty()
            && inner.contains(':')
            && inner
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
        return if ipv6 {
            Ok(inner.to_owned())
        } else {
            Err(TargetError::HostCharacter)
        };
    }
    if host.starts_with('-') {
        return Err(TargetError::HostDash);
    }
    if !host.chars().all(plain) {
        return Err(TargetError::HostCharacter);
    }
    Ok(host.to_owned())
}

/// Checks a user name.
pub fn check_user(user: &str) -> Result<(), TargetError> {
    if user.starts_with('-') {
        return Err(TargetError::UserDash);
    }
    if user.is_empty() || user.chars().count() > MAX_USER_CHARS || !user.chars().all(plain) {
        return Err(TargetError::UserCharacter);
    }
    Ok(())
}

impl Target {
    /// A target from its parts, each checked.
    pub fn new(host: &str, user: Option<&str>, port: Option<u16>) -> Result<Self, TargetError> {
        let host = check_host(host)?;
        if let Some(user) = user {
            check_user(user)?;
        }
        if port == Some(0) {
            return Err(TargetError::Port);
        }
        Ok(Self {
            host,
            user: user.map(str::to_owned),
            port,
        })
    }

    /// The target of an `sftp` place.
    pub fn from_place(place: &Place) -> Result<Self, TargetError> {
        let remote = place
            .remote()
            .filter(|r| r.scheme() == "sftp")
            .ok_or(TargetError::NotSftp)?;
        Self::new(remote.host(), remote.user(), remote.port())
    }

    /// The host as ssh is given it.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The user, when one was named.
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The port, when one was named.
    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

impl fmt::Display for Target {
    /// The target as the owner reads it: "alice@build.example",
    /// "build.example:2222", `[::1]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(user) = &self.user {
            write!(f, "{user}@")?;
        }
        if self.host.contains(':') {
            write!(f, "[{}]", self.host)?;
        } else {
            write!(f, "{}", self.host)?;
        }
        match self.port {
            Some(port) => write!(f, ":{port}"),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R25 commit 50's table of hostile host strings, refused in words
    /// before anything starts.
    #[test]
    fn hostile_hosts_are_refused_in_words() {
        for (host, error) in [
            ("", TargetError::NoHost),
            ("-oProxyCommand=calc.exe", TargetError::HostDash),
            ("-V", TargetError::HostDash),
            ("--", TargetError::HostDash),
            ("build example", TargetError::HostCharacter),
            ("build.example;calc", TargetError::HostCharacter),
            ("build.example&calc", TargetError::HostCharacter),
            ("build.example|calc", TargetError::HostCharacter),
            ("$(calc)", TargetError::HostCharacter),
            ("`calc`", TargetError::HostCharacter),
            ("alice@build.example", TargetError::HostCharacter),
            ("build.example%eth0", TargetError::HostCharacter),
            ("build\nexample", TargetError::HostCharacter),
            ("build\u{0}example", TargetError::HostCharacter),
            ("build\u{202E}elpmaxe", TargetError::HostCharacter),
            ("bu\u{0456}ld.example", TargetError::HostCharacter),
            ("[::1;calc]", TargetError::HostCharacter),
            ("[]", TargetError::HostCharacter),
            ("[-oProxyCommand=x]", TargetError::HostCharacter),
            ("C:\\Windows", TargetError::HostCharacter),
        ] {
            assert_eq!(check_host(host), Err(error.clone()), "{host:?}");
        }
        assert_eq!(check_host(&"a".repeat(254)), Err(TargetError::HostLong));
        assert_eq!(
            TargetError::HostDash.to_string(),
            "The host name starts with a dash."
        );
    }

    #[test]
    fn plain_hosts_users_and_ports_pass() {
        assert_eq!(check_host("build.example").unwrap(), "build.example");
        assert_eq!(check_host("nas").unwrap(), "nas");
        assert_eq!(check_host("10.0.0.7").unwrap(), "10.0.0.7");
        assert_eq!(check_host("[::1]").unwrap(), "::1");
        assert_eq!(check_host("[fe80::1:2]").unwrap(), "fe80::1:2");
        assert_eq!(
            check_host("host_with-dash.example").unwrap(),
            "host_with-dash.example"
        );
        let target = Target::new("build.example", Some("ada"), Some(2222)).unwrap();
        assert_eq!(target.to_string(), "ada@build.example:2222");
        assert_eq!(
            Target::new("[::1]", None, None).unwrap().to_string(),
            "[::1]"
        );
        for (user, error) in [
            ("-oProxyCommand=x", TargetError::UserDash),
            ("ada lovelace", TargetError::UserCharacter),
            ("ada@x", TargetError::UserCharacter),
            ("", TargetError::UserCharacter),
        ] {
            assert_eq!(
                Target::new("build.example", Some(user), None),
                Err(error),
                "{user:?}"
            );
        }
        assert_eq!(
            Target::new("build.example", None, Some(0)),
            Err(TargetError::Port)
        );
    }

    #[test]
    fn places_give_their_target() {
        let place = Place::parse("sftp://ada@build.example:2222/srv").unwrap();
        let target = Target::from_place(&place).unwrap();
        assert_eq!(
            (target.host(), target.user(), target.port()),
            ("build.example", Some("ada"), Some(2222))
        );
        let other = Place::parse("memfs://box/x").unwrap();
        assert_eq!(Target::from_place(&other), Err(TargetError::NotSftp));
    }
}
