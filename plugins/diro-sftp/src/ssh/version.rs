//! What `ssh -V` says: the flavor and the version, and what they allow
//! (R24 section 1.1; answer 85).
//!
//! The Windows build says `OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2`;
//! every other build says `OpenSSH_10.3p1, OpenSSL 3.5.6 ...` (Git for
//! Windows' MSYS build among them). Both go to standard error.

use std::cmp::Ordering;
use std::fmt;

/// Which build of OpenSSH.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flavor {
    /// Microsoft's Windows port (`OpenSSH_for_Windows_`).
    Windows,
    /// The portable release: Linux, macOS, and Git's MSYS build.
    Portable,
}

/// A version of OpenSSH: `9.5p2` is major 9, minor 5, portable patch 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SshVersion {
    /// Which build.
    pub flavor: Flavor,
    /// The major number.
    pub major: u32,
    /// The minor number.
    pub minor: u32,
    /// The portable patch level (the number after `p`), 0 when absent.
    pub patch: u32,
}

/// The first version that honors `SSH_ASKPASS_REQUIRE=force`, the only
/// road to a password with no console window (spike SF1).
pub const ASKPASS_FORCE: (u32, u32) = (8, 4);

fn number(text: &str) -> Option<(u32, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let n = text[..end].parse().ok()?;
    Some((n, &text[end..]))
}

impl SshVersion {
    /// Reads the first line of `ssh -V`; `None` when it is not OpenSSH.
    pub fn parse(output: &str) -> Option<Self> {
        let line = output.lines().map(str::trim).find(|l| !l.is_empty())?;
        let (flavor, rest) = if let Some(rest) = line.strip_prefix("OpenSSH_for_Windows_") {
            (Flavor::Windows, rest)
        } else {
            (Flavor::Portable, line.strip_prefix("OpenSSH_")?)
        };
        let (major, rest) = number(rest)?;
        let rest = rest.strip_prefix('.')?;
        let (minor, rest) = number(rest)?;
        let patch = rest
            .strip_prefix('p')
            .and_then(number)
            .map_or(0, |(n, _)| n);
        Some(Self {
            flavor,
            major,
            minor,
            patch,
        })
    }

    /// Whether this ssh can ask a password through the helper with no
    /// console (8.4 and later); below it, keys and the agent only.
    pub fn askpass_force(&self) -> bool {
        (self.major, self.minor) >= ASKPASS_FORCE
    }

    /// The version in words, flavor first: "OpenSSH for Windows 9.5",
    /// "OpenSSH 10.3".
    pub fn words(&self) -> String {
        match self.flavor {
            Flavor::Windows => format!("OpenSSH for Windows {}.{}", self.major, self.minor),
            Flavor::Portable => format!("OpenSSH {}.{}", self.major, self.minor),
        }
    }

    /// The order of choice (answer 85): the higher version wins, and on a
    /// tie the Windows build.
    pub fn preference(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.flavor, other.flavor) {
                (Flavor::Windows, Flavor::Portable) => Ordering::Greater,
                (Flavor::Portable, Flavor::Windows) => Ordering::Less,
                _ => Ordering::Equal,
            })
    }
}

impl fmt::Display for SshVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}p{}", self.words(), self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> SshVersion {
        SshVersion::parse(text).unwrap()
    }

    #[test]
    fn every_known_form_reads() {
        let windows = v("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2\r\n");
        assert_eq!(
            windows,
            SshVersion {
                flavor: Flavor::Windows,
                major: 9,
                minor: 5,
                patch: 2
            }
        );
        assert_eq!(windows.words(), "OpenSSH for Windows 9.5");
        let msys = v("OpenSSH_10.3p1, OpenSSL 3.5.6 7 Apr 2026");
        assert_eq!(
            (msys.flavor, msys.major, msys.minor),
            (Flavor::Portable, 10, 3)
        );
        assert_eq!(msys.words(), "OpenSSH 10.3");
        assert_eq!(
            v("OpenSSH_8.9p1 Ubuntu-3ubuntu0.10, OpenSSL 3.0.2").patch,
            1
        );
        assert_eq!(v("OpenSSH_for_Windows_7.7p1, LibreSSL 2.6.5").minor, 7);
        assert_eq!(v("OpenSSH_9.0, LibreSSL 3.3.6").patch, 0);
        for bad in ["", "ssh: illegal option -- V", "OpenSSH_x.y", "PuTTY 0.81"] {
            assert!(SshVersion::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn askpass_force_starts_at_8_4() {
        assert!(!v("OpenSSH_for_Windows_7.7p1, LibreSSL 2.6.5").askpass_force());
        assert!(!v("OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2").askpass_force());
        assert!(v("OpenSSH_8.4p1, OpenSSL 1.1.1").askpass_force());
        assert!(v("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2").askpass_force());
        assert!(v("OpenSSH_10.3p1, OpenSSL 3.5.6").askpass_force());
    }

    #[test]
    fn the_higher_version_wins_and_windows_on_a_tie() {
        let windows = v("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2");
        let msys = v("OpenSSH_10.3p1, OpenSSL 3.5.6");
        let same = v("OpenSSH_9.5p2, OpenSSL 3.0");
        assert_eq!(msys.preference(&windows), Ordering::Greater);
        assert_eq!(windows.preference(&same), Ordering::Greater);
        assert_eq!(same.preference(&windows), Ordering::Less);
        assert_eq!(
            v("OpenSSH_9.5p1, x").preference(&v("OpenSSH_9.5p2, x")),
            Ordering::Less
        );
    }
}
