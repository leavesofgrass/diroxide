//! Package commands (alpha.7, R30 commit 19): the command that installs
//! a tool from this system's packages, said as the next step for a tool
//! that is not found: "Install: winget install JohnMacFarlane.Pandoc",
//! "Install: brew install pandoc", "Install: sudo apt install pandoc".
//!
//! The rule is textweaver's (its `package_command_for`, the owner's own
//! work, relicensed by the owner under answers 3 and 41), with winget
//! added for Windows: macOS names Homebrew; Linux reads `/etc/os-release`
//! and names apt for Debian and its children, dnf for Fedora and its
//! relatives, pacman for Arch and its children, by `ID` and `ID_LIKE`; any
//! other system names nothing, and the tool's sentence is said instead.
//! The command is for the owner to read or copy; diroxide never runs it.

/// A tool's package names, per package manager; `None` where that
/// manager has no package for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Packages {
    /// The winget id, on Windows: `JohnMacFarlane.Pandoc`.
    pub winget: Option<&'static str>,
    /// The Homebrew formula, on macOS: `pandoc`.
    pub brew: Option<&'static str>,
    /// The Debian and Ubuntu package: `pandoc`.
    pub apt: Option<&'static str>,
    /// The Fedora package: `pandoc`.
    pub dnf: Option<&'static str>,
    /// The Arch package: `pandoc-cli`.
    pub pacman: Option<&'static str>,
}

impl Packages {
    /// No package anywhere: a tool that comes with the system, or with
    /// another tool.
    pub const NONE: Packages = Packages {
        winget: None,
        brew: None,
        apt: None,
        dnf: None,
        pacman: None,
    };

    /// The same name in Homebrew, apt, dnf and pacman, and no winget id.
    pub const fn same(name: &'static str) -> Self {
        Packages {
            winget: None,
            brew: Some(name),
            apt: Some(name),
            dnf: Some(name),
            pacman: Some(name),
        }
    }
}

/// The system, as far as package managers go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Windows: winget.
    Windows,
    /// macOS: Homebrew.
    Macos,
    /// Linux: the distribution's manager, from `/etc/os-release`.
    Linux,
    /// Anything else: no command is named.
    Other,
}

impl Platform {
    /// The system diro was built for.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else {
            Platform::Other
        }
    }
}

/// The text of `/etc/os-release` on Linux; `None` elsewhere or when it
/// cannot be read.
pub fn os_release() -> Option<String> {
    if cfg!(target_os = "linux") {
        std::fs::read_to_string("/etc/os-release").ok()
    } else {
        None
    }
}

/// The package manager a Linux system uses, from its `/etc/os-release`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Apt,
    Dnf,
    Pacman,
}

/// Reads `ID` and `ID_LIKE` (each word, quotes taken off, any case) and
/// names the family: Debian or Ubuntu, then Fedora, RHEL or CentOS, then
/// Arch.
fn family(os_release: &str) -> Option<Family> {
    let mut words: Vec<String> = Vec::new();
    for line in os_release.lines() {
        let value = line
            .strip_prefix("ID=")
            .or_else(|| line.strip_prefix("ID_LIKE="));
        if let Some(value) = value {
            let value = value.trim().trim_matches(['"', '\'']);
            words.extend(value.split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    let any = |names: &[&str]| words.iter().any(|word| names.contains(&word.as_str()));
    if any(&["debian", "ubuntu"]) {
        Some(Family::Apt)
    } else if any(&["fedora", "rhel", "centos"]) {
        Some(Family::Dnf)
    } else if any(&["arch"]) {
        Some(Family::Pacman)
    } else {
        None
    }
}

/// The command that installs a tool with these packages on `platform`;
/// on Linux from the text of `/etc/os-release`. `None` when this system's
/// manager has no package for it, or the manager is not known.
pub fn package_command_for(
    packages: &Packages,
    platform: Platform,
    os_release: Option<&str>,
) -> Option<String> {
    match platform {
        Platform::Windows => packages.winget.map(|id| format!("winget install {id}")),
        Platform::Macos => packages.brew.map(|name| format!("brew install {name}")),
        Platform::Linux => match family(os_release?)? {
            Family::Apt => packages.apt.map(|name| format!("sudo apt install {name}")),
            Family::Dnf => packages.dnf.map(|name| format!("sudo dnf install {name}")),
            Family::Pacman => packages.pacman.map(|name| format!("sudo pacman -S {name}")),
        },
        Platform::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANDOC: Packages = Packages {
        winget: Some("JohnMacFarlane.Pandoc"),
        brew: Some("pandoc"),
        apt: Some("pandoc"),
        dnf: Some("pandoc"),
        pacman: Some("pandoc-cli"),
    };

    #[test]
    fn each_system_names_its_own_manager() {
        let on = |platform, os: Option<&str>| package_command_for(&PANDOC, platform, os);
        assert_eq!(
            on(Platform::Windows, None).as_deref(),
            Some("winget install JohnMacFarlane.Pandoc")
        );
        assert_eq!(
            on(Platform::Macos, None).as_deref(),
            Some("brew install pandoc")
        );
        assert_eq!(on(Platform::Other, None), None);
        assert_eq!(on(Platform::Linux, None), None);
    }

    /// The distribution decides, by ID and ID_LIKE, quoted or not.
    #[test]
    fn linux_follows_the_distribution() {
        let linux = |os: &str| package_command_for(&PANDOC, Platform::Linux, Some(os));
        assert_eq!(
            linux("PRETTY_NAME=\"Debian GNU/Linux 13\"\nID=debian\n").as_deref(),
            Some("sudo apt install pandoc")
        );
        assert_eq!(
            linux("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n").as_deref(),
            Some("sudo apt install pandoc")
        );
        assert_eq!(
            linux("ID=fedora\n").as_deref(),
            Some("sudo dnf install pandoc")
        );
        assert_eq!(
            linux("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n").as_deref(),
            Some("sudo dnf install pandoc")
        );
        assert_eq!(
            linux("ID=manjaro\nID_LIKE=arch\n").as_deref(),
            Some("sudo pacman -S pandoc-cli")
        );
        assert_eq!(linux("ID=gentoo\n"), None);
    }

    #[test]
    fn a_manager_without_the_package_names_nothing() {
        let xorriso = Packages {
            pacman: Some("libisoburn"),
            ..Packages::same("xorriso")
        };
        assert_eq!(package_command_for(&xorriso, Platform::Windows, None), None);
        assert_eq!(
            package_command_for(&xorriso, Platform::Linux, Some("ID=arch")).as_deref(),
            Some("sudo pacman -S libisoburn")
        );
        assert_eq!(
            package_command_for(&Packages::NONE, Platform::Macos, None),
            None
        );
    }
}
