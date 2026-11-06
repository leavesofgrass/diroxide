//! git's version, read once a session from `git --version` (R29 2.2): 2.30
//! is the least this plugin works with, for porcelain v2 status,
//! `--no-optional-locks`, `switch` and `restore`, `init
//! --initial-branch`, `--show-scope` and `--force-if-includes`.

use std::fmt;

/// A git version: major, minor and patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// The major number, 2.
    pub major: u32,
    /// The minor number, 54.
    pub minor: u32,
    /// The patch number, 0.
    pub patch: u32,
}

/// The least git this plugin runs: 2.30, from December 2020.
pub const MINIMUM: Version = Version {
    major: 2,
    minor: 30,
    patch: 0,
};

impl Version {
    /// Reads `git --version`'s line: "git version 2.54.0.windows.1",
    /// "git version 2.47.3", "git version 2.39.5 (Apple Git-154)". `None`
    /// for anything else.
    pub fn parse(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix("git version ")?;
        let number = rest.split_whitespace().next()?;
        let mut parts = number.split('.');
        let mut next = || -> Option<u32> {
            let part = parts.next()?;
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        };
        let major = next()?;
        let minor = next()?;
        let patch = next().unwrap_or(0);
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    /// Whether this plugin can work with it.
    pub fn is_enough(self) -> bool {
        self >= MINIMUM
    }
}

impl fmt::Display for Version {
    /// Major and minor, as the words say it: "2.20".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_system_s_line_reads() {
        let v = |major, minor, patch| Version {
            major,
            minor,
            patch,
        };
        assert_eq!(
            Version::parse("git version 2.54.0.windows.1"),
            Some(v(2, 54, 0))
        );
        assert_eq!(Version::parse("git version 2.47.3\n"), Some(v(2, 47, 3)));
        assert_eq!(
            Version::parse("git version 2.39.5 (Apple Git-154)"),
            Some(v(2, 39, 5))
        );
        assert_eq!(Version::parse("git version 2.30"), Some(v(2, 30, 0)));
        assert_eq!(Version::parse("git version 2.45.rc1"), Some(v(2, 45, 0)));
        assert_eq!(Version::parse("hg version 6"), None);
        assert_eq!(Version::parse("git version x.y"), None);
        assert_eq!(Version::parse(""), None);
    }

    #[test]
    fn the_minimum_is_2_30() {
        assert!(Version::parse("git version 2.30.0").unwrap().is_enough());
        assert!(Version::parse("git version 3.0.0").unwrap().is_enough());
        assert!(!Version::parse("git version 2.29.9").unwrap().is_enough());
        assert!(!Version::parse("git version 1.99.0").unwrap().is_enough());
        assert_eq!(MINIMUM.to_string(), "2.30");
    }
}
